/**
 * SQLite store — the shared cache between pipeline (writer) and bot (reader).
 * PRD Section 6 #4: bot serves from this cache, zero LLM calls in request path.
 */
import Database from "better-sqlite3";
import type { DailySlate, Subscriber } from "@oddsports/shared";
import { Tier } from "@oddsports/shared";

export function openDb(path = process.env.DATABASE_PATH ?? "./data/oddsports.sqlite"): Database.Database {
  const db = new Database(path);
  db.pragma("journal_mode = WAL");
  db.exec(`
    CREATE TABLE IF NOT EXISTS slates (
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
    CREATE TABLE IF NOT EXISTS token_spend (
      date TEXT NOT NULL,
      model TEXT NOT NULL,
      input_tokens INTEGER NOT NULL,
      output_tokens INTEGER NOT NULL,
      cost_usd REAL NOT NULL
    );
  `);
  return db;
}

export function saveSlate(db: Database.Database, slate: DailySlate): void {
  db.prepare("INSERT OR REPLACE INTO slates (date, json) VALUES (?, ?)").run(
    slate.date,
    JSON.stringify(slate)
  );
}

export function loadSlate(db: Database.Database, date: string): DailySlate | null {
  const row = db.prepare("SELECT json FROM slates WHERE date = ?").get(date) as { json: string } | undefined;
  return row ? (JSON.parse(row.json) as DailySlate) : null;
}

export function getSubscriberByTelegram(db: Database.Database, telegramUserId: number): Subscriber | null {
  const row = db
    .prepare("SELECT * FROM subscribers WHERE telegram_user_id = ?")
    .get(telegramUserId) as any;
  if (!row) return null;
  return {
    beehiivId: row.beehiiv_id,
    email: row.email,
    tier: row.tier as Tier,
    telegramUserId: row.telegram_user_id ?? undefined,
    bankrollUsd: row.bankroll_usd ?? undefined,
    linkedAt: row.linked_at ?? undefined,
  };
}

export function upsertSubscriber(db: Database.Database, sub: Subscriber): void {
  db.prepare(
    `INSERT INTO subscribers (beehiiv_id, email, tier, telegram_user_id, bankroll_usd, linked_at)
     VALUES (@beehiivId, @email, @tier, @telegramUserId, @bankrollUsd, @linkedAt)
     ON CONFLICT(beehiiv_id) DO UPDATE SET
       email = excluded.email,
       tier = excluded.tier,
       telegram_user_id = COALESCE(excluded.telegram_user_id, subscribers.telegram_user_id),
       bankroll_usd = COALESCE(excluded.bankroll_usd, subscribers.bankroll_usd)`
  ).run({
    beehiivId: sub.beehiivId,
    email: sub.email,
    tier: sub.tier,
    telegramUserId: sub.telegramUserId ?? null,
    bankrollUsd: sub.bankrollUsd ?? null,
    linkedAt: sub.linkedAt ?? null,
  });
}
