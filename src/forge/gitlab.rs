use super::{Forge, SocialMetrics};
use anyhow::{anyhow, Result};
use reqwest::Client;

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

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let mut req = self.client.get(url).header("User-Agent", "activity-metrics-collector");
        if let Some(t) = &self.token {
            req = req.header("PRIVATE-TOKEN", t);
        }
        let resp = super::send_with_retry(req, 3).await?;
        if !resp.status().is_success() {
            return Err(anyhow!("gitlab {} returned {}", url, resp.status()));
        }
        Ok(resp.json::<T>().await?)
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
    async fn social(&self, full_name: &str) -> Result<SocialMetrics> {
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

        Ok(SocialMetrics {
            stars: project.star_count,
            forks: project.forks_count,
            issues_open: stats.statistics.counts.opened,
            issues_closed: stats.statistics.counts.closed,
        })
    }
}
