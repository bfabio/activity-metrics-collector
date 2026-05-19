use super::{Forge, SocialMetrics};
use anyhow::{anyhow, Result};
use reqwest::Client;

pub struct GitHub {
    client: Client,
    base: String,
    token: Option<String>,
}

impl GitHub {
    pub fn new(client: Client, base: String, token: Option<String>) -> Self {
        Self {
            client,
            base,
            token,
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let mut req = self
            .client
            .get(url)
            .header("User-Agent", "activity-metrics-collector")
            .header("Accept", "application/vnd.github+json")
            .query(query);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = super::send_with_retry(req, 3).await?;
        if !resp.status().is_success() {
            return Err(anyhow!("github {} returned {}", url, resp.status()));
        }
        Ok(resp.json::<T>().await?)
    }
}

#[derive(serde::Deserialize)]
struct Repo {
    stargazers_count: u64,
    forks_count: u64,
    open_issues_count: u64,
}

#[derive(serde::Deserialize)]
struct Search {
    total_count: u64,
}

#[async_trait::async_trait]
impl Forge for GitHub {
    async fn social(&self, full_name: &str) -> Result<SocialMetrics> {
        // open_issues_count also counts open pull requests; stored raw, the UI decides.
        let repo: Repo = self
            .get_json(&format!("{}/repos/{}", self.base, full_name), &[])
            .await?;

        let closed_q = format!("repo:{full_name} type:issue state:closed");
        let search: Search = self
            .get_json(
                &format!("{}/search/issues", self.base),
                &[("q", closed_q.as_str()), ("per_page", "1")],
            )
            .await?;

        Ok(SocialMetrics {
            stars: repo.stargazers_count,
            forks: repo.forks_count,
            issues_open: repo.open_issues_count,
            issues_closed: search.total_count,
        })
    }
}
