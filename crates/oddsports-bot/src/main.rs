//! OddSports Telegram bot. PRD Section 5.
//! ZERO LLM calls in the request path (PRD 6 #4) — every command serves
//! pre-generated slate content from SQLite or does pure data lookups.

mod beehiiv;
mod ratelimit;

use anyhow::Result;
use chrono::{Duration, Utc};
use oddsports_pipeline::grading::{build_reveal_post, rolling_record};
use oddsports_pipeline::model::scale_units_to_bankroll;
use oddsports_pipeline::store::{get_subscriber_by_telegram, load_slate, open_db, set_bankroll};
use oddsports_shared::{DailySlate, PickBlock, Tier, RG_DISCLOSURE, RELATED_PARTY_DISCLOSURE};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use teloxide::{prelude::*, utils::command::BotCommands};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    #[command(description = "onboarding + tier explanation")]
    Start,
    #[command(description = "list commands")]
    Help,
    #[command(description = "today's picks at your tier depth")]
    Today,
    #[command(description = "full slate (Starter+)")]
    Picks,
    #[command(description = "unit sizing table (Analyst+)")]
    Units,
    #[command(description = "set bankroll for dollar sizing (Sharp)")]
    Bankroll(String),
    #[command(description = "graded track record + yesterday's reveal (free)")]
    Record,
    #[command(description = "model factor breakdown for a game (Analyst+)")]
    Why(String),
    #[command(description = "line movement for a game (Analyst+)")]
    Line(String),
    #[command(description = "raw model output for a game (Sharp)")]
    Raw(String),
    #[command(description = "link your newsletter subscription")]
    Link(String),
    #[command(description = "complete linking")]
    Verify(String),
    #[command(description = "responsible gambling resources")]
    Rg,
}

/// Connection is not Sync — a Mutex is fine at bot request rates.
type Db = Arc<Mutex<rusqlite::Connection>>;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok(); // load .env if present; real env vars win
    tracing_subscriber::fmt::init();
    let token = std::env::var("TELEGRAM_BOT_TOKEN")
        .map_err(|_| anyhow::anyhow!("TELEGRAM_BOT_TOKEN is not set (see .env.example)"))?;
    let bot = Bot::new(token);
    let db: Db = Arc::new(Mutex::new(open_db()?));
    tracing::info!("bot starting (long-poll dev mode — switch to webhooks in prod)");

    Command::repl(bot, move |bot: Bot, msg: Message, cmd: Command| {
        let db = db.clone();
        async move {
            if let Err(e) = handle(&bot, &msg, cmd, db).await {
                tracing::error!(error = %e, "handler error");
                let _ = bot.send_message(msg.chat.id, "Something went wrong — try again.").await;
            }
            Ok(())
        }
    })
    .await;
    Ok(())
}

