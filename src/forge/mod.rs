use anyhow::{anyhow, Result};
use futures::stream::StreamExt;
use std::time::Duration as StdDuration;
use time::Date;

pub mod gitea;
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
        let has_retry_after = resp.headers().contains_key("retry-after");
        let rate_limited = status == 429 || (status == 403 && (remaining_zero || has_retry_after));

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
pub enum ForgeResult {
    Unsupported,
    Failed,
    Ok(ForgeMetrics),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeMetrics {
    pub stars: u64,
    pub forks: u64,
    pub issues_open: u64,
    pub issues_closed: u64,
    pub pull_requests_all_time: u64,
    pub pull_requests_recent: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    GitHub,
    GitLab,
    /// Gitea and Forgejo share the same API.
    Gitea,
}

impl ForgeKind {
    fn label(self) -> &'static str {
        match self {
            ForgeKind::GitHub => "github",
            ForgeKind::GitLab => "gitlab",
            ForgeKind::Gitea => "gitea",
        }
    }
}

#[async_trait::async_trait]
pub trait Forge: Send + Sync {
    async fn metrics(&self, full_name: &str, recent_cutoff: Date) -> Result<ForgeMetrics>;
}

/// Resolves the hosts the collector knows without asking them anything.
/// `gitlab_hosts` short circuits the probe below, for an instance that
/// refuses the version endpoint or is unreachable from where this runs.
pub fn resolve_kind(host: &str, gitlab_hosts: &[String]) -> Option<ForgeKind> {
    if host == "github.com" {
        return Some(ForgeKind::GitHub);
    }
    if host == "gitlab.com" || gitlab_hosts.iter().any(|h| h == host) {
        return Some(ForgeKind::GitLab);
    }
    None
}

/// GitLab answers /api/v4/version on every instance, with 401 when the
/// caller is anonymous. Both the status and the JSON content type are
/// required: a plain 404 page or a forge that returns JSON errors for
/// unknown paths would otherwise pass.
fn looks_like_gitlab(status: u16, content_type: Option<&str>) -> bool {
    matches!(status, 200 | 401)
        && content_type.is_some_and(|c| c.to_ascii_lowercase().contains("json"))
}

/// Some instances (gitlab.gnome.org among them) answer 406 to a request
/// without a User-Agent, and reqwest sends none by default.
fn probe_get(http: &reqwest::Client, url: String) -> reqwest::RequestBuilder {
    http.get(url)
        .header(reqwest::header::USER_AGENT, "activity-metrics-collector")
        .timeout(StdDuration::from_secs(15))
}

async fn probe_gitlab(http: &reqwest::Client, host: &str) -> Option<ForgeKind> {
    let resp = probe_get(http, format!("https://{host}/api/v4/version"))
        .send()
        .await
        .ok()?;
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    looks_like_gitlab(resp.status().as_u16(), ct.as_deref()).then_some(ForgeKind::GitLab)
}

/// Gitea and Forgejo both serve their OpenAPI document at /swagger.v1.json
/// and name themselves in its title. Forgejo carries the Gitea API
/// unchanged, so one kind covers both.
fn looks_like_gitea(body: &str) -> bool {
    body.contains(r#""Gitea API""#) || body.contains(r#""Forgejo API""#)
}

/// How much of the swagger document to read: the title sits in the
/// first few hundred bytes and the whole file is several hundred KB.
const SWAGGER_PREFIX: usize = 1024;

async fn body_prefix(mut resp: reqwest::Response, max: usize) -> String {
    let mut buf: Vec<u8> = Vec::with_capacity(max);
    while buf.len() < max {
        match resp.chunk().await {
            Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
            _ => break,
        }
    }
    buf.truncate(max);
    String::from_utf8_lossy(&buf).into_owned()
}

async fn probe_gitea(http: &reqwest::Client, host: &str) -> Option<ForgeKind> {
    let resp = http
        .get(format!("https://{host}/swagger.v1.json"))
        .header(
            reqwest::header::RANGE,
            format!("bytes=0-{}", SWAGGER_PREFIX - 1),
        )
        .timeout(StdDuration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    looks_like_gitea(&body_prefix(resp, SWAGGER_PREFIX).await).then_some(ForgeKind::Gitea)
}

async fn probe(http: &reqwest::Client, host: &str) -> Option<ForgeKind> {
    if let Some(k) = probe_gitlab(http, host).await {
        return Some(k);
    }
    probe_gitea(http, host).await
}

/// Classifies every host once, probing only the ones not already known.
/// Self-hosted GitLab, Gitea and Forgejo are the common case in a
/// federated catalog and there is no list of instances to maintain
/// anywhere, so asking the host is the only way to avoid silently
/// dropping its metrics.
pub async fn resolve_kinds<I>(
    http: &reqwest::Client,
    hosts: I,
    gitlab_hosts: &[String],
) -> std::collections::HashMap<String, ForgeKind>
where
    I: IntoIterator<Item = String>,
{
    let mut unknown: Vec<String> = Vec::new();
    let mut known = std::collections::HashMap::new();

    for host in hosts {
        if known.contains_key(&host) || unknown.contains(&host) {
            continue;
        }
        match resolve_kind(&host, gitlab_hosts) {
            Some(kind) => {
                known.insert(host, kind);
            }
            None => unknown.push(host),
        }
    }

    let probed = futures::stream::iter(unknown.into_iter().map(|host| async move {
        let kind = probe(http, &host).await;
        (host, kind)
    }))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;

    for (host, kind) in probed {
        match kind {
            Some(k) => {
                eprintln!("detected forge: {host} speaks the {} api", k.label());
                known.insert(host, k);
            }
            None => eprintln!("no forge api at {host}, git metrics only"),
        }
    }

    known
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hosts_need_no_probe() {
        assert_eq!(resolve_kind("github.com", &[]), Some(ForgeKind::GitHub));
        assert_eq!(resolve_kind("gitlab.com", &[]), Some(ForgeKind::GitLab));
        assert_eq!(resolve_kind("example.com", &[]), None);
    }

    #[test]
    fn configured_gitlab_host_overrides_the_probe() {
        let hosts = vec!["gitlab.example.org".to_string()];
        assert_eq!(
            resolve_kind("gitlab.example.org", &hosts),
            Some(ForgeKind::GitLab)
        );
    }

    #[test]
    fn anonymous_gitlab_answers_401_json() {
        assert!(looks_like_gitlab(401, Some("application/json")));
        assert!(looks_like_gitlab(
            200,
            Some("application/json; charset=utf-8")
        ));
    }

    #[test]
    fn a_json_404_is_not_a_gitlab() {
        assert!(!looks_like_gitlab(404, Some("application/json")));
        assert!(!looks_like_gitlab(401, Some("text/html")));
        assert!(!looks_like_gitlab(401, None));
    }

    #[test]
    fn swagger_title_names_gitea_or_forgejo() {
        assert!(looks_like_gitea(
            r#"{"swagger": "2.0", "info": {"description": "This documentation describes the Gitea API.", "title": "Gitea API"}}"#
        ));
        assert!(looks_like_gitea(r#"{"info": {"title": "Forgejo API"}}"#));
    }

    #[test]
    fn other_swagger_documents_are_not_gitea() {
        assert!(!looks_like_gitea(r#"{"info": {"title": "Some API"}}"#));
        assert!(!looks_like_gitea("<html>not found</html>"));
        assert!(!looks_like_gitea(""));
    }
}
