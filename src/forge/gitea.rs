use super::{Forge, ForgeMetrics};
use anyhow::{anyhow, Result};
use reqwest::Client;
use time::Date;

/// Pages of pull requests walked when counting the recent ones. The
/// list endpoints have no created-after filter, so the collector reads
/// newest first and stops at the cutoff. 20 pages of 50 cover any
/// repository that opens fewer than 1000 pull requests per window.
/// Pull requests are read through the issues endpoint: /pulls computes
/// diff stats for every item and is an order of magnitude slower.
const MAX_PR_PAGES: u32 = 20;
const PR_PAGE: u32 = 50;

pub struct Gitea {
    client: Client,
    base: String,
}

impl Gitea {
    pub fn new(client: Client, base: String) -> Self {
        Self { client, base }
    }

    fn req(&self, url: &str) -> reqwest::RequestBuilder {
        self.client
            .get(url)
            .header("User-Agent", "activity-metrics-collector")
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response> {
        let resp = super::send_with_retry(self.req(url), 3).await?;
        if !resp.status().is_success() {
            return Err(anyhow!("gitea {} returned {}", url, resp.status()));
        }
        Ok(resp)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        Ok(self.get(url).await?.json::<T>().await?)
    }

    /// Total number of items behind a paginated list, read from the
    /// `X-Total-Count` header that Gitea sets on list endpoints.
    async fn count(&self, url: &str) -> Result<u64> {
        let resp = self.get(url).await?;
        Ok(resp
            .headers()
            .get("x-total-count")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0))
    }

    async fn recent_pull_requests(&self, full_name: &str, cutoff: &str) -> Result<u64> {
        let mut n = 0u64;
        for page in 1..=MAX_PR_PAGES {
            let prs: Vec<Pull> = self
                .get_json(&format!(
                    "{}/api/v1/repos/{}/issues?type=pulls&state=all&limit={}&page={}",
                    self.base, full_name, PR_PAGE, page
                ))
                .await?;
            let recent = prs
                .iter()
                .filter(|p| p.created_at.as_str() >= cutoff)
                .count() as u64;
            n += recent;
            if recent < prs.len() as u64 || prs.len() < PR_PAGE as usize {
                break;
            }
        }
        Ok(n)
    }
}

#[derive(serde::Deserialize)]
struct Repo {
    stars_count: u64,
    forks_count: u64,
    open_issues_count: u64,
}

#[derive(serde::Deserialize)]
struct Pull {
    created_at: String,
}

#[async_trait::async_trait]
impl Forge for Gitea {
    async fn metrics(&self, full_name: &str, recent_cutoff: Date) -> Result<ForgeMetrics> {
        let repo: Repo = self
            .get_json(&format!("{}/api/v1/repos/{}", self.base, full_name))
            .await?;
        let issues_closed = self
            .count(&format!(
                "{}/api/v1/repos/{}/issues?state=closed&type=issues&limit=1",
                self.base, full_name
            ))
            .await?;
        let pull_requests_all_time = self
            .count(&format!(
                "{}/api/v1/repos/{}/issues?type=pulls&state=all&limit=1",
                self.base, full_name
            ))
            .await?;
        let cutoff = format!(
            "{:04}-{:02}-{:02}",
            recent_cutoff.year(),
            u8::from(recent_cutoff.month()),
            recent_cutoff.day()
        );
        let pull_requests_recent = self.recent_pull_requests(full_name, &cutoff).await?;

        Ok(ForgeMetrics {
            stars: repo.stars_count,
            forks: repo.forks_count,
            issues_open: repo.open_issues_count,
            issues_closed,
            pull_requests_all_time,
            pull_requests_recent: Some(pull_requests_recent),
        })
    }
}
