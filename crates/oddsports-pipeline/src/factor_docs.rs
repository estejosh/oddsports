//! Document source for `ai_factors`. Fetches ESPN's public injury report per
//! sport and writes one SOURCE DOCUMENT per team into `FACTOR_CONTEXT_DIR`
//! (`<game_id>.json`), which `ai_factors` then reads. The model only ever sees
//! these documents and must cite them, so every factor traces to a URL and a
//! timestamp.
//!
//! Best effort: a fetch failure logs a warning and leaves that game without
//! documents (so no factors), never an error. No-op unless
//! `FACTOR_CONTEXT_DIR` is set. Stale files are removed when a game has no
//! current injury entries, so an old report is never reused.

use crate::ai_factors::SourceDoc;
use chrono::{DateTime, Utc};
use oddsports_shared::{Game, Sport};
use serde_json::Value;
use std::collections::HashMap;

const MAX_PLAYERS_PER_TEAM: usize = 15;
const MAX_DOC_CHARS: usize = 1500;

fn injuries_url(sport: Sport) -> Option<String> {
    crate::scores::espn_path(sport)
        .map(|p| format!("https://site.api.espn.com/apis/site/v2/sports/{p}/injuries"))
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

/// One document per team of `game` that has injury entries. Pure.
pub fn build_docs(report: &Value, game: &Game, url: &str, now: DateTime<Utc>) -> Vec<SourceDoc> {
    let as_of = report["timestamp"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .filter(|t| *t <= now)
        .unwrap_or(now)
        .to_rfc3339();

    let mut docs = Vec::new();
    for team in [&game.home, &game.away] {
        let Some(entry) = report["injuries"]
            .as_array()
            .and_then(|a| a.iter().find(|t| t["displayName"].as_str().map(str::to_lowercase) == Some(team.to_lowercase())))
        else {
            continue;
        };
        let players: Vec<String> = entry["injuries"]
            .as_array()
            .map(|a| {
                a.iter()
                    .take(MAX_PLAYERS_PER_TEAM)
                    .filter_map(|i| {
                        let who = i["athlete"]["displayName"].as_str()?;
                        let status = i["status"].as_str().unwrap_or("listed");
                        let note = i["shortComment"].as_str().unwrap_or("");
                        Some(format!("{who} ({status}): {note}"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if players.is_empty() {
            continue;
        }
        let mut text = format!("{team} injury report as published by ESPN. {}", players.join(" | "));
        if text.len() > MAX_DOC_CHARS {
            let mut end = MAX_DOC_CHARS;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        docs.push(SourceDoc { url: format!("{url}#{}", slug(team)), as_of: as_of.clone(), text });
    }
    docs
}

/// Refresh `FACTOR_CONTEXT_DIR/<game_id>.json` for every game. Never fails the run.
pub async fn refresh(client: &reqwest::Client, games: &[Game]) {
    let Ok(dir) = std::env::var("FACTOR_CONTEXT_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "factor docs: cannot create context dir");
        return;
    }
    let mut reports: HashMap<String, Option<Value>> = HashMap::new();
    let now = Utc::now();
    let mut written = 0;
    for g in games {
        let Some(url) = injuries_url(g.sport) else { continue };
        if g.id.is_empty() || g.id.contains(['/', '\\']) || g.id.contains("..") {
            continue;
        }
        if !reports.contains_key(&url) {
            let fetched = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
                Ok(r) => {
                    tracing::warn!(status = %r.status(), %url, "factor docs: fetch skipped");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, %url, "factor docs: fetch failed");
                    None
                }
            };
            reports.insert(url.clone(), fetched);
        }
        let path = dir.join(format!("{}.json", g.id));
        let docs = reports
            .get(&url)
            .and_then(|r| r.as_ref())
            .map(|r| build_docs(r, g, &url, now))
            .unwrap_or_default();
        if docs.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if let Ok(json) = serde_json::to_string(&docs) {
            if std::fs::write(&path, json).is_ok() {
                written += 1;
            }
        }
    }
    tracing::info!(written, "factor docs refreshed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn game() -> Game {
        Game {
            id: "g1".into(),
            sport: Sport::Nba,
            home: "Atlanta Hawks".into(),
            away: "Boston Celtics".into(),
            starts_at: "2026-10-20T00:00:00Z".into(),
            spread_lines: vec![],
            moneyline_lines: vec![],
            total_lines: vec![],
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn report() -> Value {
        json!({
            "timestamp": "2026-10-15T09:00:00Z",
            "injuries": [
                {"displayName": "Atlanta Hawks", "injuries": [
                    {"athlete": {"displayName": "K. Gilbert"}, "status": "Out", "shortComment": "knee"}]},
                {"displayName": "Boston Celtics", "injuries": []},
                {"displayName": "Other Team", "injuries": [
                    {"athlete": {"displayName": "X"}, "status": "Out", "shortComment": "n/a"}]}
            ]
        })
    }

    #[test]
    fn builds_a_cited_doc_only_for_teams_with_entries() {
        let docs = build_docs(&report(), &game(), "https://example.test/injuries", now());
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].url, "https://example.test/injuries#atlanta-hawks");
        assert!(docs[0].text.contains("K. Gilbert (Out): knee"));
        assert_eq!(docs[0].as_of, "2026-10-15T09:00:00+00:00");
    }

    #[test]
    fn future_or_missing_timestamp_falls_back_to_now() {
        let mut r = report();
        r["timestamp"] = json!("2030-01-01T00:00:00Z");
        let docs = build_docs(&r, &game(), "u", now());
        assert_eq!(docs[0].as_of, now().to_rfc3339());
        r["timestamp"] = json!(null);
        assert_eq!(build_docs(&r, &game(), "u", now())[0].as_of, now().to_rfc3339());
    }

    #[test]
    fn output_validates_through_the_factor_gate() {
        // A factor citing the supplied doc url is kept; one citing anything else is not.
        let docs = build_docs(&report(), &game(), "https://example.test/injuries", now());
        let good = format!(
            r#"[{{"name":"Gilbert out","direction":"against","weight":0.5,"detail":"Gilbert is out.","source":"{}","as_of":"{}"}}]"#,
            docs[0].url, docs[0].as_of
        );
        let v = crate::ai_factors::parse_and_validate(&good, &docs, now());
        assert_eq!(v.kept.len(), 1, "{:?}", v.rejected);
        let bad = good.replace(&docs[0].url, "https://made-up.test/x");
        assert_eq!(crate::ai_factors::parse_and_validate(&bad, &docs, now()).kept.len(), 0);
    }
}
