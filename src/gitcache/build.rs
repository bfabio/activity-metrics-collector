use super::{codec, Cache, DayEntry, TagEntry};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use time::format_description::well_known::Iso8601;
use time::macros::format_description;
use time::{Date, Duration, OffsetDateTime, UtcOffset};

struct RawCommit {
    date: Date,
    email: String,
    is_merge: bool,
}

fn parse_iso_date(s: &str) -> Result<Date> {
    let dt = OffsetDateTime::parse(s, &Iso8601::DEFAULT).with_context(|| format!("bad date {s}"))?;
    Ok(dt.to_offset(UtcOffset::UTC).date())
}

fn run_git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .context("failed to run git")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn empty_shallow_window(stderr: &str) -> bool {
    stderr.contains("error processing shallow info")
        || stderr.contains("no commits selected for shallow requests")
}

fn parse_commits(log: &str) -> Result<Vec<RawCommit>> {
    let mut commits = Vec::new();
    for line in log.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\u{1f}');
        let _hash = parts.next();
        let date = parse_iso_date(parts.next().ok_or_else(|| anyhow!("missing date"))?)?;
        let email = parts
            .next()
            .ok_or_else(|| anyhow!("missing email"))?
            .to_string();
        let parents = parts.next().unwrap_or("").trim();
        let is_merge = parents.split_whitespace().count() > 1;
        commits.push(RawCommit {
            date,
            email,
            is_merge,
        });
    }
    Ok(commits)
}

fn parse_tag_days(tag_dates: &str) -> Result<BTreeMap<Date, u16>> {
    let fmt = format_description!("[year]-[month]-[day]");
    let mut tag_days: BTreeMap<Date, u16> = BTreeMap::new();
    for line in tag_dates.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let d = Date::parse(line, &fmt)?;
        *tag_days.entry(d).or_insert(0) += 1;
    }
    Ok(tag_days)
}

fn entries_from_days(days: BTreeMap<Date, (u16, u16, Vec<u16>)>, first_entry: Date) -> Vec<DayEntry> {
    let mut entries = Vec::with_capacity(days.len());
    let mut prev = first_entry;
    for (date, (commits, merges, mut a)) in days {
        let delta = (date - prev).whole_days().clamp(0, u16::MAX as i64) as u16;
        a.sort_unstable();
        entries.push(DayEntry {
            delta,
            commits,
            merges,
            authors: a,
        });
        prev = date;
    }
    entries
}

fn tags_from_days(tag_days: BTreeMap<Date, u16>, first_entry: Date) -> Vec<TagEntry> {
    let mut tags = Vec::with_capacity(tag_days.len());
    let mut prev = first_entry;
    for (date, count) in tag_days {
        let delta = (date - prev).whole_days().clamp(0, u16::MAX as i64) as u16;
        tags.push(TagEntry { delta, count });
        prev = date;
    }
    tags
}

fn build_cache(commits: Vec<RawCommit>, tag_dates: &str) -> Result<Cache> {
    if commits.is_empty() {
        return Err(anyhow!("no commits"));
    }

    let mut authors: Vec<String> = Vec::new();
    let mut author_id: BTreeMap<String, u16> = BTreeMap::new();
    let mut days: BTreeMap<Date, (u16, u16, Vec<u16>)> = BTreeMap::new();
    let mut oldest = commits[0].date;

    for c in &commits {
        if c.date < oldest {
            oldest = c.date;
        }
        let id = *author_id.entry(c.email.clone()).or_insert_with(|| {
            authors.push(c.email.clone());
            (authors.len() - 1) as u16
        });
        let day = days.entry(c.date).or_insert((0, 0, Vec::new()));
        day.0 = day.0.saturating_add(1);
        if c.is_merge {
            day.1 = day.1.saturating_add(1);
        }
        if !day.2.contains(&id) {
            day.2.push(id);
        }
    }

    let first_entry = *days.keys().next().unwrap();
    let entries = entries_from_days(days, first_entry);
    let tags = tags_from_days(parse_tag_days(tag_dates)?, first_entry);
    let now = OffsetDateTime::now_utc().date();

    Ok(Cache {
        last_updated: now,
        oldest_commit: oldest,
        first_entry,
        authors,
        entries,
        tags,
    })
}

pub fn build_from_clone(git_url: &str, since: Option<Date>) -> Result<Option<Cache>> {
    let tmp = tempfile::Builder::new().prefix("vitality-").tempdir()?;
    let dir = tmp.path().to_str().ok_or_else(|| anyhow!("bad tmp path"))?;

    let fmt = format_description!("[year]-[month]-[day]");
    let mut clone_args: Vec<String> = vec![
        "clone".into(),
        "--filter=blob:none".into(),
        "--bare".into(),
    ];
    if let Some(d) = since {
        clone_args.push(format!("--shallow-since={}", d.format(&fmt).unwrap_or_default()));
    }
    clone_args.push(git_url.to_string());
    clone_args.push(dir.to_string());

    let out = Command::new("git")
        .args(&clone_args)
        .output()
        .context("failed to run git clone")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // git aborts a --shallow-since clone when no commit falls in the
        // window. On an incremental refresh that just means the repo had no
        // new activity since the last run, not a failure.
        if since.is_some() && empty_shallow_window(&stderr) {
            return Ok(None);
        }
        return Err(anyhow!("git {:?} failed: {}", clone_args, stderr));
    }

    let log = run_git(&[
        "-C",
        dir,
        "log",
        "--all",
        "--pretty=format:%H\u{1f}%aI\u{1f}%ae\u{1f}%P",
    ])?;
    let commits = parse_commits(&log)?;

    let tag_dates = run_git(&[
        "-C",
        dir,
        "for-each-ref",
        "--format=%(creatordate:short)",
        "refs/tags",
    ])?;

    build_cache(commits, &tag_dates).map(Some)
}

