//! SQLite store — the shared cache between pipeline (writer) and bot (reader).
//! PRD Section 6 #4: bot serves from this cache, zero LLM calls in request path.

use anyhow::Result;
use oddsports_shared::{DailySlate, Subscriber, Tier};
use rusqlite::{params, Connection, OptionalExtension};
use std::env;
use std::path::Path;

pub fn open_db() -> Result<Connection> {
    let path = env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/oddsports.sqlite".into());
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = Connection::open(&path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    init_schema(&db)?;
    Ok(db)
}

/// Shared by open_db and the in-memory test harness so tests exercise the
/// exact production DDL.
fn init_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS slates (
            date TEXT PRIMARY KEY,
            json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS subscribers (
            beehiiv_id TEXT PRIMARY KEY,
            email TEXT NOT NULL UNIQUE,
            tier INTEGER NOT NULL DEFAULT 0,
            telegram_user_id INTEGER UNIQUE,
            bankroll_usd REAL,
            linked_at TEXT
        );
        CREATE TABLE IF NOT EXISTS link_tokens (
            token TEXT PRIMARY KEY,
            email TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS line_snapshots (
            game_id TEXT NOT NULL,
            sport TEXT NOT NULL,
            starts_at TEXT NOT NULL,
            book TEXT NOT NULL,
            home_line REAL NOT NULL,
            taken_at TEXT NOT NULL,
            PRIMARY KEY (game_id, book, taken_at)
        );
        CREATE INDEX IF NOT EXISTS idx_snapshots_game ON line_snapshots (game_id, taken_at);
        CREATE TABLE IF NOT EXISTS graded_picks (
            date TEXT NOT NULL,
            game_id TEXT NOT NULL,
            side TEXT NOT NULL,
            suggested_units REAL NOT NULL,
            confidence INTEGER NOT NULL,
            result TEXT NOT NULL,
            units_delta REAL NOT NULL,
            closing_line REAL,
            clv REAL,
            reveal_body TEXT NOT NULL,
            sport TEXT,
            graded_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (date, game_id, side)
        );",
    )?;
    Ok(())
}

pub fn save_slate(db: &Connection, slate: &DailySlate) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO slates (date, json) VALUES (?1, ?2)",
        params![slate.date, serde_json::to_string(slate)?],
    )?;
    Ok(())
}

