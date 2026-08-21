use crate::aggregate::catalog_analysis;
use crate::catalog_client::{CatalogClient, Software};
use crate::config::Config;
use crate::forge::{gitea::Gitea, github::GitHub, gitlab::GitLab, resolve_kinds, Forge, ForgeKind, ForgeMetrics, ForgeResult};
use crate::gitcache::{
    build::read_or_build,
    derive::derive,
    paths::{cache_path, cache_root},
    CacheOutcome,
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
    pub cache: CacheStats,
}

struct Collector {
    cfg: Config,
    http: Client,
    github_metrics: HashMap<String, ForgeMetrics>,
    forge_kinds: HashMap<String, ForgeKind>,
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

/// opencode.de carries synced mirrors of projects developed elsewhere.
/// The clone is identical, so git history is right either way, but the
/// mirror's own star and fork counts sit near zero while the upstream
/// repository has thousands: OpenProject reads 1 there against 15897 on
/// github. Only the forge side follows the home publiccode.yml declares.
const MIRROR_HOSTS: [&str; 1] = ["gitlab.opencode.de"];

/// The repository url at the root of publiccode.yml. Only column zero
/// counts: the nested url keys under legal, maintenance and the rest
/// point at licences and contacts, not at the code.
fn declared_repo_url(yml: &str) -> Option<String> {
    yml.lines()
        .find_map(|line| line.strip_prefix("url:"))
        .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
        .filter(|v| !v.is_empty())
}

/// Where forge metrics come from, which is the crawled repository for
/// everything except a mirror host declaring a home on another forge.
fn forge_target(sw: &Software) -> Option<(String, String)> {
    let (host, full_name) = parse_repo(&sw.url)?;
    if !MIRROR_HOSTS.contains(&host.as_str()) {
        return Some((host, full_name));
    }
    match sw
        .publiccode_yml
        .as_deref()
        .and_then(declared_repo_url)
        .and_then(|d| parse_repo(&d))
    {
        // Some entries declare a project homepage rather than a
        // repository. An owner/name path is the cheap way to tell the
        // two apart, and following a homepage would lose the metrics
        // the mirror does have.
        Some((dh, dn)) if dh != host && dn.contains('/') => Some((dh, dn)),
        _ => Some((host, full_name)),
    }
}

impl Collector {
    fn forge_for(&self, host: &str) -> Option<Box<dyn Forge>> {
        match self.forge_kinds.get(host).copied()? {
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
            ForgeKind::Gitea => Some(Box::new(Gitea::new(self.http.clone(), format!("https://{host}")))),
        }
    }

    async fn collect(&self, sw: &Software) -> Result<(SoftwareMetrics, CacheOutcome)> {
        let (host, full_name) = parse_repo(&sw.url).ok_or_else(|| anyhow!("bad url {}", sw.url))?;
        let (forge_host, forge_name) = forge_target(sw).unwrap_or((host.clone(), full_name.clone()));

        let path = cache_path(&self.cache_root, &host, &full_name);
        let (cache, outcome) = read_or_build(&path, &sw.url)?;
        let now = OffsetDateTime::now_utc().date();
        let git = derive(&cache, now, self.cfg.recent_days);
        let recent_cutoff = now - Duration::days(self.cfg.recent_days as i64);

        let forge = match self.forge_kinds.get(&forge_host).copied() {
            Some(ForgeKind::GitHub) => match self.github_metrics.get(&forge_name).cloned() {
                Some(m) => ForgeResult::Ok(m),
                None => ForgeResult::Unsupported,
            },
            Some(ForgeKind::GitLab | ForgeKind::Gitea) => match self.forge_for(&forge_host) {
                Some(f) => match f.metrics(&forge_name, recent_cutoff).await {
                    Ok(m) => ForgeResult::Ok(m),
                    Err(_) => ForgeResult::Failed,
                },
                None => ForgeResult::Unsupported,
            },
            None => ForgeResult::Unsupported,
        };

        Ok((
            SoftwareMetrics {
                git,
                forge,
                recent_days: self.cfg.recent_days,
            },
            outcome,
        ))
    }
}

pub async fn run(cfg: Config) -> Result<Summary> {
    let api = CatalogClient::new(Client::new(), cfg.api_base_url.clone(), cfg.api_token.clone());

    eprintln!("fetching software list from {} ...", cfg.api_base_url);
    let software = api.list_software(cfg.catalog.as_deref()).await?;
    let total = software.len();

    let http = Client::new();
    let forge_kinds = resolve_kinds(
        &http,
        software.iter().filter_map(forge_target).map(|(h, _)| h),
        &cfg.gitlab_hosts,
    )
    .await;

    let github_repos: Vec<String> = software
        .iter()
        .filter_map(forge_target)
        .filter(|(host, _)| forge_kinds.get(host) == Some(&ForgeKind::GitHub))
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
        http,
        github_metrics,
        forge_kinds,
        cache_root: cache_root(),
    };

