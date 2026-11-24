"""Render the maintained README comparisons for the static homepage."""

from datetime import date, datetime, timezone
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
# Release metadata is a dated snapshot. Capability evidence can use older revisions.
LATEST_RELEASES = {
    "luaur": ("0.1.8", "2026-07-06", "https://crates.io/crates/luaur/0.1.8"),
    "ulua": ("0.1.7", "2026-10-08", "https://crates.io/crates/ulua/0.1.7"),
    "ruau": ("0.4.0", "2026-08-17", "https://crates.io/crates/ruau/0.4.0"),
    "luau-rs/luau": ("0.739.0", "2026-09-23", "https://www.npmjs.com/package/@luau-rs/luau/v/0.739.0"),
    "mlua": ("0.12.3", "2026-11-22", "https://github.com/mlua-rs/mlua/releases/tag/v0.12.3"),
    "Lune": ("0.11.1", "2026-10-21", "https://github.com/lune-org/lune/releases/tag/v0.11.1"),
}
RELEASES_CHECKED = "24 November 2026"
PLATFORMS = {"Linux": "linux", "macOS": "macos", "Windows": "windows", "iOS": "ios", "Android": "android"}
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
            href = README_URL + href
        parts[index] = f'<a href="{escape(href)}">{escape(label)}</a>'
    return "".join(parts)


def read_table(readme: str, header: str) -> list[list[str]]:
    lines = readme[readme.index(header):].split("\n\n", 1)[0].splitlines()
    rows = [[cell.strip() for cell in line.strip("|").split("|")] for line in lines]
    if len(rows) < 3 or any(len(row) != len(rows[0]) for row in rows):
        raise ValueError(f"Invalid README comparison table: {header}")
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



def render_release(name: str) -> str:
    if name == "Rustia":
        return "No public release"
    version, published, url = LATEST_RELEASES[name]
    release_date = date.fromisoformat(published)
    return (
        f'<span class="runtime-release"><a href="{escape(url)}">{escape(version)}</a> '
        f'<time datetime="{published}">{release_date.day} {release_date:%b %Y}</time></span>'
    )


def render_platform_link(url: str, icons: list[tuple[str, str]], label: str) -> str:
    images = "".join(
        f'<img class="platform-icon" src="/icons/{icon}.svg" alt="{escape(name)}">'
        for name, icon in icons
    )
    return f'<a class="platform-link" href="{escape(url)}" title="{escape(label)}" aria-label="{escape(label)}">{images}</a>'


def render_platform_links(project: str, platforms: str, browser: str, note: str) -> str:
    evidence_labels = {
        "Rustia": "Desktop",
        "luaur": "Desktop CI",
        "ulua": "Desktop CI",
        "ruau": "Linux run",
        "mlua": "Emscripten",
        "Lune": "Desktop releases",
    }
    if project == "luau-rs/luau":
        native_url = "https://github.com/luau-rs/luau/blob/99cc8ce63190b7535a65752faa84ce620b4230c0/.github/workflows/ci.yml"
    else:
        native_url = re.search(rf"\[{evidence_labels[project]}\]\(([^)]+)\)", note)[1].split("#")[0]
    names = re.findall(r"!\[([^\]]+)\]\[rustia-[a-z]+\]", platforms)
    links = [render_platform_link(native_url, [(name, PLATFORMS[name]) for name in names], ", ".join(names) + " evidence")]

    if project == "Rustia":
        mobile_url = re.search(r"\[Mobile\]\(([^)]+)\)", note)[1]
        links.append(render_platform_link(mobile_url, [("iOS", "ios"), ("Android", "android")],
                                          "iOS and Android evidence"))
    if browser != "—":
        kind, url = re.fullmatch(r"\[([^\]]+)\]\(([^)]+)\)", unescape(browser)).groups()
        kind = " ".join(kind.split())
        label = f"Browser Wasm · {kind}"
        links.append(render_platform_link(url, [("Browser Wasm", "browser")], label))
    return '<span class="platform-links">' + "".join(links) + "</span>"


def render_luau_comparison(rows: list[list[str]], readme: str) -> str:
    headings = ["Project", "Runtime", "Platforms", "Hot Reload", "Replay", "Latest release"]
    reload_support = {"Rollback", "Reload module", "Replace module"}
    projects = []
    cards = []
    for name, engine_cell, platforms, browser_cell, edits, replay_cell in rows[1:]:
        project = render_cell(name)
        source = re.search(r"#luau-note-\d+", platforms)[0]
        note = re.search(rf'<a id="{source[1:]}"></a>([^\n]+)', readme)[1]
        status = "Yes" if edits.replace("&nbsp;", " ") in reload_support else "Unverified"
        runtime = engine_cell.replace("Upstream Luau (C++)", "Luau C++")
        replay = "Unverified" if replay_cell == "—" else replay_cell
        values = [
            render_cell(runtime),
            render_platform_links(project, platforms, browser_cell, note),
            render_cell(f"[{status}]({source})"),
            render_cell(replay),
            render_release(project),
        ]
        projects.append([project, *values])
        fields = "".join(
            f"<div><dt>{render_cell(label)}</dt><dd>{value}</dd></div>"
            for label, value in zip(headings[1:], values)
        )
        cards.append(f"<details><summary>{project}</summary><dl>{fields}</dl></details>")
    return (
        '<div class="luau-comparison-wide">'
        + render_table(headings, projects, "luau-comparison")
        + '</div><div class="luau-comparison-narrow">'
        + "".join(cards) + "</div>"
    )


