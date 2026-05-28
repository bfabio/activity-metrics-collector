use crate::aggregate::catalog_analysis;
use crate::catalog_client::{CatalogClient, Software};
use crate::config::Config;
use crate::forge::{github::GitHub, gitlab::GitLab, resolve_kind, Forge, ForgeKind, ForgeMetrics};
use crate::gitcache::{
    build::read_or_build,
    derive::derive,
    paths::{cache_path, cache_root},
};
use crate::metrics::SoftwareMetrics;
use anyhow::{anyhow, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use time::{Duration, OffsetDateTime};
use url::Url;

pub struct Summary {
    pub processed: usize,
    pub failed: usize,
}

struct Collector {
    cfg: Config,
    http: Client,
    github_metrics: HashMap<String, ForgeMetrics>,
    cache_root: PathBuf,
}

fn parse_repo(raw: &str) -> Option<(String, String)> {
    let parsed = Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_string();
    let full_name = parsed
        .path()
        .trim_matches('/')
        .trim_end_matches(".git")
        .to_string();

    Some((host, full_name))
}

impl Collector {
    fn forge_for(&self, host: &str) -> Option<Box<dyn Forge>> {
        match resolve_kind(host, &self.cfg.gitlab_hosts)? {
            ForgeKind::GitHub => Some(Box::new(GitHub::new(
                self.http.clone(),
                "https://api.github.com".into(),
                self.cfg.github_token.clone(),
            ))),
            ForgeKind::GitLab => Some(Box::new(GitLab::new(
                self.http.clone(),
                format!("https://{host}"),
                self.cfg.gitlab_token.clone(),
            ))),
        }
    }

    async fn collect(&self, sw: &Software) -> Result<SoftwareMetrics> {
        let (host, full_name) = parse_repo(&sw.url).ok_or_else(|| anyhow!("bad url {}", sw.url))?;

        let path = cache_path(&self.cache_root, &host, &full_name);
        let cache = read_or_build(&path, &sw.url)?;
        let now = OffsetDateTime::now_utc().date();
        let git = derive(&cache, now, self.cfg.recent_days);
        let recent_cutoff = now - Duration::days(self.cfg.recent_days as i64);

        let forge = match resolve_kind(&host, &self.cfg.gitlab_hosts) {
            Some(ForgeKind::GitHub) => self.github_metrics.get(&full_name).cloned(),
            Some(ForgeKind::GitLab) => match self.forge_for(&host) {
                Some(forge) => forge.metrics(&full_name, recent_cutoff).await.ok(),
                None => None,
            },
            None => None,
        };

        Ok(SoftwareMetrics {
            git,
            forge,
            recent_days: self.cfg.recent_days,
        })
    }
}

pub async fn run(cfg: Config) -> Result<Summary> {
    let api = CatalogClient::new(Client::new(), cfg.api_base_url.clone(), cfg.api_token.clone());

    eprintln!("fetching software list from {} ...", cfg.api_base_url);
    let software = api.list_software(cfg.catalog.as_deref()).await?;
    let total = software.len();

    let github_repos: Vec<String> = software
        .iter()
        .filter_map(|sw| parse_repo(&sw.url))
        .filter(|(host, _)| resolve_kind(host, &cfg.gitlab_hosts) == Some(ForgeKind::GitHub))
        .map(|(_, full_name)| full_name)
        .collect();

    let recent_cutoff = OffsetDateTime::now_utc().date() - Duration::days(cfg.recent_days as i64);
    let github_metrics = if github_repos.is_empty() {
        HashMap::new()
    } else {
        eprintln!(
            "fetching github forge metrics for {} repos via graphql ...",
            github_repos.len()
        );
        let gh = GitHub::new(
            Client::new(),
            "https://api.github.com".into(),
            cfg.github_token.clone(),
        );
        gh.metrics_batch(&github_repos, recent_cutoff).await
    };

    let collector = Collector {
        cfg: cfg.clone(),
        http: Client::new(),
        github_metrics,
        cache_root: cache_root(),
    };

    eprintln!("collecting metrics for {total} software (concurrency={})", cfg.concurrency);
    let done = AtomicUsize::new(0);

    let collected: Vec<(Option<String>, SoftwareMetrics)> = stream::iter(software)
        .map(|sw| {
            let collector = &collector;
            let api = &api;
            let done = &done;
            let dry_run = cfg.dry_run;
            async move {
                let result = collector.collect(&sw).await;
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                match result {
                    Ok(m) => {
                        let body = serde_json::json!({ "activity": m.to_namespace() });
                        if dry_run {
                            println!("PATCH /software/{}/analysis {}", sw.id, body);
                        } else if let Err(e) = api.patch_software_analysis(&sw.id, &body).await {
                            eprintln!("[{n}/{total}] patch {} failed: {e}", sw.url);
                            return None;
                        }
                        eprintln!("[{n}/{total}] ok {}", sw.url);
                        Some((sw.catalog_id.clone(), m))
                    }
                    Err(e) => {
                        eprintln!("[{n}/{total}] collect {} failed: {e}", sw.url);
                        None
                    }
                }
            }
        })
        .buffer_unordered(cfg.concurrency)
        .collect::<Vec<Option<(Option<String>, SoftwareMetrics)>>>()
        .await
        .into_iter()
        .flatten()
        .collect();

    let processed = collected.len();
    let failed = total - processed;

    let mut by_catalog: BTreeMap<Option<String>, Vec<SoftwareMetrics>> = BTreeMap::new();
    for (cat, m) in collected {
        let key = match &cfg.catalog {
            Some(id) => Some(id.clone()),
            None => cat,
        };
        by_catalog.entry(key).or_default().push(m);
    }

    let root_id = if by_catalog.contains_key(&None) {
        api.resolve_root_catalog_id().await.ok()
    } else {
        None
    };

    for (cat, metrics) in &by_catalog {
        let analysis = catalog_analysis(metrics, cfg.recent_days);
        let body = serde_json::json!({ "activity": analysis });
        let id = match cat {
            Some(c) => c.clone(),
            None => root_id.clone().unwrap_or_else(|| "<root>".to_string()),
        };
        if cfg.dry_run {
            println!("PATCH /catalogs/{}/analysis {}", id, body);
        } else if let Err(e) = api.patch_catalog_analysis(&id, &body).await {
            eprintln!("patch catalog {id} failed: {e}");
        }
    }

    Ok(Summary { processed, failed })
}
