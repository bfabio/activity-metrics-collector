mod aggregate;
mod catalog_client;
mod config;
mod forge;
mod gitcache;
mod metrics;
mod runner;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::Config::parse();
    let summary = runner::run(cfg).await?;
    println!("processed={} failed={}", summary.processed, summary.failed);
    Ok(())
}
