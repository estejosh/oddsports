//! Beehiiv sync — Beehiiv is the tier source of truth (PRD 5.1).
//! Account linking: /link <email> → magic token emailed → /verify <token>.
//! Desync tolerance (PRD 5.3 #4): on sync failure, keep last-known tier,
//! NEVER upgrade silently.

use anyhow::Result;
use chrono::{Duration, Utc};
use oddsports_pipeline::store::upsert_subscriber;
use oddsports_shared::{Subscriber, Tier};
use rand::Rng;
use rusqlite::{params, Connection, OptionalExtension};
use std::env;

pub async fn fetch_beehiiv_tier(email: &str) -> Result<Option<(String, Tier)>> {
    let (Ok(key), Ok(pub_id)) = (env::var("BEEHIIV_API_KEY"), env::var("BEEHIIV_PUBLICATION_ID")) else {
        tracing::warn!("beehiiv not configured");
        return Ok(None);
    };
    let url = format!(
        "https://api.beehiiv.com/v2/publications/{pub_id}/subscriptions/by_email/{}",
        urlencoding_simple(email)
    );
    let res = reqwest::Client::new()
        .get(&url)
        .bearer_auth(key)
        .send()
        .await?;
    if !res.status().is_success() {
        return Ok(None);
    }
    let data: serde_json::Value = res.json().await?;
    let tier_name = data["data"]["subscription_tier"].as_str().unwrap_or("free");
    let beehiiv_id = data["data"]["id"].as_str().unwrap_or(email).to_string();
    Ok(Some((beehiiv_id, Tier::from_beehiiv_name(tier_name))))
}

/// Create a link token for magic-link email verification.
pub fn create_link_token(db: &Connection, email: &str) -> Result<String> {
    let token: String = {
        let mut rng = rand::thread_rng();
        (0..32).map(|_| format!("{:x}", rng.gen_range(0..16u8))).collect()
    };
    let expires = (Utc::now() + Duration::minutes(15)).to_rfc3339();
    db.execute(
        "INSERT OR REPLACE INTO link_tokens (token, email, expires_at) VALUES (?1, ?2, ?3)",
        params![token, email, expires],
    )?;
    // TODO(fable): actually send the token via Beehiiv transactional email or SES.
    tracing::info!(email, token, "link token created (send via email in prod!)");
    Ok(token)
}

/// Verify a token and bind the Telegram user. Returns bound tier or None.
pub async fn verify_link_token(
    db: &Connection,
    token: &str,
    telegram_user_id: i64,
) -> Result<Option<Tier>> {
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT email, expires_at FROM link_tokens WHERE token = ?1",
            params![token],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let Some((email, expires_at)) = row else { return Ok(None) };
    if chrono::DateTime::parse_from_rfc3339(&expires_at)? < Utc::now() {
        return Ok(None);
    }
    db.execute("DELETE FROM link_tokens WHERE token = ?1", params![token])?;

    let beehiiv = fetch_beehiiv_tier(&email).await?;
    let (beehiiv_id, tier) = beehiiv.unwrap_or_else(|| (email.clone(), Tier::Free));
    upsert_subscriber(
        db,
        &Subscriber {
            beehiiv_id,
            email,
            tier,
            telegram_user_id: Some(telegram_user_id),
            bankroll_usd: None,
            linked_at: Some(Utc::now().to_rfc3339()),
        },
    )?;
    Ok(Some(tier))
}

/// Daily re-sync of all linked subscribers. Downgrade + auto-kick from paid
/// channels on lapse handled by caller. On API failure: keep last-known tier.
#[allow(dead_code)] // called from the Phase 2 sync cron, not yet wired
pub async fn daily_tier_sync(db: &Connection) -> Result<()> {
    let emails: Vec<String> = db
        .prepare("SELECT email FROM subscribers WHERE telegram_user_id IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;

    for email in emails {
        match fetch_beehiiv_tier(&email).await {
            Ok(Some((_, tier))) => {
                db.execute(
                    "UPDATE subscribers SET tier = ?1 WHERE email = ?2",
                    params![tier as u8, email],
                )?;
            }
            // API failure / not found → keep last-known tier, per PRD desync tolerance.
            Ok(None) => {}
            Err(e) => tracing::warn!(email, error = %e, "sync failed — keeping last-known tier"),
        }
    }
    Ok(())
}

fn urlencoding_simple(s: &str) -> String {
    s.replace('@', "%40").replace('+', "%2B")
}
