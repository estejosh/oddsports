/**
 * Post-hoc grading + "The Record" reveal. PRD P0 #11.
 *
 * After games settle: grade every pick published to any paid tier, compute
 * closing-line value, and build a reveal post that shows the FULL
 * Analyst/Sharp-depth content to ALL tiers (including Free).
 *
 * Hard rule: losses are NEVER omitted or edited. The reveal includes every
 * graded pick from the slate, win or lose, and rows are immutable once
 * written. Selective publication would destroy the record's value and is a
 * deceptive-marketing exposure.
 */
import type Database from "better-sqlite3";
import { Tier } from "@oddsports/shared";
import type { DailySlate, Sport } from "@oddsports/shared";
import { loadSlate } from "./store.js";

export type GradeResult = "win" | "loss" | "push" | "void";

export interface GradedPick {
  date: string;
  gameId: string;
  side: string;
  suggestedUnits: number;
  confidence: number;
  result: GradeResult;
  unitsDelta: number; // +units on win (at price), -units on loss, 0 push/void
  closingLine: number | null;
  clv: number | null; // our line vs closing — positive = beat the close
  /** The deepest (Analyst/Sharp) body that paid tiers saw — revealed to everyone. */
  revealBody: string;
}

export interface RollingRecord {
  wins: number;
  losses: number;
  pushes: number;
  unitsNet: number;
  avgClv: number | null;
  graded: number;
}

export function ensureGradingTables(db: Database.Database): void {
  db.exec(`
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
    );
  `);
}

/** Final scores keyed by gameId. TODO(fable): fetch from the odds API's scores endpoint. */
export interface FinalScore {
  gameId: string;
  homeScore: number;
  awayScore: number;
  closingSpread: number | null; // home-perspective closing line
}

export async function fetchFinalScores(_date: string): Promise<FinalScore[]> {
  // the-odds-api: GET /sports/{key}/scores?daysFrom=1 — wire up with ODDS_API_KEY.
  console.warn("[grading] fetchFinalScores not implemented — returning []");
  return [];
}

/**
 * Grade yesterday's slate. Idempotent: PRIMARY KEY + INSERT OR IGNORE means
 * a pick, once graded, is immutable (re-runs cannot rewrite history).
 */
export async function gradeSlate(db: Database.Database, date: string): Promise<GradedPick[]> {
  ensureGradingTables(db);
  const slate = loadSlate(db, date);
  if (!slate) {
    console.warn(`[grading] no slate for ${date}`);
    return [];
  }
  const scores = await fetchFinalScores(date);
  const scoreById = new Map(scores.map((s) => [s.gameId, s]));

  const graded: GradedPick[] = [];
  const insert = db.prepare(`
    INSERT OR IGNORE INTO graded_picks
      (date, game_id, side, suggested_units, confidence, result, units_delta, closing_line, clv, reveal_body, sport)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `);

  for (const pick of deepestBlocks(slate)) {
    const score = scoreById.get(pick.gameId);
    if (!score) continue; // not settled yet — next run picks it up

    const g = gradePick(date, pick, score);
    insert.run(
      g.date, g.gameId, g.side, g.suggestedUnits, g.confidence,
      g.result, g.unitsDelta, g.closingLine, g.clv, g.revealBody, sportOf(slate, pick.gameId)
    );
    graded.push(g);
  }
  console.log(`[grading] ${date}: graded ${graded.length} picks`);
  return graded;
}

/** One block per game — the deepest tier depth that was published. */
function deepestBlocks(slate: DailySlate) {
  const byGame = new Map<string, (typeof slate.picks)[number]>();
  for (const p of slate.picks) {
    const cur = byGame.get(p.gameId);
    if (!cur || p.minTier > cur.minTier) byGame.set(p.gameId, p);
  }
  return [...byGame.values()];
}

