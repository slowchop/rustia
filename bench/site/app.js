const state = {
  records: [],
  target: "",
  workload: "",
  wasmGate: null,
  wasmGateError: "",
};

const colors = [
  "#2563eb",
  "#0f766e",
  "#b45309",
  "#b91c1c",
  "#7c3aed",
  "#047857",
  "#be185d",
  "#475569",
];

const els = {
  reload: document.querySelector("#reload"),
  file: document.querySelector("#history-file"),
  status: document.querySelector("#status"),
  target: document.querySelector("#target-select"),
  workload: document.querySelector("#workload-select"),
  records: document.querySelector("#metric-records"),
  engines: document.querySelector("#metric-engines"),
  workloads: document.querySelector("#metric-workloads"),
  commit: document.querySelector("#metric-commit"),
  wasmGateStatus: document.querySelector("#wasm-gate-status"),
  wasmGateSummary: document.querySelector("#wasm-gate-summary"),
  wasmGateProgress: document.querySelector("#wasm-gate-progress"),
  wasmGateCount: document.querySelector("#wasm-gate-count"),
  wasmGateList: document.querySelector("#wasm-gate-list"),
  leaderboardSubtitle: document.querySelector("#leaderboard-subtitle"),
  leaderboardBody: document.querySelector("#leaderboard-body"),
  chartSubtitle: document.querySelector("#chart-subtitle"),
  chart: document.querySelector("#history-chart"),
  legend: document.querySelector("#legend"),
  runsSubtitle: document.querySelector("#runs-subtitle"),
  runsBody: document.querySelector("#runs-body"),
};

els.reload.addEventListener("click", () => loadDefaultHistory());
els.file.addEventListener("change", () => loadSelectedFile());
els.target.addEventListener("change", () => {
  state.target = els.target.value;
  render();
});
els.workload.addEventListener("change", () => {
  state.workload = els.workload.value;
  render();
});
window.addEventListener("resize", () => drawChart());

loadDefaultHistory();
loadWasmGate();

