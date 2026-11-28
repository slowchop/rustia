"""Render the shared site header and the homepage comparison charts."""

from argparse import ArgumentParser
from hashlib import sha256
from html import escape, unescape
import json
from decimal import Decimal
from pathlib import Path
import re
import sys


SITE = Path(__file__).resolve().parent
README_URL = "https://github.com/slowchop/rustia"
CHART_ENGINES = {
    "rustia": "Rustia",
    "luaur": "luaur",
    "ruau": "ruau",
    "mlua-luau": "mlua + Luau",
    "mlua-luau-jit": "mlua + Luau JIT",
    "koto": "Koto",
    "rhai": "Rhai",
    "roto": "Roto",
    "ulua": "ulua",
    "luau-rs": "luau-rs/luau",
    "mlua-lua54": "mlua + Lua 5.4",
    "piccolo": "Piccolo",
    "rune": "Rune",
    "starlark": "Starlark",
    "boa": "Boa",
    "rquickjs": "rquickjs",
    "wasmi": "Wasmi",
    "wasmtime": "Wasmtime",
}
MARKER = "<!-- home-comparisons -->"
COMPARISON_HEADER = "| Project | Runs | Written in | Platforms |"
COMPARISON_LEGEND = "✓ Ran in Rustia checks · B Build or type-check only · U Upstream evidence"
METRICS = (
    ("steady", "Steady-state time", "µs per work unit"),
    ("cold", "Cold start", "µs"),
    ("first", "First call", "µs"),
)
ARRAY_CAP = 10


def render_cell(cell: str) -> str:
    cell = re.sub(r"\[\\\[(\d+)\\\]\]\(#(luau-note-\d+)\)", r"[\1](#\2)", cell)
    cell = unescape(cell.replace("]&nbsp;", "] ").strip("*"))
    parts = re.split(r"(\[[^\]]+\]\((?:https://|#)[^\s)]+\))", cell)
    for index, part in enumerate(parts):
        match = re.fullmatch(r"\[([^\]]+)\]\(([^)]+)\)", part)
        if not match:
            parts[index] = escape(part)
            continue
        label, href = match.groups()
        if href.startswith("#"):
            href = "/docs/api/rustia/" + href
        parts[index] = f'<a href="{escape(href)}">{escape(label)}</a>'
    return "".join(parts)


def read_table(readme: str, header: str) -> list[list[str]]:
    lines = readme[readme.index(header):].split("\n\n", 1)[0].splitlines()
    rows = [[cell.strip() for cell in line.strip("|").split("|")] for line in lines]
    if len(rows) < 3 or any(len(row) != len(rows[0]) for row in rows):
        raise ValueError(f"Invalid comparison table: {header}")
    return [rows[0], *rows[2:]]


def render_table(headings: list[str], body_rows: list[list[str]], heading_id: str) -> str:
    titles = "".join(f'<th scope="col">{render_cell(cell)}</th>' for cell in headings)
    body = []
    for name, *values in body_rows:
        cells = "".join(f"<td>{cell}</td>" for cell in values)
        body.append(f'<tr><th scope="row">{name}</th>{cells}</tr>')
    return (
        f'<div class="home-table" role="region" aria-labelledby="{heading_id}" tabindex="0">\n'
        f'<table><thead><tr>{titles}</tr></thead><tbody>\n'
        + "\n".join(body) + "\n</tbody></table></div>"
    )


def render_platform_links(cell: str, project: str, row: int, variant: str) -> str:
    icon = r"!\[([^]]+)\]\[rustia-([a-z]+)\]"
    badge = r'\[((?:!\[[^]]+\]\[rustia-[a-z]+\] )+)(✓|B|U)\]\((https://[^ )]+) "([^"]+)"\)'
    statuses = {"✓": "Ran in Rustia checks", "B": "Build or type-check only", "U": "Upstream evidence"}
    groups = dict((status, []) for status in statuses)
    for images, status, url, label in re.findall(badge, cell):
        groups[status].append((re.findall(icon, images), url, label.removeprefix(status + " · ")))
    if not any(groups.values()):
        raise ValueError(f"No platform evidence for {project}")
    buttons = []
    for index, (status, checks) in enumerate(groups.items()):
        if not checks:
            continue
        platforms = dict(platform for icons, _, _ in checks for platform in icons)
        name = escape(f"{project} on {', '.join(platforms)}")
        badge_id = f"platform-evidence-{variant}-{row}-{index}"
        images = "".join(
            f'<img class="platform-icon" src="/icons/{asset}.svg" alt="">' for asset in platforms.values()
        )
        evidence = []
        for icons, url, label in checks:
            target = url.removeprefix("https://rustia.dev") if url.startswith("https://rustia.dev/docs/") else url
            names = escape(", ".join(name for name, _ in icons))
            evidence.append(f'<li><a href="{escape(target)}">{names}</a> · {escape(label)}</li>')
        buttons.append(
            f'<button type="button" class="platform-link preview-platform" popovertarget="{badge_id}" '
            f'aria-expanded="false" aria-label="{name}" aria-describedby="{badge_id}-description">'
            f'{images}<sup aria-hidden="true">{status}</sup></button>'
            f'<div class="platform-evidence-popover" id="{badge_id}" popover="manual">'
            f'<strong>{escape(project)}</strong><p id="{badge_id}-description">{statuses[status]}</p>'
            f'<ul class="platform-evidence-checks">{"".join(evidence)}</ul></div>'
        )
    return f'<div class="platform-links">{"".join(buttons)}</div>'


