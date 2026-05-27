# activity-metrics-collector

Collects repository activity metrics of a catalog and writes them to the
`analysis` field of the software-catalog-api.

Metrics come from two sources: the git history (contributors,
commits, merges, tags, repository age) and the forge API (stars,
forks, open and closed issues) for GitHub and GitLab.
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
- `GITLAB_HOSTS` comma separated self-hosted GitLab hosts

## Run

```sh
activity-metrics-collector --recent-days 180 --concurrency 4
```

Flags:

- `--recent-days` window in days for the recent metrics (default 180)
- `--concurrency` repositories processed at once (default 4)
- `--dry-run` print the PATCH requests instead of sending them

## Output

Per software it sends `PATCH /software/{id}/analysis` with an
`activity` namespace:

```json
{
  "activity": {
    "v": 1,
    "contributors": 12,
    "commitsAllTime": 3400,
    "pullRequestsAllTime": 210,
    "commitsRecent": 180,
    "pullRequestsRecent": 14,
    "releases": 22,
    "oldestCommit": "2016-04-03",
    "recentDays": 180,
    "stars": 87,
    "forks": 13,
    "issuesOpen": 5,
    "issuesClosed": 140
  }
}
```

`pullRequests*` count merge commits and `releases` counts tags. The
social fields (`stars`, `forks`, `issuesOpen`, `issuesClosed`) are
omitted when the forge is unsupported or its API call fails.

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
    }
  }
}
```

`stats` holds one entry per metric (`contributors`, `commitsAllTime`,
`pullRequestsAllTime`, `commitsRecent`, `pullRequestsRecent`,
`releases`, and the social metrics when present).

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
