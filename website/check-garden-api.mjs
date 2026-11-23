// Offline check. Ask the same analyzer the editor uses for its actual suggestions.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import createAnalyzer from './analysis/analyzer.mjs';

const read = (path, encoding) => readFile(new URL(path, import.meta.url), encoding);
const settings = JSON.parse(await read('./.vscode/settings.json', 'utf8'));
const definitions = settings['luau-lsp.types.definitionFiles']['@garden'];
assert.equal(definitions, 'garden.d.luau');
const registry = (await read('./sprite-names.txt', 'utf8')).trim().split(/\s+/).sort();
const engine = await createAnalyzer({wasmBinary: await read('./analysis/analyzer.wasm')});
const issues = [], labels = [];
engine.onIssue = (_line, message) => issues.push(message);
engine.onLabel = label => labels.push(label);
const init = engine.cwrap('init_analysis', 'number', ['string']);
assert.ok(init(await read(definitions, 'utf8')), issues.join('\n'));
const setSource = engine.cwrap('set_source', null, ['string', 'string']);
const complete = engine.cwrap('complete', null, ['string', 'number', 'number']);
for (const name of ['kinds', 'Sprite']) {
  const source = 'local shape = ' + name + '.';
  setSource('garden.luau', source);
  labels.length = 0;
  complete('garden.luau', 0, source.length);
  assert.deepEqual(labels.sort(), registry, name + ' completions differ from sprite-names.txt');
}
setSource('garden.luau', 'return Flower.');
labels.length = 0;
complete('garden.luau', 0, 'return Flower.'.length);
assert.ok(labels.includes('random'));
console.log('Garden declarations match the runtime sprite names and offer Flower.random.');

const hover = engine.cwrap('hover', 'string', ['string', 'number', 'number']);
function hoverLerp(line) {
  setSource('garden.luau', line);
  const column = Buffer.byteLength(line.slice(0, line.lastIndexOf('lerp') + 1));
  return hover('garden.luau', 0, column);
}
assert.match(hoverLerp('local label = "🌱"; local v = vector.lerp(vector.zero, vector.one, 0.5)'),
  /\(vec1: vector, vec2: vector, t: number\) -> vector/);
assert.equal(hoverLerp('local vector = {lerp = 2}; local n = vector.lerp'), 'number');
assert.ok(!hoverLerp('-- vector.lerp'));
console.log('Garden hover resolves the actual member in the edited source.');
