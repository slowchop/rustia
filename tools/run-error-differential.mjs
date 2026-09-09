// Correctness only: isolated workers, bounded sources/output and per-case timeout.
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const worker = resolve(process.env.RUSTIA_DIFF_WORKER ?? 'fuzz/target/debug/error-oracle-worker');
const count = Number(process.env.RUSTIA_DIFF_CASES ?? 256);
if (!Number.isSafeInteger(count) || count < 1 || count > 1000000) throw new Error('invalid case count');
const artifactDir = resolve(process.env.RUSTIA_DIFF_ARTIFACTS ?? 'fuzz/artifacts/differential');
let seed = 0x6572726f;
function next() { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return seed >>> 0; }
function generate(index) {
  const n = next() % 100;
  const prefix = `observed = ${n};\n`;
  switch (index % 8) {
    case 0: return prefix + 'local function unused() local x = end';
    case 1: return prefix + 'local x = nil; return x.field';
    case 2: return prefix + 'local t = {}; local ok,e = pcall(error,t); return tostring(ok)..":"..tostring(rawequal(t,e))';
    case 3: return prefix + 'local ok,e = xpcall(function() error("first",0) end,function(e) return "handled:"..e end); return tostring(ok)..":"..e';
    case 4: return prefix + 'local ok,e = xpcall(function() error("first",0) end,function() error("second",0) end); return tostring(ok)..":"..e';
    case 5: return prefix + 'local c=coroutine.create(function() coroutine.yield("pause"); error("later",0) end); local a,b=coroutine.resume(c); local d,e=coroutine.resume(c); return tostring(a)..":"..b..":"..tostring(d)..":"..e';
    case 6: return prefix + 'local f; pcall(function() local x=41; f=function() x=x+1; return x end; error("stop",0) end); return tostring(f())..":"..tostring(f())';
    default: return prefix + 'local t={}; local ok,e=pcall(function() t[0/0]=1 end); return tostring(ok)..":"..type(e)';
  }
}
function run(engine, source) {
  const result = spawnSync(worker, [engine], { input: source, encoding: 'utf8', timeout: 5000, maxBuffer: 65536 });
  if (result.error || result.status !== 0 || result.signal) {
    return { failure: String(result.error ?? result.signal ?? result.status), stderr: result.stderr };
  }
  return { output: result.stdout };
}
for (let index = 0; index < count; index++) {
  const source = generate(index);
  const luau = run('luau', source);
  const rustia = run('rustia', source);
  if (luau.failure || rustia.failure || luau.output !== rustia.output) {
    mkdirSync(artifactDir, { recursive: true });
    const id = `case-${index}-${seed}`;
    writeFileSync(resolve(artifactDir, `${id}.luau`), source);
    writeFileSync(resolve(artifactDir, `${id}.json`), JSON.stringify({ index, seed, luau, rustia }, null, 2));
    throw new Error(`differential finding saved as ${id}`);
  }
}
console.log(`Passed ${count} isolated differential cases against Luau 0.709`);
