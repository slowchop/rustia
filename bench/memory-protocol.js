"use strict";

// Version the measurement semantics separately from the stable history container.
// Missing row metadata means the original, unadjusted historical protocol.
const LEGACY_PROTOCOL = "cross_engine_memory_v1";
const MEMORY_PROTOCOL = "cross_engine_memory_v2";
const QUIESCENCE_MS = 100;

function rowProtocol(row) {
  const protocol = row.measurement_protocol ?? LEGACY_PROTOCOL;
  if (![LEGACY_PROTOCOL, MEMORY_PROTOCOL].includes(protocol)) {
    throw new Error(`unknown memory measurement protocol ${JSON.stringify(protocol)}`);
  }
  return protocol;
}

function requireSingleProtocol(rows, label) {
  const protocols = new Set(rows.map(rowProtocol));
  if (protocols.size > 1) {
    throw new Error(`${label} mixes memory measurement protocols; re-measure the complete comparison`);
  }
  return protocols.values().next().value ?? LEGACY_PROTOCOL;
}

function retainedMetric(row) {
  return rowProtocol(row) === MEMORY_PROTOCOL
    ? "control_adjusted_retained_delta_anon_kib"
    : "quiescent_retained_delta_anon_kib";
}

function validateProtocolMetrics(row) {
  if (rowProtocol(row) !== MEMORY_PROTOCOL) {
    if (row.metrics?.control_adjusted_retained_delta_anon_kib || row.metrics?.anonymous_memory_area_kib_seconds_per_unit) {
      throw new Error("legacy memory row contains unlabelled v2 metrics");
    }
    return;
  }
  for (const metric of ["control_adjusted_retained_delta_anon_kib", "null_retained_delta_anon_kib", "anonymous_memory_area_kib_seconds_per_unit"]) {
    const value = row.metrics?.[metric];
    if (!value || !Number.isSafeInteger(value.sample_count) || value.sample_count < 9 ||
        !["min", "q1", "median", "q3", "max"].every(key => Number.isFinite(value[key]))) {
      throw new Error(`v2 memory row lacks valid ${metric}`);
    }
  }
  const count = row.metrics.control_adjusted_retained_delta_anon_kib.sample_count;
  for (const metric of ["null_retained_delta_anon_kib", "anonymous_memory_area_kib_seconds_per_unit", "quiescent_retained_delta_anon_kib"]) {
    if (row.metrics[metric]?.sample_count !== count) throw new Error("v2 memory metric sample counts differ");
  }
  if (!Array.isArray(row.control_sample_ids) || row.control_sample_ids.length !== count ||
      new Set(row.control_sample_ids).size !== count || !Array.isArray(row.sample_ids) ||
      row.sample_ids.length !== count || new Set(row.sample_ids).size !== count ||
      row.control_sample_ids.some(id => row.sample_ids.includes(id))) {
    throw new Error("v2 memory row lacks distinct matched control/sample identities");
  }
}

module.exports = { LEGACY_PROTOCOL, MEMORY_PROTOCOL, QUIESCENCE_MS, rowProtocol,
  requireSingleProtocol, retainedMetric, validateProtocolMetrics };
