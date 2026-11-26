"""Render the shared site header and the homepage comparison charts."""

from argparse import ArgumentParser
from datetime import datetime, timezone
from html import escape, unescape
import json
from math import ceil
from pathlib import Path
import re
import sys


SITE = Path(__file__).resolve().parent
README_URL = "https://github.com/slowchop/rustia"
BENCHMARK_URL = README_URL + "/blob/94c6a34246c8a07badfd7ef88e89c4a9a643919b/bench"
CHART_ENGINES = {
    "rustia": "Rustia",
    "luaur": "luaur",
    "ruau": "ruau",
    "mlua-luau": "mlua + Luau",
    "mlua-luau-jit": "mlua + Luau JIT",
    "koto": "Koto",
    "rhai": "Rhai",
    "roto": "Roto",
}
MARKER = "<!-- home-comparisons -->"


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


def render_platform_group(icons: list[tuple[str, str]], label: str, url: str = "") -> str:
    images = "".join(
        f'<img class="platform-icon" src="/icons/{icon}.svg" alt="{escape(name)}">'
        for name, icon in icons
    )
    if url:
        return f'<a class="platform-link" href="{escape(url)}" title="{escape(label)}" aria-label="{escape(label)}">{images}</a>'
    return f'<span class="platform-link" title="{escape(label)}">{images}</span>'


def render_platform_links(cell: str) -> str:
    # The shared table uses single icons or groups linked to platform evidence.
    image = r"!\[([^]]+)\]\[rustia-([a-z]+)\]"
    linked_icons = r'\[((?:!\[[^]]+\]\[rustia-[a-z]+\]\s*)+)\]\((https://[^ )]+) "([^"]+)"\)'
    groups = []
    for images, url, title, name, icon in re.findall(f"{linked_icons}|{image}", cell):
        if images:
            groups.append(render_platform_group(re.findall(image, images), title, url))
        else:
            groups.append(render_platform_group([(name, icon)], name))
    return '<span class="platform-links">' + "".join(groups) + "</span>"


def render_project_links(cell: str) -> tuple[str, str]:
    links = re.findall(r"\[([^]]+)\]\((https://[^ )]+)\)", cell)
    (project, repository), *websites = links
    project = project.strip("*")
    icons = []
    for icon, url, destination in [
        ("github", repository, "GitHub repository"),
        *(("globe", url, label.lower()) for label, url in websites),
    ]:
        label = escape(f"{project} {destination}")
        icons.append(
            f'<a href="{escape(url)}" title="{label}" aria-label="{label}">'
            f'<img src="/icons/{icon}.svg" width="16" height="16" alt=""></a>'
        )
    return escape(project), '<span class="project-links">' + "".join(icons) + "</span>"


def render_luau_comparison(rows: list[list[str]]) -> str:
    headings, *data = rows
    projects = []
    cards = []
    for name, runtime, platforms, reload, replay in data:
        project_name, links = render_project_links(name)
        values = [
            render_cell(runtime),
            render_platform_links(platforms),
            render_cell(reload),
            render_cell(replay),
        ]
        projects.append([project_name + links, *values])
        fields = "".join(
            f"<div><dt>{render_cell(label)}</dt><dd>{value}</dd></div>"
            for label, value in zip(headings[1:], values)
        )
        cards.append(
            f"<details><summary>{project_name}</summary><dl>"
            f"<div><dt>Project links</dt><dd>{links}</dd></div>{fields}</dl></details>"
        )
    return (
        '<div class="luau-comparison-wide">'
        + render_table(headings, projects, "luau-comparison")
        + '</div><div class="luau-comparison-narrow">'
        + "".join(cards) + "</div>"
    )


def read_speed_snapshot(snapshot: dict, labels: list[str]) -> list[tuple[str, str, list[float]]]:
    history = SITE.parent / "bench/history/runs.jsonl"
    engines = CHART_ENGINES
    times = {}
    with history.open(encoding="utf-8") as runs:
        for line in runs:
            row = json.loads(line)
            if (row["commit"] != snapshot["commit"] or row["target"] != snapshot["target"]
                    or not snapshot["start"] <= row["timestamp_unix"] <= snapshot["end"]
                    or row["engine"] not in engines or row["workload"] not in snapshot["workloads"]):
                continue
            key = (row["workload"], row["engine"])
            time = row["ns_per_unit_min"] / 1000
            if key in times or time <= 0:
                raise ValueError(f"Invalid benchmark time for {key}")
            times[key] = time
    if len(labels) != len(snapshot["workloads"]):
        raise ValueError("Benchmark workload labels do not match the snapshot")
    return [
        (workload, label, [times[workload, engine] for engine in engines])
        for label, workload in zip(labels, snapshot["workloads"])
    ]