    eprintln!("collecting metrics for {total} software (concurrency={})", cfg.concurrency);
    let done = AtomicUsize::new(0);

    let collected: Vec<(Option<String>, SoftwareMetrics, CacheOutcome)> = stream::iter(software)
        .map(|sw| {
            let collector = &collector;
            let api = &api;
            let done = &done;
            let dry_run = cfg.dry_run;
            async move {
                let result = collector.collect(&sw).await;
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                match result {
                    Ok((m, outcome)) => {
                        let body = serde_json::json!({ "activity": m.to_namespace() });
                        if dry_run {
                            println!("PATCH /software/{}/analysis {}", sw.id, body);
                        } else if let Err(e) = api.patch_software_analysis(&sw.id, &body).await {
                            eprintln!("[{n}/{total}] patch {} failed: {e}", sw.url);
                            return None;
                        }
                        eprintln!("[{n}/{total}] ok {}", sw.url);
                        Some((sw.catalog_id.clone(), m, outcome))
                    }
                    Err(e) => {
                        eprintln!("[{n}/{total}] collect {} failed: {e}", sw.url);
                        None
                    }
                }
            }
        })
        .buffer_unordered(cfg.concurrency)
        .collect::<Vec<Option<(Option<String>, SoftwareMetrics, CacheOutcome)>>>()
        .await
        .into_iter()
        .flatten()
        .collect();

    let processed = collected.len();
    let failed = total - processed;

    let mut cache = CacheStats::default();
    let mut by_catalog: BTreeMap<Option<String>, Vec<SoftwareMetrics>> = BTreeMap::new();
    for (cat, m, outcome) in collected {
        cache.record(&outcome);
        let key = match &cfg.catalog {
            Some(id) => Some(id.clone()),
            None => cat,
        };
        by_catalog.entry(key).or_default().push(m);
    }

    for (cat, metrics) in &by_catalog {
        let Some(id) = cat else { continue };
        let analysis = catalog_analysis(metrics, cfg.recent_days, OffsetDateTime::now_utc());
        let body = serde_json::json!({ "activity": analysis });
        if cfg.dry_run {
            println!("PATCH /catalogs/{}/analysis {}", id, body);
        } else if let Err(e) = api.patch_catalog_analysis(id, &body).await {
            eprintln!("patch catalog {id} failed: {e}");
        }
    }

    let catalog_count = by_catalog.keys().filter(|k| k.is_some()).count();
    if cfg.catalog.is_none() && catalog_count >= 2 {
        let root_id = match api.resolve_root_catalog_id().await {
            Ok(id) => Some(id),
            Err(e) => {
                eprintln!("warning: root catalog unavailable ({e}), writing per-catalog stats only (no global aggregate)");
                eprintln!(
                    "hint: create the root catalog to also get global stats:\n  curl -s -X POST {}/catalogs \\\n    -H 'Content-Type: application/json' \\\n    -H 'Authorization: Bearer <token>' \\\n    -d '{{\"name\":\"Root\",\"alternativeId\":\"\u{2205}\"}}'",
                    cfg.api_base_url
                );
                None
            }
        };
        if let Some(id) = &root_id {
            let all: Vec<SoftwareMetrics> = by_catalog.values().flatten().cloned().collect();
            let analysis = catalog_analysis(&all, cfg.recent_days, OffsetDateTime::now_utc());
            let body = serde_json::json!({ "activity": analysis });
            if cfg.dry_run {
                println!("PATCH /catalogs/{}/analysis {}", id, body);
            } else if let Err(e) = api.patch_catalog_analysis(id, &body).await {
                eprintln!("patch catalog {id} (root) failed: {e}");
            }
        }
    }

    Ok(Summary {
        processed,
        failed,
        cache,
    })
}

#[derive(Debug, Default, PartialEq)]
pub struct CacheStats {
    hit: usize,
    incremental: usize,
    cold: usize,
    noop: usize,
    bytes: u64,
}

