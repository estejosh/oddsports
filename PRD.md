# PRD: OddSports — Tiered Sports Betting Analysis Newsletter + Telegram Bot

**Owner:** estejosh
**Status:** Draft v2 — for handoff to Fable (build agent)
**Last updated:** 2026-07-02
**Changes from v1:** Added Telegram bot as a first-class delivery surface, AI token-budget architecture, and audience acquisition plan.

---

## 1. Problem Statement

Sports bettors are underserved by free content: most "picks" newsletters give a binary tip with no visibility into the reasoning, no stake sizing guidance, and no way to pay more for deeper analysis when they want it. Serious bettors will pay for granular, model-backed breakdowns (line movement, injury-adjusted projections, correlated-parlay math), but the market offers either free low-effort tout sheets or expensive one-size-fits-all subscriptions. OddSports fills this gap with a product where **analysis depth scales directly with subscription tier**, delivered via email (Beehiiv) AND Telegram bot, monetized by subscriptions plus sportsbook referral links — primarily our own licensed platform, **Betchu**, presented odds-shopping-agnostically alongside competitor affiliate links.

## 2. Goals

1. **Launch a 4-tier product** (Free, Starter, Analyst, Sharp) on two surfaces: Beehiiv newsletter + Telegram bot, covering all major sports in season.
2. **Drive Betchu signups and first deposits** — trackable referral CTA in every issue and every bot response.
3. **Generate affiliate revenue** from non-Betchu sportsbook links as secondary monetization (odds-comparison format).
4. **Convert free → paid at 3–5%** within the first 90 days (industry-benchmark starting target; recalibrate with real data).
5. **Keep AI cost per subscriber per day below a hard budget** (see Section 8) so margins improve, not degrade, as content depth grows.
6. **Build a 1,000-subscriber opt-in list in 60 days** via the acquisition channels in Section 10 — earned/opt-in only, no purchased or scraped lists.

## 3. Non-Goals (v1)

- **Not building Betchu itself** — this PRD only needs Betchu's referral link format and (ideally) odds feed.
- **Not placing bets for users.** Analysis and stake-sizing *suggestions* only; no wallet integration, no bet execution, not even via the Telegram bot.
- **Not a custom mobile app.** Telegram bot IS the mobile/real-time surface for v1; a native app is P2.
- **Not buying, scraping, or cold-emailing contact lists.** All list growth is opt-in (see Section 10). Purchased/scraped lists are explicitly out of scope — they violate CAN-SPAM/GDPR, kill email deliverability, and endanger Betchu's license.
- **Not resolving multi-jurisdiction compliance in this PRD** — blocking legal questions listed in Section 11.

## 4. Tiering Model — Analysis Granularity Ladder

Core mechanic: **more payment = deeper analysis, earlier delivery, more personalization.** Same ladder applies across both surfaces (email + Telegram).

| Tier | Price (Fable to finalize) | Content Depth | Email Cadence | Telegram Access |
|---|---|---|---|---|
| **Free** | $0 | Top 3–5 picks/day, one-line rationale, confidence stars, risk warning. Betchu + affiliate CTAs. | Daily digest | Public channel (broadcast only) |
| **Starter** | ~$15–25/mo | Full slate + basic stats (form, H2H, injuries) + confidence rating. | Daily | Private channel + `/picks`, `/slate` commands |
| **Analyst** | ~$40–60/mo | + Model breakdown (factors behind the number), line movement/steam tracking, props/parlays with correlation notes, suggested unit sizing. | Daily, earlier send | + `/why <game>`, `/line <game>`, `/units` commands |
| **Sharp** | ~$100–150+/mo | + Live in-game alerts, raw model output tables, weekly office-hours recap, earliest delivery, personalized bankroll pacing. | Daily + real-time | + live alert pushes, `/bankroll` personalization, `/raw <game>` |

**Design principle:** tiers unlock *reasoning, timing, and personalization* — never just "more picks."

## 5. Telegram Bot Specification

The bot is the real-time surface; email is the daily-depth surface. One content pipeline feeds both (critical for token budget — see Section 8).

### 5.1 Architecture
- **Bot framework:** grammY or Telegraf (Node/TS) or aiogram (Python) — Fable's choice; must support webhooks (not polling) for cost/latency.
- **Tier gating:** subscriber's Beehiiv tier is source of truth. Sync via Beehiiv API webhook → bot DB (SQLite/Postgres). Users link accounts with `/link <email>` + magic-link verification to their subscribed email.
- **Channels:** one public free broadcast channel; private invite-link channels per paid tier (bot auto-kicks on subscription lapse via daily sync job).
- **Payments:** primary path is Beehiiv checkout (web) → tier sync. Optionally Telegram Stars/native payments as P1 — but beware Telegram's gambling-content payment policies (open question).

