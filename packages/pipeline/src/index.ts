/**
 * Daily pipeline orchestrator. PRD Section 6 #1: ONE generation pass per day.
 * data → deterministic model → tiered LLM prose → compliance check → cache + send.
 *
 * Run via cron/scheduler once daily per slate window (and per-sport refreshes
 * as needed). Marginal cost per SUBSCRIBER is zero — cost scales with games.
 */
import { Tier, assertCompliant, trackedLink, availableBooks } from "@oddsports/shared";
import type { DailySlate, PickBlock, Sport } from "@oddsports/shared";
import { fetchGames } from "./ingest.js";
import { runModel } from "./model.js";
import { writePickProse, complianceFooter } from "./prose.js";
import { TokenBudget } from "./budget.js";
import { openDb, saveSlate } from "./store.js";

const LAUNCH_SPORTS: Sport[] = ["nfl", "nba", "mlb", "nhl", "soccer_epl", "soccer_ucl", "mma", "boxing"];
const FREE_PICK_COUNT = 5;

export async function runDailyPipeline(date = new Date().toISOString().slice(0, 10)): Promise<DailySlate> {
  const budget = new TokenBudget();
  budget.onAlert = (spent, ceiling) =>
    console.warn(`[budget] ALERT: $${spent.toFixed(2)} of $${ceiling} daily budget spent`);

  console.log(`[pipeline] ${date} — ingesting…`);
  const games = await fetchGames(LAUNCH_SPORTS);
  console.log(`[pipeline] ${games.length} games fetched`);

  const modelOutputs = runModel(games); // sorted best-edge first
  console.log(`[pipeline] ${modelOutputs.length} actionable edges found`);

  const picks: PickBlock[] = [];
  let totalIn = 0;
  let totalOut = 0;

  for (let i = 0; i < modelOutputs.length; i++) {
    const mo = modelOutputs[i];
    const game = games.find((g) => g.id === mo.gameId)!;

    // Tier depths to render for this pick. Free tier only gets top N picks.
    const tiers: Tier[] = i < FREE_PICK_COUNT ? [Tier.Free, Tier.Starter, Tier.Analyst] : [Tier.Starter, Tier.Analyst];

    for (const tier of tiers) {
      const prose = await writePickProse(game, mo, tier, budget);
      totalIn += prose.inputTokens;
      totalOut += prose.outputTokens;

      const links = availableBooks().map((book) => ({
        book,
        url: trackedLink(book, { surface: "email", tier: Tier[tier], gameId: game.id }),
      }));

      const body = [
        `**${game.away} @ ${game.home}** — ${mo.side}`,
        `Confidence: ${"★".repeat(mo.confidence)}${"☆".repeat(5 - mo.confidence)}`,
        prose.text,
        tier >= Tier.Analyst ? `Suggested size: ${mo.suggestedUnits}u` : "",
        `Bet at: ${links.map((l) => `[${l.book}](${l.url})`).join(" · ")}`,
        complianceFooter(),
      ]
        .filter(Boolean)
        .join("\n\n");

      // Template lock — throws (and aborts the send) if compliance blocks missing.
      assertCompliant(body);

      picks.push({
        gameId: game.id,
        minTier: tier,
        body,
        confidence: mo.confidence,
        riskWarning:
          mo.edgePct >= 2
            ? "Large line deviation — verify news (injury/lineup) before betting."
            : "Standard variance applies — size responsibly.",
        links,
      });
    }
  }

  const slate: DailySlate = {
    date,
    sports: LAUNCH_SPORTS,
    picks,
    generation: { inputTokens: totalIn, outputTokens: totalOut, costUsd: budget.spent },
  };

  const db = openDb();
  saveSlate(db, slate);
  console.log(
    `[pipeline] done — ${picks.length} blocks, $${budget.spent.toFixed(4)} AI cost (${totalIn} in / ${totalOut} out tokens)`
  );

  // TODO(fable): push to Beehiiv via API (draft post per tier) — human review
  // gate stays ON at launch (PRD open question: content ops).
  // TODO(fable): notify bot process to warm its cache (or bot reads sqlite directly).

  return slate;
}

// CLI entry
if (process.argv[1]?.endsWith("index.js")) {
  runDailyPipeline().catch((err) => {
    console.error("[pipeline] FAILED:", err);
    process.exit(1);
  });
}
