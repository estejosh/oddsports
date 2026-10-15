//! Stage 4 — AI factors. An LLM reads SOURCE DOCUMENTS we hand it (injury
//! reports, lineup news, ...) and proposes factors. Strict rules, enforced in
//! code rather than trusted to the prompt:
//!
//!   * a factor must cite a source URL that is one of the documents we supplied
//!     (the model cannot invent a source);
//!   * it must carry an RFC 3339 `as_of` timestamp that is not in the future;
//!   * anything unsourced, unparseable or banned-phrase-tainted is DROPPED.
//!
//! No documents (or no AI configured) means no call and no factors. The
//! model is never asked to recall facts from its own memory.
//!
//! Direction convention: relative to the HOME team (For = favours home).

use crate::ai_config::{chat_once, AiConfig};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use oddsports_shared::{lint_content, Factor, FactorDirection, Game};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceDoc {
    pub url: String,
    /// RFC 3339: when this document was current.
    pub as_of: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct RawFactor {
    name: String,
    direction: String,
    #[serde(default)]
    weight: f64,
    detail: String,
    source: String,
    #[serde(default)]
    as_of: String,
}

#[derive(Debug, Default)]
pub struct Validated {
    pub kept: Vec<Factor>,
    pub rejected: Vec<String>,
}

const MAX_DETAIL_CHARS: usize = 300;
const FUTURE_SLACK_MINUTES: i64 = 5;

/// Pull the JSON array out of a model reply (tolerates ``` fences and chatter).
fn extract_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    (end > start).then(|| &text[start..=end])
}

