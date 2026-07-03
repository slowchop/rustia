# script-bench site

Static dashboard for benchmark history.

## Files

- `index.html` - dashboard shell.
- `style.css` - responsive dashboard styling.
- `app.js` - JSONL parser, current leaderboard, run table, and canvas history chart.

The site consumes `../history/runs.jsonl` directly and has no build step.

## Local Preview

From the repository root:

```sh
python3 -m http.server 8000
```

Then open:

```text
http://127.0.0.1:8000/bench/site/
```

The file picker can load a local JSONL file when the page is opened outside a
static server.
