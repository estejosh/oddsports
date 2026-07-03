# OddSports

Tiered sports betting analysis — Beehiiv newsletter + Telegram bot, cross-promoting **Betchu** (our licensed sportsbook) alongside odds-agnostic affiliate comparisons.

**Read [PRD.md](./PRD.md) first.** [docs/FABLE_HANDOFF.md](./docs/FABLE_HANDOFF.md) is the build order.

Rust workspace — same stack and operational model as our other services (single binary, systemd, SQLite).

## Core design rules (do not violate)

1. **Tokens are spent per slate, never per user.** One daily generation pass; the bot serves 100% from cache. No LLM calls in the bot request path.
2. **LLM writes prose, code computes numbers.** All odds math, edges, unit sizing, bankroll scaling is deterministic Rust in `crates/oddsports-pipeline/src/model.rs`.
3. **Compliance is a template lock, not a checklist.** `assert_compliant()` errors and blocks the send if RG disclosure, related-party disclosure, or banned-phrase lint fails.
4. **Beehiiv is the tier source of truth.** Bot syncs from it; on sync failure, degrade to last-known tier, never upgrade silently.
5. **The Record is immutable.** Every published pick gets graded and revealed — losses included, always. `INSERT OR IGNORE` on the primary key: re-runs cannot rewrite history.
6. **Opt-in audience only.** No purchased or scraped lists, ever.

## Structure

```
crates/
  oddsports-shared/    tiers, types, compliance blocks + lint, tracked links
  oddsports-pipeline/  daily run: ingest → deterministic model → tiered LLM prose → SQLite cache
                       + post-hoc grading ("The Record")
  oddsports-bot/       teloxide Telegram bot, tier-gated commands, serves from cache
docs/                  handoff plan, acquisition checklist
```

## Setup

```bash
cp .env.example .env          # fill in tokens/keys
cargo build --release
cargo run -p oddsports-pipeline   # generates today's slate into data/oddsports.sqlite
cargo run -p oddsports-bot        # starts the bot (long-poll dev mode)
cargo test                        # compliance lint tests etc.
```

## Status

Scaffold — interfaces and control flow are real; these need filling in:
- Real projections per sport in `model.rs` (v1 is book-consensus deviation only)
- `fetch_final_scores` (odds API scores endpoint) for grading
- Beehiiv API post creation (send path) + transactional email for `/verify` tokens
- `/why`, `/line`, `/raw` game matching in the bot
- Channel auto-kick job on tier lapse; webhook mode for prod
- Odds API account + affiliate program signups
