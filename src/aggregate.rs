use crate::metrics::SoftwareMetrics;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stats {
    pub max: u64,
    pub min: u64,
    pub count: u64,
    pub mean: f64,
    pub median: f64,
    pub p95: u64,
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

pub fn stats(values: &[u64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_unstable();

    let n = v.len();
    let sum: u128 = v.iter().map(|x| *x as u128).sum();
    let mean = round1(sum as f64 / n as f64);
    let median = if n.is_multiple_of(2) {
        round1((v[n / 2 - 1] + v[n / 2]) as f64 / 2.0)
    } else {
        v[n / 2] as f64
    };
    let rank = ((0.95 * n as f64).ceil() as usize).max(1) - 1;
    let p95 = v[rank.min(n - 1)];

    Some(Stats {
        max: v[n - 1],
        min: v[0],
        count: n as u64,
        mean,
        median,
        p95,
    })
}

#[derive(Serialize)]
pub struct CatalogAnalysis {
    pub v: u32,
    #[serde(rename = "softwareCount")]
    pub software_count: u64,
    #[serde(rename = "recentDays")]
    pub recent_days: u32,
    pub stats: BTreeMap<String, Stats>,
}

pub fn catalog_analysis(metrics: &[SoftwareMetrics], recent_days: u32) -> CatalogAnalysis {
    let mut cols: BTreeMap<&'static str, Vec<u64>> = BTreeMap::new();

    for m in metrics {
        cols.entry("contributors").or_default().push(m.git.contributors);
        cols.entry("commitsAllTime").or_default().push(m.git.commits_all_time);
        cols.entry("pullRequestsAllTime").or_default().push(m.git.pull_requests_all_time);
        cols.entry("commitsRecent").or_default().push(m.git.commits_recent);
        cols.entry("pullRequestsRecent").or_default().push(m.git.pull_requests_recent);
        cols.entry("releases").or_default().push(m.git.releases);
        if let Some(s) = &m.social {
            cols.entry("stars").or_default().push(s.stars);
            cols.entry("forks").or_default().push(s.forks);
            cols.entry("issuesOpen").or_default().push(s.issues_open);
            cols.entry("issuesClosed").or_default().push(s.issues_closed);
        }
    }

    let mut out = BTreeMap::new();
    for (k, vals) in cols {
        if let Some(s) = stats(&vals) {
            out.insert(k.to_string(), s);
        }
    }

    CatalogAnalysis {
        v: 1,
        software_count: metrics.len() as u64,
        recent_days,
        stats: out,
    }
}
