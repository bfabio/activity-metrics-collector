# activity-metrics-collector

Collects repository activity metrics for every software in the
developers-italia catalog and writes them, unprocessed, to the
`analysis` field of the developers-italia API. It does not compute a
score. Normalization, weighting and ranking are left to the UI.

Metrics come from two sources: the git history (contributors,
commits, pull requests, releases, repository age) from a local
clone cached under `$XDG_CACHE_HOME` or `~/.cache`, and the forge
API (stars, forks, open and closed issues) for GitHub and GitLab.

## Build

```sh
cargo build --release
```

## Configure

Settings are read from the environment, or from a `.env` file in the
working directory:

- `API_BASEURL` base URL of the developers-italia API
- `API_BEARER_TOKEN` token with write access to the `analysis` field
- `GITHUB_TOKEN` token for the GitHub GraphQL API
- `GITLAB_TOKEN` token for the GitLab API
- `GITLAB_HOSTS` comma separated self-hosted GitLab hosts

Set `ACTIVITY_METRICS_COLLECTOR_ENV` to load a `.env` from another
path.

## Run

```sh
activity-metrics-collector --recent-days 180 --concurrency 4
```

Flags:

- `--recent-days` window in days for the recent metrics (default 180)
- `--concurrency` repositories processed at once (default 4)
- `--dry-run` print the PATCH requests instead of sending them
- `--catalog <id>` process only the software in catalog <id>

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
