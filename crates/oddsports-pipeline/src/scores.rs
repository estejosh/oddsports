//! Public-source final scores — sports oracle v0 (docs/SPORTS_ORACLE.md).
//! No accounts, no keys, history back years. Written to be lifted verbatim
//! into a HoneMesh `sports` node role later: fetch → normalize →
//! (team names, date) → score, nothing OddSports-specific in the surface.

use anyhow::Result;
use oddsports_shared::Sport;
use std::collections::HashMap;

/// Completed games only, keyed by lowercase (home_team, away_team). A key
/// maps to EVERY distinct final seen across the queried dates — series and
/// doubleheaders produce multiple entries, and lookup() refuses to guess
/// between them.
pub type PublicScores = HashMap<(String, String), Vec<(i32, i32)>>;

/// ESPN public scoreboard path per sport. Fight sports return None —
/// there's no spread settlement for them in v0 anyway.
fn espn_path(sport: Sport) -> Option<&'static str> {
    match sport {
        Sport::Nfl => Some("football/nfl"),
        Sport::Nba => Some("basketball/nba"),
        Sport::Mlb => Some("baseball/mlb"),
        Sport::Nhl => Some("hockey/nhl"),
        Sport::SoccerEpl => Some("soccer/eng.1"),
        Sport::SoccerUcl => Some("soccer/uefa.champions"),
        Sport::Mma | Sport::Boxing => None,
    }
}

/// Fetch completed-game finals for the given sports across the given dates
/// (`YYYY-MM-DD`; pass the slate date and the day after — late games cross
/// midnight UTC). Source failures degrade to fewer entries, never an error:
/// grading treats a missing score as "not settled yet".
pub async fn fetch_public_scores(
    client: &reqwest::Client,
    sports: &[Sport],
    dates: &[String],
) -> Result<PublicScores> {
    let mut out = PublicScores::new();
    for sport in sports {
        let Some(path) = espn_path(*sport) else { continue };
        for date in dates {
            let compact: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
            let url = format!(
                "https://site.api.espn.com/apis/site/v2/sports/{path}/scoreboard?dates={compact}"
            );
            let res = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    tracing::warn!(sport = ?sport, date, status = %r.status(), "public scores skipped");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(sport = ?sport, date, error = %e, "public scores unreachable");
                    continue;
                }
            };
            let Ok(body) = res.json::<serde_json::Value>().await else { continue };
            for ev in body["events"].as_array().unwrap_or(&vec![]) {
                let Some(comp) = ev["competitions"].as_array().and_then(|c| c.first()) else {
                    continue;
                };
                if !comp["status"]["type"]["completed"].as_bool().unwrap_or(false) {
                    continue;
                }
                let mut home: Option<(String, i32)> = None;
                let mut away: Option<(String, i32)> = None;
                for c in comp["competitors"].as_array().unwrap_or(&vec![]) {
                    let name = c["team"]["displayName"].as_str().unwrap_or_default();
                    // Score arrives as a string ("5") or occasionally a number.
                    let score = match &c["score"] {
                        serde_json::Value::String(s) => s.parse::<i32>().ok(),
                        v => v.as_i64().map(|n| n as i32),
                    };
                    let (Some(score), false) = (score, name.is_empty()) else { continue };
                    match c["homeAway"].as_str() {
                        Some("home") => home = Some((name.to_string(), score)),
                        Some("away") => away = Some((name.to_string(), score)),
                        _ => {}
                    }
                }
                if let (Some((h, hs)), Some((a, as_))) = (home, away) {
                    let entry = out.entry((h.to_lowercase(), a.to_lowercase())).or_default();
                    if !entry.contains(&(hs, as_)) {
                        entry.push((hs, as_));
                    }
                }
            }
        }
    }
    tracing::info!(entries = out.len(), "public scores fetched");
    Ok(out)
}

/// Look up a pick's matchup ("Away @ Home", odds-api display names — same
/// full names ESPN uses) in the public score set. Returns None unless the
/// match is UNAMBIGUOUS: series games and doubleheaders in the window
/// produce multiple distinct finals for the same key, and grading from a
/// guess is worse than not grading — the pick waits for a better source
/// (oracle v1 carries event start times, which disambiguates).
pub fn lookup(scores: &PublicScores, matchup: &str) -> Option<(i32, i32)> {
    let (away, home) = matchup.split_once(" @ ")?;
    match scores.get(&(home.to_lowercase(), away.to_lowercase()))?.as_slice() {
        [only] => Some(*only),
        several => {
            tracing::warn!(matchup, candidates = several.len(), "ambiguous public score — skipping");
            None
        }
    }
}
