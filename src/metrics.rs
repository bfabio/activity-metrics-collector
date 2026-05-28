use crate::forge::ForgeMetrics;
use crate::gitcache::derive::GitMetrics;
use serde::Serialize;
use time::macros::format_description;

#[derive(Debug, Clone)]
pub struct SoftwareMetrics {
    pub git: GitMetrics,
    pub forge: Option<ForgeMetrics>,
    pub recent_days: u32,
}

#[derive(Serialize)]
pub struct ActivityNamespace {
    pub v: u32,
    pub contributors: u64,
    #[serde(rename = "commitsAllTime")]
    pub commits_all_time: u64,
    #[serde(rename = "commitsRecent")]
    pub commits_recent: u64,
    pub tags: u64,
    #[serde(rename = "oldestCommit")]
    pub oldest_commit: String,
    #[serde(rename = "recentDays")]
    pub recent_days: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forks: Option<u64>,
    #[serde(rename = "issuesOpen", skip_serializing_if = "Option::is_none")]
    pub issues_open: Option<u64>,
    #[serde(rename = "issuesClosed", skip_serializing_if = "Option::is_none")]
    pub issues_closed: Option<u64>,
    #[serde(rename = "pullRequestsAllTime", skip_serializing_if = "Option::is_none")]
    pub pull_requests_all_time: Option<u64>,
    #[serde(rename = "pullRequestsRecent", skip_serializing_if = "Option::is_none")]
    pub pull_requests_recent: Option<u64>,
}

impl SoftwareMetrics {
    pub fn to_namespace(&self) -> ActivityNamespace {
        let fmt = format_description!("[year]-[month]-[day]");
        ActivityNamespace {
            v: 1,
            contributors: self.git.contributors,
            commits_all_time: self.git.commits_all_time,
            commits_recent: self.git.commits_recent,
            tags: self.git.tags,
            oldest_commit: self.git.oldest_commit.format(&fmt).unwrap_or_default(),
            recent_days: self.recent_days,
            stars: self.forge.as_ref().map(|f| f.stars),
            forks: self.forge.as_ref().map(|f| f.forks),
            issues_open: self.forge.as_ref().map(|f| f.issues_open),
            issues_closed: self.forge.as_ref().map(|f| f.issues_closed),
            pull_requests_all_time: self.forge.as_ref().map(|f| f.pull_requests_all_time),
            pull_requests_recent: self.forge.as_ref().map(|f| f.pull_requests_recent),
        }
    }
}
