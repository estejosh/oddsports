# Fable Handoff — Build Order

Work through phases in order. PRD.md is the spec; this is the execution sequence.
Stack: Rust workspace (`crates/`) — teloxide bot, pipeline binary, rusqlite, reqwest.

## Phase 1 — make the pipeline real
1. Sign up for an odds API (the-odds-api.com free tier to start) → fill `ODDS_API_KEY`.
2. Run `cargo run -p oddsports-pipeline` end-to-end; verify slate lands in SQLite with compliance blocks.
2b. Schedule `oddsports-pipeline snapshot` every 20 min (systemd timer / cron). This feeds
    steam detection and closing-line capture — the record's CLV is only as honest as this
    job's uptime. Watch the odds API request quota: 8 sports × 3/hr ≈ 550 requests/day.
3. Replace the consensus-deviation model in `crates/oddsports-pipeline/src/model.rs` with per-sport projections (start NFL/NBA). Keep the `ModelOutput` struct stable.
4. Add Beehiiv API post creation: pipeline output → draft post per tier → **human review → send** (review gate stays ON at launch).
5. Set up Beehiiv publication with 4 tiers named exactly `free/starter/analyst/sharp` (must match `Tier::from_beehiiv_name` in `crates/oddsports-shared/src/tiers.rs`).

## Phase 2 — bot to production
1. BotFather bot + channels (1 public free, 3 private paid) → fill env IDs.
2. Transactional email for `/verify` tokens (Beehiiv transactional or SES).
3. Implement `/why`, `/line`, `/raw` with fuzzy game-name matching.
4. Daily `dailyTierSync` cron + auto-kick from paid channels on lapse.
5. Switch long-poll → webhook; deploy (Railway/Fly/VPS).

## Phase 3 — monetization breadth
1. Apply to affiliate programs; add real templates to `AFFILIATE_TEMPLATES` in `packages/shared/src/links.ts`.
2. Betchu referral link format + attribution dashboard (clicks → signups → first deposit).
3. The Record (P0 #11): implement `fetch_final_scores` (odds API scores endpoint) in `grading.rs`, add `Sport` to `PickBlock`, publish the daily reveal to the free channel + all newsletter tiers. Rule: every published pick gets graded and revealed — losses included, reveals immutable. (Settlement already uses structured `pick_team`/`picked_line` — no string parsing.)
4. Public web page for The Record (rolling performance, per-sport splits) — the acquisition landing page.
4. Sharp live alerts: template-driven from line-move triggers (no LLM in alert path).

## Hard gates (do not launch without)
- [ ] Legal sign-off: jurisdictions, related-party disclosure wording, Betchu-list emailing
- [ ] `assertCompliant` wired into EVERY send path (email + bot broadcast)
- [ ] Token budget alerting live (`DAILY_TOKEN_BUDGET_USD`)
- [ ] Human review gate on daily sends
- [ ] Age/jurisdiction attestation at Beehiiv signup and Telegram channel join

## Token budget guardrails (recap of PRD §6)
- Generation cost scales with games, never users. If a change makes cost scale with subscribers, it's wrong.
- Haiku-class for Free/Starter prose, Sonnet-class only Analyst/Sharp. A/B whether Haiku suffices for Analyst too.
- Shared cached system prompt across all prose calls; use the Batch API for the daily run once volume justifies it.
- On budget exhaustion: template-fallback prose, drop lowest-edge games first, never drop compliance blocks.
