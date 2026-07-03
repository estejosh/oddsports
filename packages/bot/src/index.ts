/**
 * OddSports Telegram bot. PRD Section 5.
 * ZERO LLM calls in the request path (PRD 6 #4) — every command serves
 * pre-generated slate content from SQLite or does pure data lookups.
 */
import { Bot } from "grammy";
import { Tier, TIERS, canAccess, nextTier, RG_DISCLOSURE, RELATED_PARTY_DISCLOSURE } from "@oddsports/shared";
import type { DailySlate, PickBlock } from "@oddsports/shared";
import { openDb, loadSlate, getSubscriberByTelegram } from "@oddsports/pipeline/dist/store.js";
import { scaleUnitsToBankroll } from "@oddsports/pipeline/dist/model.js";
import { rollingRecord, buildRevealPost } from "@oddsports/pipeline/dist/grading.js";
import { createLinkToken, verifyLinkToken } from "./beehiiv.js";
import { allow } from "./ratelimit.js";

const token = process.env.TELEGRAM_BOT_TOKEN;
if (!token) throw new Error("TELEGRAM_BOT_TOKEN not set");

const bot = new Bot(token);
const db = openDb();

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

function userTier(telegramUserId: number): Tier {
  return getSubscriberByTelegram(db, telegramUserId)?.tier ?? Tier.Free;
}

function slateOr404(): DailySlate | null {
  return loadSlate(db, today());
}

function upgradeNudge(have: Tier, need: Tier): string {
  const next = nextTier(have);
  const needed = TIERS[need];
  return (
    `🔒 That's a **${needed.name}** feature.\n` +
    (next ? `Upgrade to ${next.name} ($${next.priceUsd}/mo) unlocks: ${next.unlocks.join(", ")}.\n` : "") +
    `Manage your subscription from the newsletter → account page.`
  );
}

function picksFor(slate: DailySlate, tier: Tier): PickBlock[] {
  // Serve each game at the DEEPEST depth the user's tier allows.
  const byGame = new Map<string, PickBlock>();
  for (const p of slate.picks) {
    if (!canAccess(tier, p.minTier)) continue;
    const existing = byGame.get(p.gameId);
    if (!existing || p.minTier > existing.minTier) byGame.set(p.gameId, p);
  }
  return [...byGame.values()];
}

// --- middleware: rate limit ---
bot.use(async (ctx, next) => {
  const id = ctx.from?.id;
  if (id && !allow(id)) {
    await ctx.reply("Slow down a little — try again in a minute. ⏳");
    return;
  }
  await next();
});

// --- commands ---
bot.command("start", async (ctx) => {
  await ctx.reply(
    [
      "🏟 **OddSports** — tiered sports betting analysis.",
      "",
      "Free: top daily picks. Paid tiers unlock the full slate, model breakdowns, line movement, unit sizing, and live alerts.",
      "",
      "Commands: /today /odds /link /help /rg",
      "",
      `Subscribe: (Beehiiv signup link here)`,
      "",
      RELATED_PARTY_DISCLOSURE,
      RG_DISCLOSURE,
    ].join("\n"),
    { parse_mode: "Markdown" }
  );
});

bot.command("help", (ctx) =>
  ctx.reply(
    [
      "/today — today's picks (your tier depth)",
      "/picks /slate — full slate (Starter+)",
      "/why <game> — model breakdown (Analyst+)",
      "/line <game> — line movement (Analyst+)",
      "/units — unit sizing table (Analyst+)",
      "/bankroll <amount> — personalize sizing (Sharp)",
      "/raw <game> — raw model output (Sharp)",
      "/odds <game> — odds comparison, all books",
      "/record — graded track record + yesterday's full reveal (free for everyone)",
      "/link <email> — link your newsletter subscription",
      "/verify <token> — complete linking",
      "/rg — responsible gambling resources",
    ].join("\n")
  )
);

bot.command("rg", (ctx) =>
  ctx.reply(
    `${RG_DISCLOSURE}\n\nSelf-exclusion: contact your sportsbook's responsible gambling page. US helpline: 1-800-GAMBLER.`
  )
);

bot.command("link", async (ctx) => {
  const email = ctx.match?.trim();
  if (!email || !email.includes("@")) {
    await ctx.reply("Usage: /link you@example.com — we'll email you a verification token.");
    return;
  }
  createLinkToken(db, email);
  await ctx.reply("📧 Check your email for a verification token, then send: /verify <token>");
});

