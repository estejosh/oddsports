# OddSports

A vendor-neutral odds-analysis engine. It aggregates odds from public sources, removes the vig, and publishes **our own estimated fair odds** and the edge versus market consensus — via a Beehiiv newsletter and a Telegram bot. A public, immutable track record grades every published pick.

> **Analysis, not betting advice.** OddSports is independent analysis and does not endorse any sportsbook, operator or affiliate program. Nothing here is financial advice and no outcome is certain. 21+ (or legal age in your jurisdiction); gambling involves risk — 1-800-GAMBLER.

**Read [PRD.md](./PRD.md) first.** [docs/FABLE_HANDOFF.md](./docs/FABLE_HANDOFF.md) is the build order. [docs/DATA_SOURCES.md](./docs/DATA_SOURCES.md) lists every data source with its licence and attribution terms.

**License:** [Usufruct License (UFL) v2.2](./LICENSE.md), Operational Scope: **Noncommercial** (tag `UFL-N-1a`) — source-available: read it, run it, fork it, learn from it. **Commercial use is not permitted** without a separate license. Cite as `LicenseRef-UFL-2.2-N`; canonical text at [estejosh/UFL-Usufruct-License](https://github.com/estejosh/UFL-Usufruct-License).

Rust workspace — single binary per service, systemd, SQLite.

## What it does

1. **Aggregate** odds from many books and strip each book's vig (no-vig probabilities).
2. **Consensus score:** average the no-vig probabilities; measure how much the books disagree (dispersion) and turn that into a 1–5 confidence.
3. **AI layer (optional):** an LLM extracts factors (injuries, lineup news) from source documents *you supply*. Every factor must cite one of those sources and carry a timestamp, or it is dropped.
4. **Real odds score:** fair odds, nudged by a small, capped amount for sourced factors.
5. **Publish** our fair odds, confidence and edge. Never an individual book's price; a configured vendor list, if any, is shown separately and alphabetically.
6. **Track record:** every pick graded and revealed, losses included; scored with Brier score, log loss and closing-line value.

## Neutrality (hard rules)

- **No vendor is hardcoded, featured or ranked first.** Sportsbook vendors are optional, configured data: a deployment lists them in a JSON file (`VENDORS_FILE`, or `vendors.json` inside `TEMPLATE_DIR`; see [docs/vendors.example.json](./docs/vendors.example.json)). Any vendor can be one of many; none is special.
- Vendors are listed **alphabetically with equal prominence**, never next to odds, and always with code-generated disclosure: a commission notice if any vendor pays us. There is no "affiliated" vendor option: every vendor is neutral. Disclosure cannot be omitted by a template.
- With no vendors configured, output has no sportsbook names, links or CTAs at all.
- Raw per-source odds are never published. `publish::assert_no_source_odds` blocks any output that names a book that quoted the game (the vendor list itself is exempt: it is a plain list, not odds).
- Built-in templates are neutral. `TEMPLATE_DIR` can restyle output; the compliance footer, data credit and vendor disclosure are added by code, so an override cannot drop them.

## Core design rules (do not violate)

1. **Tokens are spent per slate, never per user.** One daily generation pass; the bot serves 100% from cache. No LLM calls in the bot request path.
2. **LLM writes prose, code computes numbers.** All odds math is deterministic Rust (`aggregate`, `odds_score`, `real_odds_score`, `model`, `calibration`).
3. **Compliance is a template lock, not a checklist.** `assert_compliant()` errors and blocks the send if the RG disclosure, the neutrality disclaimer, the "Analysis, not betting advice." line, or the banned-phrase lint fails.
4. **Beehiiv is the tier source of truth.** Bot syncs from it; on sync failure, degrade to last-known tier, never upgrade silently.
5. **The Record is immutable.** Every published pick gets graded and revealed — losses included, always. `INSERT OR IGNORE` on the primary key: re-runs cannot rewrite history.
6. **Opt-in audience only.** No purchased or scraped lists, ever.
7. **Provider-agnostic AI.** Any OpenAI-compatible endpoint, configured only by environment variables. No vendor defaults.

## Structure

```
crates/
  oddsports-shared/    tiers, types, compliance blocks + lint, template override loader
  oddsports-pipeline/  stages: sources -> aggregate -> odds_score -> ai_factors
                       -> real_odds_score -> publish -> track record (grading + calibration)
                       also: model (spread edges), prose (LLM wording), budget, store
  oddsports-bot/       teloxide Telegram bot, tier-gated commands, serves from cache
docs/                  handoff plan, data sources, acquisition checklist
```

## Setup

```bash
cp .env.example .env          # fill in tokens/keys
cargo build --release
cargo run -p oddsports-pipeline   # generates today's slate into data/oddsports.sqlite
cargo run -p oddsports-bot        # starts the bot (long-poll dev mode)
cargo test                        # math, compliance, neutrality and template tests
```

### Configuration highlights

| Variable | Purpose |
|---|---|
| `ODDS_API_BASE`, `ODDS_API_KEY` | Odds feed (Odds-API-shaped JSON). See docs/DATA_SOURCES.md for terms. |
| `DATA_ATTRIBUTION` | Neutral data credit appended to every pick (e.g. the feed's required attribution). |
| `AI_API_BASE`, `AI_API_KEY`, `AI_MODEL_CHEAP`, `AI_MODEL_STRONG` | Any OpenAI-compatible endpoint. Unset = deterministic template prose. |
| `AI_FACTORS_ENABLED`, `FACTOR_CONTEXT_DIR` | Turn on sourced-factor extraction; directory of `<game_id>.json` source documents. |
| `VENDORS_FILE` | Optional JSON list of sportsbook vendors (equal treatment, auto-disclosed). |
| `TEMPLATE_DIR` | Directory with `pick.md` / `bot_start.txt` overrides (see `crates/oddsports-shared/src/templates.rs`). |

## Status

Scaffold — interfaces and control flow are real; these need filling in:
- Real projections per sport in `model.rs` (v1 is market-consensus deviation only)
- `fetch_final_scores` (odds API scores endpoint) for grading; wire `calibration` into the reveal post
- Beehiiv API post creation (send path) + transactional email for `/verify` tokens
- `/why`, `/line`, `/raw` game matching in the bot
- Channel auto-kick job on tier lapse; webhook mode for prod
- A document source for `ai_factors` (the repo ships none; a deployment supplies its own and records its terms in docs/DATA_SOURCES.md)
