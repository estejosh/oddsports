//! LLM prose generation with tiered model routing. PRD Section 6 #3:
//! cheap model for Free/Starter, strong model for Analyst/Sharp only.
//! The LLM receives structured factors from the deterministic model and
//! turns them into readable prose — it never invents numbers or picks.
//!
//! Provider-agnostic: any OpenAI-compatible chat-completions endpoint, set
//! purely through environment variables (see `ai_config`). If no endpoint is
//! configured the deterministic template is used, so the pipeline always runs.

use crate::ai_config::{AiConfig, ChatResponse};
use crate::budget::TokenBudget;
use anyhow::{bail, Result};
use oddsports_shared::{lint_content, Game, ModelOutput, Tier};
use serde_json::json;

const SYSTEM_PROMPT: &str = "You are the analysis writer for OddSports, an independent sports odds-analysis publication.\n\
You will receive structured model output (pick, edge, confidence, factors). Write prose that\n\
explains it at the requested depth. Rules:\n\
- NEVER invent statistics, numbers, or factors not present in the input.\n\
- NEVER name or mention any sportsbook, bookmaker, operator or affiliate, and never quote any individual book's odds. Refer only to \"the market\" or \"consensus\".\n\
- NEVER include links, calls to action, promotions, or suggestions to sign up or deposit anywhere.\n\
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

/// Deterministic, always-compliant fallback — used on budget exhaustion,
/// when no AI endpoint is configured, and when the LLM cannot produce
/// lint-clean prose.
pub fn template_fallback(model_out: &ModelOutput) -> ProseResult {
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

    let cfg = AiConfig::from_env();
    let (Some(url), Some(llm_model)) = (cfg.chat_url(), cfg.model_for(tier >= Tier::Analyst).map(str::to_string))
    else {
        tracing::warn!("AI_API_BASE / AI_MODEL_* not configured — using template prose");
        return Ok(template_fallback(model_out));
    };

    let (depth, max_tokens) = match tier {
        Tier::Analyst | Tier::Sharp => ("deep", 800),
        Tier::Starter => ("standard", 300),
        Tier::Free => ("brief", 100),
    };

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

        // Cold model loads surface as transient 5xx from a gateway, and a
        // busy CPU node can time out entirely — retry with backoff, then fall
        // back to the template rather than failing the whole daily run.
        let mut parsed: Option<ChatResponse> = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt as u64)).await;
            }
            let mut req = client.post(&url).json(&body);
            if let Some(k) = &cfg.key {
                req = req.header("Authorization", format!("Bearer {k}"));
            }
            let res = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "inference request failed — retrying");
                    continue;
                }
            };
            let status = res.status();
            if status.is_server_error() {
                tracing::warn!(%status, attempt, "inference 5xx — retrying");
                continue;
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                tracing::warn!(attempt, "inference rate limited — waiting 65s");
                tokio::time::sleep(std::time::Duration::from_secs(65)).await;
                continue;
            }
            if !status.is_success() {
                // Other 4xx = config problem (bad key, unknown model) — surface it loudly.
                bail!("inference endpoint {}: {}", status, res.text().await.unwrap_or_default());
            }
            parsed = Some(res.json().await?);
            break;
        }
        let Some(parsed) = parsed else {
            tracing::warn!(game = %game.id, tier = ?tier, "inference unreachable — template fallback");
            return Ok(template_fallback(model_out));
        };
        budget.record(&llm_model, parsed.usage.prompt_tokens, parsed.usage.completion_tokens)?;
        totals.0 += parsed.usage.prompt_tokens;
        totals.1 += parsed.usage.completion_tokens;

        let text: String = parsed
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default();
        if text.is_empty() {
            // Some gateways return 200 with empty content (model warm-up,
            // or a model name nothing serves).
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
    Ok(ProseResult { input_tokens: totals.0, output_tokens: totals.1, ..template_fallback(model_out) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oddsports_shared::{MarketType, PickTeam};

    #[test]
    fn system_prompt_forbids_operator_mentions_and_ctas() {
        let p = SYSTEM_PROMPT.to_lowercase();
        assert!(p.contains("never name or mention any sportsbook"));
        assert!(p.contains("calls to action"));
    }

    #[test]
    fn template_fallback_is_lint_clean_and_names_no_book() {
        let mo = ModelOutput {
            game_id: "g".into(),
            market: MarketType::Spread,
            side: "Celtics -4.5".into(),
            pick_team: PickTeam::Home,
            picked_line: -4.5,
            fair_line: -6.0,
            edge_pct: 1.5,
            confidence: 3,
            suggested_units: 1.0,
            factors: vec![],
            line_history: vec![],
        };
        let t = template_fallback(&mo);
        assert!(lint_content(&t.text).is_empty());
        assert_eq!(t.input_tokens + t.output_tokens, 0);
    }
}
