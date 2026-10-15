# PROBLEMS.md — Oddsports

Written by an AI audit pass on 2026-09-27 while building Josh's project knowledge vault.
This file exists so the next agent (human or AI) who opens this repo sees the known
issues immediately, instead of rediscovering them. Findings came from actually
reading the source, not from the README.

## Stubbed third-party integrations
**Severity:** incomplete feature
**What we found:** The compliance lint (banned phrases, RG disclosure) and the immutable Record grading ledger are real and enforced. But the Beehiiv publishing integration is still a TODO stub — the code that would actually publish through it isn't wired up yet.
**Suggested fix:** Check the TODO markers in `crates/oddsports-pipeline/src/lib.rs` for exact locations. (Update 2026-10: the repo is now vendor-neutral — no operator, sportsbook or affiliate integration exists or is planned here.)
