use anyhow::Result;

pub mod github;
pub mod gitlab;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialMetrics {
    pub stars: u64,
    pub forks: u64,
    pub issues_open: u64,
    pub issues_closed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    GitHub,
    GitLab,
}

#[async_trait::async_trait]
pub trait Forge: Send + Sync {
    async fn social(&self, full_name: &str) -> Result<SocialMetrics>;
}

pub fn resolve_kind(host: &str, gitlab_hosts: &[String]) -> Option<ForgeKind> {
    if host == "github.com" {
        return Some(ForgeKind::GitHub);
    }
    if host == "gitlab.com" || gitlab_hosts.iter().any(|h| h == host) {
        return Some(ForgeKind::GitLab);
    }
    None
}
