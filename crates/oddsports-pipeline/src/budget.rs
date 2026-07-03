//! Token budget metering. PRD Section 6 #6: hard daily USD ceiling,
//! alert at 80%, degrade gracefully at 100% (skip lowest-value games,
//! never skip compliance blocks).

use std::env;

/// $/MTok — keep in sync with https://docs.anthropic.com/en/docs/about-claude/pricing
fn pricing(model: &str) -> Option<(f64, f64)> {
    match model {
        m if m.starts_with("claude-haiku-4-5") => Some((1.0, 5.0)),
        m if m.starts_with("claude-sonnet-5") => Some((3.0, 15.0)),
        // Ollama-style "name:tag" models served via the BTCPC gateway bill
        // in dreams at the node, not USD here — the USD ceiling still guards
        // any hosted-API models in the mix.
        m if m.contains(':') => Some((0.0, 0.0)),
        _ => None,
    }
}

pub struct TokenBudget {
    spent_usd: f64,
    pub ceiling_usd: f64,
    pub alert_threshold: f64,
    alerted: bool,
}

impl TokenBudget {
    pub fn from_env() -> Self {
        Self {
            spent_usd: 0.0,
            ceiling_usd: env::var("DAILY_TOKEN_BUDGET_USD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
            alert_threshold: env::var("BUDGET_ALERT_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.8),
            alerted: false,
        }
    }

    /// Record a call's usage; returns its cost. Logs an alert once at threshold.
    pub fn record(&mut self, model: &str, input_tokens: u64, output_tokens: u64) -> anyhow::Result<f64> {
        let (inp, out) = pricing(model).ok_or_else(|| anyhow::anyhow!("unknown model pricing: {model}"))?;
        let cost = (input_tokens as f64 * inp + output_tokens as f64 * out) / 1_000_000.0;
        self.spent_usd += cost;
        if !self.alerted && self.spent_usd >= self.ceiling_usd * self.alert_threshold {
            self.alerted = true;
            tracing::warn!(
                spent = self.spent_usd,
                ceiling = self.ceiling_usd,
                "budget ALERT: {:.0}% of daily token budget spent",
                self.alert_threshold * 100.0
            );
        }
        Ok(cost)
    }

    pub fn spent(&self) -> f64 {
        self.spent_usd
    }

    pub fn exhausted(&self) -> bool {
        self.spent_usd >= self.ceiling_usd
    }
}
