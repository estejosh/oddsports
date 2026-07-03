//! LLM prose generation with tiered model routing. PRD Section 6 #3:
//! cheap model for Free/Starter, strong model for Analyst/Sharp only.
//! The LLM receives structured factors from the deterministic model and
//! turns them into readable prose — it never invents numbers or picks.
//!
//! BTCPC inference gateway (OpenAI-compatible /v1/chat/completions) via
//! plain reqwest — no SDK needed. Point BTCPC_API_BASE at a local
//! btcpc-node (default) or the hosted gateway; fees bill in dreams to
//! the account behind BTCPC_API_KEY, not USD.

use crate::budget::TokenBudget;
use anyhow::{bail, Result};
use oddsports_shared::{lint_content, Game, ModelOutput, Tier};
use serde::Deserialize;
use serde_json::json;
use std::env;

const SYSTEM_PROMPT: &str = "You are the analysis writer for OddSports, a sports betting newsletter.\n\
You will receive structured model output (pick, edge, confidence, factors). Write prose that\n\
explains it at the requested depth. Rules:\n\
- NEVER invent statistics, numbers, or factors not present in the input.\n\
- NEVER use language implying certainty: no \"lock\", \"guaranteed\", \"can't lose\", \"sure thing\", \"free money\", \"risk-free\".\n\
- The banned words above may not appear AT ALL, even negated (\"nothing is guaranteed\" is still a violation — write \"no outcome is certain\" instead).\n\
- Always frame as analysis/opinion, acknowledge variance and risk.\n\
- Match the requested depth exactly: \"brief\" = 1-2 sentences; \"standard\" = short paragraph with the key stats; \"deep\" = full factor-by-factor breakdown with line movement context.";

#[derive(Debug)]
pub struct ProseResult {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Deserialize)]
struct ApiResponse {
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
    /// BTCPC-specific: inference fee debited from the account, in dreams.
    #[serde(default)]
    fee_dreams: u64,
}

/// Deterministic, always-compliant fallback — used on budget exhaustion
/// and when the LLM cannot produce lint-clean prose.
fn template_fallback(model_out: &ModelOutput) -> ProseResult {
    ProseResult {
        text: format!(
            "{} — edge {} pts vs market consensus. Confidence {}/5.",
            model_out.side, model_out.edge_pct, model_out.confidence
        ),
        input_tokens: 0,
        output_tokens: 0,
    }
}

pub async fn write_pick_prose(
    client: &reqwest::Client,
    game: &Game,
    model_out: &ModelOutput,
    tier: Tier,
    budget: &mut TokenBudget,
) -> Result<ProseResult> {
    if budget.exhausted() {
        // Graceful degradation: template fallback, never a skipped compliance block.
        return Ok(template_fallback(model_out));
    }

    let (depth, max_tokens) = match tier {
        Tier::Analyst | Tier::Sharp => ("deep", 800),
        Tier::Starter => ("standard", 300),
        Tier::Free => ("brief", 100),
    };
    let llm_model = if tier >= Tier::Analyst {
        env::var("MODEL_STRONG").unwrap_or_else(|_| "gemma4:26b".into())
    } else {
        env::var("MODEL_CHEAP").unwrap_or_else(|_| "gemma4:latest".into())
    };

    let api_base = env::var("BTCPC_API_BASE").unwrap_or_else(|_| "http://localhost:4242".into());
    let api_key = env::var("BTCPC_API_KEY")?;
    let user_content = format!(
        "Depth: {depth}\nGame: {} @ {} ({:?}, starts {})\nModel output:\n{}",
        game.away, game.home, game.sport, game.starts_at,
        serde_json::to_string_pretty(model_out)?
    );

    let mut totals = (0u64, 0u64);
    let mut lint_note = String::new();
    for lint_attempt in 0..2u32 {
        let body = json!({
            "model": llm_model,
            "max_tokens": max_tokens,
            "stream": false,
            "messages": [
                // Identical system prompt across all calls (PRD 6 #5).
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": format!("{user_content}{lint_note}")}
            ]
        });

        // Cold model loads surface as transient 5xx from the gateway, and a
        // busy CPU node can time out entirely — retry with backoff, then fall
        // back to the template rather than failing the whole daily run.
        let mut parsed: Option<ApiResponse> = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt as u64)).await;
            }
            let res = match client
                .post(format!("{api_base}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "BTCPC request failed — retrying");
                    continue;
                }
            };
            let status = res.status();
            if status.is_server_error() {
                tracing::warn!(%status, attempt, "BTCPC inference 5xx — retrying");
                continue;
            }
            if !status.is_success() {
                // 4xx = config problem (bad key, unknown model) — surface it loudly.
                bail!("BTCPC inference {}: {}", status, res.text().await.unwrap_or_default());
            }
            parsed = Some(res.json().await?);
            break;
        }
        let Some(parsed) = parsed else {
            tracing::warn!(game = %game.id, tier = ?tier, "inference unreachable — template fallback");
            return Ok(template_fallback(model_out));
        };
        budget.record(&llm_model, parsed.usage.prompt_tokens, parsed.usage.completion_tokens)?;
        tracing::debug!(model = %llm_model, fee_dreams = parsed.usage.fee_dreams, "inference fee");
        totals.0 += parsed.usage.prompt_tokens;
        totals.1 += parsed.usage.completion_tokens;

        let text: String = parsed
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default();
        if text.is_empty() {
            // The gateway can return 200 with empty content (e.g. model warm-up).
            tracing::warn!(model = %llm_model, lint_attempt, "empty completion — retrying");
            continue;
        }

        // Lint the LLM's output too — the system prompt forbids banned phrases but we verify.
        let violations = lint_content(&text);
        if violations.is_empty() {
            return Ok(ProseResult { text, input_tokens: totals.0, output_tokens: totals.1 });
        }
        let phrases: Vec<String> = violations.iter().map(|v| v.phrase.clone()).collect();
        tracing::warn!(model = %llm_model, lint_attempt, phrases = ?phrases, "prose failed compliance lint");
        lint_note = format!(
            "\n\nIMPORTANT: your previous draft was rejected for banned phrasing ({}). \
             Rewrite it without those words appearing anywhere, in any form, even negated.",
            phrases.join(", ")
        );
    }

    // Compliance is a hard gate; readability is not. Publish the template.
    tracing::warn!(game = %game.id, tier = ?tier, "prose fell back to template after lint retries");
    Ok(template_fallback(model_out))
}
