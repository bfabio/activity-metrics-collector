use super::{Forge, ForgeMetrics};
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use time::Date;

const FORGE_FRAGMENT: &str = "fragment S on Repository { \
    stargazerCount forkCount \
    openI: issues(states: OPEN) { totalCount } \
    closedI: issues(states: CLOSED) { totalCount } \
    prAll: pullRequests { totalCount } }";

/// Each repo in a batch is an aliased `repository` lookup. A smaller
/// batch keeps the query well under GitHub's GraphQL complexity limit
/// and limits how many repos lose their metrics when a chunk fails.
const FORGE_CHUNK: usize = 50;

/// Recent pull requests need a date-filtered `search`, which is far more
/// expensive than a `repository` lookup, so they run in their own smaller
/// batches to keep any single query under GitHub's complexity limit.
const SEARCH_CHUNK: usize = 20;

fn iso_date(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

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

    async fn graphql(&self, query: String, variables: Value) -> Result<Value> {
        let mut req = self
            .client
            .post(format!("{}/graphql", self.base))
            .header("User-Agent", "activity-metrics-collector")
            .json(&json!({ "query": query, "variables": variables }));
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
        Ok(body)
    }

    /// Fetches forge metrics for many repos at once via the GraphQL API.
    /// GraphQL requires a token, so without one this returns an empty map.
    /// full_name keys are echoed back unchanged.
    pub async fn metrics_batch(
        &self,
        full_names: &[String],
        recent_cutoff: Date,
    ) -> HashMap<String, ForgeMetrics> {
        if self.token.is_none() {
            eprintln!("github graphql requires a token; skipping github forge metrics");
            return HashMap::new();
        }

        let mut out = HashMap::new();
        for chunk in full_names.chunks(FORGE_CHUNK) {
            match self.fetch_chunk(chunk).await {
                Ok(m) => out.extend(m),
                Err(e) => eprintln!("github graphql batch failed: {e}"),
            }
        }

        let cutoff = iso_date(recent_cutoff);
        for chunk in full_names.chunks(SEARCH_CHUNK) {
            match self.fetch_recent_prs(chunk, &cutoff).await {
                Ok(counts) => {
                    for (full, n) in counts {
                        if let Some(m) = out.get_mut(&full) {
                            m.pull_requests_recent = Some(n);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("github graphql pr-recent unavailable ({e}); pullRequestsRecent will be 0");
                    break;
                }
            }
        }
        out
    }

    async fn fetch_chunk(&self, chunk: &[String]) -> Result<HashMap<String, ForgeMetrics>> {
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
            "query batch({}){{{}}} {FORGE_FRAGMENT}",
            decls.join(","),
            selections
        );

        let body = self.graphql(query, Value::Object(variables)).await?;
        let data = &body["data"];

        let mut out = HashMap::new();
        for (i, full) in chunk.iter().enumerate() {
            let repo = &data[format!("r{i}")];
            if repo.is_null() {
                continue;
            }
            out.insert(
                full.clone(),
                ForgeMetrics {
                    stars: repo["stargazerCount"].as_u64().unwrap_or(0),
                    forks: repo["forkCount"].as_u64().unwrap_or(0),
                    issues_open: repo["openI"]["totalCount"].as_u64().unwrap_or(0),
                    issues_closed: repo["closedI"]["totalCount"].as_u64().unwrap_or(0),
                    pull_requests_all_time: repo["prAll"]["totalCount"].as_u64().unwrap_or(0),
                    pull_requests_recent: None,
                },
            );
        }
        Ok(out)
    }

    async fn fetch_recent_prs(&self, chunk: &[String], cutoff: &str) -> Result<HashMap<String, u64>> {
        let mut selections = String::new();
        for (i, full) in chunk.iter().enumerate() {
            selections.push_str(&format!(
                "s{i}: search(query:\"repo:{full} is:pr created:>={cutoff}\",type:ISSUE,first:1){{issueCount}} "
            ));
        }
        let body = self.graphql(format!("query{{{selections}}}"), json!({})).await?;
        let data = &body["data"];

        let mut out = HashMap::new();
        for (i, full) in chunk.iter().enumerate() {
            if let Some(n) = data[format!("s{i}")]["issueCount"].as_u64() {
                out.insert(full.clone(), n);
            }
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
    async fn metrics(&self, full_name: &str, recent_cutoff: Date) -> Result<ForgeMetrics> {
        // open_issues_count also counts open pull requests; stored raw, the UI decides.
        let repo: Repo = self
            .get_json(&format!("{}/repos/{}", self.base, full_name), &[])
            .await?;

        let closed_q = format!("repo:{full_name} type:issue state:closed");
        let closed: Search = self
            .get_json(
                &format!("{}/search/issues", self.base),
                &[("q", closed_q.as_str()), ("per_page", "1")],
            )
            .await?;

        let pr_q = format!("repo:{full_name} type:pr");
        let pr_all: Search = self
            .get_json(
                &format!("{}/search/issues", self.base),
                &[("q", pr_q.as_str()), ("per_page", "1")],
            )
            .await?;

        let pr_recent_q = format!("repo:{full_name} type:pr created:>={}", iso_date(recent_cutoff));
        let pr_recent: Search = self
            .get_json(
                &format!("{}/search/issues", self.base),
                &[("q", pr_recent_q.as_str()), ("per_page", "1")],
            )
            .await?;

        Ok(ForgeMetrics {
            stars: repo.stargazers_count,
            forks: repo.forks_count,
            issues_open: repo.open_issues_count,
            issues_closed: closed.total_count,
            pull_requests_all_time: pr_all.total_count,
            pull_requests_recent: Some(pr_recent.total_count),
        })
    }
}
