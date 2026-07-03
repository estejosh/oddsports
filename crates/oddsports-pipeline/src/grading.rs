//! Post-hoc grading + "The Record" reveal. PRD P0 #11.
//!
//! After games settle: grade every pick published to any paid tier, compute
//! closing-line value, and build a reveal post showing the FULL
//! Analyst/Sharp-depth content to ALL tiers (including Free).
//!
//! Hard rule: losses are NEVER omitted or edited. `INSERT OR IGNORE` on the
//! primary key makes graded rows immutable — re-runs cannot rewrite history.
//! Selective publication would destroy the record's value and is a
//! deceptive-marketing exposure.

use crate::store::load_slate;
use anyhow::Result;
use oddsports_shared::{DailySlate, PickBlock, PickTeam};
use rusqlite::{params, Connection};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradeResult {
    Win,
    Loss,
    Push,
    Void,
}

impl GradeResult {
    fn as_str(&self) -> &'static str {
        match self {
            GradeResult::Win => "win",
            GradeResult::Loss => "loss",
            GradeResult::Push => "push",
            GradeResult::Void => "void",
        }
    }
}

/// Final scores keyed by game id.
/// TODO(fable): fetch from the odds API scores endpoint (GET /sports/{key}/scores?daysFrom=1).
pub struct FinalScore {
    pub game_id: String,
    pub home_score: i32,
    pub away_score: i32,
    /// Home-perspective closing spread.
    pub closing_spread: Option<f64>,
}

pub async fn fetch_final_scores(sports: &[oddsports_shared::Sport]) -> Result<Vec<FinalScore>> {
    let (Ok(base), Ok(key)) = (std::env::var("ODDS_API_BASE"), std::env::var("ODDS_API_KEY")) else {
        tracing::warn!("ODDS_API_* not configured — cannot grade");
        return Ok(vec![]);
    };
    let client = reqwest::Client::new();
    let mut scores = Vec::new();

    for sport in sports {
        let url = format!(
            "{base}/sports/{}/scores?daysFrom=2&apiKey={key}",
            sport.odds_api_key()
        );
        let res = client.get(&url).send().await?;
        if !res.status().is_success() {
            tracing::warn!(sport = ?sport, status = %res.status(), "scores fetch skipped");
            continue;
        }
        let events: Vec<serde_json::Value> = res.json().await?;
        for ev in events {
            if !ev["completed"].as_bool().unwrap_or(false) {
                continue;
            }
            let home_team = ev["home_team"].as_str().unwrap_or_default();
            let parse_score = |team: &str| -> Option<i32> {
                ev["scores"].as_array()?.iter().find_map(|s| {
                    (s["name"].as_str() == Some(team))
                        .then(|| s["score"].as_str()?.parse().ok())
                        .flatten()
                })
            };
            let away_team = ev["away_team"].as_str().unwrap_or_default();
            let (Some(home_score), Some(away_score)) = (parse_score(home_team), parse_score(away_team)) else {
                continue;
            };
            scores.push(FinalScore {
                game_id: ev["id"].as_str().unwrap_or_default().to_string(),
                home_score,
                away_score,
                // TODO(fable): capture true closing line via a pre-game snapshot
                // just before start time; the scores endpoint doesn't carry it.
                closing_spread: None,
            });
        }
    }
    Ok(scores)
}

/// Grade a date's slate. Idempotent — already-graded picks are skipped.
pub async fn grade_slate(db: &Connection, date: &str) -> Result<usize> {
    let Some(slate) = load_slate(db, date)? else {
        tracing::warn!(date, "no slate to grade");
        return Ok(0);
    };
    let sports: Vec<oddsports_shared::Sport> = {
        let mut v: Vec<_> = slate.picks.iter().map(|p| p.sport).collect();
        v.sort_by_key(|s| s.odds_api_key());
        v.dedup();
        v
    };
    let scores: HashMap<String, FinalScore> = fetch_final_scores(&sports)
        .await?
        .into_iter()
        .map(|s| (s.game_id.clone(), s))
        .collect();

    let mut graded = 0;
    for pick in deepest_blocks(&slate) {
        let Some(score) = scores.get(&pick.game_id) else { continue }; // not settled yet

        let result = settle_spread(pick, score);
        let units = pick.model.suggested_units;
        let units_delta = match result {
            GradeResult::Win => units * 0.91, // -110 juice assumption
            GradeResult::Loss => -units,
            _ => 0.0,
        };
        // CLV from structured data — no string parsing (home-perspective line).
        let our_home_line = match pick.model.pick_team {
            PickTeam::Home => pick.model.picked_line,
            PickTeam::Away => -pick.model.picked_line,
        };
        let clv = score.closing_spread.map(|c| ((c - our_home_line) * 100.0).round() / 100.0);

        graded += db.execute(
            "INSERT OR IGNORE INTO graded_picks
               (date, game_id, side, suggested_units, confidence, result, units_delta, closing_line, clv, reveal_body, sport)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                date,
                pick.game_id,
                pick.model.side,
                units,
                pick.confidence,
                result.as_str(),
                (units_delta * 100.0).round() / 100.0,
                score.closing_spread,
                clv,
                pick.body,
                serde_json::to_string(&pick.sport)?,
            ],
        )?;
    }
    tracing::info!(date, graded, "grading complete");
    Ok(graded)
}