bot.command("verify", async (ctx) => {
  const tok = ctx.match?.trim();
  if (!tok || !ctx.from) {
    await ctx.reply("Usage: /verify <token>");
    return;
  }
  const tier = await verifyLinkToken(db, tok, ctx.from.id);
  if (tier === null) {
    await ctx.reply("Invalid or expired token. Run /link again.");
    return;
  }
  await ctx.reply(`✅ Linked! Your tier: **${TIERS[tier].name}**.`, { parse_mode: "Markdown" });
});

bot.command("today", async (ctx) => {
  const slate = slateOr404();
  if (!slate) {
    await ctx.reply("Today's slate isn't out yet — check back soon.");
    return;
  }
  const tier = userTier(ctx.from!.id);
  const picks = picksFor(slate, tier);
  const shown = tier === Tier.Free ? picks.slice(0, 5) : picks;
  for (const p of shown.slice(0, 10)) {
    await ctx.reply(p.body, { parse_mode: "Markdown" });
  }
  if (tier === Tier.Free) {
    await ctx.reply(upgradeNudge(tier, Tier.Starter), { parse_mode: "Markdown" });
  }
});

for (const cmd of ["picks", "slate"] as const) {
  bot.command(cmd, async (ctx) => {
    const tier = userTier(ctx.from!.id);
    if (!canAccess(tier, Tier.Starter)) {
      await ctx.reply(upgradeNudge(tier, Tier.Starter), { parse_mode: "Markdown" });
      return;
    }
    const slate = slateOr404();
    if (!slate) return void (await ctx.reply("Slate not ready yet."));
    for (const p of picksFor(slate, tier).slice(0, 20)) {
      await ctx.reply(p.body, { parse_mode: "Markdown" });
    }
  });
}

bot.command("units", async (ctx) => {
  const tier = userTier(ctx.from!.id);
  if (!canAccess(tier, Tier.Analyst)) {
    await ctx.reply(upgradeNudge(tier, Tier.Analyst), { parse_mode: "Markdown" });
    return;
  }
  const slate = slateOr404();
  if (!slate) return void (await ctx.reply("Slate not ready yet."));
  const sub = getSubscriberByTelegram(db, ctx.from!.id);
  const lines = picksFor(slate, tier).map((p) => {
    const units = /Suggested size: ([\d.]+)u/.exec(p.body)?.[1];
    if (!units) return null;
    const scaled =
      tier >= Tier.Sharp && sub?.bankrollUsd
        ? ` ($${scaleUnitsToBankroll(Number(units), sub.bankrollUsd)})`
        : "";
    return `• ${p.gameId}: ${units}u${scaled} ${"★".repeat(p.confidence)}`;
  });
  await ctx.reply(["📏 **Today's sizing**", ...lines.filter(Boolean)].join("\n"), { parse_mode: "Markdown" });
});

bot.command("bankroll", async (ctx) => {
  const tier = userTier(ctx.from!.id);
  if (!canAccess(tier, Tier.Sharp)) {
    await ctx.reply(upgradeNudge(tier, Tier.Sharp), { parse_mode: "Markdown" });
    return;
  }
  const amount = Number(ctx.match?.trim());
  if (!amount || amount <= 0) {
    await ctx.reply("Usage: /bankroll 5000 — sets your bankroll so sizing scales to it.");
    return;
  }
  db.prepare("UPDATE subscribers SET bankroll_usd = ? WHERE telegram_user_id = ?").run(amount, ctx.from!.id);
  await ctx.reply(
    `💰 Bankroll set to $${amount}. 1 unit = $${(amount / 100).toFixed(2)}. /units now shows dollar sizing.\n\n${RG_DISCLOSURE}`
  );
});

// /record — FREE tier on purpose (PRD P0 #11): the graded track record and
// yesterday's full paid-depth reveal are public trust assets. Pure data, no AI.
bot.command("record", async (ctx) => {
  const rec = rollingRecord(db);
  if (rec.graded === 0) {
    await ctx.reply("📊 No graded picks yet — the record starts after the first settled slate.");
    return;
  }
  const yesterday = new Date(Date.now() - 86_400_000).toISOString().slice(0, 10);
  const reveal = buildRevealPost(db, yesterday);
  // Telegram message cap is 4096 chars — send summary, then chunk the reveal.
  for (let i = 0; i < reveal.length; i += 4000) {
    await ctx.reply(reveal.slice(i, i + 4000), { parse_mode: "Markdown" });
  }
});

// /why, /line, /raw — pre-generated content lookups keyed by fuzzy game match.
// TODO(fable): implement game-name fuzzy matching against today's slate and
// serve the Analyst-depth (deep) block / lineHistory / raw ModelOutput tables.

bot.catch((err) => console.error("[bot] error:", err));

console.log("[bot] starting (long-poll dev mode — switch to webhooks in prod)");
bot.start();
