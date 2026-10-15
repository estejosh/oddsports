//! Forecast ledger. For every game we price, store the home-win probability we
//! published BEFORE the game, then settle it against the final score. This is
//! what feeds calibration (Brier, log loss) in the reveal post, and it covers
//! every priced game, not only the ones that became picks.
//!
//! The first forecast per game is kept (`INSERT OR IGNORE`): later runs cannot
//! revise history. "Home win" means the home side wins outright, so a soccer
//! draw counts as "did not happen". Ties in other sports are left unsettled.

use crate::calibration::{self, Forecast};
use anyhow::Result;
use oddsports_shared::{Game, Sport};
use rusqlite::{params, Connection};
use std::collections::HashMap;

/// Calibration numbers are only shown once there are enough settled forecasts
/// for them to mean something.
pub const MIN_SETTLED_FOR_DISPLAY: usize = 20;

pub fn ensure_table(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS forecasts (
            game_id TEXT PRIMARY KEY,
            date TEXT NOT NULL,
            sport TEXT NOT NULL,
            matchup TEXT NOT NULL,
            home_prob REAL NOT NULL,
            home_won INTEGER,
            settled_at TEXT
        );",
    )?;
    Ok(())
}

/// Record a pre-game forecast. No-op if this game already has one.
pub fn record(db: &Connection, date: &str, game: &Game, home_prob: f64) -> Result<()> {
    ensure_table(db)?;
    db.execute(
        "INSERT OR IGNORE INTO forecasts (game_id, date, sport, matchup, home_prob)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            game.id,
            date,
            serde_json::to_string(&game.sport)?,
            format!("{} @ {}", game.away, game.home),
            home_prob
        ],
    )?;
    Ok(())
}

/// Settle every open forecast made on `date`. Idempotent. Never settles from
/// disputed data: if the odds API and the public source disagree, it waits.
pub async fn settle(db: &Connection, date: &str) -> Result<usize> {
    ensure_table(db)?;
    let pending: Vec<(String, String, String)> = {
        let mut stmt = db.prepare(
            "SELECT game_id, sport, matchup FROM forecasts WHERE date = ?1 AND home_won IS NULL",
        )?;
        let rows = stmt.query_map(params![date], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    if pending.is_empty() {
        return Ok(0);
    }
    let mut sports: Vec<Sport> = pending
        .iter()
        .filter_map(|(_, s, _)| serde_json::from_str(s).ok())
        .collect();
    sports.sort_by_key(|s| s.odds_api_key());
    sports.dedup();

    let api: HashMap<String, crate::grading::FinalScore> = crate::grading::fetch_final_scores(&sports)
        .await?
        .into_iter()
        .map(|s| (s.game_id.clone(), s))
        .collect();
    let client = reqwest::Client::new();
    let mut dates = vec![date.to_string()];
    if let Ok(d) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        dates.push((d + chrono::Duration::days(1)).format("%Y-%m-%d").to_string());
    }
    let public = crate::scores::fetch_public_scores(&client, &sports, &dates).await?;

    let mut settled = 0;
    for (game_id, _, matchup) in &pending {
        let pub_score = crate::scores::lookup(&public, matchup);
        let (h, a) = match (api.get(game_id), pub_score) {
            (Some(x), Some(p)) if (x.home_score, x.away_score) != p => continue,
            (Some(x), _) => (x.home_score, x.away_score),
            (None, Some(p)) => p,
            (None, None) => continue,
        };
        let soccer_like = matchup_is_draw_sport(db, game_id);
        if h == a && !soccer_like {
            continue; // tie in a no-draw sport: leave unsettled
        }
        settled += db.execute(
            "UPDATE forecasts SET home_won = ?1, settled_at = datetime('now')
             WHERE game_id = ?2 AND home_won IS NULL",
            params![(h > a) as i32, game_id],
        )?;
    }
    tracing::info!(date, settled, "forecasts settled");
    Ok(settled)
}

fn matchup_is_draw_sport(db: &Connection, game_id: &str) -> bool {
    db.query_row("SELECT sport FROM forecasts WHERE game_id = ?1", params![game_id], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|s| serde_json::from_str::<Sport>(&s).ok())
    .map(|s| matches!(s, Sport::SoccerEpl | Sport::SoccerUcl))
    .unwrap_or(false)
}

pub fn settled_forecasts(db: &Connection) -> Result<Vec<Forecast>> {
    ensure_table(db)?;
    let mut stmt = db.prepare("SELECT home_prob, home_won FROM forecasts WHERE home_won IS NOT NULL")?;
    let rows = stmt.query_map([], |r| {
        Ok(Forecast { prob: r.get(0)?, happened: r.get::<_, i64>(1)? != 0 })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// One line for the reveal post, or an empty string when there is nothing yet.
pub fn calibration_line(db: &Connection) -> Result<String> {
    let fs = settled_forecasts(db)?;
    if fs.is_empty() {
        return Ok(String::new());
    }
    if fs.len() < MIN_SETTLED_FOR_DISPLAY {
        return Ok(format!(
            "Calibration: building ({}/{} settled forecasts before we publish Brier and log loss).",
            fs.len(),
            MIN_SETTLED_FOR_DISPLAY
        ));
    }
    let (Some(b), Some(l)) = (calibration::brier_score(&fs), calibration::log_loss(&fs)) else {
        return Ok(String::new());
    };
    Ok(format!(
        "Calibration over {} priced games (home-win probability, every game we priced, not just picks): \
         Brier {b:.3}, log loss {l:.3}. A coin flip scores 0.250 and 0.693; lower is better.",
        fs.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    fn game(id: &str) -> Game {
        Game {
            id: id.into(),
            sport: Sport::Nba,
            home: "H".into(),
            away: "A".into(),
            starts_at: "2026-01-01T00:00:00Z".into(),
            spread_lines: vec![],
            moneyline_lines: vec![],
            total_lines: vec![],
        }
    }

    #[test]
    fn first_forecast_wins() {
        let d = db();
        record(&d, "2026-01-01", &game("g1"), 0.60).unwrap();
        record(&d, "2026-01-02", &game("g1"), 0.90).unwrap();
        let p: f64 = d.query_row("SELECT home_prob FROM forecasts", [], |r| r.get(0)).unwrap();
        assert!((p - 0.60).abs() < 1e-12);
    }

    #[test]
    fn empty_ledger_has_no_line() {
        assert_eq!(calibration_line(&db()).unwrap(), "");
    }

    #[test]
    fn small_sample_shows_progress_not_scores() {
        let d = db();
        record(&d, "2026-01-01", &game("g1"), 0.6).unwrap();
        d.execute("UPDATE forecasts SET home_won = 1", []).unwrap();
        let s = calibration_line(&d).unwrap();
        assert!(s.contains("building") && !s.contains("Brier 0"), "{s}");
    }

    #[test]
    fn enough_samples_show_scores() {
        let d = db();
        for i in 0..MIN_SETTLED_FOR_DISPLAY {
            record(&d, "2026-01-01", &game(&format!("g{i}")), 0.5).unwrap();
        }
        d.execute("UPDATE forecasts SET home_won = 1", []).unwrap();
        let s = calibration_line(&d).unwrap();
        assert!(s.contains("Brier 0.250"), "{s}");
        assert!(s.contains("log loss 0.693"), "{s}");
    }
}
