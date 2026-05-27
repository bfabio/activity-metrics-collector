use super::{Forge, SocialMetrics};
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const SOCIAL_FRAGMENT: &str = "fragment S on Repository { \
    stargazerCount forkCount \
    openI: issues(states: OPEN) { totalCount } \
    closedI: issues(states: CLOSED) { totalCount } }";

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

    /// Fetches social metrics for many repos at once via the GraphQL API,
    /// 100 per request. GraphQL requires a token, so without one this
    /// returns an empty map. full_name keys are echoed back unchanged.
    pub async fn social_batch(&self, full_names: &[String]) -> HashMap<String, SocialMetrics> {
        if self.token.is_none() {
            eprintln!("github graphql requires a token; skipping github social metrics");
            return HashMap::new();
        }

        let mut out = HashMap::new();
        for chunk in full_names.chunks(100) {
            match self.fetch_chunk(chunk).await {
                Ok(m) => out.extend(m),
                Err(e) => eprintln!("github graphql batch failed: {e}"),
            }
        }
        out
    }

    async fn fetch_chunk(&self, chunk: &[String]) -> Result<HashMap<String, SocialMetrics>> {
        let mut decls = Vec::new();
        let mut selections = String::new();
        let mut variables = Map::new();
        for (i, full) in chunk.iter().enumerate() {
            let Some((owner, name)) = full.split_once('/') else {
                continue;
            };
            if name.contains('/') {
                continue;
            }
            decls.push(format!("$o{i}:String!,$n{i}:String!"));
            selections.push_str(&format!("r{i}: repository(owner:$o{i},name:$n{i}){{...S}} "));
            variables.insert(format!("o{i}"), Value::from(owner));
            variables.insert(format!("n{i}"), Value::from(name));
        }
        let query = format!(
            "query batch({}){{{}}} {SOCIAL_FRAGMENT}",
            decls.join(","),
            selections
        );

        let mut req = self
            .client
            .post(format!("{}/graphql", self.base))
            .header("User-Agent", "activity-metrics-collector")
            .json(&json!({ "query": query, "variables": Value::Object(variables) }));
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }

        let resp = super::send_with_retry(req, 3).await?;
        if !resp.status().is_success() {
            return Err(anyhow!("github graphql returned {}", resp.status()));
        }
        let body: Value = resp.json().await?;
        if let Some(errors) = body.get("errors") {
            eprintln!("github graphql errors: {errors}");
        }
        let data = &body["data"];

        let mut out = HashMap::new();
        for (i, full) in chunk.iter().enumerate() {
            let repo = &data[format!("r{i}")];
            if repo.is_null() {
                continue;
            }
            out.insert(
                full.clone(),
                SocialMetrics {
                    stars: repo["stargazerCount"].as_u64().unwrap_or(0),
                    forks: repo["forkCount"].as_u64().unwrap_or(0),
                    issues_open: repo["openI"]["totalCount"].as_u64().unwrap_or(0),
                    issues_closed: repo["closedI"]["totalCount"].as_u64().unwrap_or(0),
                },
            );
        }
        Ok(out)
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
