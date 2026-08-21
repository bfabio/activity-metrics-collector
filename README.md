# activity-metrics-collector

Collects repository activity metrics of a catalog and writes them to the
`analysis` field of the software-catalog-api.

Metrics come from two sources: the git history (contributors,
commits, tags, repository age) and the forge API (stars, forks,
open and closed issues, pull requests) for GitHub, GitLab, Gitea and
Forgejo. Self-hosted GitLab, Gitea and Forgejo instances are detected
by probing each host once.
The git history is read from a blobless bare clone made in a temp
dir. Only a compact binary summary is kept under `$XDG_CACHE_HOME`
or `~/.cache`.

## Build

```sh
cargo build --release
```

## Configure

Settings are read from the environment, or from a `.env` file in the
working directory:

- `SOFTWARE_CATALOG_API_BASE_URL` base URL of the software-catalog-api
- `SOFTWARE_CATALOG_API_BEARER_TOKEN` token with write access to the `analysis` field
- `GITHUB_TOKEN` token for the GitHub GraphQL API
- `GITLAB_TOKEN` token for the GitLab API
- `GITLAB_HOSTS` comma separated self-hosted GitLab hosts, for an
  instance the probe cannot reach

## Run

```sh
activity-metrics-collector --recent-days 180 --concurrency 4
```

Flags:

- `--recent-days` window in days for the recent metrics (default 180)
- `--concurrency` repositories processed at once (default 4)
- `--dry-run` print the PATCH requests instead of sending them
- `--catalog <id>` process only the software in catalog <id>

## Output

Per software it sends `PATCH /software/{id}/analysis` with an
`activity` namespace:

```json
{
  "activity": {
    "v": 1,
    "contributors": 12,
    "commitsAllTime": 3400,
    "commitsRecent": 180,
    "tags": 22,
    "oldestCommit": "2016-04-03",
    "recentDays": 180,
    "stars": 87,
    "forks": 13,
    "issuesOpen": 5,
    "issuesClosed": 140,
    "pullRequestsAllTime": 210,
    "pullRequestsRecent": 14
  }
}
```

`tags` is the git tag count. The forge fields (`stars`, `forks`,
`issuesOpen`, `issuesClosed`, `pullRequestsAllTime`,
`pullRequestsRecent`) come from the forge API. They are omitted when
the forge is unsupported and sent as `null` when its call fails. The
API replaces the namespace on every PATCH, so when the call fails for
a software whose stored analysis already holds a measured forge value
the PATCH is skipped: the stored numbers stay, and their `t` keeps
saying when they were last refreshed. Pull requests count the forge
PRs/MRs (`is:pr` on GitHub), not git merge commits.

Per catalog it sends `PATCH /catalogs/{id}/analysis` with per metric
statistics over the catalog software:

```json
{
  "activity": {
    "v": 1,
    "softwareCount": 240,
    "recentDays": 180,
    "stats": {
      "commitsAllTime": { "max": 3400, "min": 0, "count": 240, "mean": 412.5, "median": 120, "p95": 1800 }
      // ... one entry per metric, truncated
    }
  }
}
```

`stats` holds one entry per metric: `contributors`, `commitsAllTime`,
`commitsRecent`, `tags`, and the forge metrics (`stars`, `forks`,
`issuesOpen`, `issuesClosed`, `pullRequestsAllTime`,
`pullRequestsRecent`) when present.

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
