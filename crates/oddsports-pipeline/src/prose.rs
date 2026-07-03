//! LLM prose generation with tiered model routing. PRD Section 6 #3:
//! cheap model for Free/Starter, strong model for Analyst/Sharp only.
//! The LLM receives structured factors from the deterministic model and
//! turns them into readable prose — it never invents numbers or picks.
//!
//! Anthropic API via plain reqwest — no SDK needed.

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
    content: Vec<ContentBlock>,
    usage: Usage,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
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
        return Ok(ProseResult {
            text: format!(
                "{} — edge {} pts vs market consensus. Confidence {}/5.",
                model_out.side, model_out.edge_pct, model_out.confidence
            ),
            input_tokens: 0,
            output_tokens: 0,
        });
    }

    let (depth, max_tokens) = match tier {
        Tier::Analyst | Tier::Sharp => ("deep", 800),
        Tier::Starter => ("standard", 300),
        Tier::Free => ("brief", 100),
    };
    let llm_model = if tier >= Tier::Analyst {
        env::var("MODEL_STRONG").unwrap_or_else(|_| "claude-sonnet-5".into())
    } else {
        env::var("MODEL_CHEAP").unwrap_or_else(|_| "claude-haiku-4-5-20251001".into())
    };

    let api_key = env::var("ANTHROPIC_API_KEY")?;
    let body = json!({
        "model": llm_model,
        "max_tokens": max_tokens,
        // Shared cached system prompt across all calls (PRD 6 #5).
        "system": [{"type": "text", "text": SYSTEM_PROMPT, "cache_control": {"type": "ephemeral"}}],
        "messages": [{
            "role": "user",
            "content": format!(
                "Depth: {depth}\nGame: {} @ {} ({:?}, starts {})\nModel output:\n{}",
                game.away, game.home, game.sport, game.starts_at,
                serde_json::to_string_pretty(model_out)?
            )
        }]
    });

    let res = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?;

    if !res.status().is_success() {
        bail!("anthropic API {}: {}", res.status(), res.text().await.unwrap_or_default());
    }
    let parsed: ApiResponse = res.json().await?;
    budget.record(&llm_model, parsed.usage.input_tokens, parsed.usage.output_tokens)?;

    let text: String = parsed
        .content
        .iter()
        .filter(|b| b.kind == "text")
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Lint the LLM's output too — the system prompt forbids banned phrases but we verify.
    let violations = lint_content(&text);
    if !violations.is_empty() {
        bail!(
            "LLM prose failed compliance lint: {}",
            violations.iter().map(|v| v.phrase.clone()).collect::<Vec<_>>().join(", ")
        );
    }

    Ok(ProseResult {
        text,
        input_tokens: parsed.usage.input_tokens,
        output_tokens: parsed.usage.output_tokens,
    })
}
