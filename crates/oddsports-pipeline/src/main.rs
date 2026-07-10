use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok(); // load .env if present; real env vars win
    tracing_subscriber::fmt::init();
    match std::env::args().nth(1).as_deref() {
        // `oddsports-pipeline snapshot` — every 15–30 min via timer (steam + CLV data).
        Some("snapshot") => {
            let rows = oddsports_pipeline::run_snapshot().await?;
            println!("snapshot: {rows} line rows saved");
        }
        // `oddsports-pipeline grade` — grading catch-up only; no slate
        // generation, no AI spend. Safe to run any time.
        Some("grade") => {
            let db = oddsports_pipeline::store::open_db()?;
            let today = oddsports_pipeline::today().format("%Y-%m-%d").to_string();
            let dates = oddsports_pipeline::run_grading(&db, &today).await?;
            println!("graded catch-up over {} slate date(s)", dates.len());
        }
        // Default: full daily generation pass.
        _ => {
            let slate = oddsports_pipeline::run_daily_pipeline(oddsports_pipeline::today()).await?;
            println!(
                "slate {}: {} blocks, ${:.4} AI cost ({} in / {} out tokens)",
                slate.date,
                slate.picks.len(),
                slate.generation.cost_usd,
                slate.generation.input_tokens,
                slate.generation.output_tokens
            );
        }
    }
    Ok(())
}
