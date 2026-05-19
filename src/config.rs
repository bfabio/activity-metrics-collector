use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "activity-metrics-collector")]
pub struct Config {
    #[arg(long, env = "SOFTWARE_CATALOG_API_BASE_URL")]
    pub api_base_url: String,

    #[arg(long, env = "SOFTWARE_CATALOG_API_BEARER_TOKEN")]
    pub api_token: String,

    #[arg(long, env = "GITHUB_TOKEN")]
    pub github_token: Option<String>,

    #[arg(long, env = "GITLAB_TOKEN")]
    pub gitlab_token: Option<String>,

    #[arg(long, default_value_t = 180)]
    pub recent_days: u32,

    #[arg(long, default_value_t = 4)]
    pub concurrency: usize,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    #[arg(long, value_delimiter = ',')]
    pub gitlab_hosts: Vec<String>,
}
