use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let slate = oddsports_pipeline::run_daily_pipeline(oddsports_pipeline::today()).await?;
    println!(
        "slate {}: {} blocks, ${:.4} AI cost ({} in / {} out tokens)",
        slate.date,
        slate.picks.len(),
        slate.generation.cost_usd,
        slate.generation.input_tokens,
        slate.generation.output_tokens
    );
    Ok(())
}
