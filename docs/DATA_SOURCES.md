# OddSports Data Sources

This document lists every data source the OddSports codebase calls, their licensing terms, and any required attribution or usage restrictions.

## Odds and Lines

| Source | Purpose | API | Docs | Attribution Required | Redistribution Allowed | Unverified |
|--------|---------|-----|------|----------------------|----------------------|------------|
| **The Odds API** | Odds, lines, schedules for all major sports | the-odds-api.com | https://the-odds-api.com/docs/ | Yes; must link to their site | Yes, non-commercial only | ✓ |

### The Odds API — Details

- **Terms link:** https://the-odds-api.com/terms.html
- **Attribution:** Must include a link back to the-odds-api.com when displaying odds
- **Redistribution:** Free tier allows non-commercial redistribution; paid tiers may have broader rights
- **Rate limits:** Free tier ~500 requests/month; production use requires a paid account
- **Current usage:** ODDS_API_KEY and ODDS_API_BASE environment variables (default: free tier endpoint)
- **Replacement plan:** HoneMesh sports oracle v2 (docs/SPORTS_ORACLE.md) will phase this out

---

## Final Scores and Results

| Source | Purpose | API | Docs | Attribution Required | Redistribution Allowed | Unverified |
|--------|---------|-----|------|----------------------|----------------------|------------|
| **ESPN Public API** | Final scores and game results | site.api.espn.com | https://www.espn.com/ | No explicit attribution required (public endpoint) | Yes (terms of service) | ✓ |

### ESPN Public API — Details

- **Endpoint:** `https://site.api.espn.com/apis/site/v2/sports/{league}/scoreboard?dates={date}`
- **Attribution:** No explicit attribution requirement in public API terms
- **Redistribution:** Covered under ESPN's Terms of Use; non-commercial, non-business use is generally permitted
- **Coverage:** NFL, NBA, MLB, NHL, EPL (England Division 1), UEFA Champions League
- **Limitations:** Fight sports (MMA, Boxing) not available via public scoreboard endpoint
- **Usage:** `crates/oddsports-pipeline/src/scores.rs` — fetch_public_scores()
- **Fallback behavior:** If ESPN is unavailable, grading is deferred; no error is raised

---

## Tier Sync and Audience Data

| Source | Purpose | API | Docs | Attribution Required | Redistribution Allowed | Unverified |
|--------|---------|-----|------|----------------------|----------------------|------------|
| **Beehiiv API** | Newsletter tier memberships and audience data | Beehiiv.com | https://www.beehiiv.com/api | Yes; user privacy policy | Internal use only | ✓ |

### Beehiiv API — Details

- **Purpose:** Sync subscriber tiers from newsletter to Telegram bot for gating commands
- **Auth:** Bearer token (BEEHIIV_API_KEY)
- **Data:** Subscriber email, tier name, subscription status
- **Attribution:** Required in privacy policy and terms; OddSports is not a first-party Beehiiv property
- **Redistribution:** User data is not redistributed; only internal gating
- **Terms:** Covered under Beehiiv's Platform API Terms of Use; no redistribution of subscriber lists

---

## AI Inference (LLM Prose Generation)

| Source | Purpose | API | Docs | Attribution Required | Redistribution Allowed | Unverified |
|--------|---------|-----|------|----------------------|----------------------|------------|
| **Any OpenAI-compatible chat-completions endpoint** (local server, hosted gateway or commercial API — your choice) | LLM prose for write-ups; sourced-factor extraction | `AI_API_BASE` | Your provider's terms | Per your provider's terms | Model output is redistributed | ✓ |

### AI inference — Details

- **Provider-agnostic:** nothing in the code assumes a vendor. There are no default URLs or model names.
- **Configuration:** `AI_API_BASE`, `AI_API_KEY` (optional), `AI_MODEL_CHEAP`, `AI_MODEL_STRONG`. Older variable names (`HONE_API_BASE`/`BTCPC_API_BASE`, `HONE_API_KEY`/`BTCPC_API_KEY`, `MODEL_CHEAP`, `MODEL_STRONG`) are still read as fallbacks.
- **Unconfigured:** the pipeline falls back to deterministic template prose and still runs.
- **Billing:** whatever your provider charges; the daily token budget (`DAILY_TOKEN_BUDGET_USD`) caps spend.
- **Attribution:** check your provider's terms; the code adds none.
- **Redistribution:** generated prose is redistributed via newsletter and Telegram. Check that your provider's terms allow that.

## Factor Sources (optional)

AI-extracted factors (injuries, lineup news, ...) are only produced from documents you supply. The repo ships **no** retrieval or scraping. A deployment can drop documents in `FACTOR_CONTEXT_DIR/<game_id>.json` (`[{"url","as_of","text"}]`). Each factor must cite one of those URLs and carry a timestamp, or it is dropped. **You are responsible for each document source's licence and attribution terms** — list them here when you add one.

| Source | Purpose | Attribution Required | Redistribution Allowed |
|--------|---------|----------------------|------------------------|
| *(none configured in this repo)* | | | |

---

## Summary: Attribution in Output

### Required in every published pick or newsletter issue:
- **The Odds API:** Link or mention "Odds data via The Odds API" when displaying aggregated odds
- **ESPN:** Optional; no explicit attribution required, but mentioning "Official scores via ESPN" is recommended

### Not required but recommended:
- **AI inference:** If disclosing that analysis used AI assistance, mention it in methodology

### What we publish, and what we never do:
- We publish **our own** fair odds, the consensus agreement and the edge. We never publish an individual book's price, and the render step blocks any output that names a book that quoted the game (`publish::assert_no_source_odds`).
- No vendor is hardcoded. Sportsbook vendors, if any, come from the deployment's `VENDORS_FILE`, are listed alphabetically with equal prominence, and carry code-generated commission disclosure (every vendor is neutral; no affiliated vendors). Record each vendor's programme terms in your deployment's own notes.
- Never publish API keys or endpoint URLs, subscriber data or lists, or AI provider credentials.
