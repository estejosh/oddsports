//! One-off data maintenance. Today: remove legacy sportsbook promo text
//! (referral rows, affiliation lines) from stored slates and from the stored
//! reveal bodies of graded picks.
//!
//! What it touches: free text only (pick bodies). It never changes a graded
//! pick's side, result, units, closing line or CLV, so the track record's
//! numbers stay exactly as they were. Idempotent: a second run changes nothing.

use anyhow::Result;
use oddsports_shared::{scrub_legacy_promo, DailySlate};
use rusqlite::{params, Connection};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScrubReport {
    pub slates_changed: usize,
    pub bodies_changed: usize,
    pub graded_changed: usize,
}

/// Rewrite stored promo text. With `dry_run`, counts what would change and
/// writes nothing. Runs in one transaction.
pub fn scrub_stored_promo(db: &mut Connection, dry_run: bool) -> Result<ScrubReport> {
    let tx = db.transaction()?;
    let mut report = ScrubReport::default();

    let slates: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT date, json FROM slates")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for (date, json) in slates {
        let mut slate: DailySlate = serde_json::from_str(&json)?;
        let mut changed = 0;
        for p in &mut slate.picks {
            let clean = scrub_legacy_promo(&p.body);
            if clean != p.body {
                p.body = clean;
                changed += 1;
            }
        }
        if changed > 0 {
            report.slates_changed += 1;
            report.bodies_changed += changed;
            if !dry_run {
                tx.execute(
                    "UPDATE slates SET json = ?1 WHERE date = ?2",
                    params![serde_json::to_string(&slate)?, date],
                )?;
            }
        }
    }

    let graded: Vec<(String, String, String, String)> = {
        let mut stmt = tx.prepare("SELECT date, game_id, side, reveal_body FROM graded_picks")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    for (date, game_id, side, body) in graded {
        let clean = scrub_legacy_promo(&body);
        if clean != body {
            report.graded_changed += 1;
            if !dry_run {
                tx.execute(
                    "UPDATE graded_picks SET reveal_body = ?1 WHERE date = ?2 AND game_id = ?3 AND side = ?4",
                    params![clean, date, game_id, side],
                )?;
            }
        }
    }

    if dry_run {
        tx.rollback()?;
    } else {
        tx.commit()?;
    }
    Ok(report)
}

/// Copy the database to `dest` (a consistent snapshot, safe with WAL).
pub fn backup_to(db: &Connection, dest: &str) -> Result<()> {
    db.execute("VACUUM INTO ?1", params![dest])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "**A @ B**\\n\\nprose\\n\\nBet at: [x](https://ref.example/r)\\n\\n---\\nDisclosure: OddSports and X are affiliated companies.\\n1-800-GAMBLER";

    fn setup() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE slates (date TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE graded_picks (date TEXT, game_id TEXT, side TEXT, result TEXT,
               units_delta REAL, reveal_body TEXT, PRIMARY KEY (date, game_id, side));",
        )
        .unwrap();
        let slate = format!(
            r#"{{"date":"2026-01-01","generation":{{"input_tokens":0,"output_tokens":0,"cost_usd":0.0}},"picks":[{{"game_id":"g","sport":"nba","matchup":"A @ B","min_tier":"Free","body":"{OLD}","confidence":3,"risk_warning":"","model":{{"game_id":"g","market":"spread","side":"B -3","pick_team":"home","picked_line":-3.0,"fair_line":-4.0,"edge_pct":1.0,"confidence":3,"suggested_units":1.0,"factors":[],"line_history":[]}}}}]}}"#
        );
        db.execute("INSERT INTO slates VALUES ('2026-01-01', ?1)", params![slate]).unwrap();
        db.execute(
            "INSERT INTO graded_picks VALUES ('2026-01-01','g','B -3','win',0.91, ?1)",
            params![OLD.replace("\\n", "\n")],
        )
        .unwrap();
        db
    }

    #[test]
    fn dry_run_changes_nothing() {
        let mut db = setup();
        let r = scrub_stored_promo(&mut db, true).unwrap();
        assert_eq!((r.slates_changed, r.graded_changed), (1, 1));
        let body: String = db.query_row("SELECT reveal_body FROM graded_picks", [], |r| r.get(0)).unwrap();
        assert!(body.contains("Bet at"));
    }

    #[test]
    fn scrub_cleans_text_keeps_results_and_is_idempotent() {
        let mut db = setup();
        let r = scrub_stored_promo(&mut db, false).unwrap();
        assert_eq!((r.slates_changed, r.bodies_changed, r.graded_changed), (1, 1, 1));
        let (body, result, units): (String, String, f64) = db
            .query_row("SELECT reveal_body, result, units_delta FROM graded_picks", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert!(!body.contains("Bet at") && !body.to_lowercase().contains("affiliated"));
        assert_eq!((result.as_str(), units), ("win", 0.91));
        let json: String = db.query_row("SELECT json FROM slates", [], |r| r.get(0)).unwrap();
        assert!(!json.contains("ref.example"));
        assert_eq!(scrub_stored_promo(&mut db, false).unwrap(), ScrubReport::default());
    }
}