/// One block per game — the deepest tier depth that was published.
fn deepest_blocks(slate: &DailySlate) -> Vec<&PickBlock> {
    let mut by_game: HashMap<&str, &PickBlock> = HashMap::new();
    for p in &slate.picks {
        by_game
            .entry(p.game_id.as_str())
            .and_modify(|cur| {
                if p.min_tier > cur.min_tier {
                    *cur = p;
                }
            })
            .or_insert(p);
    }
    by_game.into_values().collect()
}

/// Settle a spread pick against the final score using structured pick data.
fn settle_spread(pick: &PickBlock, score: &FinalScore) -> GradeResult {
    let margin = (score.home_score - score.away_score) as f64;
    let covered = match pick.model.pick_team {
        PickTeam::Home => margin + pick.model.picked_line,
        PickTeam::Away => -margin + pick.model.picked_line,
    };
    if covered > 0.0 {
        GradeResult::Win
    } else if covered < 0.0 {
        GradeResult::Loss
    } else {
        GradeResult::Push
    }
}

#[derive(Debug, Default)]
pub struct RollingRecord {
    pub wins: i64,
    pub losses: i64,
    pub pushes: i64,
    pub units_net: f64,
    pub avg_clv: Option<f64>,
    pub graded: i64,
}

pub fn rolling_record(db: &Connection) -> Result<RollingRecord> {
    db.query_row(
        "SELECT
           COALESCE(SUM(result = 'win'), 0),
           COALESCE(SUM(result = 'loss'), 0),
           COALESCE(SUM(result = 'push'), 0),
           COALESCE(ROUND(SUM(units_delta), 2), 0),
           ROUND(AVG(clv), 2),
           COUNT(*)
         FROM graded_picks WHERE result != 'void'",
        [],
        |r| {
            Ok(RollingRecord {
                wins: r.get(0)?,
                losses: r.get(1)?,
                pushes: r.get(2)?,
                units_net: r.get(3)?,
                avg_clv: r.get(4)?,
                graded: r.get(5)?,
            })
        },
    )
    .map_err(Into::into)
}

/// Build the daily reveal post — full paid-tier depth, shown to everyone.
pub fn build_reveal_post(db: &Connection, date: &str) -> Result<String> {
    let mut stmt = db.prepare(
        "SELECT side, result, units_delta, clv, reveal_body
         FROM graded_picks WHERE date = ?1 ORDER BY units_delta DESC",
    )?;
    let rows: Vec<(String, String, f64, Option<f64>, String)> = stmt
        .query_map(params![date], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<std::result::Result<_, _>>()?;

    if rows.is_empty() {
        return Ok(format!("📊 **The Record — {date}**\n\nNo settled picks yet."));
    }

    let rec = rolling_record(db)?;
    let mut out = vec![format!("📊 **The Record — {date}**"), String::new()];

    for (side, result, delta, clv, _) in &rows {
        let icon = match result.as_str() {
            "win" => "✅",
            "loss" => "❌",
            _ => "➖",
        };
        let clv_str = clv.map(|c| format!(" · CLV {}{c}", if c > 0.0 { "+" } else { "" })).unwrap_or_default();
        out.push(format!(
            "{icon} {side} — {} ({}{delta}u{clv_str})",
            result.to_uppercase(),
            if *delta > 0.0 { "+" } else { "" }
        ));
    }

    out.push(String::new());
    out.push(format!(
        "**Rolling: {}-{}{}, {}{}u{}** (all picks since launch — losses included, always)",
        rec.wins,
        rec.losses,
        if rec.pushes > 0 { format!("-{}", rec.pushes) } else { String::new() },
        if rec.units_net > 0.0 { "+" } else { "" },
        rec.units_net,
        rec.avg_clv.map(|c| format!(", avg CLV {c}")).unwrap_or_default(),
    ));
    out.push(String::new());
    out.push("Yesterday's full paid-tier analysis, revealed:".into());
    for (_, _, _, _, body) in &rows {
        out.push(format!("---\n_What paid tiers saw:_\n\n{body}"));
    }

    Ok(out.join("\n"))
}
