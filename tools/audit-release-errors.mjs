// Correctness audit only. Known differences are reported, never silently blessed.
// Build first: cargo build --locked --manifest-path fuzz/Cargo.toml --features oracle --bin error-oracle-worker
// Run: node tools/audit-release-errors.mjs > /tmp/release-error-audit.json
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

const inventory = JSON.parse(readFileSync(new URL('../design/release-error-audit.json', import.meta.url)));
const worker = resolve(process.env.RUSTIA_DIFF_WORKER ?? 'fuzz/target/debug/error-oracle-worker');
function run(engine, source) {
  const result = spawnSync(worker, [engine], {
    input: source, encoding: 'utf8', timeout: 5000, maxBuffer: 65536,
  });
  if (result.error || result.signal || result.status !== 0) {
    throw new Error(`${engine} worker failed: ${result.error ?? result.signal ?? result.stderr}`);
  }
  return result.stdout;
}
const cases = inventory.cases.map(({ name, source }) => {
  const luau = run('luau', source);
  const rustia = run('rustia', source);
  const same = luau.replaceAll('[string "differential.luau"]', 'differential.luau') === rustia;
  return { name, source, same, luau, rustia };
});
console.log(JSON.stringify({
  oracle: inventory.oracle,
  comparison: inventory.comparison,
  matched: cases.filter(row => row.same).length,
  differences: cases.filter(row => !row.same).length,
  cases,
}, null, 2));
// This is a finding-producing audit, not an allowlisted conformance pass.
if (cases.some(row => !row.same)) process.exitCode = 1;
