use super::Cache;
use time::{Date, Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMetrics {
    pub contributors: u64,
    pub commits_all_time: u64,
    pub pull_requests_all_time: u64,
    pub commits_recent: u64,
    pub pull_requests_recent: u64,
    pub releases: u64,
    pub oldest_commit: Date,
}

pub fn derive(cache: &Cache, now: Date, recent_days: u32) -> GitMetrics {
    let cutoff = now - Duration::days(recent_days as i64);

    let mut commits_all_time = 0u64;
    let mut pull_requests_all_time = 0u64;
    let mut commits_recent = 0u64;
    let mut pull_requests_recent = 0u64;

    let mut cur = cache.first_entry;
    for e in &cache.entries {
        cur = cur + Duration::days(e.delta as i64);
        commits_all_time += e.commits as u64;
        // merge commits in the git history stand in for merged pull requests
        pull_requests_all_time += e.merges as u64;
        if cur >= cutoff {
            commits_recent += e.commits as u64;
            pull_requests_recent += e.merges as u64;
        }
    }

    let releases = cache.tags.iter().map(|t| t.count as u64).sum();

    GitMetrics {
        contributors: cache.authors.len() as u64,
        commits_all_time,
        pull_requests_all_time,
        commits_recent,
        pull_requests_recent,
        releases,
        oldest_commit: cache.oldest_commit,
    }
}