def render_project_links(cell: str) -> tuple[str, str]:
    links = re.findall(r"\[([^]]+)\]\((https://[^ )]+)\)", cell)
    (project, repository), *websites = links
    host, icon = (
        ("Codeberg", "codeberg-favicon.svg") if repository.startswith("https://codeberg.org/")
        else ("GitHub", "github.svg")
    )
    destinations = [(repository, f"source on {host}", icon)]
    for label, url in websites:
        asset = "crates-io.ico" if url.startswith("https://crates.io/crates/") else "globe.svg"
        destinations.append((url, label.lower(), asset))
    icons = []
    for url, label, asset in destinations:
        name = escape(f"{project} {label}")
        icons.append(
            f'<a href="{escape(url)}" title="{name}" aria-label="{name}">'
            f'<img src="/icons/{asset}" width="16" height="16" alt=""></a>'
        )
    return project, f'<span class="project-links">{"".join(icons)}</span>'


def render_luau_comparison(rows: list[list[str]]) -> str:
    headings, *data = rows
    projects = []
    cards = []
    for row, (name, runs, implementation, platforms, reload, replay) in enumerate(data):
        project_name, links = render_project_links(name)
        fields = [render_cell(runs), render_cell(implementation)]
        behavior = [render_cell(reload), render_cell(replay)]
        projects.append([
            escape(project_name) + links, *fields,
            render_platform_links(platforms, project_name, row, "table"), *behavior,
        ])
        mobile = [*fields, render_platform_links(platforms, project_name, row, "card"), *behavior]
        details = "".join(
            f"<div><dt>{render_cell(label)}</dt><dd>{value}</dd></div>"
            for label, value in zip(headings[1:], mobile)
        )
        cards.append(
            f"<details><summary>{escape(project_name)}</summary><dl>"
            f"<div><dt>Project links</dt><dd>{links}</dd></div>{details}</dl></details>"
        )
    return (
        '<div class="luau-comparison-wide">'
        + render_table(headings, projects, "luau-comparison")
        + '</div><div class="luau-comparison-narrow">'
        + "".join(cards) + "</div>"
    )


def read_speed_snapshot() -> tuple[dict, str]:
    root = SITE.parent
    profile = json.loads((root / "design/public-source-profile.json").read_text(encoding="utf-8"))
    publication = profile["website_comparison"]
    path = publication["path"]
    data = (root / path).read_bytes()
    if sha256(data).hexdigest() != publication["sha256"]:
        raise ValueError("The website comparison differs from the approved snapshot")
    return json.loads(data), path


def render_measure(key, label, value, baseline, scale, units, method):
    outlier = value > scale
    cap_description = f"Bar capped at {scale / baseline:g}× Rustia"
    ratio = value / baseline
    relative = f"{ratio:.2f}" if ratio >= .01 else f"{ratio:.2g}"
    if value == baseline:
        relative = "1"
    formatted_time = format(Decimal(f"{value:.4g}"), ",f")
    if "." in formatted_time:
        formatted_time = formatted_time.rstrip("0").rstrip(".")
    hidden = "" if key == "steady" else " hidden"
    details = [label, f"{relative}× Rustia", f"{formatted_time} {units}"]
    if outlier:
        details.append(cap_description)
    if method:
        details.append(method.capitalize())
    tooltip = escape(" · ".join(details))
    ratio_cell = f'<span class="benchmark-value">{relative}×</span>'
    exact_cell = f'<span class="benchmark-exact">{formatted_time}<span class="benchmark-sr-only"> {units}</span></span>'
    outlier_class = " benchmark-outlier" if outlier else ""
    marker = (
        f'<div class="benchmark-measure{outlier_class}" data-benchmark-view="{key}" data-benchmark-style="{key}" data-benchmark-tooltip="{tooltip}"{hidden}>'
        f'<span class="benchmark-measure-label">{label}</span>'
    )
    track_label = f'role="img" aria-label="{cap_description}"' if outlier else 'aria-hidden="true"'
    cap_marker = '<span class="benchmark-cap-marker" aria-hidden="true">→</span>' if outlier else ""
    return (
        marker + f'<span class="speed-track" {track_label}>'
        f'<span class="speed-fill" style="width:{min(value / scale, 1):.6%}"></span>{cap_marker}</span>'
        f'{ratio_cell}{exact_cell}</div>'
    )



