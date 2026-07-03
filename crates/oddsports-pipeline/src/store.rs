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
    Ok(db)
}

pub fn save_slate(db: &Connection, slate: &DailySlate) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO slates (date, json) VALUES (?1, ?2)",
        params![slate.date, serde_json::to_string(slate)?],
    )?;
    Ok(())
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
    let latest: Option<String> = db
        .query_row(
            "SELECT MAX(taken_at) FROM line_snapshots
             WHERE game_id = ?1 AND taken_at <= starts_at",
            params![game_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(taken_at) = latest else { return Ok(None) };

    let mut stmt = db.prepare(
        "SELECT home_line FROM line_snapshots
         WHERE game_id = ?1 AND taken_at = ?2 ORDER BY home_line",
    )?;
    let lines: Vec<f64> = stmt
        .query_map(params![game_id, taken_at], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    if lines.is_empty() {
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
