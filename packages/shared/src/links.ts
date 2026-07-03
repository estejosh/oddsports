/**
 * Tracked link building. PRD P0 #3-4: every pick carries Betchu + >=1
 * affiliate book as tracked links, odds-shopping-agnostic presentation.
 */

export interface LinkContext {
  /** "email" | "telegram" — attribution per surface. */
  surface: string;
  tier: string;
  gameId?: string;
  /** Per-user attribution where available (telegram user id, beehiiv id). */
  subscriberRef?: string;
}

const BETCHU_BASE = process.env.BETCHU_REFERRAL_BASE ?? "https://betchu.example/r/oddsports";

/** Affiliate program URL templates. `{sub}` is replaced with the subId payload. */
const AFFILIATE_TEMPLATES: Record<string, string> = {
  betchu: `${BETCHU_BASE}?sub={sub}`,
  // Fill in real program links as affiliate accounts are approved:
  // draftkings: "https://dkng.co/oddsports?subid={sub}",
  // fanduel: "https://fanduel.com/aff/oddsports?sub={sub}",
};

export function trackedLink(book: string, ctx: LinkContext): string {
  const template = AFFILIATE_TEMPLATES[book.toLowerCase()];
  if (!template) throw new Error(`No affiliate template for book: ${book}`);
  const sub = encodeURIComponent(
    [ctx.surface, ctx.tier, ctx.gameId ?? "-", ctx.subscriberRef ?? "-"].join("_")
  );
  return template.replace("{sub}", sub);
}

export function availableBooks(): string[] {
  return Object.keys(AFFILIATE_TEMPLATES);
}