### 5.2 Commands (tier-gated)
| Command | Min tier | Behavior |
|---|---|---|
| `/start`, `/help` | — | Onboarding, tier explanation, Betchu CTA, RG disclosure |
| `/today` | Free | Today's free-tier picks (cached, zero AI tokens) |
| `/picks`, `/slate` | Starter | Full slate (cached) |
| `/why <game>` | Analyst | Model factor breakdown for a game (pre-generated, cached) |
| `/line <game>` | Analyst | Current line + movement history (data lookup, no AI) |
| `/units` | Analyst | Today's suggested unit sizing table (cached) |
| `/bankroll <amount>` | Sharp | Set bankroll; sizing responses scale to it (arithmetic, no AI) |
| `/raw <game>` | Sharp | Raw model output table (data dump, no AI) |
| `/odds <game>` | Free | Odds comparison across books incl. Betchu, all links tracked (data lookup) |
| `/rg` | — | Responsible gambling resources, self-exclusion pointer |

### 5.3 Bot Requirements (P0)
1. Every bot response that includes a pick includes the confidence rating, risk warning, and Betchu + affiliate odds links.
2. RG disclosure in `/start` and pinned in every channel; `/rg` always available.
3. Rate limiting per user (protects token budget and API costs).
4. Tier desync tolerance: if Beehiiv sync fails, bot degrades to last-known tier, never upgrades silently.
5. All outbound links wrapped in tracked short-links (attribution per user per surface).

## 6. AI Token-Budget Architecture (P0 constraint)

**Principle: generate once, serve many. AI tokens are spent per *slate*, never per *user*.**

1. **Single daily generation pass.** The pipeline runs once per day per sport slate: data ingestion → model/statistical layer (NOT LLM — pure math/code computes projections, edges, unit sizing) → LLM writes the *prose* for each tier's version. Output is stored as structured content blocks (per game × per tier).
2. **LLM writes prose, code computes numbers.** Odds math, line movement, CLV, unit sizing, bankroll scaling are all deterministic code. The LLM never does arithmetic and is never called to "analyze" raw numbers ad hoc.
3. **Tiered model routing:** cheap/fast model (Haiku-class) for Free/Starter prose and routine summaries; strong model (Sonnet/Fable-class) only for Analyst/Sharp model-breakdown prose and the weekly recap. Never use a frontier model where a small one scores equivalently — Fable should A/B this and lock in routing.
4. **Bot serves from cache.** Every Telegram command returns pre-generated blocks or pure data lookups. **Zero LLM calls in the bot's request path** in v1. (A conversational `/ask` command is P2, and only with per-user daily token caps.)
5. **Prompt caching + batch API.** Daily generation uses batch/async API pricing where available; shared system prompts structured for prompt-cache hits across the slate.
6. **Hard budget + metering.** Set a daily token budget (e.g., $X/day ceiling); pipeline logs tokens per issue per tier; alert at 80%, degrade gracefully at 100% (skip lowest-value games, never skip compliance blocks).
7. **Live alerts (Sharp) are template-driven:** "Line moved {X}→{Y} on {game}" from data triggers — no LLM in the alert path except an optional daily-capped enrichment pass.

**Acceptance:** marginal AI cost of subscriber #10,000 ≈ $0. Cost scales with number of games covered, not number of users.

## 7. Requirements (consolidated)

### Must-Have (P0)
1. Beehiiv with 4 tiers and content-gated blocks per tier.
2. Daily automated pipeline: data → deterministic model → tiered LLM prose → compliance-block insertion → scheduled send + bot cache load.
3. Betchu trackable referral CTA in every issue and every pick-bearing bot response.
4. ≥1 non-Betchu affiliate book shown per pick in odds-comparison format, tracked links.
5. RG disclosure + age/jurisdiction disclaimer on every issue and pinned in every Telegram channel; template-locked (send blocked without it).
6. Confidence rating + plain-language risk warning on every pick ("recommendations with warnings").
7. Telegram bot per Section 5 with tier sync from Beehiiv.
8. Token budget instrumentation per Section 6 (metering, alerts, routing).
9. Launch coverage: NFL, NBA, MLB, NHL, EPL/UCL soccer, UFC/boxing; expand opportunistically.
10. Related-party disclosure on Betchu links ("OddSports and Betchu are affiliated") — pending legal wording.

