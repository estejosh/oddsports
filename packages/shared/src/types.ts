import type { Tier } from "./tiers.js";

export type Sport =
  | "nfl"
  | "nba"
  | "mlb"
  | "nhl"
  | "soccer_epl"
  | "soccer_ucl"
  | "mma"
  | "boxing";

export type MarketType = "spread" | "moneyline" | "total" | "prop" | "parlay";

/** One book's price on one market. */
export interface BookLine {
  book: string; // "betchu", "draftkings", ...
  americanOdds: number; // e.g. -110
  line?: number; // spread/total number if applicable
  fetchedAt: string; // ISO
}

export interface Game {
  id: string;
  sport: Sport;
  home: string;
  away: string;
  startsAt: string; // ISO
  lines: Record<MarketType, BookLine[]>;
}

/** Deterministic model output — produced by code, never by an LLM. */
export interface ModelOutput {
  gameId: string;
  market: MarketType;
  side: string; // "home -3.5", "over 44.5", ...
  fairLine: number; // model's fair number
  marketLine: number; // best available
  edgePct: number; // fair vs market edge
  confidence: 1 | 2 | 3 | 4 | 5;
  suggestedUnits: number; // fractional Kelly-derived, capped
  /** Structured factors — LLM turns these into prose, never invents its own. */
  factors: Array<{ name: string; direction: "for" | "against"; weight: number; detail: string }>;
  lineHistory: Array<{ at: string; line: number }>;
}

/** A pick as rendered content — one per game per tier depth. */
export interface PickBlock {
  gameId: string;
  minTier: Tier;
  /** Rendered markdown/text for this tier depth. Includes compliance block. */
  body: string;
  confidence: 1 | 2 | 3 | 4 | 5;
  riskWarning: string;
  /** Tracked links: betchu first entry by convention, then affiliates. */
  links: Array<{ book: string; url: string }>;
}

/** One day's fully generated content — the unit the pipeline produces once. */
export interface DailySlate {
  date: string; // YYYY-MM-DD
  sports: Sport[];
  picks: PickBlock[];
  /** Token/cost accounting for this generation run. */
  generation: { inputTokens: number; outputTokens: number; costUsd: number };
}

export interface Subscriber {
  beehiivId: string;
  email: string;
  tier: Tier;
  telegramUserId?: number;
  bankrollUsd?: number; // Sharp-only personalization
  linkedAt?: string;
}
