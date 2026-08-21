use crate::forge::{ForgeMetrics, ForgeResult};
use crate::gitcache::derive::GitMetrics;
use serde::{Serialize, Serializer};
use time::macros::format_description;

pub enum MaybeNull<T> {
    Absent,
    Null,
    Value(T),
}

impl<T> MaybeNull<T> {
    pub fn is_absent(&self) -> bool {
        matches!(self, MaybeNull::Absent)
    }
}

impl<T: Serialize> Serialize for MaybeNull<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            MaybeNull::Absent | MaybeNull::Null => s.serialize_none(),
            MaybeNull::Value(v) => v.serialize(s),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SoftwareMetrics {
    pub git: GitMetrics,
    pub forge: ForgeResult,
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
    #[serde(skip_serializing_if = "MaybeNull::is_absent")]
    pub stars: MaybeNull<u64>,
    #[serde(skip_serializing_if = "MaybeNull::is_absent")]
    pub forks: MaybeNull<u64>,
    #[serde(rename = "issuesOpen", skip_serializing_if = "MaybeNull::is_absent")]
    pub issues_open: MaybeNull<u64>,
    #[serde(rename = "issuesClosed", skip_serializing_if = "MaybeNull::is_absent")]
    pub issues_closed: MaybeNull<u64>,
    #[serde(rename = "pullRequestsAllTime", skip_serializing_if = "MaybeNull::is_absent")]
    pub pull_requests_all_time: MaybeNull<u64>,
    #[serde(rename = "pullRequestsRecent", skip_serializing_if = "MaybeNull::is_absent")]
    pub pull_requests_recent: MaybeNull<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<&'static str>,
}

fn forge_field(result: &ForgeResult, f: impl Fn(&ForgeMetrics) -> u64) -> MaybeNull<u64> {
    match result {
        ForgeResult::Unsupported => MaybeNull::Absent,
        ForgeResult::Failed => MaybeNull::Null,
        ForgeResult::Ok(m) => MaybeNull::Value(f(m)),
    }
}

fn forge_opt_field(
    result: &ForgeResult,
    f: impl Fn(&ForgeMetrics) -> Option<u64>,
) -> MaybeNull<u64> {
    match result {
        ForgeResult::Unsupported => MaybeNull::Absent,
        ForgeResult::Failed => MaybeNull::Null,
        ForgeResult::Ok(m) => match f(m) {
            Some(n) => MaybeNull::Value(n),
            None => MaybeNull::Absent,
        },
    }
}

impl SoftwareMetrics {
    pub fn to_namespace(&self) -> ActivityNamespace {
        let fmt = format_description!("[year]-[month]-[day]");
        let disabled = match &self.forge {
            ForgeResult::Ok(m) if m.issues_disabled => vec!["issues"],
            _ => Vec::new(),
        };
        ActivityNamespace {
            v: 1,
            contributors: self.git.contributors,
            commits_all_time: self.git.commits_all_time,
            commits_recent: self.git.commits_recent,
            tags: self.git.tags,
            oldest_commit: self.git.oldest_commit.format(&fmt).unwrap_or_default(),
            recent_days: self.recent_days,
            stars: forge_field(&self.forge, |f| f.stars),
            forks: forge_field(&self.forge, |f| f.forks),
            issues_open: forge_opt_field(&self.forge, |f| f.issues_open),
            issues_closed: forge_opt_field(&self.forge, |f| f.issues_closed),
            pull_requests_all_time: forge_field(&self.forge, |f| f.pull_requests_all_time),
            pull_requests_recent: forge_opt_field(&self.forge, |f| f.pull_requests_recent),
            disabled,
        }
    }
}