### Nice-to-Have (P1)
1. Telegram Stars/native payments (pending gambling-policy check).
2. SMS fallback for Sharp live alerts.
3. CLV tracker / public performance record (major trust + acquisition asset).
4. Subscriber referral program.
5. Personalized bankroll web dashboard.

### Future (P2)
1. `/ask` conversational bot command with per-user token caps.
2. Native mobile app.
3. Niche sports (esports, tennis challengers).
4. API/data product for power users; white-label engine.

## 8. Success Metrics

**Leading:** free→paid conversion (target 3–5%/30d), Betchu CTR per issue and per bot user, open/click rates by tier, bot DAU/linked-account rate, **AI cost per issue and per subscriber-day** (must trend down as subs grow).
**Lagging:** Betchu first-deposit conversions attributed to OddSports, affiliate revenue/sub/month, tier-upgrade rate, churn by tier, pick CLV performance.

## 9. Compliance Guardrails (P0 constraints)

- RG language + helpline (1-800-GAMBLER or jurisdiction equivalent) everywhere content appears.
- Analysis/entertainment framing; banned vocabulary list ("lock," "guaranteed," "can't lose") enforced in the pipeline as a lint step.
- Age/jurisdiction gating at signup; Telegram channel join flow includes the same attestation.
- Related-party disclosure for Betchu (owned platform ≠ neutral affiliate).
- Telegram-specific: verify Telegram ToS and Stars policy on gambling-adjacent content before building native payments.

## 10. Audience Acquisition — who to send this to (opt-in only)

No scraped or purchased email lists (see Non-Goals; illegal + deliverability suicide). The plan is to go where bettors already are and pull them into the opt-in funnel. Fable should execute this as a checklist:

1. **Reddit** — r/sportsbook (~2M members), r/sportsbetting, r/dfsports, plus league subs (r/nfl gamethreads etc.). Play: post genuinely useful free analysis (CLV records, line-movement writeups) with the free newsletter as signature. Respect each sub's self-promo rules; value-first or you get banned.
2. **Twitter/X betting community** — build an OddSports account posting free daily card + one deep-dive thread; the thread format converts well to newsletter signups. Engage the #GamblingTwitter graph.
3. **Telegram itself** — the free public channel is an acquisition asset; list it in Telegram channel directories and cross-promote with adjacent (non-scam) sports channels.
4. **Discord** — sports betting and DFS servers allow partnerships/AMAs; offer free Starter-tier trials to server members.
5. **Beehiiv ecosystem** — Beehiiv Boosts (paid cross-newsletter recommendations) and the Beehiiv ad network; recommendation swaps with adjacent sports newsletters.
6. **Lead magnet** — free "Bankroll Management 101" or a public pick-performance tracker page as the signup hook; this is what converts cold traffic.
7. **Betchu's existing user base** — the one list we CAN email directly (existing business relationship, subject to Betchu's own privacy policy and opt-in terms — legal to confirm). Likely the highest-quality seed audience.
8. **Season-timed paid spend (P1)** — small test budgets on X ads / Reddit ads around NFL kickoff, only after organic funnel proves conversion.

**Explicitly rejected:** buying email lists, scraping emails from forums/social, cold outreach to harvested addresses.

## 11. Open Questions

- **[Legal — BLOCKING]** Jurisdictions for marketing Betchu + offshore-friendly affiliate links; what geofencing is required.
- **[Legal — BLOCKING]** Related-party disclosure wording for Betchu promotion.
- **[Legal — BLOCKING]** Can we email Betchu's existing customer list under its current privacy policy/opt-ins?
- **[Product]** Telegram Stars gambling-content policy — native payments viable or web-checkout only?
- **[Finance]** Final tier pricing vs Beehiiv fees and competitors (Action Network, Unabated as references).
- **[Data/Eng]** Odds/stats data source and cost per sport (paid odds API vs Betchu's own feed) — determines "all sports" feasibility and the deterministic-model layer.
- **[Content Ops]** Human-in-the-loop review gate before daily send — recommended ON at launch given gambling exposure.

## 12. Phasing

- **Phase 1:** Beehiiv 4-tier setup, daily pipeline (NFL/NBA/MLB/NHL), Free + Starter live, Betchu + 1 affiliate, compliance locks, free Telegram broadcast channel, token metering. Launch.
- **Phase 2:** Analyst tier (model prose, line movement, units), full tier-gated Telegram bot with account linking, soccer + UFC coverage, CLV tracker.
- **Phase 3:** Sharp tier (live alerts, bankroll personalization, office hours), acquisition paid-spend tests, P1 items as data justifies.
- **Timing lever:** launch ahead of NFL season (September) — highest acquisition window of the year.
