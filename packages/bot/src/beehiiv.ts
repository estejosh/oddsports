/**
 * Beehiiv sync — Beehiiv is the tier source of truth (PRD 5.1).
 * Account linking: /link <email> → magic token emailed → /verify <token>.
 * Desync tolerance (PRD 5.3 #4): on sync failure, keep last-known tier,
 * NEVER upgrade silently.
 */
import { Tier } from "@oddsports/shared";
import type Database from "better-sqlite3";
import { randomBytes } from "node:crypto";
import { upsertSubscriber } from "@oddsports/pipeline/dist/store.js";

const API = "https://api.beehiiv.com/v2";

/** Map Beehiiv subscription tier names → our Tier enum. Configure to match Beehiiv setup. */
const TIER_NAME_MAP: Record<string, Tier> = {
  free: Tier.Free,
  starter: Tier.Starter,
  analyst: Tier.Analyst,
  sharp: Tier.Sharp,
};

export async function fetchBeehiivTier(email: string): Promise<{ beehiivId: string; tier: Tier } | null> {
  const key = process.env.BEEHIIV_API_KEY;
  const pub = process.env.BEEHIIV_PUBLICATION_ID;
  if (!key || !pub) {
    console.warn("[beehiiv] not configured");
    return null;
  }
  const res = await fetch(
    `${API}/publications/${pub}/subscriptions/by_email/${encodeURIComponent(email)}`,
    { headers: { Authorization: `Bearer ${key}` } }
  );
  if (!res.ok) return null;
  const data = (await res.json()) as any;
  const tierName = (data.data?.subscription_tier ?? "free").toLowerCase();
  return {
    beehiivId: data.data?.id ?? email,
    tier: TIER_NAME_MAP[tierName] ?? Tier.Free,
  };
}

/** Create a link token for magic-link email verification. */
export function createLinkToken(db: Database.Database, email: string): string {
  const token = randomBytes(16).toString("hex");
  const expires = new Date(Date.now() + 15 * 60_000).toISOString();
  db.prepare("INSERT OR REPLACE INTO link_tokens (token, email, expires_at) VALUES (?, ?, ?)").run(
    token,
    email,
    expires
  );
  // TODO(fable): actually send the token via Beehiiv transactional email or SES.
  console.log(`[beehiiv] link token for ${email}: ${token} (send via email in prod!)`);
  return token;
}

/** Verify a token and bind the Telegram user. Returns bound tier or null. */
export async function verifyLinkToken(
  db: Database.Database,
  token: string,
  telegramUserId: number
): Promise<Tier | null> {
  const row = db
    .prepare("SELECT email, expires_at FROM link_tokens WHERE token = ?")
    .get(token) as { email: string; expires_at: string } | undefined;
  if (!row || new Date(row.expires_at) < new Date()) return null;

  db.prepare("DELETE FROM link_tokens WHERE token = ?").run(token);

  const beehiiv = await fetchBeehiivTier(row.email);
  const tier = beehiiv?.tier ?? Tier.Free;
  upsertSubscriber(db, {
    beehiivId: beehiiv?.beehiivId ?? row.email,
    email: row.email,
    tier,
    telegramUserId,
    linkedAt: new Date().toISOString(),
  });
  return tier;
}

/**
 * Daily re-sync of all linked subscribers. Downgrade + auto-kick from paid
 * channels on lapse handled by caller. On API failure: keep last-known tier.
 */
export async function dailyTierSync(db: Database.Database): Promise<void> {
  const rows = db.prepare("SELECT email FROM subscribers WHERE telegram_user_id IS NOT NULL").all() as Array<{ email: string }>;
  for (const { email } of rows) {
    try {
      const fresh = await fetchBeehiivTier(email);
      if (fresh) {
        db.prepare("UPDATE subscribers SET tier = ? WHERE email = ?").run(fresh.tier, email);
      }
      // null (API failure) → keep last-known tier, per PRD desync tolerance.
    } catch (err) {
      console.warn(`[beehiiv] sync failed for ${email} — keeping last-known tier`, err);
    }
  }
}