fn persist(cache_path: &Path, cache: &Cache) -> Result<()> {
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(cache_path, codec::encode(cache)?)?;
    Ok(())
}

fn merge_caches(existing: Cache, fresh: Cache) -> Cache {
    let mut day_data: BTreeMap<Date, (u16, u16, Vec<String>)> = BTreeMap::new();

    for cache in [&existing, &fresh] {
        let mut cur = cache.first_entry;
        for e in &cache.entries {
            cur = cur + Duration::days(e.delta as i64);
            let emails: Vec<String> = e
                .authors
                .iter()
                .filter_map(|id| cache.authors.get(*id as usize).cloned())
                .collect();
            day_data.insert(cur, (e.commits, e.merges, emails));
        }
    }

    let first_entry = day_data.keys().next().copied().unwrap_or(fresh.first_entry);
    let mut authors: Vec<String> = Vec::new();
    let mut author_id: BTreeMap<String, u16> = BTreeMap::new();
    let mut entries = Vec::new();
    let mut prev = first_entry;
    for (date, (commits, merges, emails)) in day_data {
        let mut ids = Vec::new();
        for email in emails {
            let id = *author_id.entry(email.clone()).or_insert_with(|| {
                authors.push(email.clone());
                (authors.len() - 1) as u16
            });
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.sort_unstable();
        let delta = (date - prev).whole_days().clamp(0, u16::MAX as i64) as u16;
        entries.push(DayEntry {
            delta,
            commits,
            merges,
            authors: ids,
        });
        prev = date;
    }

    // A --shallow-since clone only carries tags whose target is inside the
    // window, so fresh.tags drops every older release. Reconstruct the dated
    // tags from both caches and let fresh win on collisions.
    let mut tag_days: BTreeMap<Date, u16> = BTreeMap::new();
    for cache in [&existing, &fresh] {
        let mut cur = cache.first_entry;
        for t in &cache.tags {
            cur += Duration::days(t.delta as i64);
            tag_days.insert(cur, t.count);
        }
    }
    let tags = tags_from_days(tag_days, first_entry);

    Cache {
        last_updated: fresh.last_updated,
        oldest_commit: existing.oldest_commit.min(fresh.oldest_commit),
        first_entry,
        authors,
        entries,
        tags,
    }
}

pub fn read_or_build(cache_path: &Path, git_url: &str) -> Result<Cache> {
    let today = OffsetDateTime::now_utc().date();
    if let Ok(bytes) = std::fs::read(cache_path) {
        if let Ok(existing) = codec::decode(&bytes) {
            if existing.last_updated >= today {
                return Ok(existing);
            }
            match build_from_clone(git_url, Some(existing.last_updated))? {
                Some(fresh) => {
                    let merged = merge_caches(existing, fresh);
                    persist(cache_path, &merged)?;
                    return Ok(merged);
                }
                None => {
                    let mut existing = existing;
                    existing.last_updated = today;
                    persist(cache_path, &existing)?;
                    return Ok(existing);
                }
            }
        }
    }
    let built = build_from_clone(git_url, None)?
        .ok_or_else(|| anyhow!("clone produced no commits"))?;
    persist(cache_path, &built)?;
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::empty_shallow_window;

    #[test]
    fn detects_empty_shallow_window() {
        assert!(empty_shallow_window(
            "fatal: error processing shallow info: 4\n"
        ));
        assert!(empty_shallow_window(
            "fatal: no commits selected for shallow requests\n"
        ));
    }

    #[test]
    fn ignores_unrelated_git_errors() {
        assert!(!empty_shallow_window("fatal: repository not found"));
        assert!(!empty_shallow_window(
            "fatal: could not read Username: terminal prompts disabled"
        ));
    }

    #[test]
    fn merge_preserves_tags_absent_from_shallow_fresh() {
        use super::{merge_caches, Cache, DayEntry, TagEntry};
        use time::macros::date;

        let existing = Cache {
            last_updated: date!(2026 - 05 - 25),
            oldest_commit: date!(2026 - 04 - 01),
            first_entry: date!(2026 - 04 - 01),
            authors: vec!["a@b.c".into()],
            entries: vec![DayEntry {
                delta: 0,
                commits: 1,
                merges: 0,
                authors: vec![0],
            }],
            tags: vec![TagEntry { delta: 0, count: 3 }],
        };
        let fresh = Cache {
            last_updated: date!(2026 - 05 - 26),
            oldest_commit: date!(2026 - 05 - 26),
            first_entry: date!(2026 - 05 - 26),
            authors: vec!["a@b.c".into()],
            entries: vec![DayEntry {
                delta: 0,
                commits: 1,
                merges: 0,
                authors: vec![0],
            }],
            tags: vec![],
        };

        let merged = merge_caches(existing, fresh);

        let releases: u16 = merged.tags.iter().map(|t| t.count).sum();
        assert_eq!(releases, 3);
    }
}
