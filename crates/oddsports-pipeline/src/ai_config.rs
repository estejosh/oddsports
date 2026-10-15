//! Provider-agnostic AI configuration. Any OpenAI-compatible
//! `/chat/completions` endpoint works: a local server, a hosted gateway, or a
//! commercial API. Nothing here assumes a vendor and there are no defaults for
//! base URL or model names — they come from the environment.
//!
//!   AI_API_BASE      e.g. http://localhost:11434   (a trailing /v1 is fine)
//!   AI_API_KEY       optional; local servers often need none
//!   AI_MODEL_CHEAP   Free/Starter prose
//!   AI_MODEL_STRONG  Analyst/Sharp prose and factor extraction
//!
//! Older deployments' variable names are still read as fallbacks:
//! HONE_API_BASE / BTCPC_API_BASE, HONE_API_KEY / BTCPC_API_KEY,
//! MODEL_CHEAP, MODEL_STRONG.

use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, PartialEq)]
pub struct AiConfig {
    pub base: Option<String>,
    pub key: Option<String>,
    pub model_cheap: Option<String>,
    pub model_strong: Option<String>,
}

fn first(get: &dyn Fn(&str) -> Option<String>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .filter_map(|n| get(n))
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

impl AiConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(&|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Self {
        Self {
            base: first(get, &["AI_API_BASE", "HONE_API_BASE", "BTCPC_API_BASE"]),
            key: first(get, &["AI_API_KEY", "HONE_API_KEY", "BTCPC_API_KEY"]),
            model_cheap: first(get, &["AI_MODEL_CHEAP", "MODEL_CHEAP"]),
            // Strong falls back to cheap so a one-model setup works.
            model_strong: first(get, &["AI_MODEL_STRONG", "MODEL_STRONG", "AI_MODEL_CHEAP", "MODEL_CHEAP"]),
        }
    }

    /// True when there is somewhere to send requests and a model to ask for.
    pub fn configured(&self) -> bool {
        self.base.is_some() && self.model_cheap.is_some()
    }

    pub fn chat_url(&self) -> Option<String> {
        let base = self.base.as_deref()?.trim_end_matches('/');
        Some(if base.ends_with("/v1") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        })
    }

    pub fn model_for(&self, strong: bool) -> Option<&str> {
        if strong { self.model_strong.as_deref() } else { self.model_cheap.as_deref() }
    }
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: Message,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

/// One chat-completion call, no retries. Returns (text, prompt_tokens, completion_tokens).
pub async fn chat_once(
    client: &reqwest::Client,
    cfg: &AiConfig,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<(String, u64, u64)> {
    let Some(url) = cfg.chat_url() else {
        bail!("AI_API_BASE not configured");
    };
    let mut req = client.post(url).json(&json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": false,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    }));
    if let Some(k) = &cfg.key {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let res = req.send().await?;
    let status = res.status();
    if !status.is_success() {
        bail!("AI endpoint returned {status}: {}", res.text().await.unwrap_or_default());
    }
    let parsed: ChatResponse = res.json().await?;
    let text = parsed
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .unwrap_or_default();
    Ok((text, parsed.usage.prompt_tokens, parsed.usage.completion_tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> AiConfig {
        let m: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        AiConfig::from_lookup(&move |k| m.get(k).cloned())
    }

    #[test]
    fn nothing_set_means_unconfigured_with_no_vendor_defaults() {
        let c = cfg(&[]);
        assert!(!c.configured());
        assert!(c.base.is_none() && c.model_cheap.is_none() && c.chat_url().is_none());
    }

    #[test]
    fn generic_names_configure_it() {
        let c = cfg(&[("AI_API_BASE", "http://x:1"), ("AI_MODEL_CHEAP", "small"), ("AI_MODEL_STRONG", "big")]);
        assert!(c.configured());
        assert_eq!(c.model_for(false), Some("small"));
        assert_eq!(c.model_for(true), Some("big"));
        assert_eq!(c.chat_url().unwrap(), "http://x:1/v1/chat/completions");
    }

    #[test]
    fn generic_names_win_over_legacy() {
        let c = cfg(&[("AI_API_BASE", "http://new"), ("HONE_API_BASE", "http://old"), ("AI_MODEL_CHEAP", "m")]);
        assert_eq!(c.base.as_deref(), Some("http://new"));
    }

    #[test]
    fn legacy_names_still_work() {
        let c = cfg(&[("BTCPC_API_BASE", "http://old"), ("MODEL_CHEAP", "m"), ("HONE_API_KEY", "k")]);
        assert!(c.configured());
        assert_eq!(c.key.as_deref(), Some("k"));
    }

    #[test]
    fn strong_falls_back_to_cheap() {
        let c = cfg(&[("AI_API_BASE", "http://x"), ("AI_MODEL_CHEAP", "only")]);
        assert_eq!(c.model_for(true), Some("only"));
    }

    #[test]
    fn blank_values_are_unset() {
        let c = cfg(&[("AI_API_BASE", "  "), ("AI_MODEL_CHEAP", "m")]);
        assert!(!c.configured());
    }

    #[test]
    fn url_handles_trailing_slash_and_v1() {
        assert_eq!(cfg(&[("AI_API_BASE", "http://h/")]).chat_url().unwrap(), "http://h/v1/chat/completions");
        assert_eq!(cfg(&[("AI_API_BASE", "http://h/v1")]).chat_url().unwrap(), "http://h/v1/chat/completions");
        assert_eq!(cfg(&[("AI_API_BASE", "http://h/v1/")]).chat_url().unwrap(), "http://h/v1/chat/completions");
    }

    #[test]
    fn key_is_optional() {
        let c = cfg(&[("AI_API_BASE", "http://h"), ("AI_MODEL_CHEAP", "m")]);
        assert!(c.configured() && c.key.is_none());
    }
}