def render_speed_chart(snapshot: dict) -> str:
    panels = []
    for workload, info in snapshot["workloads"].items():
        data = info["times_us"]
        baseline = data["rustia"]
        scales = {
            metric: baseline[metric] * ARRAY_CAP if workload == "dense_tables"
            else max(values[metric] for values in data.values())
            for metric, _, _ in METRICS
        }
        items = []
        for engine in sorted(data, key=lambda item: data[item]["steady"]):
            name = escape(CHART_ENGINES[engine])
            values = data[engine]
            method = info.get("notes", {}).get(engine)
            bars = "".join(
                render_measure(metric, label, values[metric], baseline[metric], scales[metric], units, method)
                for metric, label, units in METRICS
            )
            baseline_class = ' speed-baseline' if engine == "rustia" else ""
            sort_data = "".join(f' data-sort-{metric}="{value}"' for metric, value in values.items())
            items.append(
                f'<li class="benchmark-engine{baseline_class}"{sort_data}><span>{name}</span>'
                f'<div class="benchmark-engine-measures">{bars}</div></li>'
            )
        for engine, reason in info.get("skipped", {}).items():
            name = escape(CHART_ENGINES[engine])
            items.append(
                f'<li class="benchmark-engine benchmark-skipped"><span>{name}</span>'
                f'<span class="benchmark-skip-reason">Skipped · {escape(reason)}</span></li>'
            )
        label = escape(info["label"])
        panels.append(
            f'<div role="group" aria-labelledby="chart-{workload}"><div class="benchmark-card-heading">'
            f'<h4 id="chart-{workload}">{label}</h4>'
            '<span class="benchmark-time-heading" title="Microseconds">µs</span></div>'
            f'<ul role="list">{"".join(items)}</ul></div>'
        )
    return f'<div class="speed-chart" id="benchmark-charts">{"".join(panels)}</div>'


def render_description_metadata(homepage: str) -> str:
    marker = "<!-- project-description-meta -->"
    paragraphs = re.findall(
        r"<!-- project-copy: (intro|origin) -->\s*<p>(.*?)</p>\s*<!-- /project-copy: \1 -->",
        homepage, re.S,
    )
    if homepage.count(marker) != 1 or [name for name, _ in paragraphs] != ["intro", "origin"]:
        raise ValueError("Expected one description metadata marker and one intro and origin paragraph each")
    description = " ".join(
        unescape(re.sub(r"<[^>]+>", "", paragraph)) for _, paragraph in paragraphs
    )
    content = escape(" ".join(description.split()))
    tags = (
        f'<meta name="description" content="{content}">',
        f'<meta property="og:description" content="{content}">',
        f'<meta name="twitter:description" content="{content}">',
    )
    return homepage.replace(marker, "\n".join(tags))


