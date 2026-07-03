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

pub fn set_bankroll(db: &Connection, telegram_user_id: i64, bankroll_usd: f64) -> Result<()> {
    db.execute(
        "UPDATE subscribers SET bankroll_usd = ?1 WHERE telegram_user_id = ?2",
        params![bankroll_usd, telegram_user_id],
    )?;
    Ok(())
}
