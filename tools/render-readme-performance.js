"use strict";

const assert = require("node:assert/strict");

// Display order groups implementations by language, independent of their times.
const engines = [
  { id: "rustia", name: "Rustia", mode: "interpreter" },
  { id: "luaur", name: "luaur", mode: "interpreter" },
  { id: "ruau", name: "ruau", mode: "interpreter" },
  { id: "mlua-luau", name: "Luau (C++) via mlua", mode: "interpreter" },
  { id: "mlua-luau-jit", name: "Luau (C++) via mlua", mode: "jit" },
  { id: "koto", name: "Koto", mode: "interpreter" },
  { id: "rhai", name: "Rhai", mode: "interpreter" },
  { id: "roto", name: "Roto", mode: "native" },
];
const modeLabels = { interpreter: "Interpreter", jit: "JIT", native: "Native compiled" };

const headings = {
  dense_tables: ["Array sort", "256 items"],
  sparse_tables: ["Sparse lookup", "128 keys"],
  closures_upvalues: ["State update", "one call"],
  generic_iteration: ["Iteration", "custom + tables"],
  stdlib_math_table: ["Math + table", "calls + appends"],
};

function measuredTime(cohort, engine, mode, workload, unit) {
  const label = engine + "/" + workload;
  const matches = cohort.filter(row => row.engine === engine && row.workload === workload);
  assert.equal(matches.length, 1, "expected one history row for " + label + ", found " + matches.length);
  const [row] = matches;
  assert.ok(row.host === "ci-bench" && row.execution_mode === mode, label + " must be a ci-bench " + mode + " result");
  const family = engine === "mlua-luau-jit" ? "mlua-luau" : engine;
  assert.equal(row.mode_family, family, label + " has the wrong execution-mode family");
  assert.equal(row.unit, unit, label + ": unexpected history unit " + JSON.stringify(row.unit));
  assert.ok(Number.isFinite(row.ns_per_unit_min) && row.ns_per_unit_min > 0, label + " has no valid minimum time");
  if (mode === "jit") {
    assert.ok(row.native_verified === true && row.native_verification === "ab-native-sentinel"
      && Number.isFinite(row.native_verification_ratio) && row.native_verification_ratio >= 1.5,
    label + " lacks its verified Luau JIT sentinel");
  } else if (mode === "native") {
    assert.ok(row.native_verified === true && row.native_verification === "cranelift-function-construction",
      label + " lacks verified Cranelift function construction");
  }
  return row.ns_per_unit_min;
}

// Same fastest-time / observed-time bands as the absolute workload heatmap in
// bench/render-current-results.js. The printed values also work without color.
const shades = [
  { atLeast: 0.90, fill: "#195c48", ink: "#ffffff" },
  { atLeast: 0.50, fill: "#39745d", ink: "#ffffff" },
  { atLeast: 0.25, fill: "#8cbba1", ink: "#142c23" },
  { atLeast: 0.10, fill: "#bad8c6", ink: "#142c23" },
  { atLeast: 0.04, fill: "#d6e8dc", ink: "#142c23" },
  { atLeast: 0.00, fill: "#edf3ee", ink: "#142c23" },
];

function formatTime(ns) {
  return (ns / 1000).toFixed(3).replace(/\B(?=(\d{3})+\.)/g, ",");
}

function renderPerformance(snapshot, cohort) {
  const workloads = Object.keys(headings);
  assert.deepEqual(Object.keys(snapshot.workloads).sort(), [...workloads].sort(), "chart requires all five retained workloads");
  assert.equal(snapshot.target, "linux-x86_64", "chart requires the Linux x86-64 target");
  const times = engines.map(engine => workloads.map(workload =>
    measuredTime(cohort, engine.id, engine.mode, workload, snapshot.workloads[workload].unit)));
  const fastest = workloads.map((_, column) => Math.min(...times.map(row => row[column])));
  const descriptions = engines.map((engine, i) =>
    engine.name + ", " + modeLabels[engine.mode] + ": " + times[i].map(formatTime).join(", ") + ".");
  const date = new Date(snapshot.start * 1000).toISOString().slice(0, 10);
  const source = "ci-bench · Linux x86-64 · " + date + " UTC · " + snapshot.commit.slice(0, 7);
  const rotoNote = "Roto searches a list for sparse lookups and calls a regular function for the state update.";
  const svg = [
    '<svg xmlns="http://www.w3.org/2000/svg" width="860" height="471" viewBox="0 0 860 471" style="max-width:100%;height:auto" role="img" aria-labelledby="rustia-perf-title rustia-perf-desc">',
    '<title id="rustia-perf-title">Time per work unit in microseconds</title>',
    '<desc id="rustia-perf-desc">' + source + '. Best observed steady-state times, lower is better. Workloads in order: '
      + workloads.map(workload => snapshot.workloads[workload].label).join(", ") + ". " + rotoNote + " " + descriptions.join(" ") + '</desc>',
    '<rect width="860" height="471" rx="12" fill="#fafcf9"/>',
    '<g font-family="system-ui, sans-serif" fill="#172d25">',
    '<text x="20" y="36" font-size="26" font-weight="700">Time per work unit</text>',
    '<text x="20" y="60" font-size="15">Best observed steady-state time in microseconds (µs). Lower is better.</text>',
    '<text x="20" y="104" font-size="15" font-weight="650">Engine</text>',
    '<text x="174" y="104" font-size="15" font-weight="650">Mode</text>',
  ];
  const firstColumn = 292;
  const columnWidth = 110;
  workloads.forEach((workload, column) => {
    const x = firstColumn + columnWidth * column + columnWidth / 2;
    const [label, detail] = headings[workload];
    svg.push(
      '<text x="' + x + '" y="95" text-anchor="middle" font-size="15" font-weight="650">' + label + '</text>',
      '<text x="' + x + '" y="113" text-anchor="middle" font-size="13" fill="#476054">' + detail + '</text>');
  });
  engines.forEach((engine, row) => {
    const y = 123 + row * 33;
    const baseline = y + 21;
    const weight = engine.id === "rustia" ? "700" : "550";
    svg.push('<text x="20" y="' + baseline + '" font-size="15" font-weight="' + weight + '">' + engine.name + '</text>',
      '<text x="174" y="' + baseline + '" font-size="14" fill="#395346">' + modeLabels[engine.mode] + '</text>');
    times[row].forEach((ns, column) => {
      const x = firstColumn + columnWidth * column;
      const score = fastest[column] / ns;
      const { fill, ink } = shades.find(shade => score >= shade.atLeast);
      svg.push('<rect x="' + (x + 1) + '" y="' + y + '" width="108" height="30" rx="4" fill="' + fill + '"/>',
        '<text x="' + (x + columnWidth / 2) + '" y="' + baseline + '" text-anchor="middle" font-size="16" font-weight="600" fill="' + ink + '">' + formatTime(ns) + '</text>');
    });
  });
  svg.push('<path d="M20 396H840" stroke="#c7d6cc"/>',
    '<text x="20" y="418" font-size="14" fill="#395346">Darker cells mark shorter times within each column.</text>',
    '<text x="20" y="438" font-size="14" fill="#395346">' + rotoNote + '</text>',
    '<text x="20" y="460" font-size="13" fill="#476054">' + source + '</text>',
    '</g>', '</svg>');
  return svg.join("\n") + "\n";
}

module.exports = { measuredTime, renderPerformance };