def render_homepage(homepage: str) -> str:
    if homepage.count(MARKER) != 1:
        raise ValueError("Expected exactly one homepage comparison marker")
    snapshot, snapshot_path = read_speed_snapshot()
    comparison = (SITE.parent / "crates/rustia/compare.md").read_text(encoding="utf-8")
    sources_checked = re.search(r"\*\*Sources and details\*\* · Checked ([\w ]+)\.", comparison)[1]
    luau_rows = read_table(comparison, COMPARISON_HEADER)
    luau = render_luau_comparison(luau_rows)
    chart = render_speed_chart(snapshot)
    options = []
    sort_options = []
    for key, label, _ in METRICS:
        checked_attr = " checked" if key == "steady" else ""
        selected = " selected" if key == "steady" else ""
        options.append(
            f'<label><input class="benchmark-sr-only" type="checkbox" name="benchmark-measure" value="{key}" autocomplete="off"{checked_attr}>'
            '<span class="benchmark-toggle"><span class="benchmark-toggle-mark" aria-hidden="true"></span>'
            f'<span class="benchmark-swatch" data-benchmark-style="{key}" aria-hidden="true"></span>{label}</span></label>'
        )
        sort_options.append(f'<option value="{key}"{selected}>{label}</option>')
    evidence = re.search(r'<details class="speed-details">.*?</details>', comparison, re.S)[0]
    section = f'''<h3 id="luau-comparison">Luau runtimes and other scripting projects</h3>
    <p>{COMPARISON_LEGEND}. Sources checked {escape(sources_checked)}.</p>
    {luau}
    {evidence}
    <p><a href="/docs/api/rustia/#compare-luau-projects">Luau comparison sources and notes</a></p>
    <h3 id="speed-comparison">Runtime speed</h3>
    <p><strong>Lower is better.</strong> 1× is Rustia’s time for that task. Bars follow the selected measures in order. Each measure has its own scale. The array chart caps bars at {ARRAY_CAP}× and marks higher times with an arrow.</p>
<fieldset class="benchmark-measures" id="benchmark-measures">
  <legend>Measures</legend>
  <span id="benchmark-measure-minimum" hidden>Select another measure before turning this one off.</span>
  {"".join(options)}
</fieldset>
<div class="benchmark-options">
  <label class="benchmark-sort">Sort by
    <span class="benchmark-sort-control">
      <select id="benchmark-sort" autocomplete="off">
        {"".join(sort_options)}
        <option value="name">Engine name</option>
      </select>
    </span>
  </label>
  <label><input class="benchmark-sr-only" type="checkbox" id="benchmark-exact-times">
    <span class="benchmark-toggle"><span class="benchmark-toggle-mark" aria-hidden="true"></span>Show times</span>
  </label>
</div>
    {chart}
    <div id="benchmark-tooltip" popover="manual" aria-hidden="true"></div>
    <p>Linux x86-64 · {escape(snapshot["date_utc"])} UTC · Median of {snapshot["sample_count"]} samples. Cold start includes preparation and the first call. Workload implementations vary by engine.</p>
    <p><a href="{README_URL}/blob/main/{escape(snapshot_path)}">Benchmark data</a> · <a href="{README_URL}/blob/main/bench/COMPETITORS.md#expanded-runtime-comparison">Methodology and limitations</a></p>'''
    return render_description_metadata(homepage).replace(MARKER, section)


def render_rustdoc(rustdoc: str) -> str:
    comparison = (SITE.parent / "crates/rustia/compare.md").read_text(encoding="utf-8")
    table = render_luau_comparison(read_table(comparison, COMPARISON_HEADER)).replace(
        'aria-labelledby="luau-comparison"', 'aria-labelledby="compare-luau-projects"',
    )
    heading = rustdoc.index('<h3 id="compare-luau-projects">')
    start = rustdoc.index("</h3>", heading) + len("</h3>")
    end = rustdoc.index('<details class="speed-details">', start)
    if "<th>Project</th>" not in rustdoc[start:end]:
        raise ValueError("Expected the native Rustdoc comparison table")
    rustdoc = (
        rustdoc[:heading] + '<div class="rustdoc-comparison">' + rustdoc[heading:start]
        + f'<p>{COMPARISON_LEGEND}.</p>{table}</div>' + rustdoc[end:]
    )
    assets = (
        '<link rel="stylesheet" href="/project-comparison.css">'
        '<link rel="stylesheet" href="/rustdoc-comparison.css">'
        '<script src="/platform-evidence.mjs" type="module"></script>'
    )
    return rustdoc.replace("</head>", assets + "</head>", 1)


def main() -> None:
    parser = ArgumentParser(description=__doc__)
    parser.add_argument(
        "page", nargs="?", default="index.html",
        choices=("index.html", "doc-index.html", "garden-source.html"),
    )
    parser.add_argument("--rustdoc", type=Path, help="Decorate a generated Rustia facade page")
    args = parser.parse_args()
    if args.rustdoc:
        sys.stdout.write(render_rustdoc(args.rustdoc.read_text(encoding="utf-8")))
        return
    page = (SITE / args.page).read_text(encoding="utf-8")
    marker = "<!-- site-header -->"
    if page.count(marker) != 1:
        raise ValueError(f"Expected exactly one site header marker in {args.page}")
    header = (SITE / "site-header.html").read_text(encoding="utf-8").rstrip()
    current = {
        "index.html": "",
        "doc-index.html": ' aria-current="page"',
        "garden-source.html": ' aria-current="location"',
    }
    page = page.replace(marker, header.replace("{docs_current}", current[args.page]))
    if args.page == "index.html":
        page = render_homepage(page)
    sys.stdout.write(page)


if __name__ == "__main__":
    main()