async fn handle(bot: &Bot, msg: &Message, cmd: Command, db: Db) -> Result<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    if !ratelimit::allow(user_id) {
        bot.send_message(msg.chat.id, "Slow down a little — try again in a minute. ⏳").await?;
        return Ok(());
    }
    let tier = {
        let db = db.lock().unwrap();
        get_subscriber_by_telegram(&db, user_id)?.map(|s| s.tier).unwrap_or(Tier::Free)
    };

    match cmd {
        Command::Start => {
            let text = format!(
                "🏟 *OddSports* — tiered sports betting analysis.\n\n\
                 Free: top daily picks. Paid tiers unlock the full slate, model breakdowns, \
                 line movement, unit sizing, and live alerts.\n\n\
                 Commands: /today /record /link /help /rg\n\n\
                 Subscribe: (Beehiiv signup link here)\n\n\
                 {RELATED_PARTY_DISCLOSURE}\n{RG_DISCLOSURE}"
            );
            bot.send_message(msg.chat.id, text).await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string()).await?;
        }
        Command::Rg => {
            bot.send_message(
                msg.chat.id,
                format!("{RG_DISCLOSURE}\n\nSelf-exclusion: contact your sportsbook's responsible gambling page. US helpline: 1-800-GAMBLER."),
            )
            .await?;
        }
        Command::Today => {
            let Some(slate) = today_slate(&db)? else {
                bot.send_message(msg.chat.id, "Today's slate isn't out yet — check back soon.").await?;
                return Ok(());
            };
            let picks = picks_for(&slate, tier);
            let shown: Vec<_> = if tier == Tier::Free { picks.into_iter().take(5).collect() } else { picks };
            for p in shown.iter().take(10) {
                bot.send_message(msg.chat.id, &p.body).await?;
            }
            if tier == Tier::Free {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Starter)).await?;
            }
        }
        Command::Picks => {
            if !tier.can_access(Tier::Starter) {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Starter)).await?;
                return Ok(());
            }
            let Some(slate) = today_slate(&db)? else {
                bot.send_message(msg.chat.id, "Slate not ready yet.").await?;
                return Ok(());
            };
            for p in picks_for(&slate, tier).iter().take(20) {
                bot.send_message(msg.chat.id, &p.body).await?;
            }
        }
        Command::Units => {
            if !tier.can_access(Tier::Analyst) {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Analyst)).await?;
                return Ok(());
            }
            let Some(slate) = today_slate(&db)? else {
                bot.send_message(msg.chat.id, "Slate not ready yet.").await?;
                return Ok(());
            };
            let bankroll = {
                let db = db.lock().unwrap();
                get_subscriber_by_telegram(&db, user_id)?.and_then(|s| s.bankroll_usd)
            };
            let mut lines = vec!["📏 *Today's sizing*".to_string()];
            for p in picks_for(&slate, tier) {
                let units = p.model.suggested_units;
                let scaled = match (tier >= Tier::Sharp, bankroll) {
                    (true, Some(b)) => format!(" (${})", scale_units_to_bankroll(units, b)),
                    _ => String::new(),
                };
                lines.push(format!("• {}: {units}u{scaled} {}", p.model.side, "★".repeat(p.confidence as usize)));
            }
            bot.send_message(msg.chat.id, lines.join("\n")).await?;
        }
        Command::Bankroll(arg) => {
            if !tier.can_access(Tier::Sharp) {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Sharp)).await?;
                return Ok(());
            }
            let Ok(amount) = arg.trim().parse::<f64>() else {
                bot.send_message(msg.chat.id, "Usage: /bankroll 5000 — sets your bankroll so sizing scales to it.").await?;
                return Ok(());
            };
            if amount <= 0.0 {
                bot.send_message(msg.chat.id, "Bankroll must be positive.").await?;
                return Ok(());
            }
            {
                let db = db.lock().unwrap();
                set_bankroll(&db, user_id, amount)?;
            }
            bot.send_message(
                msg.chat.id,
                format!(
                    "💰 Bankroll set to ${amount}. 1 unit = ${:.2}. /units now shows dollar sizing.\n\n{RG_DISCLOSURE}",
                    amount / 100.0
                ),
            )
            .await?;
        }
        Command::Record => {
            // FREE tier on purpose (PRD P0 #11): the graded record and yesterday's
            // full paid-depth reveal are public trust assets. Pure data, no AI.
            let (graded, reveal) = {
                let db = db.lock().unwrap();
                let rec = rolling_record(&db)?;
                let yesterday = (Utc::now().date_naive() - Duration::days(1)).format("%Y-%m-%d").to_string();
                (rec.graded, build_reveal_post(&db, &yesterday)?)
            };
            if graded == 0 {
                bot.send_message(msg.chat.id, "📊 No graded picks yet — the record starts after the first settled slate.").await?;
                return Ok(());
            }
            // Telegram message cap is 4096 chars — chunk the reveal.
            for chunk in chunk_str(&reveal, 4000) {
                bot.send_message(msg.chat.id, chunk).await?;
            }
        }
        Command::Why(query) => {
            if !tier.can_access(Tier::Analyst) {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Analyst)).await?;
                return Ok(());
            }
            match find_pick(&db, tier, &query)? {
                FindResult::One(p) => {
                    let mut out = format!("🔍 *{}* — {}\n\nModel factors:\n", p.matchup, p.model.side);
                    for f in &p.model.factors {
                        let arrow = match f.direction {
                            oddsports_shared::FactorDirection::For => "▲",
                            oddsports_shared::FactorDirection::Against => "▼",
                        };
                        out.push_str(&format!("{arrow} {} (w{:.1}): {}\n", f.name, f.weight, f.detail));
                    }
                    out.push_str(&format!(
                        "\nFair line {} vs market {} → edge {} pts\n\n{}",
                        p.model.fair_line, p.model.picked_line, p.model.edge_pct, p.risk_warning
                    ));
                    bot.send_message(msg.chat.id, out).await?;
                }
                other => bot.send_message(msg.chat.id, other.message()).await.map(|_| ())?,
            }
        }
        Command::Line(query) => {
            if !tier.can_access(Tier::Analyst) {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Analyst)).await?;
                return Ok(());
            }
            match find_pick(&db, tier, &query)? {
                FindResult::One(p) => {
                    let mut out = format!("📈 *{}* — line history:\n", p.matchup);
                    for pt in &p.model.line_history {
                        out.push_str(&format!("• {} → {}\n", &pt.at[..16.min(pt.at.len())], pt.line));
                    }
                    bot.send_message(msg.chat.id, out).await?;
                }
                other => bot.send_message(msg.chat.id, other.message()).await.map(|_| ())?,
            }
        }
        Command::Raw(query) => {
            if !tier.can_access(Tier::Sharp) {
                bot.send_message(msg.chat.id, upgrade_nudge(tier, Tier::Sharp)).await?;
                return Ok(());
            }
            match find_pick(&db, tier, &query)? {
                FindResult::One(p) => {
                    let json = serde_json::to_string_pretty(&p.model)?;
                    for chunk in chunk_str(&format!("```\n{json}\n```"), 4000) {
                        bot.send_message(msg.chat.id, chunk).await?;
                    }
                }
                other => bot.send_message(msg.chat.id, other.message()).await.map(|_| ())?,
            }
        }
        Command::Link(email) => {
            let email = email.trim();
            if !email.contains('@') {
                bot.send_message(msg.chat.id, "Usage: /link you@example.com — we'll email you a verification token.").await?;
                return Ok(());
            }
            {
                let db = db.lock().unwrap();
                beehiiv::create_link_token(&db, email)?;
            }
            bot.send_message(msg.chat.id, "📧 Check your email for a verification token, then send: /verify <token>").await?;
        }
        Command::Verify(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                bot.send_message(msg.chat.id, "Usage: /verify <token>").await?;
                return Ok(());
            }
            // Hold the lock only around DB work inside verify (it awaits Beehiiv).
            let verified = {
                let db = db.lock().unwrap();
                // NOTE: verify_link_token awaits inside — restructure if this
                // becomes contended. Fine at scaffold scale.
                futures_block(beehiiv::verify_link_token(&db, &token, user_id))?
            };
            match verified {
                Some(t) => {
                    bot.send_message(msg.chat.id, format!("✅ Linked! Your tier: *{}*.", t.name())).await?;
                }
                None => {
                    bot.send_message(msg.chat.id, "Invalid or expired token. Run /link again.").await?;
                }
            }
        }
    }
    Ok(())
}

