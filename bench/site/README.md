# script-bench site

This directory is reserved for the standalone OSS project's static website.

Planned shape:

- ingest `../history/runs.jsonl`
- group by `engine`, `target`, and `workload`
- render per-workload history charts and current leaderboard tables
- publish as a static site with no hosted metrics dependency
- keep raw JSONL downloadable for independent analysis

The benchmark runner already emits the stable append-only history format the
site should consume.
