"""Render the maintained README comparisons for the static homepage."""

from datetime import datetime, timezone
from html import escape, unescape
import json
from math import ceil
from pathlib import Path
import re
import sys


SITE = Path(__file__).resolve().parent
README_URL = "https://github.com/slowchop/rustia"
MARKER = "<!-- home-comparisons -->"


def render_cell(cell: str) -> str:
    for icon, platform in (("Linux", "linux"), ("macOS", "macos"), ("Windows", "windows")):
        cell = cell.replace(f"![{icon}][rustia-{platform}]&nbsp;", f"{icon} ")
    cell = re.sub(r"\[\\\[(\d+)\\\]\]\(#(luau-note-\d+)\)", r"[\1](#\2)", cell)
    cell = unescape(cell.strip("*"))
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


def render_table(rows: list[list[str]], heading_id: str, numeric: bool = False) -> str:
    titles = "".join(f'<th scope="col">{render_cell(cell)}</th>' for cell in rows[0])
    body = []
    for name, *values in rows[1:]:
        cells = "".join(f"<td>{render_cell(cell)}</td>" for cell in values)
        body.append(f'<tr><th scope="row">{render_cell(name)}</th>{cells}</tr>')
    table_class = ' class="numeric"' if numeric else ""
    return (
        f'<div class="home-table" role="region" aria-labelledby="{heading_id}" tabindex="0">\n'
        f'<table{table_class}><thead><tr>{titles}</tr></thead><tbody>\n'
        + "\n".join(body) + "\n</tbody></table></div>"
    )



def render_speed_chart(rows: list[list[str]]) -> str:
    engines = rows[0][1:]
    baseline = engines.index("Rustia")
    short_labels = {
        "Build, sum, and sort a 256-item array": "Build, sum and sort an array",
        "Fill and read 128 hash entries": "Sparse table reads and writes",
        "One closure call updating a captured value": "Call a closure",
        "Custom, keyed, and array iteration pass": "Iterate over tables",
        "Math library calls and buffered appends": "Math and table appends",
    }
    workloads = rows[1:]
    ratios = []
    for _, *values in workloads:
        times = [float(value) for value in values]
        ratios.append([time / times[baseline] for time in times])
    scale = ceil(max(value for workload in ratios for value in workload))
    panels = []
    for (name, *values), times in zip(workloads, ratios):
        label = short_labels.get(name, name)
        bars = []
        for engine, time, value in zip(engines, times, values):
            engine_class = ' class="speed-baseline"' if engine == "Rustia" else ""
            ratio = "1" if engine == "Rustia" else f"{time:.2f}"
            bars.append(
                f'<li{engine_class} title="{escape(value)} µs per work unit">'
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
    speed_rows = read_table(readme, "| Work unit | Rustia |")
    luau = render_table(luau_rows, "luau-comparison")
    chart = render_speed_chart(speed_rows)
    speed = render_table(speed_rows, "timing-table-title", numeric=True)
    section = f'''<h3 id="luau-comparison">Luau runtimes</h3>
    <p>Capability snapshot checked {escape(checked)}. Under Replay, Tested means native and browser results matched. Controls means the host can fix the seed and clock. A dash means the linked sources did not establish support.</p>
    {luau}
    <p><a href="{README_URL}#rustia-and-other-luau-projects">Luau comparison sources and notes</a></p>
    <h3 id="speed-comparison">Runtime speed</h3>
    <p>Correctness comes first. We test Luau behaviour and deterministic replay while working to make Rustia faster.</p>
    <p><strong>Rustia takes 1× time on each task.</strong> Shorter bars are faster. For example, 0.5× takes half as long.</p>
    {chart}
    <p>Snapshot from {benchmark_date} UTC on Linux x86-64. These are best observed interpreter times for scripts already running. Each engine uses its own version of each task.</p>
    <details class="speed-details">
      <summary id="timing-table-title">Exact timings in microseconds per work unit</summary>
      {speed}
    </details>
    <p><a href="{README_URL}#speed-a-measured-snapshot">Benchmark methodology and results for more engines</a></p>'''
    sys.stdout.write(homepage.replace(MARKER, section))


if __name__ == "__main__":
    main()