/// All slate dates strictly before `before`, ascending — the grading
/// catch-up set. Grading is idempotent, so re-listing already-graded
/// dates is free.
pub fn slate_dates_before(db: &Connection, before: &str) -> Result<Vec<String>> {
    let mut stmt = db.prepare("SELECT date FROM slates WHERE date < ?1 ORDER BY date")?;
    let rows = stmt.query_map(params![before], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn load_slate(db: &Connection, date: &str) -> Result<Option<DailySlate>> {
    let json: Option<String> = db
        .query_row("SELECT json FROM slates WHERE date = ?1", params![date], |r| r.get(0))
        .optional()?;
    Ok(match json {
        Some(j) => Some(serde_json::from_str(&j)?),
        None => None,
    })
}

pub fn get_subscriber_by_telegram(db: &Connection, telegram_user_id: i64) -> Result<Option<Subscriber>> {
    db.query_row(
        "SELECT beehiiv_id, email, tier, telegram_user_id, bankroll_usd, linked_at
         FROM subscribers WHERE telegram_user_id = ?1",
        params![telegram_user_id],
        |r| {
            Ok(Subscriber {
                beehiiv_id: r.get(0)?,
                email: r.get(1)?,
                tier: Tier::from_u8(r.get::<_, u8>(2)?),
                telegram_user_id: r.get(3)?,
                bankroll_usd: r.get(4)?,
                linked_at: r.get(5)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn upsert_subscriber(db: &Connection, sub: &Subscriber) -> Result<()> {
    db.execute(
        "INSERT INTO subscribers (beehiiv_id, email, tier, telegram_user_id, bankroll_usd, linked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(beehiiv_id) DO UPDATE SET
           email = excluded.email,
           tier = excluded.tier,
           telegram_user_id = COALESCE(excluded.telegram_user_id, subscribers.telegram_user_id),
           bankroll_usd = COALESCE(excluded.bankroll_usd, subscribers.bankroll_usd)",
        params![
            sub.beehiiv_id,
            sub.email,
            sub.tier as u8,
            sub.telegram_user_id,
            sub.bankroll_usd,
            sub.linked_at
        ],
    )?;
    Ok(())
}

/// Persist one snapshot row per book's home-perspective spread. Run every
/// 15–30 min via the `snapshot` subcommand — this is the steam-detection and
/// closing-line data source. Pure data, zero AI cost.
pub fn save_line_snapshots(db: &Connection, games: &[oddsports_shared::Game]) -> Result<usize> {
    let taken_at = chrono::Utc::now().to_rfc3339();
    let mut n = 0;
    let mut stmt = db.prepare(
        "INSERT OR IGNORE INTO line_snapshots (game_id, sport, starts_at, book, home_line, taken_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for g in games {
        for l in &g.spread_lines {
            if let Some(line) = l.line {
                n += stmt.execute(params![
                    g.id,
                    serde_json::to_string(&g.sport)?,
                    g.starts_at,
                    l.book,
                    line,
                    taken_at
                ])?;
            }
        }
    }
    Ok(n)
}

/// Median home-perspective line per snapshot batch — feeds ModelOutput::line_history.
pub fn line_history(db: &Connection, game_id: &str) -> Result<Vec<oddsports_shared::LinePoint>> {
    let mut stmt = db.prepare(
        "SELECT taken_at, home_line FROM line_snapshots
         WHERE game_id = ?1 ORDER BY taken_at, home_line",
    )?;
    let rows: Vec<(String, f64)> = stmt
        .query_map(params![game_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;

    // Group by batch (same taken_at), median within each.
    let mut history = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let at = rows[i].0.clone();
        let batch: Vec<f64> = rows[i..].iter().take_while(|(t, _)| *t == at).map(|(_, l)| *l).collect();
        i += batch.len();
        let mid = batch.len() / 2;
        let median = if batch.len() % 2 == 1 { batch[mid] } else { (batch[mid - 1] + batch[mid]) / 2.0 };
        history.push(oddsports_shared::LinePoint { at, line: median });
    }
    Ok(history)
}

/// Closing line: median of the latest snapshot batch at or before game start.
/// This is what makes CLV grading honest — captured live, not reconstructed.
pub fn closing_spread(db: &Connection, game_id: &str) -> Result<Option<f64>> {
    let starts_at: Option<String> = db
        .query_row(
            "SELECT MAX(starts_at) FROM line_snapshots WHERE game_id = ?1",
            params![game_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();

    // Batches are ordered by taken_at, but "at or before start" must be an
    // INSTANT comparison: our writes end in "+00:00" while odds-API starts_at
    // ends in "Z", and '+00:00' < 'Z' lexicographically — a string compare
    // classified post-kickoff snapshots as pre-start (S-04). Fall back to the
    // legacy byte compare only when a timestamp doesn't parse.
    let start_instant = starts_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let at_or_before_start = |taken_at: &str| -> bool {
        match (
            chrono::DateTime::parse_from_rfc3339(taken_at).ok(),
            start_instant,
            starts_at.as_deref(),
        ) {
            (Some(t), Some(s), _) => t <= s,
            (_, _, Some(s)) => taken_at <= s,
            _ => true,
        }
    };

    let mut stmt = db.prepare(
        "SELECT taken_at, home_line FROM line_snapshots
         WHERE game_id = ?1 ORDER BY taken_at, home_line",
    )?;
    let rows: Vec<(String, f64)> = stmt
        .query_map(params![game_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;

    let mut closing: Option<(String, Vec<f64>)> = None;
    let mut i = 0;
    while i < rows.len() {
        let at = rows[i].0.clone();
        if !at_or_before_start(&at) {
            break;
        }
        let batch: Vec<f64> = rows[i..].iter().take_while(|(t, _)| *t == at).map(|(_, l)| *l).collect();
        i += batch.len();
        closing = Some((at.clone(), batch));
    }
    let Some((taken_at, lines)) = closing else {
        return Ok(None);
    };
    if lines.is_empty() {
        tracing::debug!(game_id, taken_at, "closing snapshot batch empty");
        return Ok(None);
    }
    let mid = lines.len() / 2;
    Ok(Some(if lines.len() % 2 == 1 { lines[mid] } else { (lines[mid - 1] + lines[mid]) / 2.0 }))
}

pub fn set_bankroll(db: &Connection, telegram_user_id: i64, bankroll_usd: f64) -> Result<()> {
    db.execute(
        "UPDATE subscribers SET bankroll_usd = ?1 WHERE telegram_user_id = ?2",
        params![bankroll_usd, telegram_user_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oddsports_shared::Subscriber;

    fn mem_db() -> Connection {
        let db = Connection::open_in_memory().expect("in-memory db");
        init_schema(&db).expect("schema");
        db
    }

    /// Direct snapshot inserts — save_line_snapshots stamps its own taken_at,
    /// and these tests need deterministic batch timestamps.
    fn insert_batch(db: &Connection, game_id: &str, starts_at: &str, taken_at: &str, lines: &[f64]) {
        for (i, line) in lines.iter().enumerate() {
            db.execute(
                "INSERT INTO line_snapshots (game_id, sport, starts_at, book, home_line, taken_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![game_id, "\"nfl\"", starts_at, format!("book{i}"), line, taken_at],
            )
            .expect("insert snapshot");
        }
    }

    #[test]
    fn slate_round_trips_through_json() {
        let db = mem_db();
        let slate = DailySlate { date: "2026-08-21".into(), picks: vec![], generation: Default::default() };
        save_slate(&db, &slate).unwrap();
        let loaded = load_slate(&db, "2026-08-21").unwrap().unwrap();
        assert_eq!(loaded.date, "2026-08-21");
        assert!(load_slate(&db, "2026-08-20").unwrap().is_none());
        assert!(slate_dates_before(&db, "2026-08-22").unwrap() == vec!["2026-08-21".to_string()]);
    }

    #[test]
    fn upsert_preserves_existing_telegram_and_bankroll() {
        let db = mem_db();
        let sub = Subscriber {
            beehiiv_id: "b1".into(),
            email: "a@example.com".into(),
            tier: Tier::Analyst,
            telegram_user_id: Some(42),
            bankroll_usd: None,
            linked_at: Some("t".into()),
        };
        upsert_subscriber(&db, &sub).unwrap();
        // Re-sync: tier update, no telegram id, bankroll now known.
        let updated = Subscriber { tier: Tier::Sharp, telegram_user_id: None, bankroll_usd: Some(5000.0), ..sub };
        upsert_subscriber(&db, &updated).unwrap();

        let got = get_subscriber_by_telegram(&db, 42).unwrap().unwrap();
        assert_eq!(got.tier, Tier::Sharp); // updated
        assert_eq!(got.bankroll_usd, Some(5000.0)); // updated
        assert_eq!(got.telegram_user_id, Some(42)); // COALESCE kept the binding
    }

    #[test]
    fn line_history_medians_per_batch() {
        let db = mem_db();
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T18:00:00+00:00", &[-4.0, -4.5, -5.0]);
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T19:00:00+00:00", &[-5.5, -6.0, -6.5]);
        let h = line_history(&db, "g1").unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].line, h[1].line), (-4.5, -6.0));
    }

    #[test]
    fn closing_line_excludes_post_kickoff_snapshots_across_offset_formats() {
        // Regression for the "+00:00" vs "Z" lexicographic bug: the 20:05
        // batch sorts before "…T20:00:00Z" as a string and used to be picked
        // as the closing line even though it was captured after kickoff.
        let db = mem_db();
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T19:55:00+00:00", &[-4.0, -5.0, -6.0]);
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T20:05:00+00:00", &[99.0, 100.0, 101.0]);
        assert_eq!(closing_spread(&db, "g1").unwrap(), Some(-5.0));
    }

    #[test]
    fn closing_line_is_none_without_a_pre_kickoff_batch() {
        let db = mem_db();
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T20:05:00+00:00", &[1.0, 2.0]);
        assert_eq!(closing_spread(&db, "g1").unwrap(), None);
    }

    #[test]
    fn closing_line_median_of_last_pre_kickoff_batch() {
        let db = mem_db();
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T18:00:00+00:00", &[-3.0, -3.5, -4.0]);
        insert_batch(&db, "g1", "2026-08-22T20:00:00Z", "2026-08-22T19:59:00+00:00", &[-5.0, -5.5, -6.0]);
        assert_eq!(closing_spread(&db, "g1").unwrap(), Some(-5.5));
    }

    #[test]
    fn bankroll_update_targets_the_right_subscriber() {
        let db = mem_db();
        for (id, beehiiv) in [(7i64, "b7"), (8, "b8")] {
            upsert_subscriber(
                &db,
                &Subscriber {
                    beehiiv_id: beehiiv.into(),
                    email: format!("{beehiiv}@example.com"),
                    tier: Tier::Sharp,
                    telegram_user_id: Some(id),
                    bankroll_usd: None,
                    linked_at: None,
                },
            )
            .unwrap();
        }
        set_bankroll(&db, 8, 2500.0).unwrap();
        assert_eq!(get_subscriber_by_telegram(&db, 7).unwrap().unwrap().bankroll_usd, None);
        assert_eq!(get_subscriber_by_telegram(&db, 8).unwrap().unwrap().bankroll_usd, Some(2500.0));
    }
}
