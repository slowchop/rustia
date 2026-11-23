"""Render the two maintained README comparison tables for the static homepage."""

from datetime import datetime, timezone
from html import escape, unescape
import json
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


def render_table(readme: str, header: str, heading_id: str, numeric: bool = False) -> str:
    lines = readme[readme.index(header):].split("\n\n", 1)[0].splitlines()
    rows = [[cell.strip() for cell in line.strip("|").split("|")] for line in lines]
    if len(rows) < 3 or any(len(row) != len(rows[0]) for row in rows):
        raise ValueError(f"Invalid README comparison table: {header}")
    titles = "".join(f'<th scope="col">{render_cell(cell)}</th>' for cell in rows[0])
    body = []
    for name, *values in rows[2:]:
        cells = "".join(f"<td>{render_cell(cell)}</td>" for cell in values)
        body.append(f'<tr><th scope="row">{render_cell(name)}</th>{cells}</tr>')
    table_class = ' class="numeric"' if numeric else ""
    return (
        f'<div class="home-table" role="region" aria-labelledby="{heading_id}" tabindex="0">\n'
        f'<table{table_class}><thead><tr>{titles}</tr></thead><tbody>\n'
        + "\n".join(body) + "\n</tbody></table></div>"
    )


def main() -> None:
    readme = (SITE.parent / "README.md").read_text(encoding="utf-8")
    homepage = (SITE / "index.html").read_text(encoding="utf-8")
    if homepage.count(MARKER) != 1:
        raise ValueError("Expected exactly one homepage comparison marker")
    snapshot = json.loads(readme.split("<!-- rustia-benchmark-snapshot", 1)[1].split("-->", 1)[0])
    benchmark_date = datetime.fromtimestamp(snapshot["start"], timezone.utc).date().isoformat()
    checked = re.search(r"\*\*Sources and details\*\* · Checked ([\w ]+)\.", readme)[1]
    luau = render_table(readme, "| Project | Engine | Desktop |", "luau-comparison")
    speed = render_table(readme, "| Work unit | Rustia |", "speed-comparison", numeric=True)
    section = f'''<h3 id="luau-comparison">Luau runtimes</h3>
    <p>Capability snapshot checked {escape(checked)}. Under Replay, Tested means native and browser results matched. Controls means the host can fix the seed and clock. A dash means the linked sources did not establish support.</p>
    {luau}
    <p><a href="{README_URL}#rustia-and-other-luau-projects">Luau comparison sources and notes</a></p>
    <h3 id="speed-comparison">Runtime speed</h3>
    <p>Microseconds per work unit (lower is better). Best observed steady-state interpreter times on Linux x86-64 on {benchmark_date} UTC. The measurements cover scripts already running. Each engine uses its own version of the workload.</p>
    {speed}
    <p><a href="{README_URL}#speed-a-measured-snapshot">Benchmark methodology and results for more engines</a></p>'''
    sys.stdout.write(homepage.replace(MARKER, section))


if __name__ == "__main__":
    main()
