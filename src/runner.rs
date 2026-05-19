use crate::aggregate::catalog_analysis;
use crate::catalog_client::{CatalogClient, Software};
use crate::config::Config;
use crate::forge::{github::GitHub, gitlab::GitLab, resolve_kind, Forge, ForgeKind};
use crate::gitcache::{
    build::read_or_build,
    derive::derive,
    paths::{cache_path, cache_root},
};
use crate::metrics::SoftwareMetrics;
use anyhow::{anyhow, Result};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::collections::BTreeMap;
use std::path::PathBuf;
use time::OffsetDateTime;
use url::Url;

pub struct Summary {
    pub processed: usize,
    pub failed: usize,
}

struct Collector {
    cfg: Config,
    http: Client,
    cache_root: PathBuf,
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
        let parsed = Url::parse(&sw.url.url)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("no host in {}", sw.url.url))?
            .to_string();
        let full_name = parsed
            .path()
            .trim_matches('/')
            .trim_end_matches(".git")
            .to_string();

        let path = cache_path(&self.cache_root, &host, &full_name);
        let cache = read_or_build(&path, &sw.url.url)?;
        let now = OffsetDateTime::now_utc().date();
        let git = derive(&cache, now, self.cfg.recent_days);

        let social = match self.forge_for(&host) {
            Some(forge) => forge.social(&full_name).await.ok(),
            None => None,
        };

        Ok(SoftwareMetrics {
            git,
            social,
            recent_days: self.cfg.recent_days,
        })
    }
}

pub async fn run(cfg: Config) -> Result<Summary> {
    let api = CatalogClient::new(Client::new(), cfg.api_base_url.clone(), cfg.api_token.clone());
    let collector = Collector {
        cfg: cfg.clone(),
        http: Client::new(),
        cache_root: cache_root(),
    };

    let software = api.list_software().await?;
    let total = software.len();

    let collected: Vec<(Option<String>, SoftwareMetrics)> = stream::iter(software)
        .map(|sw| {
            let collector = &collector;
            let api = &api;
            let dry_run = cfg.dry_run;
            async move {
                match collector.collect(&sw).await {
                    Ok(m) => {
                        let body = serde_json::json!({ "activity": m.to_namespace() });
                        if dry_run {
                            println!("PATCH /software/{}/analysis {}", sw.id, body);
                        } else if let Err(e) = api.patch_software_analysis(&sw.id, &body).await {
                            eprintln!("patch {} failed: {e}", sw.id);
                            return None;
                        }
                        Some((sw.catalog_id.clone(), m))
                    }
                    Err(e) => {
                        eprintln!("collect {} failed: {e}", sw.id);
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
        by_catalog.entry(cat).or_default().push(m);
    }

    let root_id = api.resolve_root_catalog_id().await.ok();

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
