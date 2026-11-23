// Exercise the same sessions that the garden worker creates for live edits.
import assert from 'node:assert/strict';
import {readFile, readdir} from 'node:fs/promises';
import init, {Session} from './dist/pkg/rustia_garden.js';

const read = path => readFile(new URL(path, import.meta.url));
await init({module_or_path: await read('dist/pkg/rustia_garden_bg.wasm')});
const shapes = [];
globalThis.gardenOutput = {sprite(...args) { shapes.push(args); }};
const source = `
local starts, initial = 0, 0
function setup(radius)
    starts += 1
    initial = radius
end
function update(time, mouse, radius)
    sprite(vector(starts, initial))
end`;
for (const initial of [128, 256]) {
  const session = Session.withDrawing(source, (await read('drawing.luau')).toString(), initial);
  for (const radius of [initial, initial + 10]) {
    session.tickWithRadius(1, 0, 0, radius);
    assert.deepEqual(shapes.pop(), [0, 1, initial, 0]);
  }
  session.free();
}
const exports = new Session('local n = 0; return {setup = function() n += 1 end, frame = function() assert(n == 1) end}');
exports.tick(0, 0, 0);
exports.free();

assert.throws(() => new Session('function setup() error("setup failed") end; function frame() end'), /setup failed/);
assert.throws(() => new Session('setup = 2; function frame() end'), /setup must be a function/);

const preferred = new Session('function frame() error("legacy frame called") end; function update() end');
preferred.tick(0, 0, 0);
preferred.free();
const namedExport = new Session('return { update = function() end }');
namedExport.tick(0, 0, 0);
namedExport.free();
assert.throws(() => new Session('function frame() end; update = 2'), /Define function update/);

const starter = new Session((await read('garden.luau')).toString(), 280);
starter.tickWithRadius(0, 0, 0, 280);
assert.deepEqual(shapes.pop(), [2, 0, 0, 0]);
starter.free();

const flowers = new Session((await read('examples/garden-studies/wandering-flowers.luau')).toString(), 280);
let previousKinds;
for (let i = 0; i < 4; i++) {
  flowers.tickWithRadius(i, i === 0 ? 40 : undefined, i === 0 ? 0 : undefined, 280);
  assert.equal(shapes.length, 10);
  const kinds = shapes.map(([kind]) => kind);
  if (previousKinds) assert.deepEqual(kinds, previousKinds);
  else assert.ok(Math.abs(shapes[0][1] - 40 * 0.0065) < 1e-5);
  previousKinds = kinds;
  assert.ok(shapes.every(([, x, y]) => Math.hypot(x, y) <= 280));
  shapes.length = 0;
}
// The live-edit worker transfers these rows before it ticks the replacement VM.
const starterCode = (await read('examples/garden-studies/wandering-flowers.luau')).toString();
const retained = flowers.savedFlowers();
assert.equal(retained.length, 70);
const resize = (count, rows) => Session.fromEdit(starterCode.replace('flowerCount = 10', 'flowerCount = ' + count),
    undefined, undefined, rows, 280);
const larger = resize(11, retained);
assert.deepEqual(larger.savedFlowers().slice(0, retained.length), retained);
assert.equal(larger.savedFlowers().length, 77);
const smaller = resize(9, larger.savedFlowers());
assert.deepEqual(smaller.savedFlowers(), retained.slice(0, 63));
const empty = resize(0, smaller.savedFlowers());
assert.equal(empty.savedFlowers().length, 0);
const restarted = resize(10, new Float64Array());
assert.equal(restarted.savedFlowers()[0], 0);
assert.notDeepEqual(restarted.savedFlowers(), retained);
for (const vm of [larger, smaller, empty, restarted]) vm.free();
flowers.free();
console.log('Setup, resize, fresh sessions, error handling and both garden examples passed.');

const registry = (await read('sprite-names.txt')).toString().trim().split(/\s+/);
const picks = new Session(`
    function update()
        math.randomseed(42)
        for _ = 1, 128 do sprite(vector.zero, Flower.random()) end
    end
`);
picks.tick(0, 0, 0);
const first = shapes.splice(0).map(([kind]) => kind);
assert.deepEqual([...new Set(first)].sort((a,b) => a-b), registry.map((_, i) => i));
picks.tick(1, 0, 0);
assert.deepEqual(shapes.map(([kind]) => kind), first);
picks.free();
console.log('Random sprite kinds cover the registry and replay from the same seed.');

const pointer = new Session(`
    function update(...)
        local time, mouse, radius = ...
        assert(select("#", ...) == (if radius then 3 else 2))
        assert((mouse == nil) == (time == 0))
        if mouse then assert(mouse == vector(-10000, -10000)) end
    end
`);
for (const [time, x, y] of [[0], [1, -10000, -10000], [0]]) {
  pointer.tick(time, x, y);
  pointer.tickWithRadius(time, x, y, 280);
}
pointer.free();

const studyRoot = new URL('./examples/garden-studies/', import.meta.url);
for (const file of (await readdir(studyRoot)).filter(file => file.endsWith('.luau'))) {
  const session = new Session(await readFile(new URL(file, studyRoot), 'utf8'), 280);
  for (const coords of [[undefined, undefined], [20, 20], [undefined, undefined]]) {
    shapes.length = 0;
    session.tickWithRadius(1, ...coords, 280);
    assert.ok(shapes.length > 0, file);
  }
  session.free();
}
console.log('Garden studies handle an absent, present and departed pointer.');
