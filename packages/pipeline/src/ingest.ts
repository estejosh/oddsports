/**
 * Data ingestion — odds, lines, schedules. No AI here.
 * v1 target: the-odds-api.com (cheap, covers all launch sports).
 * TODO(fable): swap in Betchu's own odds feed when available (PRD open question).
 */
import type { Game, Sport, BookLine, MarketType } from "@oddsports/shared";

const SPORT_KEYS: Record<Sport, string> = {
  nfl: "americanfootball_nfl",
  nba: "basketball_nba",
  mlb: "baseball_mlb",
  nhl: "icehockey_nhl",
  soccer_epl: "soccer_epl",
  soccer_ucl: "soccer_uefa_champs_league",
  mma: "mma_mixed_martial_arts",
  boxing: "boxing_boxing",
};

export async function fetchGames(sports: Sport[]): Promise<Game[]> {
  const base = process.env.ODDS_API_BASE;
  const key = process.env.ODDS_API_KEY;
  if (!base || !key) {
    console.warn("[ingest] ODDS_API_* not configured — returning empty slate");
    return [];
  }

  const games: Game[] = [];
  for (const sport of sports) {
    const url = `${base}/sports/${SPORT_KEYS[sport]}/odds?regions=us&markets=h2h,spreads,totals&oddsFormat=american&apiKey=${key}`;
    const res = await fetch(url);
    if (!res.ok) {
      console.warn(`[ingest] ${sport}: HTTP ${res.status} — skipping`);
      continue;
    }
    const events = (await res.json()) as any[];
    for (const ev of events) {
      games.push(normalizeEvent(sport, ev));
    }
  }
  return games;
}

function normalizeEvent(sport: Sport, ev: any): Game {
  const lines: Record<MarketType, BookLine[]> = {
    spread: [],
    moneyline: [],
    total: [],
    prop: [],
    parlay: [],
  };
  for (const bm of ev.bookmakers ?? []) {
    for (const market of bm.markets ?? []) {
      const type: MarketType | null =
        market.key === "spreads" ? "spread" : market.key === "h2h" ? "moneyline" : market.key === "totals" ? "total" : null;
      if (!type) continue;
      for (const outcome of market.outcomes ?? []) {
        lines[type].push({
          book: bm.key,
          americanOdds: outcome.price,
          line: outcome.point,
          fetchedAt: new Date().toISOString(),
        });
      }
    }
  }
  return {
    id: ev.id,
    sport,
    home: ev.home_team,
    away: ev.away_team,
    startsAt: ev.commence_time,
    lines,
  };
}