impl CacheStats {
    fn record(&mut self, outcome: &CacheOutcome) {
        match outcome {
            CacheOutcome::Cold { bytes } => {
                self.cold += 1;
                self.bytes += bytes;
            }
            CacheOutcome::Incremental { bytes } => {
                self.incremental += 1;
                self.bytes += bytes;
            }
            CacheOutcome::Noop => self.noop += 1,
            CacheOutcome::Hit => self.hit += 1,
        }
    }
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cache: {} hit, {} incremental, {} cold, {} noop, {} fetched",
            self.hit,
            self.incremental,
            self.cold,
            self.noop,
            human_bytes(self.bytes),
        )
    }
}

fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    match bytes {
        b if b < KB => format!("{b} B"),
        b if b < MB => format!("{:.1} KB", b as f64 / KB as f64),
        b if b < GB => format!("{:.1} MB", b as f64 / MB as f64),
        b => format!("{:.1} GB", b as f64 / GB as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::{declared_repo_url, forge_target, human_bytes, CacheStats};
    use crate::catalog_client::Software;
    use crate::gitcache::CacheOutcome;

    fn sw(url: &str, yml: Option<&str>) -> Software {
        Software {
            id: "x".into(),
            url: url.into(),
            catalog_id: None,
            publiccode_yml: yml.map(str::to_string),
        }
    }

    #[test]
    fn declared_url_reads_only_the_root_key() {
        let yml = "name: Thing\nurl: https://github.com/org/repo\nlegal:\n  url: https://example.org/licence\n";
        assert_eq!(
            declared_repo_url(yml).as_deref(),
            Some("https://github.com/org/repo")
        );
    }

    #[test]
    fn declared_url_absent_or_empty_is_none() {
        assert_eq!(declared_repo_url("name: Thing\n"), None);
        assert_eq!(declared_repo_url("url:   \n"), None);
    }

    #[test]
    fn mirror_follows_the_declared_home() {
        let s = sw(
            "https://gitlab.opencode.de/org/mirror.git",
            Some("url: https://github.com/opf/openproject\n"),
        );
        assert_eq!(
            forge_target(&s),
            Some(("github.com".into(), "opf/openproject".into()))
        );
    }

    #[test]
    fn mirror_declaring_its_own_host_stays_put() {
        let s = sw(
            "https://gitlab.opencode.de/org/thing.git",
            Some("url: https://gitlab.opencode.de/org/thing\n"),
        );
        assert_eq!(
            forge_target(&s),
            Some(("gitlab.opencode.de".into(), "org/thing".into()))
        );
    }

    #[test]
    fn mirror_without_publiccode_stays_put() {
        let s = sw("https://gitlab.opencode.de/org/thing.git", None);
        assert_eq!(
            forge_target(&s),
            Some(("gitlab.opencode.de".into(), "org/thing".into()))
        );
    }

    // Only the mirror host redirects: a github repo pointing its
    // publiccode.yml elsewhere keeps its own metrics.
    #[test]
    fn a_declared_homepage_is_not_a_repository() {
        let s = sw(
            "https://gitlab.opencode.de/org/thing.git",
            Some("url: https://www.digitale-doerfer.de\n"),
        );
        assert_eq!(
            forge_target(&s),
            Some(("gitlab.opencode.de".into(), "org/thing".into()))
        );
    }

    #[test]
    fn other_hosts_never_redirect() {
        let s = sw(
            "https://github.com/org/repo.git",
            Some("url: https://gitlab.com/other/repo\n"),
        );
        assert_eq!(
            forge_target(&s),
            Some(("github.com".into(), "org/repo".into()))
        );
    }

    #[test]
    fn cache_stats_folds_outcomes() {
        let outcomes = [
            CacheOutcome::Hit,
            CacheOutcome::Hit,
            CacheOutcome::Cold { bytes: 1000 },
            CacheOutcome::Incremental { bytes: 1536 },
            CacheOutcome::Noop,
        ];

        let mut stats = CacheStats::default();
        for o in &outcomes {
            stats.record(o);
        }

        assert_eq!(
            stats,
            CacheStats {
                hit: 2,
                incremental: 1,
                cold: 1,
                noop: 1,
                bytes: 2536,
            }
        );
    }

    #[test]
    fn cache_stats_display() {
        let stats = CacheStats {
            hit: 2,
            incremental: 1,
            cold: 1,
            noop: 1,
            bytes: 2536,
        };

        assert_eq!(
            stats.to_string(),
            "cache: 2 hit, 1 incremental, 1 cold, 1 noop, 2.5 KB fetched"
        );
    }

    #[test]
    fn human_bytes_scales_by_magnitude() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5_767_168), "5.5 MB");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }
}