function gradePick(date: string, pick: { gameId: string; body: string; confidence: number }, score: FinalScore): GradedPick {
  const side = /\*\* — (.+?)$/m.exec(pick.body)?.[1] ?? "unknown";
  const units = Number(/Suggested size: ([\d.]+)u/.exec(pick.body)?.[1] ?? 1);

  const result = settleSpread(side, score);
  const unitsDelta = result === "win" ? units * 0.91 : result === "loss" ? -units : 0; // -110 juice assumption

  const ourLine = extractLine(side);
  const clv =
    ourLine !== null && score.closingSpread !== null
      ? Math.round((score.closingSpread - ourLine) * 100) / 100
      : null;

  return {
    date,
    gameId: pick.gameId,
    side,
    suggestedUnits: units,
    confidence: pick.confidence,
    result,
    unitsDelta: Math.round(unitsDelta * 100) / 100,
    closingLine: score.closingSpread,
    clv,
    revealBody: pick.body,
  };
}

/** Settle a home/away spread side against the final score. */
function settleSpread(side: string, score: FinalScore): GradeResult {
  const line = extractLine(side);
  if (line === null) return "void";
  const margin = score.homeScore - score.awayScore;
  // Side string names the team; we grade home-perspective if the line sign implies it.
  // TODO(fable): carry structured side info in PickBlock instead of parsing strings.
  const covered = margin + line;
  if (covered > 0) return "win";
  if (covered < 0) return "loss";
  return "push";
}

function extractLine(side: string): number | null {
  const m = /([+-]?\d+(?:\.\d+)?)\s*$/.exec(side.trim());
  return m ? Number(m[1]) : null;
}

export function rollingRecord(db: Database.Database, opts?: { sport?: Sport; sinceDate?: string }): RollingRecord {
  ensureGradingTables(db);
  const where: string[] = ["result != 'void'"];
  const params: any[] = [];
  if (opts?.sport) { where.push("sport = ?"); params.push(opts.sport); }
  if (opts?.sinceDate) { where.push("date >= ?"); params.push(opts.sinceDate); }

  const row = db.prepare(`
    SELECT
      SUM(result = 'win') AS wins,
      SUM(result = 'loss') AS losses,
      SUM(result = 'push') AS pushes,
      ROUND(SUM(units_delta), 2) AS unitsNet,
      ROUND(AVG(clv), 2) AS avgClv,
      COUNT(*) AS graded
    FROM graded_picks WHERE ${where.join(" AND ")}
  `).get(...params) as any;

  return {
    wins: row.wins ?? 0,
    losses: row.losses ?? 0,
    pushes: row.pushes ?? 0,
    unitsNet: row.unitsNet ?? 0,
    avgClv: row.avgClv,
    graded: row.graded ?? 0,
  };
}

/** Build the daily reveal post — full paid-tier depth, shown to everyone. */
export function buildRevealPost(db: Database.Database, date: string): string {
  ensureGradingTables(db);
  const rows = db
    .prepare("SELECT * FROM graded_picks WHERE date = ? ORDER BY units_delta DESC")
    .all(date) as any[];
  if (rows.length === 0) return `📊 **The Record — ${date}**\n\nNo settled picks yet.`;

  const record = rollingRecord(db);
  const lines = rows.map((r) => {
    const icon = r.result === "win" ? "✅" : r.result === "loss" ? "❌" : "➖";
    const clv = r.clv !== null ? ` · CLV ${r.clv > 0 ? "+" : ""}${r.clv}` : "";
    return `${icon} ${r.side} — ${r.result.toUpperCase()} (${r.units_delta > 0 ? "+" : ""}${r.units_delta}u${clv})`;
  });
  const reveals = rows.map((r) => `---\n_What paid tiers saw:_\n\n${r.reveal_body}`);

  return [
    `📊 **The Record — ${date}**`,
    "",
    ...lines,
    "",
    `**Rolling: ${record.wins}-${record.losses}${record.pushes ? `-${record.pushes}` : ""}, ${record.unitsNet > 0 ? "+" : ""}${record.unitsNet}u${record.avgClv !== null ? `, avg CLV ${record.avgClv}` : ""}** (all picks since launch — losses included, always)`,
    "",
    "Yesterday's full paid-tier analysis, revealed:",
    ...reveals,
  ].join("\n");
}