/// Validate a model reply against the documents it was given. Pure.
pub fn parse_and_validate(reply: &str, docs: &[SourceDoc], now: DateTime<Utc>) -> Validated {
    let mut out = Validated::default();
    let Some(arr) = extract_array(reply) else {
        out.rejected.push("no JSON array in reply".into());
        return out;
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(arr) else {
        out.rejected.push("reply is not valid JSON".into());
        return out;
    };

    for (i, item) in items.into_iter().enumerate() {
        let raw: RawFactor = match serde_json::from_value(item) {
            Ok(r) => r,
            Err(e) => {
                out.rejected.push(format!("#{i}: malformed ({e})"));
                continue;
            }
        };
        match validate_one(raw, docs, now) {
            Ok(f) => out.kept.push(f),
            Err(why) => out.rejected.push(format!("#{i}: {why}")),
        }
    }
    out
}

fn validate_one(raw: RawFactor, docs: &[SourceDoc], now: DateTime<Utc>) -> Result<Factor, String> {
    let name = raw.name.trim();
    let detail = raw.detail.trim();
    if name.is_empty() || detail.is_empty() {
        return Err("empty name or detail".into());
    }
    if detail.chars().count() > MAX_DETAIL_CHARS {
        return Err("detail too long".into());
    }
    let direction = match raw.direction.trim().to_lowercase().as_str() {
        "for" => FactorDirection::For,
        "against" => FactorDirection::Against,
        other => return Err(format!("bad direction {other:?}")),
    };
    if !raw.weight.is_finite() {
        return Err("non-finite weight".into());
    }
    let source = raw.source.trim();
    if source.is_empty() {
        return Err("no source".into());
    }
    if !docs.iter().any(|d| d.url == source) {
        return Err("source is not one of the supplied documents".into());
    }
    let as_of = raw.as_of.trim();
    if as_of.is_empty() {
        return Err("no as_of timestamp".into());
    }
    let ts = DateTime::parse_from_rfc3339(as_of)
        .map_err(|_| "as_of is not RFC 3339".to_string())?
        .with_timezone(&Utc);
    if ts > now + Duration::minutes(FUTURE_SLACK_MINUTES) {
        return Err("as_of is in the future".into());
    }
    if !lint_content(name).is_empty() || !lint_content(detail).is_empty() {
        return Err("banned phrasing".into());
    }
    Ok(Factor {
        name: name.to_string(),
        direction,
        weight: raw.weight.clamp(0.0, 1.0),
        detail: detail.to_string(),
        source: source.to_string(),
        as_of: as_of.to_string(),
    })
}

/// Optional context provider: `FACTOR_CONTEXT_DIR/<game_id>.json` holding a
/// JSON array of `SourceDoc`. A private deployment can drop retrieved news
/// there; the public repo ships no retrieval of its own.
pub fn load_context_docs(game_id: &str) -> Vec<SourceDoc> {
    let Ok(dir) = std::env::var("FACTOR_CONTEXT_DIR") else {
        return vec![];
    };
    load_context_docs_from(std::path::Path::new(&dir), game_id)
}

pub fn load_context_docs_from(dir: &std::path::Path, game_id: &str) -> Vec<SourceDoc> {
    if game_id.is_empty() || game_id.contains(['/', '\\']) || game_id.contains("..") {
        return vec![];
    }
    std::fs::read_to_string(dir.join(format!("{game_id}.json")))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

const SYSTEM: &str = "You extract factual, relevant game factors from the SOURCE DOCUMENTS provided. \
Use ONLY those documents. Never use outside knowledge and never invent a source. \
Reply with a JSON array and nothing else. Each element: \
{\"name\": short label, \"direction\": \"for\" or \"against\" (relative to the HOME team), \
\"weight\": number 0..1, \"detail\": one factual sentence, \"source\": the exact url of the document, \
\"as_of\": the document's as_of timestamp}. If no document supports a factor, reply []. \
Do not name sportsbooks or quote betting odds.";

/// Ask the model for factors from `docs`. Returns validated factors plus token usage.
pub async fn fetch_ai_factors(
    client: &reqwest::Client,
    cfg: &AiConfig,
    game: &Game,
    docs: &[SourceDoc],
) -> Result<(Validated, u64, u64)> {
    let Some(model) = cfg.model_for(true) else {
        return Ok((Validated::default(), 0, 0));
    };
    if docs.is_empty() || !cfg.configured() {
        return Ok((Validated::default(), 0, 0));
    }
    let mut user = format!("Game: {} (away) @ {} (home), {:?}, starts {}\n\n", game.away, game.home, game.sport, game.starts_at);
    for d in docs {
        user.push_str(&format!("SOURCE url={} as_of={}\n{}\n\n", d.url, d.as_of, d.text));
    }
    let (reply, tin, tout) = chat_once(client, cfg, model, SYSTEM, &user, 700).await?;
    Ok((parse_and_validate(&reply, docs, Utc::now()), tin, tout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 2, 12, 0, 0).unwrap()
    }

    fn docs() -> Vec<SourceDoc> {
        vec![SourceDoc {
            url: "https://example.org/injury-report".into(),
            as_of: "2026-10-02T09:00:00Z".into(),
            text: "Starting guard listed out.".into(),
        }]
    }

    fn one(source: &str, as_of: &str, direction: &str) -> String {
        format!(
            r#"[{{"name":"guard out","direction":"{direction}","weight":0.6,"detail":"Starting guard is out.","source":"{source}","as_of":"{as_of}"}}]"#
        )
    }

    #[test]
    fn valid_factor_is_kept() {
        let v = parse_and_validate(&one("https://example.org/injury-report", "2026-10-02T09:00:00Z", "against"), &docs(), now());
        assert_eq!(v.kept.len(), 1, "{:?}", v.rejected);
        let f = &v.kept[0];
        assert!(f.is_sourced());
        assert_eq!(f.source, "https://example.org/injury-report");
        assert!(matches!(f.direction, FactorDirection::Against));
    }

    #[test]
    fn factor_without_a_source_is_dropped() {
        let v = parse_and_validate(&one("", "2026-10-02T09:00:00Z", "for"), &docs(), now());
        assert!(v.kept.is_empty());
        assert!(v.rejected[0].contains("no source"));
    }

    #[test]
    fn factor_with_missing_source_field_is_dropped() {
        let r = r#"[{"name":"x","direction":"for","weight":0.5,"detail":"d","as_of":"2026-10-02T09:00:00Z"}]"#;
        let v = parse_and_validate(r, &docs(), now());
        assert!(v.kept.is_empty());
    }

    #[test]
    fn invented_source_not_in_supplied_docs_is_dropped() {
        let v = parse_and_validate(&one("https://made-up.example/news", "2026-10-02T09:00:00Z", "for"), &docs(), now());
        assert!(v.kept.is_empty());
        assert!(v.rejected[0].contains("not one of the supplied"));
    }

    #[test]
    fn factor_without_timestamp_is_dropped() {
        let v = parse_and_validate(&one("https://example.org/injury-report", "", "for"), &docs(), now());
        assert!(v.kept.is_empty());
        assert!(v.rejected[0].contains("as_of"));
    }

    #[test]
    fn unparseable_timestamp_is_dropped() {
        let v = parse_and_validate(&one("https://example.org/injury-report", "yesterday", "for"), &docs(), now());
        assert!(v.kept.is_empty());
    }

    #[test]
    fn future_timestamp_is_dropped() {
        let v = parse_and_validate(&one("https://example.org/injury-report", "2026-10-03T09:00:00Z", "for"), &docs(), now());
        assert!(v.kept.is_empty());
        assert!(v.rejected[0].contains("future"));
    }

    #[test]
    fn bad_direction_is_dropped() {
        let v = parse_and_validate(&one("https://example.org/injury-report", "2026-10-02T09:00:00Z", "sideways"), &docs(), now());
        assert!(v.kept.is_empty());
    }

    #[test]
    fn banned_phrasing_is_dropped() {
        let r = r#"[{"name":"lock","direction":"for","weight":0.5,"detail":"guaranteed","source":"https://example.org/injury-report","as_of":"2026-10-02T09:00:00Z"}]"#;
        assert!(parse_and_validate(r, &docs(), now()).kept.is_empty());
    }

    #[test]
    fn one_bad_factor_does_not_sink_the_good_one() {
        let r = r#"[
            {"name":"a","direction":"for","weight":0.5,"detail":"ok","source":"https://example.org/injury-report","as_of":"2026-10-02T09:00:00Z"},
            {"name":"b","direction":"for","weight":0.5,"detail":"no source here","source":"","as_of":"2026-10-02T09:00:00Z"},
            "garbage"
        ]"#;
        let v = parse_and_validate(r, &docs(), now());
        assert_eq!(v.kept.len(), 1);
        assert_eq!(v.rejected.len(), 2);
    }

    #[test]
    fn fenced_and_chatty_replies_are_tolerated() {
        let r = format!("Sure!\n```json\n{}\n```", one("https://example.org/injury-report", "2026-10-02T09:00:00Z", "for"));
        assert_eq!(parse_and_validate(&r, &docs(), now()).kept.len(), 1);
    }

    #[test]
    fn garbage_reply_yields_nothing() {
        assert!(parse_and_validate("I cannot help with that", &docs(), now()).kept.is_empty());
        assert!(parse_and_validate("[not json", &docs(), now()).kept.is_empty());
        assert!(parse_and_validate("[]", &docs(), now()).kept.is_empty());
    }

    #[test]
    fn weight_is_clamped_and_nan_rejected() {
        let heavy = r#"[{"name":"a","direction":"for","weight":7,"detail":"ok","source":"https://example.org/injury-report","as_of":"2026-10-02T09:00:00Z"}]"#;
        assert_eq!(parse_and_validate(heavy, &docs(), now()).kept[0].weight, 1.0);
        let neg = r#"[{"name":"a","direction":"for","weight":-3,"detail":"ok","source":"https://example.org/injury-report","as_of":"2026-10-02T09:00:00Z"}]"#;
        assert_eq!(parse_and_validate(neg, &docs(), now()).kept[0].weight, 0.0);
    }

    #[test]
    fn context_loader_reads_docs_and_blocks_path_tricks() {
        let d = std::env::temp_dir().join(format!("oddsports-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("g1.json"),
            r#"[{"url":"https://example.org/a","as_of":"2026-10-02T09:00:00Z","text":"t"}]"#,
        )
        .unwrap();
        assert_eq!(load_context_docs_from(&d, "g1").len(), 1);
        assert!(load_context_docs_from(&d, "missing").is_empty());
        assert!(load_context_docs_from(&d, "../g1").is_empty());
        assert!(load_context_docs_from(&d, "a/b").is_empty());
    }

    #[test]
    fn model_computed_internal_factor_counts_as_sourced_only_with_timestamp() {
        let mut f = Factor {
            name: "n".into(),
            direction: FactorDirection::For,
            weight: 1.0,
            detail: "d".into(),
            source: "internal:book_consensus".into(),
            as_of: String::new(),
        };
        assert!(!f.is_sourced());
        f.as_of = "2026-10-02T09:00:00Z".into();
        assert!(f.is_sourced());
        f.source = "internal:".into();
        assert!(!f.is_sourced());
        f.source = "ftp://nope".into();
        assert!(!f.is_sourced());
    }
}