/// Block on a future from sync context (used only where a Mutex guard can't cross await).
fn futures_block<F: std::future::Future>(fut: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut))
}

fn today_slate(db: &Db) -> Result<Option<DailySlate>> {
    let db = db.lock().unwrap();
    let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    load_slate(&db, &today)
}

/// Serve each game at the DEEPEST depth the user's tier allows.
fn picks_for(slate: &DailySlate, tier: Tier) -> Vec<PickBlock> {
    let mut by_game: HashMap<&str, &PickBlock> = HashMap::new();
    for p in &slate.picks {
        if !tier.can_access(p.min_tier) {
            continue;
        }
        by_game
            .entry(p.game_id.as_str())
            .and_modify(|cur| {
                if p.min_tier > cur.min_tier {
                    *cur = p;
                }
            })
            .or_insert(p);
    }
    by_game.into_values().cloned().collect()
}

enum FindResult {
    One(PickBlock),
    NoSlate,
    NoMatch,
    Ambiguous(Vec<String>),
}

impl FindResult {
    fn message(&self) -> String {
        match self {
            FindResult::One(_) => unreachable!("handled by caller"),
            FindResult::NoSlate => "Today's slate isn't out yet.".into(),
            FindResult::NoMatch => "No pick matches that — try part of a team name, e.g. /why chiefs".into(),
            FindResult::Ambiguous(names) => {
                format!("Multiple matches — be more specific:\n{}", names.join("\n"))
            }
        }
    }
}

/// Fuzzy game match: case-insensitive substring against "Away @ Home".
fn find_pick(db: &Db, tier: Tier, query: &str) -> Result<FindResult> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(FindResult::NoMatch);
    }
    let Some(slate) = today_slate(db)? else { return Ok(FindResult::NoSlate) };
    let matches: Vec<PickBlock> = picks_for(&slate, tier)
        .into_iter()
        .filter(|p| p.matchup.to_lowercase().contains(&query))
        .collect();
    Ok(match matches.len() {
        0 => FindResult::NoMatch,
        1 => FindResult::One(matches.into_iter().next().unwrap()),
        _ => FindResult::Ambiguous(matches.iter().map(|p| format!("• {}", p.matchup)).collect()),
    })
}

fn upgrade_nudge(have: Tier, need: Tier) -> String {
    let mut out = format!("🔒 That's a *{}* feature.\n", need.name());
    if let Some(next) = have.next() {
        out.push_str(&format!(
            "Upgrade to {} (${}/mo) unlocks: {}.\n",
            next.name(),
            next.price_usd().unwrap_or(0),
            next.unlocks().join(", ")
        ));
    }
    out.push_str("Manage your subscription from the newsletter → account page.");
    out
}

fn chunk_str(s: &str, max: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut rest = s;
    while rest.len() > max {
        // Split on a char boundary at or below max.
        let mut cut = max;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let (head, tail) = rest.split_at(cut);
        chunks.push(head);
        rest = tail;
    }
    chunks.push(rest);
    chunks
}
