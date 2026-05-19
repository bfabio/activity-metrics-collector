use anyhow::{anyhow, Result};
use std::time::Duration as StdDuration;

pub mod github;
pub mod gitlab;

/// Sends a request, retrying on rate-limit responses (HTTP 429, or GitHub's
/// 403 with x-ratelimit-remaining: 0), waiting per Retry-After or
/// x-ratelimit-reset before each retry.
pub async fn send_with_retry(req: reqwest::RequestBuilder, max_retries: u32) -> Result<reqwest::Response> {
    let mut attempt = 0;
    loop {
        let this = req
            .try_clone()
            .ok_or_else(|| anyhow!("request not cloneable"))?;
        let resp = this.send().await?;
        let status = resp.status().as_u16();
        let remaining_zero = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            == Some("0");
        let rate_limited = status == 429 || (status == 403 && remaining_zero);

        if !rate_limited || attempt >= max_retries {
            return Ok(resp);
        }

        let wait = retry_after_secs(resp.headers()).unwrap_or(60);
        eprintln!("rate limited, waiting {wait}s (attempt {})", attempt + 1);
        tokio::time::sleep(StdDuration::from_secs(wait)).await;
        attempt += 1;
    }
}

fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    if let Some(ra) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
    {
        return Some(ra);
    }
    let reset = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    Some((reset - now).max(1) as u64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialMetrics {
    pub stars: u64,
    pub forks: u64,
    pub issues_open: u64,
    pub issues_closed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    GitHub,
    GitLab,
}

#[async_trait::async_trait]
pub trait Forge: Send + Sync {
    async fn social(&self, full_name: &str) -> Result<SocialMetrics>;
}

pub fn resolve_kind(host: &str, gitlab_hosts: &[String]) -> Option<ForgeKind> {
    if host == "github.com" {
        return Some(ForgeKind::GitHub);
    }
    if host == "gitlab.com" || gitlab_hosts.iter().any(|h| h == host) {
        return Some(ForgeKind::GitLab);
    }
    None
}
