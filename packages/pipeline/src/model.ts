/**
 * Deterministic model layer. PRD Section 6 #2: "LLM writes prose, code
 * computes numbers." Everything here is pure math — no AI calls, ever.
 *
 * v1 model is intentionally simple (market-consensus based): fair line =
 * median across books; edge = deviation of best available price from fair.
 * TODO(fable): replace with real projections per sport (pace/weather/injury
 * adjusted) — the interface stays the same.
 */
import type { Game, ModelOutput, BookLine } from "@oddsports/shared";

const MAX_UNITS = 3; // hard cap regardless of edge — bankroll discipline
const KELLY_FRACTION = 0.25; // quarter-Kelly

export function runModel(games: Game[]): ModelOutput[] {
  const outputs: ModelOutput[] = [];
  for (const game of games) {
    const spread = analyzeMarket(game, game.lines.spread);
    if (spread) outputs.push(spread);
  }
  // Highest edge first — if the token budget degrades, we drop from the tail.
  return outputs.sort((a, b) => b.edgePct - a.edgePct);
}

function analyzeMarket(game: Game, lines: BookLine[]): ModelOutput | null {
  const withPoints = lines.filter((l) => typeof l.line === "number");
  if (withPoints.length < 3) return null; // need book consensus to say anything

  const points = withPoints.map((l) => l.line as number).sort((a, b) => a - b);
  const fairLine = median(points);
  const best = withPoints.reduce((a, b) =>
    Math.abs((b.line as number) - fairLine) > Math.abs((a.line as number) - fairLine) ? b : a
  );
  const marketLine = best.line as number;
  const deviation = Math.abs(marketLine - fairLine);
  const edgePct = Math.round(deviation * 100) / 100; // crude: points of line value

  if (edgePct < 0.5) return null; // no actionable edge

  const confidence = (edgePct >= 2 ? 4 : edgePct >= 1.5 ? 3 : edgePct >= 1 ? 2 : 1) as 1 | 2 | 3 | 4 | 5;
  const suggestedUnits = Math.min(MAX_UNITS, round1(edgePct * KELLY_FRACTION * 2));

  return {
    gameId: game.id,
    market: "spread",
    side: marketLine > fairLine ? `${game.home} ${fmtLine(marketLine)}` : `${game.away} +${fmtLine(-marketLine)}`,
    fairLine,
    marketLine,
    edgePct,
    confidence,
    suggestedUnits,
    factors: [
      {
        name: "book consensus deviation",
        direction: "for",
        weight: 1,
        detail: `${best.book} posts ${fmtLine(marketLine)} vs market median ${fmtLine(fairLine)} across ${withPoints.length} books`,
      },
    ],
    lineHistory: [{ at: new Date().toISOString(), line: marketLine }],
  };
}

/** Bankroll-scaled sizing for Sharp tier. Pure arithmetic (PRD 6 — no AI). */
export function scaleUnitsToBankroll(units: number, bankrollUsd: number): number {
  const unitSize = bankrollUsd / 100; // 1 unit = 1% of bankroll
  return Math.round(units * unitSize * 100) / 100;
}

function median(sorted: number[]): number {
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function round1(n: number): number {
  return Math.round(n * 10) / 10;
}

function fmtLine(n: number): string {
  return n > 0 ? `+${n}` : `${n}`;
}
