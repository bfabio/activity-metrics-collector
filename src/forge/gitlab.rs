use super::{Forge, ForgeMetrics};
use anyhow::{anyhow, Result};
use reqwest::Client;
use time::Date;

pub struct GitLab {
    client: Client,
    base: String,
    token: Option<String>,
}

impl GitLab {
    pub fn new(client: Client, base: String, token: Option<String>) -> Self {
        Self {
            client,
            base,
            token,
        }
    }

    fn req(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url).header("User-Agent", "activity-metrics-collector");
        if let Some(t) = &self.token {
            req = req.header("PRIVATE-TOKEN", t);
        }
        req
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = super::send_with_retry(self.req(url), 3).await?;
        if !resp.status().is_success() {
            return Err(anyhow!("gitlab {} returned {}", url, resp.status()));
        }
        Ok(resp.json::<T>().await?)
    }

    /// Total number of items behind a paginated list, read from the
    /// `X-Total` header that GitLab sets on offset-paginated endpoints.
    async fn count(&self, url: &str) -> Result<u64> {
        let resp = super::send_with_retry(self.req(url), 3).await?;
        if !resp.status().is_success() {
            return Err(anyhow!("gitlab {} returned {}", url, resp.status()));
        }
        Ok(resp
            .headers()
            .get("x-total")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0))
    }
}

#[derive(serde::Deserialize)]
struct Project {
    star_count: u64,
    forks_count: u64,
}

#[derive(serde::Deserialize)]
struct IssueStats {
    statistics: Stats,
}

#[derive(serde::Deserialize)]
struct Stats {
    counts: Counts,
}

#[derive(serde::Deserialize)]
struct Counts {
    opened: u64,
    closed: u64,
}

#[async_trait::async_trait]
impl Forge for GitLab {
    async fn metrics(&self, full_name: &str, recent_cutoff: Date) -> Result<ForgeMetrics> {
        let encoded = full_name.replace('/', "%2F");
        let project: Project = self
            .get_json(&format!("{}/api/v4/projects/{}", self.base, encoded))
            .await?;
        let stats: IssueStats = self
            .get_json(&format!(
                "{}/api/v4/projects/{}/issues_statistics",
                self.base, encoded
            ))
            .await?;

        let pull_requests_all_time = self
            .count(&format!(
                "{}/api/v4/projects/{}/merge_requests?state=all&per_page=1",
                self.base, encoded
            ))
            .await?;
        let after = format!(
            "{:04}-{:02}-{:02}T00:00:00Z",
            recent_cutoff.year(),
            u8::from(recent_cutoff.month()),
            recent_cutoff.day()
        );
        let pull_requests_recent = self
            .count(&format!(
                "{}/api/v4/projects/{}/merge_requests?state=all&created_after={}&per_page=1",
                self.base, encoded, after
            ))
            .await?;

        Ok(ForgeMetrics {
            stars: project.star_count,
            forks: project.forks_count,
            issues_open: stats.statistics.counts.opened,
            issues_closed: stats.statistics.counts.closed,
            pull_requests_all_time,
            pull_requests_recent: Some(pull_requests_recent),
        })
    }
}