def read_speed_snapshot(snapshot: dict, labels: list[str]) -> list[tuple[str, list[float]]]:
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
        (label, [times[workload, engine] for engine in engines])
        for label, workload in zip(labels, snapshot["workloads"])
    ]


def render_speed_chart(rows: list[tuple[str, list[float]]]) -> str:
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
    for name, values in rows:
        baseline_time = values[baseline]
        times = [value / baseline_time for value in values]
        scale = ceil(max(times))
        label = short_labels.get(name, name)
        bars = []
        ranked = sorted(zip(engines, times, values), key=lambda result: result[1])
        for engine, time, value in ranked:
            engine_class = ' class="speed-baseline"' if engine == "Rustia" else ""
            ratio = "1" if engine == "Rustia" else f"{time:.2f}"
            bars.append(
                f'<li{engine_class} title="{value:.3f} µs per work unit">'
                f'<span>{escape(engine)}</span>'
                f'<meter min="0" max="{scale}" value="{time:.6f}" aria-hidden="true"></meter>'
                f'<span>{ratio}×</span></li>'
            )
        panels.append(f'<div><h4>{escape(label)}</h4><ul>{"".join(bars)}</ul></div>')
    return '<div class="speed-chart">' + "\n".join(panels) + '</div>'


def main() -> None:
    readme = (SITE.parent / "README.md").read_text(encoding="utf-8")
    homepage = (SITE / "index.html").read_text(encoding="utf-8")
    if homepage.count(MARKER) != 1:
        raise ValueError("Expected exactly one homepage comparison marker")
    snapshot = json.loads(readme.split("<!-- rustia-benchmark-snapshot", 1)[1].split("-->", 1)[0])
    benchmark_date = datetime.fromtimestamp(snapshot["start"], timezone.utc).date().isoformat()
    checked = re.search(r"\*\*Sources and details\*\* · Checked ([\w ]+)\.", readme)[1]
    luau_rows = read_table(readme, "| Project | Engine | Desktop |")
    labels = [row[0] for row in read_table(readme, "| Work unit | Rustia |")[1:]]
    speed_rows = read_speed_snapshot(snapshot, labels)
    luau = render_luau_comparison(luau_rows, readme)
    chart = render_speed_chart(speed_rows)
    section = f'''<h3 id="luau-comparison">Luau runtimes + Rust scripting crates</h3>
    <p>Capabilities are based on the linked source, CI and demos checked {escape(checked)}. Release versions were checked {RELEASES_CHECKED} and may be newer than the versions reviewed.</p>
    {luau}
    <details class="speed-details">
      <summary>What we checked</summary>
      <dl class="comparison-evidence">
        <dt>Runtime</dt><dd>The implementation that executes Luau. Luau C++ refers to the upstream Luau runtime.</dd>
        <dt>Platforms</dt><dd>Icons link to CI, release builds or recorded runs. The globe links to a Browser Wasm demo, test or build. Rustia’s ARM64 mobile checks ran on a physical iPad and an Android 15 emulator.</dd>
        <dt>Hot Reload</dt><dd>Yes means the host can reload or replace part of the script while the app keeps running. Unverified means the sources show how to run edited code but leave in-place replacement unverified. Rustia can keep the previous code and script state if a reload fails.</dd>
        <dt>Replay</dt><dd>Rustia’s recorded native and browser runs matched. ruau documents controls for seeds and the clock. Unverified means the reviewed sources did not establish replay support or provide a matching run.</dd>
        <dt>Latest release</dt><dd>The latest stable version found in the project’s package registry or on GitHub. Dates are in UTC.</dd>
      </dl>
    </details>
    <p><a href="{README_URL}#compare-luau-projects">Luau comparison sources and notes</a></p>
    <h3 id="speed-comparison">Runtime speed</h3>
    <p>Correctness comes first. We test Luau behaviour and deterministic replay while working to make Rustia faster.</p>
    <p><strong>Lower is better.</strong> 1× is Rustia’s time for that task. Each chart has its own scale.</p>
    {chart}
    <p class="benchmark-note">Snapshot from {benchmark_date} UTC on Linux x86-64. These are best observed times after startup, including interpreter, JIT and native results. Each engine uses its own version of each task. Roto compiles to machine code with Cranelift. Its sparse lookup uses a list, and its state update uses a regular function.</p>
    <p><a href="{BENCHMARK_URL}/RESULTS.md">Exact benchmark timings on GitHub</a> · <a href="{BENCHMARK_URL}/CLAIMS.md">Benchmark methodology and limitations</a></p>'''
    sys.stdout.write(homepage.replace(MARKER, section))


if __name__ == "__main__":
    main()
