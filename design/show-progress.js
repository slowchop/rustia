#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");

function fail(message) {
  console.error(`show-progress: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const options = {
    color: process.env.NO_COLOR == null,
    source: path.join(root, "design", "progress.json"),
    width: process.stdout.columns || 104,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--color") {
      options.color = true;
    } else if (arg === "--no-color") {
      options.color = false;
    } else if (arg === "--width") {
      index += 1;
      if (index >= argv.length) {
        fail("--width requires a value");
      }
      options.width = Number(argv[index]);
    } else if (arg.startsWith("--width=")) {
      options.width = Number(arg.slice("--width=".length));
    } else if (arg.startsWith("-")) {
      fail(`unknown option ${arg}`);
    } else {
      options.source = path.resolve(root, arg);
    }
  }

  if (!Number.isInteger(options.width) || options.width < 88) {
    fail("--width must be an integer of at least 88");
  }
  options.width = Math.min(options.width, 132);
  return options;
}

function assertPercent(value, label) {
  if (!Number.isInteger(value) || value < 0 || value > 100) {
    fail(`${label} must be an integer percentage from 0 to 100`);
  }
}

function assertText(value, label) {
  if (typeof value !== "string" || value.trim() === "") {
    fail(`${label} must be a non-empty string`);
  }
}

function assertEntry(entry, label) {
  for (const key of ["name", "state", "future_plan"]) {
    assertText(entry[key], `${label}.${key}`);
  }
  assertPercent(entry.complete, `${label}.complete`);
  assertPercent(entry.confidence, `${label}.confidence`);
}

function readData(source) {
  let data;
  try {
    data = JSON.parse(fs.readFileSync(source, "utf8"));
  } catch (error) {
    fail(`could not parse ${path.relative(root, source)}: ${error.message}`);
  }

  if (!data.metadata || typeof data.metadata !== "object") {
    fail("metadata is required");
  }
  assertText(data.metadata.date, "metadata.date");
  assertText(data.metadata.scope, "metadata.scope");

  if (!Array.isArray(data.metadata.excludes)) {
    fail("metadata.excludes must be an array");
  }
  if (!Array.isArray(data.overall_goals) || data.overall_goals.length === 0) {
    fail("overall_goals must contain at least one entry");
  }
  data.overall_goals.forEach((entry, index) => assertEntry(entry, `overall_goals[${index}]`));

  if (!Array.isArray(data.areas) || data.areas.length === 0) {
    fail("areas must contain at least one group");
  }
  data.areas.forEach((area, areaIndex) => {
    assertText(area.area, `areas[${areaIndex}].area`);
    if (!Array.isArray(area.entries) || area.entries.length === 0) {
      fail(`areas[${areaIndex}].entries must contain at least one entry`);
    }
    area.entries.forEach((entry, entryIndex) => {
      assertEntry(entry, `areas[${areaIndex}].entries[${entryIndex}]`);
    });
  });

  return data;
}

function createStyler(colorEnabled) {
  function code(value, ansiCode) {
    if (!colorEnabled) {
      return value;
    }
    return `\u001b[${ansiCode}m${value}\u001b[0m`;
  }

  return {
    fg: (color, value) => code(value, `38;5;${color}`),
    bold: (value) => code(value, "1"),
    dim: (value) => code(value, "2"),
  };
}

function stripAnsi(value) {
  return String(value).replace(/\u001b\[[0-9;]*m/g, "");
}

function visibleLength(value) {
  return stripAnsi(value).length;
}

function padRight(value, width) {
  const padding = width - visibleLength(value);
  return padding > 0 ? value + " ".repeat(padding) : value;
}

function fitPlain(value, width) {
  const text = String(value);
  if (text.length <= width) {
    return text;
  }
  if (width <= 3) {
    return text.slice(0, width);
  }
  return `${text.slice(0, width - 3)}...`;
}

function clampPercent(value) {
  return Math.max(0, Math.min(100, value));
}

function average(values) {
  if (values.length === 0) {
    return 0;
  }
  return Math.round(values.reduce((sum, value) => sum + value, 0) / values.length);
}

function scoreColor(value) {
  if (value >= 90) {
    return 48;
  }
  if (value >= 75) {
    return 39;
  }
  if (value >= 60) {
    return 220;
  }
  if (value >= 45) {
    return 208;
  }
  return 203;
}

function confidenceColor(value) {
  if (value >= 80) {
    return 48;
  }
  if (value >= 70) {
    return 75;
  }
  if (value >= 60) {
    return 220;
  }
  return 203;
}

function wordWrap(value, width) {
  const words = String(value).replace(/\s+/g, " ").trim().split(" ").filter(Boolean);
  if (words.length === 0) {
    return [""];
  }

  const lines = [];
  let line = "";
  for (const word of words) {
    if (word.length > width) {
      if (line !== "") {
        lines.push(line);
        line = "";
      }
      for (let offset = 0; offset < word.length; offset += width) {
        lines.push(word.slice(offset, offset + width));
      }
    } else if (line === "") {
      line = word;
    } else if (line.length + 1 + word.length <= width) {
      line += ` ${word}`;
    } else {
      lines.push(line);
      line = word;
    }
  }
  if (line !== "") {
    lines.push(line);
  }
  return lines;
}

function emitWrapped(lines, text, firstPrefix, nextPrefix, width, style) {
  const wrapWidth = Math.max(20, width - visibleLength(firstPrefix));
  const wrapped = wordWrap(text, wrapWidth);
  lines.push(firstPrefix + (style ? style(wrapped[0]) : wrapped[0]));
  const nextWidth = Math.max(20, width - visibleLength(nextPrefix));
  for (const line of wrapped.slice(1).flatMap((part) => wordWrap(part, nextWidth))) {
    lines.push(nextPrefix + (style ? style(line) : line));
  }
}

function gradient(text, palette, style) {
  return [...text].map((char, index) => {
    if (char === " ") {
      return char;
    }
    return style.fg(palette[index % palette.length], char);
  }).join("");
}

function progressBar(value, width, style) {
  const percent = clampPercent(value);
  const filled = Math.round((percent / 100) * width);
  const empty = width - filled;
  return [
    style.fg(244, "["),
    style.fg(scoreColor(percent), "#".repeat(filled)),
    style.fg(238, "-".repeat(empty)),
    style.fg(244, "]"),
  ].join("");
}

function percent(value, style, colorFn = scoreColor) {
  return style.fg(colorFn(value), `${String(value).padStart(3, " ")}%`);
}

function heading(lines, title, width, style, color = 39) {
  const marker = `-- ${title.toUpperCase()} `;
  const filler = "-".repeat(Math.max(0, width - marker.length));
  lines.push("");
  lines.push(style.fg(color, `${marker}${filler}`));
}

function frameLine(lines, content, width, style) {
  lines.push(`${style.fg(240, "| ")}${padRight(content, width - 4)}${style.fg(240, " |")}`);
}

function emitHeader(lines, data, sourcePath, width, style) {
  const border = style.fg(240, `+${"-".repeat(width - 2)}+`);
  const title = gradient("RUSTIA PROGRESS", [51, 45, 39, 33, 63, 99, 135, 171, 207, 214], style);
  const source = path.relative(root, sourcePath);
  const sourceLine = `${style.dim("source:")} ${source} ${style.dim("| date:")} ${data.metadata.date}`;

  lines.push(border);
  frameLine(lines, `${style.bold(title)} ${style.dim(":: 256-color console report")}`, width, style);
  frameLine(lines, sourceLine, width, style);
  for (const line of wordWrap(data.metadata.scope, width - 4)) {
    frameLine(lines, style.fg(250, line), width, style);
  }
  lines.push(border);
}

function emitLabelValue(lines, label, value, width, style) {
  const labelWidth = 18;
  const prefix = `  ${padRight(style.fg(250, label), labelWidth)} `;
  const nextPrefix = `  ${" ".repeat(labelWidth)} `;
  emitWrapped(lines, value, prefix, nextPrefix, width, (line) => style.fg(245, line));
}

function emitFacts(lines, data, width, style) {
  const facts = data.metadata.canonical_facts || {};
  heading(lines, "Canonical Facts", width, style, 45);

  if (facts.luau_target) {
    emitLabelValue(lines, "Luau target", facts.luau_target, width, style);
  }

  if (facts.conformance) {
    const conformance = facts.conformance;
    emitLabelValue(
      lines,
      "Conformance",
      `${conformance.fixtures} fixtures | ${conformance.refs} refs | ${conformance.pending} pending`,
      width,
      style,
    );
  }

  if (facts.benchmark_adapters) {
    const adapters = facts.benchmark_adapters;
    const names = Array.isArray(adapters.names) ? adapters.names.join(", ") : "unknown";
    emitLabelValue(
      lines,
      "Bench adapters",
      `${adapters.present}/${adapters.requested} present | ${names}`,
      width,
      style,
    );
    if (adapters.note) {
      emitLabelValue(lines, "Adapter note", adapters.note, width, style);
    }
  }
}

function emitGoals(lines, goals, width, style) {
  heading(lines, "Overall Goals", width, style, 51);

  const nameWidth = Math.max(30, Math.min(40, Math.floor(width * 0.36)));
  const barWidth = Math.max(18, Math.min(30, width - nameWidth - 32));
  for (const goal of goals) {
    const name = padRight(style.fg(252, fitPlain(goal.name, nameWidth)), nameWidth);
    lines.push(
      `  ${name} ${percent(goal.complete, style)} ${progressBar(goal.complete, barWidth, style)} ` +
      `${style.dim("conf")} ${percent(goal.confidence, style, confidenceColor)}`,
    );
    emitWrapped(
      lines,
      goal.state,
      `    ${style.fg(244, "state:")} `,
      "           ",
      width,
      (line) => style.fg(245, line),
    );
    emitWrapped(
      lines,
      goal.future_plan,
      `    ${style.fg(244, "next :")} `,
      "           ",
      width,
      (line) => style.fg(250, line),
    );
  }
}

function areaStats(area) {
  return {
    complete: average(area.entries.map((entry) => entry.complete)),
    confidence: average(area.entries.map((entry) => entry.confidence)),
  };
}

function emitAreaMatrix(lines, areas, width, style) {
  heading(lines, "Area Matrix", width, style, 75);

  const nameWidth = 22;
  const barWidth = 24;
  areas.forEach((area, index) => {
    const stats = areaStats(area);
    const ordinal = style.dim(String(index + 1).padStart(2, "0"));
    const name = padRight(style.fg(252, fitPlain(area.area, nameWidth)), nameWidth);
    lines.push(
      `  ${ordinal} ${name} ${percent(stats.complete, style)} ` +
      `${progressBar(stats.complete, barWidth, style)} ${style.dim("conf")} ` +
      `${percent(stats.confidence, style, confidenceColor)} ` +
      `${style.dim(`${area.entries.length} ${area.entries.length === 1 ? "item" : "items"}`)}`,
    );
  });
}

function emitAreaDrilldown(lines, areas, width, style) {
  heading(lines, "Area Drilldown", width, style, 111);

  const nameWidth = Math.max(30, Math.min(44, Math.floor(width * 0.4)));
  const barWidth = Math.max(14, Math.min(24, width - nameWidth - 38));
  areas.forEach((area) => {
    const stats = areaStats(area);
    const title = `${area.area} | ${stats.complete}% avg complete | ${stats.confidence}% avg confidence`;
    const line = `  + ${title} `;
    lines.push(style.fg(scoreColor(stats.complete), `${line}${"-".repeat(Math.max(0, width - line.length))}`));
    for (const entry of area.entries) {
      const name = padRight(style.fg(252, fitPlain(entry.name, nameWidth)), nameWidth);
      lines.push(
        `  | ${name} ${percent(entry.complete, style)} ` +
        `${progressBar(entry.complete, barWidth, style)} ${style.dim("conf")} ` +
        `${percent(entry.confidence, style, confidenceColor)}`,
      );
    }
  });
}

function emitWorkQueue(lines, areas, width, style) {
  heading(lines, "Lowest Completion Next Steps", width, style, 214);

  const items = areas
    .flatMap((area) => area.entries.map((entry) => ({ ...entry, area: area.area })))
    .sort((left, right) => (
      left.complete - right.complete ||
      left.confidence - right.confidence ||
      left.name.localeCompare(right.name)
    ))
    .slice(0, 8);

  const nameWidth = Math.max(28, Math.min(42, Math.floor(width * 0.38)));
  for (const item of items) {
    const name = padRight(style.fg(252, fitPlain(item.name, nameWidth)), nameWidth);
    lines.push(
      `  ${percent(item.complete, style)} ${name} ${style.dim(fitPlain(item.area, 18))} ` +
      `${progressBar(item.complete, 14, style)}`,
    );
    emitWrapped(
      lines,
      item.future_plan,
      `      ${style.fg(244, "next:")} `,
      "            ",
      width,
      (line) => style.fg(245, line),
    );
  }
}

function emitNotes(lines, data, width, style) {
  const notes = Array.isArray(data.metadata.notes) ? data.metadata.notes : [];
  const excludes = Array.isArray(data.metadata.excludes) ? data.metadata.excludes : [];
  if (notes.length === 0 && excludes.length === 0) {
    return;
  }

  heading(lines, "Notes", width, style, 250);
  for (const note of notes) {
    emitWrapped(lines, note, `  ${style.fg(244, "-")} `, "    ", width, (line) => style.fg(245, line));
  }
  if (excludes.length !== 0) {
    emitWrapped(
      lines,
      `Out of scope: ${excludes.join(", ")}`,
      `  ${style.fg(244, "-")} `,
      "    ",
      width,
      (line) => style.fg(245, line),
    );
  }
}

function render(data, options) {
  const style = createStyler(options.color);
  const lines = [];

  emitHeader(lines, data, options.source, options.width, style);
  emitGoals(lines, data.overall_goals, options.width, style);
  emitFacts(lines, data, options.width, style);
  emitAreaMatrix(lines, data.areas, options.width, style);
  emitWorkQueue(lines, data.areas, options.width, style);
  emitAreaDrilldown(lines, data.areas, options.width, style);
  emitNotes(lines, data, options.width, style);

  return lines.join("\n");
}

const options = parseArgs(process.argv.slice(2));
const data = readData(options.source);
console.log(render(data, options));