async function loadDefaultHistory() {
  setStatus("Loading history");
  try {
    const response = await fetch("../history/runs.jsonl", { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    const text = await response.text();
    setRecords(parseHistory(text));
    setStatus("Loaded ../history/runs.jsonl");
  } catch (error) {
    setRecords([]);
    setStatus(`Could not load history: ${error.message}`);
  }
}

async function loadWasmGate() {
  try {
    const response = await fetch("../wasm-gate.json", { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    state.wasmGate = normalizeWasmGate(await response.json());
    state.wasmGateError = "";
  } catch (error) {
    state.wasmGate = null;
    state.wasmGateError = error.message;
  }
  renderWasmGate();
}

async function loadSelectedFile() {
  const [file] = els.file.files;
  if (!file) {
    return;
  }
  setStatus(`Loading ${file.name}`);
  try {
    const text = await file.text();
    setRecords(parseHistory(text));
    setStatus(`Loaded ${file.name}`);
  } catch (error) {
    setStatus(`Could not parse file: ${error.message}`);
  }
}

function parseHistory(text) {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line, index) => {
      try {
        return normalizeRecord(JSON.parse(line));
      } catch (error) {
        throw new Error(`line ${index + 1}: ${error.message}`);
      }
    })
    .filter((record) => Number.isFinite(record.ns_per_unit_min));
}

function normalizeRecord(record) {
  return {
    timestamp_unix: Number(record.timestamp_unix),
    commit: String(record.commit ?? "unknown"),
    target: String(record.target ?? "unknown"),
    engine: String(record.engine ?? "unknown"),
    dialect: String(record.dialect ?? "unknown"),
    workload: String(record.workload ?? "unknown"),
    unit: String(record.unit ?? ""),
    n: Number(record.n),
    reps: Number(record.reps),
    ns_per_unit_min: Number(record.ns_per_unit_min),
    ns_per_unit_median: Number(record.ns_per_unit_median),
    units_per_sec: Number(record.units_per_sec),
    checksum: String(record.checksum ?? ""),
  };
}

function setRecords(records) {
  state.records = records.sort(compareRunTime);
  const targets = unique(records.map((record) => record.target));
  const workloads = unique(records.map((record) => record.workload));
  state.target = targets.includes(state.target) ? state.target : targets[0] ?? "";
  state.workload = workloads.includes(state.workload)
    ? state.workload
    : workloads[0] ?? "";
  populateSelect(els.target, targets, state.target);
  populateSelect(els.workload, workloads, state.workload);
  render();
}

function render() {
  renderMetrics();
  renderWasmGate();
  renderLeaderboard();
  renderRuns();
  drawChart();
}

function renderMetrics() {
  const latest = latestRecord(state.records);
  els.records.textContent = formatInteger(state.records.length);
  els.engines.textContent = formatInteger(
    unique(state.records.map((record) => record.engine)).length,
  );
  els.workloads.textContent = formatInteger(
    unique(state.records.map((record) => record.workload)).length,
  );
  els.commit.textContent = latest?.commit ?? "n/a";
}

function normalizeWasmGate(data) {
  const requirements = Array.isArray(data.requirements)
    ? data.requirements.map((requirement) => ({
      id: String(requirement.id ?? ""),
      label: String(requirement.label ?? requirement.id ?? "Unnamed requirement"),
      engine: String(requirement.engine ?? "unknown"),
      runtime: String(requirement.runtime ?? ""),
      required: requirement.required !== false,
      status: String(requirement.status ?? "unknown"),
      evidence: Array.isArray(requirement.evidence)
        ? requirement.evidence.map((item) => ({
          kind: String(item.kind ?? "evidence"),
          path: item.path == null ? "" : String(item.path),
          summary: String(item.summary ?? ""),
        }))
        : [],
      next: String(requirement.next ?? ""),
    }))
    : [];
  return {
    schema: String(data.schema ?? ""),
    updated: String(data.updated ?? ""),
    status: String(data.status ?? "unknown"),
    gate_complete: Boolean(data.gate_complete),
    summary: String(data.summary ?? ""),
    requirements,
  };
}

function renderWasmGate() {
  if (!state.wasmGate) {
    els.wasmGateStatus.textContent = state.wasmGateError ? "missing" : "loading";
    els.wasmGateStatus.className = "chip chip-warn";
    els.wasmGateSummary.textContent = state.wasmGateError
      ? `Could not load ../wasm-gate.json: ${state.wasmGateError}`
      : "Loading wasm gate data.";
    els.wasmGateProgress.style.inlineSize = "0%";
    els.wasmGateCount.textContent = "0 of 0 required measurements complete";
    els.wasmGateList.replaceChildren();
    return;
  }

  const required = state.wasmGate.requirements.filter((item) => item.required);
  const complete = required.filter((item) => isCompleteStatus(item.status)).length;
  const total = required.length;
  const percent = total === 0 ? 100 : Math.round((complete / total) * 100);
  els.wasmGateStatus.textContent = state.wasmGate.gate_complete ? "closed" : state.wasmGate.status;
  els.wasmGateStatus.className = `chip ${state.wasmGate.gate_complete ? "chip-good" : "chip-warn"}`;
  els.wasmGateSummary.textContent = state.wasmGate.summary;
  els.wasmGateProgress.style.inlineSize = `${percent}%`;
  els.wasmGateCount.textContent = `${complete} of ${total} required measurements complete`;
  els.wasmGateList.replaceChildren(
    ...state.wasmGate.requirements.map((requirement) => gateRequirementElement(requirement)),
  );
}

function gateRequirementElement(requirement) {
  const article = document.createElement("article");
  article.className = "gate-item";
  const heading = rowElement("div", [
    rowElement("div", [
      rowElement("strong", [document.createTextNode(requirement.label)]),
      rowElement("span", [document.createTextNode(`${requirement.engine} · ${requirement.runtime}`)]),
    ]),
    chip(requirement.status, isCompleteStatus(requirement.status) ? "good" : "warn"),
  ]);
  heading.className = "gate-item-heading";

  const evidence = requirement.evidence.length
    ? rowElement("ul", requirement.evidence.map((item) => evidenceElement(item)))
    : rowElement("p", [document.createTextNode("No committed production measurement yet.")]);
  evidence.className = requirement.evidence.length ? "evidence-list" : "gate-empty";

  const next = rowElement("p", [document.createTextNode(requirement.next)]);
  next.className = "gate-next";
  article.append(heading, evidence, next);
  return article;
}

function evidenceElement(item) {
  const text = `${item.kind}: ${item.summary}`;
  const li = document.createElement("li");
  if (item.path) {
    const anchor = document.createElement("a");
    anchor.href = `../../${item.path}`;
    anchor.textContent = text;
    li.append(anchor);
  } else {
    li.textContent = text;
  }
  return li;
}

function chip(text, tone) {
  const span = document.createElement("span");
  span.className = `chip chip-${tone}`;
  span.textContent = text;
  return span;
}

function isCompleteStatus(status) {
  return ["complete", "measured", "closed"].includes(status);
}

function renderLeaderboard() {
  const rows = currentRows().sort(
    (left, right) => left.ns_per_unit_min - right.ns_per_unit_min,
  );
  const rustia = rows.find((row) => row.engine === "rustia");
  els.leaderboardSubtitle.textContent = state.workload || "No workload";
  els.leaderboardBody.replaceChildren(
    ...rows.map((row) => {
      const previous = previousRecord(row);
      const trend = previous
        ? (row.ns_per_unit_min / previous.ns_per_unit_min - 1) * 100
        : null;
      return rowElement("tr", [
        rowElement("td", [
          engineLabel(row.engine),
        ]),
        cell(formatNumber(row.ns_per_unit_min)),
        cell(formatNumber(row.ns_per_unit_median)),
        cell(rustia ? `${(rustia.ns_per_unit_min / row.ns_per_unit_min).toFixed(2)}x` : "n/a"),
        trendCell(trend),
        cell(row.commit),
      ]);
    }),
  );
}

function renderRuns() {
  const rows = latestBy(
    state.records,
    (record) => `${record.target}\0${record.engine}\0${record.workload}`,
  )
    .sort(compareRunTime)
    .reverse()
    .slice(0, 30);
  els.runsSubtitle.textContent = `${rows.length} rows`;
  els.runsBody.replaceChildren(
    ...rows.map((row) =>
      rowElement("tr", [
        cell(formatTime(row.timestamp_unix)),
        cell(row.target),
        rowElement("td", [engineLabel(row.engine)]),
        cell(row.workload),
        cell(formatNumber(row.ns_per_unit_min)),
        cell(formatInteger(Math.round(row.units_per_sec))),
      ]),
    ),
  );
}

function drawChart() {
  const canvas = els.chart;
  const context = canvas.getContext("2d");
  const width = canvas.clientWidth || 960;
  const height = canvas.clientHeight || 360;
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.floor(width * ratio);
  canvas.height = Math.floor(height * ratio);
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, width, height);

  const records = state.records.filter(
    (record) => record.target === state.target && record.workload === state.workload,
  );
  els.chartSubtitle.textContent = state.target || "No target";

  if (records.length === 0) {
    drawEmpty(context, width, height);
    els.legend.replaceChildren();
    return;
  }

  const series = [...groupBy(records, (record) => record.engine)]
    .map(([engine, values], index) => ({
      engine,
      color: engineColor(engine, index),
      values: values.sort(compareRunTime),
    }))
    .sort((left, right) => left.engine.localeCompare(right.engine));

  const values = records.map((record) => record.ns_per_unit_min);
  const minY = Math.log10(Math.min(...values) * 0.85);
  const maxY = Math.log10(Math.max(...values) * 1.15);
  const minX = Math.min(...records.map((record) => record.timestamp_unix));
  const maxX = Math.max(...records.map((record) => record.timestamp_unix));
  const pad = { left: 64, right: 18, top: 20, bottom: 46 };
  const plotWidth = Math.max(1, width - pad.left - pad.right);
  const plotHeight = Math.max(1, height - pad.top - pad.bottom);
  const xScale = (timestamp) => {
    if (maxX === minX) {
      return pad.left + plotWidth / 2;
    }
    return pad.left + ((timestamp - minX) / (maxX - minX)) * plotWidth;
  };
  const yScale = (value) =>
    pad.top + (1 - (Math.log10(value) - minY) / (maxY - minY || 1)) * plotHeight;

  drawAxes(context, width, height, pad, minY, maxY);
  for (const item of series) {
    drawLine(context, item.values, item.color, xScale, yScale);
  }

  els.legend.replaceChildren(
    ...series.map((item) =>
      rowElement("span", [
        colorDot(item.color),
        document.createTextNode(item.engine),
      ]),
    ),
  );
}

function drawAxes(context, width, height, pad, minY, maxY) {
  context.save();
  context.strokeStyle = "#dfe3ea";
  context.fillStyle = "#616b7a";
  context.lineWidth = 1;
  context.font = "12px ui-sans-serif, system-ui, sans-serif";
  context.beginPath();
  context.moveTo(pad.left, pad.top);
  context.lineTo(pad.left, height - pad.bottom);
  context.lineTo(width - pad.right, height - pad.bottom);
  context.stroke();

  for (let i = 0; i <= 4; i += 1) {
    const t = i / 4;
    const y = pad.top + t * (height - pad.top - pad.bottom);
    const value = 10 ** (maxY - t * (maxY - minY));
    context.strokeStyle = i === 4 ? "#dfe3ea" : "#edf0f5";
    context.beginPath();
    context.moveTo(pad.left, y);
    context.lineTo(width - pad.right, y);
    context.stroke();
    context.fillText(formatNumber(value), 8, y + 4);
  }
  context.restore();
}

function drawLine(context, values, color, xScale, yScale) {
  context.save();
  context.strokeStyle = color;
  context.fillStyle = color;
  context.lineWidth = 2;
  context.beginPath();
  values.forEach((record, index) => {
    const x = xScale(record.timestamp_unix);
    const y = yScale(record.ns_per_unit_min);
    if (index === 0) {
      context.moveTo(x, y);
    } else {
      context.lineTo(x, y);
    }
  });
  context.stroke();
  for (const record of values) {
    const x = xScale(record.timestamp_unix);
    const y = yScale(record.ns_per_unit_min);
    context.beginPath();
    context.arc(x, y, 3, 0, Math.PI * 2);
    context.fill();
  }
  context.restore();
}

function drawEmpty(context, width, height) {
  context.save();
  context.fillStyle = "#616b7a";
  context.font = "14px ui-sans-serif, system-ui, sans-serif";
  context.textAlign = "center";
  context.fillText("No records", width / 2, height / 2);
  context.restore();
}

function currentRows() {
  return latestBy(
    state.records.filter(
      (record) => record.target === state.target && record.workload === state.workload,
    ),
    (record) => record.engine,
  );
}

function previousRecord(row) {
  const rows = state.records
    .filter(
      (record) =>
        record.target === row.target &&
        record.workload === row.workload &&
        record.engine === row.engine &&
        record.timestamp_unix < row.timestamp_unix,
    )
    .sort(compareRunTime);
  return rows.at(-1) ?? null;
}

function latestBy(records, keyFn) {
  const map = new Map();
  for (const record of records) {
    const key = keyFn(record);
    const existing = map.get(key);
    if (!existing || compareRunTime(existing, record) < 0) {
      map.set(key, record);
    }
  }
  return [...map.values()];
}

function latestRecord(records) {
  return records.length ? records.reduce((latest, record) =>
    compareRunTime(latest, record) < 0 ? record : latest,
  ) : null;
}

function compareRunTime(left, right) {
  return (
    left.timestamp_unix - right.timestamp_unix ||
    left.commit.localeCompare(right.commit) ||
    left.engine.localeCompare(right.engine) ||
    left.workload.localeCompare(right.workload)
  );
}

function groupBy(records, keyFn) {
  const map = new Map();
  for (const record of records) {
    const key = keyFn(record);
    const values = map.get(key) ?? [];
    values.push(record);
    map.set(key, values);
  }
  return map;
}

function populateSelect(select, values, selected) {
  select.replaceChildren(
    ...values.map((value) => {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = value;
      option.selected = value === selected;
      return option;
    }),
  );
}

function rowElement(tag, children) {
  const element = document.createElement(tag);
  element.append(...children);
  return element;
}

function cell(text) {
  const td = document.createElement("td");
  td.textContent = text;
  return td;
}

function trendCell(value) {
  const td = document.createElement("td");
  if (value === null || !Number.isFinite(value)) {
    td.textContent = "n/a";
    return td;
  }
  td.textContent = `${value > 0 ? "+" : ""}${value.toFixed(1)}%`;
  td.className = value <= 0 ? "trend-good" : "trend-bad";
  return td;
}

function engineLabel(engine) {
  const label = rowElement("span", [
    colorDot(engineColor(engine)),
    document.createTextNode(engine),
  ]);
  label.className = "engine";
  return label;
}

function colorDot(color) {
  const dot = document.createElement("span");
  dot.className = "swatch";
  dot.style.color = color;
  return dot;
}

function engineColor(engine, fallbackIndex = 0) {
  const known = {
    rustia: "#2563eb",
    koto: "#0f766e",
    "mlua-luau": "#b45309",
    "mlua-lua54": "#7c3aed",
    rhai: "#b91c1c",
    boa: "#047857",
    rquickjs: "#be185d",
  };
  return known[engine] ?? colors[fallbackIndex % colors.length];
}

function unique(values) {
  return [...new Set(values)].sort((left, right) => left.localeCompare(right));
}

function formatNumber(value) {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  if (Math.abs(value) >= 1000) {
    return Math.round(value).toLocaleString();
  }
  return value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function formatInteger(value) {
  return Number(value || 0).toLocaleString();
}

function formatTime(timestamp) {
  if (!Number.isFinite(timestamp)) {
    return "n/a";
  }
  return new Date(timestamp * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function setStatus(message) {
  els.status.textContent = message;
}