def render_speed_chart(rows: list[tuple[str, str, list[float]]]) -> str:
    engines = list(CHART_ENGINES.values())
    baseline = engines.index("Rustia")
    short_labels = {
        "Build, sum, and sort a 256-item array": "Build, sum and sort an array",
        "Fill and read 128 hash entries": "Sparse table reads and writes",
        "One closure call updating a captured value": "Call a closure",
        "Custom, keyed, and array iteration pass": "Iterate over tables",
        "Math library calls and buffered appends": "Math and table appends",
    }
    panels = []
    for workload, name, values in rows:
        baseline_time = values[baseline]
        results = [
            (engine, value / baseline_time, value)
            for engine, value in zip(engines, values)
            if (workload, engine) != ("dense_tables", "Roto")
        ]
        ranked = sorted(results, key=lambda result: result[1])
        scale = ceil(ranked[-1][1])
        label = short_labels.get(name, name)
        bars = []
        for engine, time, value in ranked:
            engine_class = ' class="speed-baseline"' if engine == "Rustia" else ""
            ratio = "1" if engine == "Rustia" else f"{time:.2f}"
            bars.append(
                f'<li{engine_class} title="{value:.3f} µs per work unit">'
                f'<span>{escape(engine)}</span>'
                '<span class="speed-track" aria-hidden="true">'
                f'<span class="speed-fill" style="width:{time / scale:.6%}"></span></span>'
                f'<span>{ratio}×</span></li>'
            )
        if workload == "dense_tables":
            bars.append(
                '<li><span>Roto</span><span class="speed-method">Handwritten quadratic sort. '
                f'<a href="{BENCHMARK_URL}/RESULTS.md#L45">Exact timing</a></span></li>'
            )
        panels.append(f'<div><h4>{escape(label)}</h4><ul>{"".join(bars)}</ul></div>')
    return '<div class="speed-chart">' + "\n".join(panels) + '</div>'


def render_description_metadata(homepage: str) -> str:
    marker = "<!-- project-description-meta -->"
    hero = re.search(r'<div class="hero-copy">(.*?)</div>', homepage, re.S)
    paragraphs = re.findall(r"<p>(.*?)</p>", hero[1], re.S) if hero else []
    if homepage.count(marker) != 1 or len(paragraphs) != 2:
        raise ValueError("Expected one description metadata marker and two hero paragraphs")
    description = " ".join(
        unescape(re.sub(r"<[^>]+>", "", paragraph)) for paragraph in paragraphs
    )
    content = escape(" ".join(description.split()))
    tags = (
        f'<meta name="description" content="{content}">',
        f'<meta property="og:description" content="{content}">',
        f'<meta name="twitter:description" content="{content}">',
    )
    return homepage.replace(marker, "\n".join(tags))


def render_homepage(homepage: str) -> str:
    readme = (SITE.parent / "README.md").read_text(encoding="utf-8")
    if homepage.count(MARKER) != 1:
        raise ValueError("Expected exactly one homepage comparison marker")
    snapshot = json.loads(readme.split("<!-- rustia-benchmark-snapshot", 1)[1].split("-->", 1)[0])
    benchmark_date = datetime.fromtimestamp(snapshot["start"], timezone.utc).date().isoformat()
    comparison = (SITE.parent / "crates/rustia/compare.md").read_text(encoding="utf-8")
    checked = re.search(r"\*\*Sources and details\*\* · Checked ([\w ]+)\.", comparison)[1]
    luau_rows = read_table(comparison, "| Project | Runtime | Platforms |")
    labels = [row[0] for row in read_table(readme, "| Work unit | Rustia |")[1:]]
    speed_rows = read_speed_snapshot(snapshot, labels)
    luau = render_luau_comparison(luau_rows)
    chart = render_speed_chart(speed_rows)
    evidence = re.search(r'<details class="speed-details">.*?</details>', comparison, re.S)[0]
    section = f'''<h3 id="luau-comparison">Luau runtimes + Rust scripting crates</h3>
    <p>Capabilities are based on the linked source, CI and demos checked {escape(checked)}.</p>
    {luau}
    {evidence}
    <p><a href="/docs/api/rustia/#compare-luau-projects">Luau comparison sources and notes</a></p>
    <h3 id="speed-comparison">Runtime speed</h3>
    <p>Correctness comes first. We test Luau behaviour and deterministic replay while working to make Rustia faster.</p>
    <p><strong>Lower is better.</strong> 1× is Rustia’s time for that task. Each chart has its own scale.</p>
    {chart}
    <p>Snapshot from {benchmark_date} UTC on Linux x86-64. These are best observed times after startup, including interpreter, JIT and native results. Each engine uses its own version of each task. Roto JIT-compiles scripts to machine code with Cranelift before running them. Its sparse lookup uses a list, and its state update uses a regular function.</p>
    <p><a href="{BENCHMARK_URL}/RESULTS.md">Exact benchmark timings on GitHub</a> · <a href="{BENCHMARK_URL}/CLAIMS.md">Benchmark methodology and limitations</a></p>'''
    return render_description_metadata(homepage).replace(MARKER, section)


def main() -> None:
    parser = ArgumentParser(description=__doc__)
    parser.add_argument(
        "page", nargs="?", default="index.html",
        choices=("index.html", "doc-index.html", "garden-source.html"),
    )
    args = parser.parse_args()
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
