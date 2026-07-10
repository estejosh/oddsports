# Sports Data Oracle — de-externalizing the data path

Decision (2026-07-10): OddSports moves off third-party sports-data APIs in
phases, ending with a decentralized oracle on HoneMesh. No Betchu feed in the
data path — OddSports stays independent of any single company, and the oracle
stays hard to shut down.

## Why

- the-odds-api free tier: 500 req/month, ~3-day score history. July 3's picks
  are permanently unsettled because no run happened inside that window.
- Single API key = single point of failure/shutdown/censorship.
- HoneMesh already has the machinery this needs: roles, signed readings,
  epoch consensus, demand-driven reward pools, query fees (Verasens pattern).

## Phases

### v0 — in-process public-source fetcher (SHIPPED, `pipeline/src/scores.rs`)

Fetch final scores from public endpoints, no accounts or keys:

- ESPN public scoreboard JSON (universal): `site.api.espn.com/apis/site/v2/sports/{path}/scoreboard?dates=YYYYMMDD`
- History goes back years → grading is backfillable forever.
- Cross-check rule: when the odds API *and* a public source both have a game
  and disagree on the final score, the pick is left unsettled and a warning
  logged. Never grade from contradicting sources.
- Fight sports (MMA/boxing) excluded in v0 — no spread settlement anyway.

This module is written to be lifted verbatim into a HoneMesh node role: pure
fetch → normalize → (team names, date) → score. No OddSports types in its
public surface beyond `Sport`.

### v1 — HoneMesh `sports` role (testnet)

- New node role alongside clock/sensor/storage: fetch the defined source set
  from the node's own vantage point each epoch, submit a signed
  `SportsReading { feed_id, event_key, kind, value, source_id, fetched_at }`.
- `event_key` = normalized `(league, date, home, away)` — no proprietary ids.
- Consensus: a fact settles when K distinct accounts on distinct hardware
  fingerprints agree within tolerance inside a window; outliers slashed via
  the Verasens anti-fraud path.
- Distributed residential/consumer IPs → no per-IP rate-limit choke point,
  no single origin to block.

### v2 — paid feeds + odds/lines

- Query endpoint `/v1/sports/{feed}` billed per query in hunits,
  demand-driven reward pool (same economics as Verasens queries).
- Odds/lines join scores as feed kinds. Sources: exchanges and books with
  public APIs (e.g. Betfair exchange). NOT built on scraping books' sites —
  bot-detection arms races make that sand, not foundation.
- OddSports consumes the oracle via `HONE_API_BASE` (`/v1/sports/...`),
  becoming the feed's first paying customer; the-odds-api dependency drops
  to zero.

## OddSports integration status

- Grading: odds-api scores by game id, then public scores by (matchup, date)
  for anything unsettled — with the contradiction rule above.
- `oddsports-pipeline grade` subcommand: grading catch-up without
  regenerating (and re-paying for) today's slate.
- Ingest (lines) still the-odds-api until oracle v2 — daily run only,
  ~250 req/month, inside the free tier.
