/** Tier ladder — the spine of the product. PRD Section 4. */
export enum Tier {
  Free = 0,
  Starter = 1,
  Analyst = 2,
  Sharp = 3,
}

export interface TierDef {
  tier: Tier;
  name: string;
  /** USD/month. null = free. Final pricing pending finance (PRD open question). */
  priceUsd: number | null;
  /** What this tier unlocks — used for upgrade prompts. */
  unlocks: string[];
}

export const TIERS: Record<Tier, TierDef> = {
  [Tier.Free]: {
    tier: Tier.Free,
    name: "Free",
    priceUsd: null,
    unlocks: ["Top 3–5 daily picks", "Confidence stars", "Odds comparison"],
  },
  [Tier.Starter]: {
    tier: Tier.Starter,
    name: "Starter",
    priceUsd: 19,
    unlocks: ["Full daily slate", "Form / H2H / injury notes", "Private Telegram channel"],
  },
  [Tier.Analyst]: {
    tier: Tier.Analyst,
    name: "Analyst",
    priceUsd: 49,
    unlocks: [
      "Model factor breakdowns",
      "Line movement & steam tracking",
      "Props/parlays with correlation notes",
      "Suggested unit sizing",
      "/why /line /units bot commands",
    ],
  },
  [Tier.Sharp]: {
    tier: Tier.Sharp,
    name: "Sharp",
    priceUsd: 129,
    unlocks: [
      "Live in-game alerts",
      "Raw model output",
      "Personalized bankroll pacing",
      "Weekly office-hours recap",
      "Earliest delivery",
    ],
  },
};

/** True if a subscriber at `have` may see content gated at `need`. */
export function canAccess(have: Tier, need: Tier): boolean {
  return have >= need;
}

/** Next tier up, for upgrade CTAs. Null at top of ladder. */
export function nextTier(t: Tier): TierDef | null {
  return t < Tier.Sharp ? TIERS[(t + 1) as Tier] : null;
}
