# OddSports

Tiered sports betting analysis — Beehiiv newsletter + Telegram bot, cross-promoting **Betchu** (our licensed sportsbook) alongside odds-agnostic affiliate comparisons.

**Read [PRD.md](./PRD.md) first.** [docs/FABLE_HANDOFF.md](./docs/FABLE_HANDOFF.md) is the build order.

## Core design rules (do not violate)

1. **Tokens are spent per slate, never per user.** One daily generation pass; the bot serves 100% from cache. No LLM calls in the bot request path.
2. **LLM writes prose, code computes numbers.** All odds math, edges, unit sizing, bankroll scaling is deterministic TypeScript in `packages/pipeline/src/model.ts`.
3. **Compliance is a template lock, not a checklist.** `assertCompliant()` throws and blocks the send if RG disclosure, related-party disclosure, or banned-phrase lint fails.
4. **Beehiiv is the tier source of truth.** Bot syncs from it; on sync failure, degrade to last-known tier, never upgrade silently.
5. **Opt-in audience only.** No purchased or scraped lists, ever.

## Structure

```
packages/
  shared/    tiers, types, compliance blocks + lint, tracked links
  pipeline/  daily run: ingest → deterministic model → tiered LLM prose → SQLite cache
  bot/       grammY Telegram bot, tier-gated commands, serves from cache
docs/        handoff plan, acquisition checklist
```

## Setup

```bash
npm install
cp .env.example .env   # fill in tokens/keys
npm run build
npm run pipeline:run   # generates today's slate into data/oddsports.sqlite
npm run bot:dev        # starts the bot (long-poll dev mode)
```

## Status

Scaffold — interfaces and control flow are real; these need filling in:
- Real projections per sport in `model.ts` (v1 is book-consensus deviation only)
- Beehiiv API post creation (send path) + transactional email for `/verify` tokens
- `/why`, `/line`, `/raw` game matching in the bot
- Channel auto-kick job on tier lapse
- Odds API account + affiliate program signups
