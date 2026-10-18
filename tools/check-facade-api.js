#!/usr/bin/env node
/**
 * Check Rustia's reachable API using the pinned compiler's semantic rustdoc JSON.
 * --write refreshes the reviewable inventory; ordinary runs never rewrite it.
 * --self-test runs negative traversal fixtures without compiling anything.
 * --check-json FILE checks one already-generated document without the baseline.
 *
 * Rustdoc JSON is unstable. Pin its schema explicitly and fail closed on an
 * upgrade. RUSTC_BOOTSTRAP is scoped to cargo rustdoc; it enables JSON output,
 * not a nightly toolchain or unstable Rustia implementation features.
 */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');

const ROOT = path.resolve(__dirname, '..');
const BASELINE = path.join(ROOT, 'design/facade-api.json');
const FORMAT = 57;
const ENGINES = new Set(['rustia_vm', 'rustia_compiler', 'rustia_parser', 'rustia_lexer']);
const TOKEN = 'rustia::function::adapter::Token';
const sorted = (items) => [...items].sort((a, b) => a.localeCompare(b, 'en'));
const kindOf = (item) => Object.keys(item.inner)[0];

function analyze(doc) {
  assert.equal(doc.format_version, FORMAT, 'rustdoc JSON schema changed; review the traversal before updating FORMAT');
  assert.equal(doc.includes_private, true, 'private items are needed to audit sealed supertraits');
  const index = doc.index;
  const local = index[doc.root].crate_id;
  const names = new Map();
  const queue = [];
  const seen = new Set();
  const entries = new Map();
  const reachableTypes = new Set();
  const blocked = new Set();
  const problems = new Set();
  const engineName = (id) => {
    const owner = doc.paths[id]?.crate_id ?? index[id]?.crate_id;
    return doc.external_crates[owner]?.name;
  };
  const canonical = (id) => names.get(String(id)) ?? doc.paths[id]?.path.join('::') ?? `unresolved:${id}`;
  const nameable = (id) => names.has(String(id));
  const enqueue = (id, label, mode = 'item') => {
    if (id === null || id === undefined) return;
    queue.push({ id: String(id), label, mode });
  };
  function exportModule(id, prefix) {
    const module = index[id]?.inner.module;
    assert(module, `missing module ${prefix}`);
    for (const childId of module.items) {
      const child = index[childId];
      if (!child || child.visibility !== 'public') continue;
      if (child.inner.use) {
        const use = child.inner.use;
        assert(use.id !== null, `unresolved public re-export ${use.source}`);
        if (use.is_glob) {
          exportModule(use.id, prefix);
        } else {
          const label = `${prefix}::${use.name}`;
          names.set(String(use.id), label);
          enqueue(use.id, label);
        }
      } else {
        const label = `${prefix}::${child.name}`;
        names.set(String(childId), label);
        enqueue(childId, label);
        if (child.inner.module) exportModule(childId, label);
      }
    }
  }
  exportModule(doc.root, 'rustia');
  for (const id of names.keys()) reachableTypes.add(id);

  const tokenEntry = Object.entries(doc.paths).find(([, item]) => item.path.join('::') === TOKEN);
  const tokenId = tokenEntry?.[0];
  if (tokenId) {
    const token = index[tokenId];
    assert(token && token.crate_id === local && token.visibility !== 'public' && !nameable(tokenId), 'adapter token must stay private');
    assert.equal(token.inner.struct?.kind, 'unit', 'adapter token construction contract changed');
    for (const implId of token.inner.struct.impls) {
      const impl = index[implId].inner.impl;
      assert(impl.is_synthetic || impl.blanket_impl !== null, 'adapter token gained an explicit implementation; audit construction before allowing it');
    }
  }

  function inspect(value, label) {
    if (!value || typeof value !== 'object') return;
    if (Array.isArray(value)) {
      value.forEach((part) => inspect(part, label));
      return;
    }
    // Path IDs are semantic, so aliases and imports cannot hide their crate.
    if ('id' in value && ('path' in value || 'name' in value)) {
      if (ENGINES.has(engineName(value.id))) {
        problems.add(`${label} exposes ${canonical(value.id)}`);
      } else if (index[value.id]?.crate_id === local) {
        if (String(value.id) === tokenId) {
          problems.add(`${label} exposes the private callback token`);
        } else {
          reachableTypes.add(String(value.id));
          enqueue(value.id, canonical(value.id), 'signature');
        }
      }
    }
    for (const part of Object.values(value)) inspect(part, label);
  }

  function normalize(value) {
    if (Array.isArray(value)) return value.map(normalize);
    if (!value || typeof value !== 'object') return value;
    const result = {};
    for (const key of Object.keys(value).sort()) {
      if (key === 'id' && ('path' in value || 'name' in value)) continue;
      result[key] = key === 'path' && 'id' in value ? canonical(value.id) : normalize(value[key]);
    }
    return result;
  }
  function record(label, kind, signature, item) {
    const attrs = (item.attrs ?? []).filter((attr) => {
      const text = typeof attr === 'string' ? attr : attr.other ?? '';
      return /non_exhaustive|must_use|deprecated|repr\(/.test(text);
    });
    const entry = { path: label, kind, signature: normalize(signature) };
    if (attrs.length) entry.attributes = attrs;
    if (item.deprecation) entry.deprecation = item.deprecation;
    const key = JSON.stringify(entry);
    entries.set(key, entry);
  }
  function child(id, parent, force = false, mode = 'item', position) {
    const item = index[id];
    assert(item, `missing child of ${parent}: ${id}`);
    if (force || item.visibility === 'public') enqueue(id, `${parent}::${item.name ?? position}`, mode);
  }
  function impls(ids, parent) {
    for (const id of ids ?? []) enqueue(id, parent, 'impl');
  }
  while (queue.length) {
    const { id, label, mode } = queue.shift();
    const key = `${id}:${mode}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const item = index[id];
    if (!item) {
      // External types need not be expanded: their semantic owner still gates
      // direct engine re-exports. No opaque external constant exception exists.
      if (ENGINES.has(engineName(id))) problems.add(`${label} re-exports ${canonical(id)}`);
      continue;
    }
    if (item.crate_id !== local) {
      if (ENGINES.has(engineName(id))) problems.add(`${label} re-exports ${canonical(id)}`);
      continue;
    }
    const kind = kindOf(item);
    const inner = item.inner[kind];
    if (kind === 'use') {
      enqueue(inner.id, label);
      continue;
    }
    if (kind === 'module') continue;
    if (kind === 'impl') {
      // Rustdoc also attaches `impl From<Public> for Private` to Public.
      // That does not make the private destination type reachable. If a public
      // signature later exposes it, its own impl list will enqueue this again.
      let privateDestination = false;
      function inspectDestination(value) {
        if (!value || typeof value !== 'object') return;
        if (value.resolved_path) {
          const typeId = String(value.resolved_path.id);
          if (index[typeId]?.crate_id === local && !reachableTypes.has(typeId)) privateDestination = true;
        }
        Object.values(value).forEach(inspectDestination);
      }
      inspectDestination(inner.for);
      if (privateDestination) { seen.delete(key); continue; }
      const isTrait = inner.trait !== null;
      const traitId = inner.trait?.id;
      // Private traits' implementation bodies are not exported. Their callable
      // supertrait declarations are inspected separately, including token use.
      if (isTrait && index[traitId]?.crate_id === local && !nameable(traitId)) continue;
      const header = { ...inner };
      delete header.items;
      inspect(header, `${label} implementation`);
      const fingerprint = createHash('sha256').update(JSON.stringify(normalize(header))).digest('hex').slice(0, 16);
      const implLabel = `${label} [impl ${inner.trait ? canonical(inner.trait.id) : 'inherent'} ${fingerprint}]`;
      record(implLabel, kind, header, item);
      for (const member of inner.items) child(member, implLabel, isTrait);
      continue;
    }
    if (kind === 'function') {
      const tokenInput = inner.sig.inputs.some(([, type]) => String(type.resolved_path?.id) === tokenId);
      if (mode === 'private-trait-member' && tokenInput && tokenId) {
        blocked.add(label);
        continue;
      }
      inspect(inner, label);
      record(label, kind, inner, item);
      continue;
    }
    if (kind === 'struct' || kind === 'union') {
      inspect(inner.generics, label);
      const shape = inner.kind;
      const fields = kind === 'union' ? inner.fields : shape?.plain?.fields ?? shape?.tuple ?? [];
      record(label, kind, { generics: inner.generics, shape: typeof shape === 'string' ? shape : shape?.plain ? 'plain' : 'tuple' }, item);
      fields.forEach((field, position) => { if (field !== null) child(field, label, false, 'item', position); });
      impls(inner.impls, label);
    } else if (kind === 'enum') {
      inspect(inner.generics, label);
      record(label, kind, { generics: inner.generics }, item);
      inner.variants.forEach((variant) => child(variant, label, true));
      impls(inner.impls, label);
    } else if (kind === 'variant') {
      const fields = inner.kind.struct?.fields ?? inner.kind.tuple ?? [];
      record(label, kind, { shape: typeof inner.kind === 'string' ? inner.kind : inner.kind.struct ? 'struct' : 'tuple', discriminant: inner.discriminant }, item);
      fields.forEach((field, position) => { if (field !== null) child(field, label, true, 'item', position); });
    } else if (kind === 'trait') {
      const { items, implementations, ...header } = inner;
      inspect(header, label);
      record(label, kind, header, item);
      for (const member of items) child(member, label, true, nameable(id) ? 'trait-member' : 'private-trait-member');
      if (nameable(id)) impls(implementations, label);
    } else if (kind === 'constant') {
      // An engine-defined value in the initializer is fine; its declared public
      // type must be facade-owned or primitive. Rustdoc values are recorded.
      inspect(inner.type, label);
      record(label, kind, inner, item);
    } else if (kind === 'macro') {
      record(label, kind, inner, item);
    } else {
      const accepted = ['struct_field', 'type_alias', 'assoc_const', 'assoc_type', 'static', 'trait_alias', 'primitive'];
      assert(accepted.includes(kind), `unhandled public rustdoc item ${kind} at ${label}`);
      inspect(inner, label);
      record(label, kind, inner, item);
    }
  }
  assert.equal(problems.size, 0, `facade engine-type leaks:\n${sorted(problems).join('\n')}`);
  return { entries: [...entries.values()].sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b), 'en')), blocked_adapter_methods: sorted(blocked) };
}

function selfTest() {
  // Small semantic graphs, independent of cargo/rustdoc, protect traversal of
  // signatures, nested types, impl headers, aliases and private supertraits.
  const item = (id, name, inner, visibility = 'public') => ({ id, name, crate_id: 0, visibility, inner, attrs: [], deprecation: null });
  const raw = { resolved_path: { path: 'CamouflagedAlias', id: 90, args: null } };
  const functionItem = (output) => ({ function: { sig: { inputs: [], output, is_c_variadic: false }, generics: { params: [], where_predicates: [] }, header: { is_const: false, is_unsafe: false, is_async: false, abi: 'Rust' }, has_body: true } });
  const fixture = () => ({ root: 0, format_version: FORMAT, includes_private: true, index: { 0: item(0, 'rustia', { module: { items: [1] } }), 1: item(1, 'probe', functionItem({ primitive: 'u32' })) }, paths: { 90: { crate_id: 1, path: ['rustia_vm', 'Vm'], kind: 'struct' } }, external_crates: { 1: { name: 'rustia_vm' } } });
  analyze(fixture());
  const fails = (mutate) => { const doc = fixture(); mutate(doc); assert.throws(() => analyze(doc), /facade engine-type leaks/); };
  fails((doc) => { doc.index[1].inner.function.sig.output = raw; });
  fails((doc) => { doc.index[1].inner.function.sig.inputs = [['x', { borrowed_ref: { lifetime: null, is_mutable: false, type: { tuple: [raw] } } }]]; });
  fails((doc) => { doc.index[1] = item(1, 'Alias', { type_alias: { type: raw, generics: {} } }); });
  fails((doc) => { doc.index[1] = item(1, 'Alias', { use: { source: 'rustia_vm::Vm', id: 90, is_glob: false, name: 'Alias' } }); });
  fails((doc) => {
    doc.index[1] = item(1, 'Api', { struct: { kind: 'unit', generics: {}, impls: [2] } });
    doc.index[2] = item(2, null, { impl: { trait: { path: 'From', id: 91, args: { angle_bracketed: { args: [{ type: raw }], constraints: [] } } }, for: { resolved_path: { path: 'Api', id: 1, args: null } }, generics: {}, items: [], is_synthetic: false, blanket_impl: null } }, 'default');
  });
  fails((doc) => {
    doc.index[1] = item(1, 'Api', { trait: { bounds: [], generics: {}, items: [2], implementations: [] } });
    doc.index[2] = item(2, 'Item', { assoc_type: { type: raw, bounds: [], generics: {} } }, 'default');
  });
  fails((doc) => {
    doc.index[1] = item(1, 'Api', { trait: { bounds: [], generics: {}, items: [], implementations: [2] } });
    doc.index[2] = item(2, null, { impl: { trait: { path: 'Api', id: 1, args: null }, for: { generic: 'T' }, generics: { where_predicates: [{ bound_predicate: { type: { generic: 'T' }, bounds: [{ trait_bound: { trait: { path: 'HiddenCoreTrait', id: 90, args: null } } }] } }] }, items: [], is_synthetic: false, blanket_impl: { generic: 'T' } } }, 'default');
  });
  // A private destination impl attached to its public source does not make the
  // destination callable. Exposing it in a signature must make the gate fail.
  const privateTarget = fixture();
  privateTarget.index[1] = item(1, 'Api', { struct: { kind: 'unit', generics: {}, impls: [2] } });
  privateTarget.index[2] = item(2, null, { impl: { trait: { path: 'From', id: 91, args: null }, for: { resolved_path: { path: 'Private', id: 3, args: null } }, generics: {}, items: [], is_synthetic: false, blanket_impl: null } }, 'default');
  privateTarget.index[3] = item(3, 'Private', { struct: { kind: { plain: { fields: [4] } }, generics: {}, impls: [] } }, 'crate');
  privateTarget.index[4] = item(4, 'raw', { struct_field: raw });
  analyze(privateTarget);
  privateTarget.index[0].inner.module.items.push(5);
  privateTarget.index[5] = item(5, 'expose_private', functionItem({ resolved_path: { path: 'Private', id: 3, args: null } }));
  assert.throws(() => analyze(privateTarget), /facade engine-type leaks/);
  const sealed = fixture();
  sealed.index[1] = item(1, 'Marker', { trait: { bounds: [{ trait_bound: { trait: { path: 'Hidden', id: 2, args: null } } }], generics: {}, items: [], implementations: [] } });
  sealed.index[2] = item(2, 'Hidden', { trait: { bounds: [], generics: {}, items: [3], implementations: [] } }, 'crate');
  sealed.index[3] = item(3, 'raw', functionItem(raw), 'default');
  sealed.index[3].inner.function.sig.inputs = [['token', { resolved_path: { path: 'Token', id: 4, args: null } }]];
  sealed.index[4] = item(4, 'Token', { struct: { kind: 'unit', generics: {}, impls: [] } }, 'crate');
  sealed.paths[4] = { crate_id: 0, path: TOKEN.split('::'), kind: 'struct' };
  assert.equal(analyze(sealed).blocked_adapter_methods.length, 1);
  const tokenEscape = structuredClone(sealed);
  tokenEscape.index[0].inner.module.items.push(5);
  tokenEscape.index[5] = item(5, 'make_token', functionItem({ resolved_path: { path: 'Token', id: 4, args: null } }));
  assert.throws(() => analyze(tokenEscape), /private callback token/);
  sealed.index[3].inner.function.sig.inputs = [];
  assert.throws(() => analyze(sealed), /facade engine-type leaks/);
}

function run(program, args, options = {}) {
  const result = spawnSync(program, args, { cwd: ROOT, encoding: 'utf8', maxBuffer: 128 * 1024 * 1024, ...options });
  if (result.status !== 0) throw new Error(`${program} ${args.join(' ')} failed\n${result.stderr ?? result.error ?? ''}`);
  return result.stdout;
}
function generate() {
  const toolchain = fs.readFileSync(path.join(ROOT, 'rust-toolchain.toml'), 'utf8').match(/channel\s*=\s*"([^"]+)"/)[1];
  assert(run('rustc', ['--version']).startsWith(`rustc ${toolchain} `), 'use the repository-pinned Rust toolchain');
  const metadata = JSON.parse(run('cargo', ['metadata', '--no-deps', '--format-version', '1', '--locked']));
  const configurations = [
    { name: 'native-default', target: null, bare: false },
    { name: 'native-bare', target: null, bare: true },
    { name: 'wasm-default', target: 'wasm32-unknown-unknown', bare: false },
    { name: 'wasm-bare', target: 'wasm32-unknown-unknown', bare: true },
  ];
  const surfaces = {};
  for (const config of configurations) {
    const args = ['rustdoc', '--locked', '-p', 'rustia', '--lib'];
    if (config.target) args.push('--target', config.target);
    if (config.bare) args.push('--no-default-features');
    args.push('--', '-Z', 'unstable-options', '--output-format', 'json', '--document-private-items', '--document-hidden-items');
    console.log(`Checking facade API: ${config.name}`);
    run('cargo', args, { env: { ...process.env, RUSTC_BOOTSTRAP: '1' } });
    const location = path.join(metadata.target_directory, ...(config.target ? [config.target] : []), 'doc/rustia.json');
    surfaces[config.name] = analyze(JSON.parse(fs.readFileSync(location, 'utf8')));
  }
  return { schema_version: 1, rust_toolchain: toolchain, rustdoc_format: FORMAT, configuration_scope: 'Native and browser-Wasm, default and bare features; implementation-only cfg(test) items excluded. Macros record declared patterns; their expansion behavior is covered by external-consumer tests.', surfaces };
}
function serialize(inventory) {
  // The same API appears in four configurations. Store each complete semantic
  // entry once and refer to its content hash; no signatures are discarded.
  const { surfaces, ...header } = inventory;
  const definitions = new Map();
  const configurations = {};
  for (const [name, surface] of Object.entries(surfaces)) {
    const ids = surface.entries.map((entry) => {
      const signature = JSON.stringify(entry);
      const digest = createHash('sha256').update(signature).digest('hex').slice(0, 24);
      assert(!definitions.has(digest) || definitions.get(digest) === signature, 'inventory digest collision');
      definitions.set(digest, signature);
      return digest;
    });
    configurations[name] = { entries: ids, blocked_adapter_methods: surface.blocked_adapter_methods };
  }
  const preamble = JSON.stringify({ ...header, surfaces: configurations }, null, 2);
  const lines = preamble.slice(0, -2).split('\n');
  lines[lines.length - 1] += ',';
  lines.push('  "definitions": {');
  const ids = sorted(definitions.keys());
  ids.forEach((id, i) => lines.push(`    ${JSON.stringify(id)}: ${definitions.get(id)}${i + 1 === ids.length ? '' : ','}`));
  lines.push('  }', '}');
  return lines.join('\n') + '\n';
}

try {
  selfTest();
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === '--self-test') {
    console.log('Facade API traversal self-tests passed.');
  } else if (args.length === 2 && args[0] === '--check-json') {
    const result = analyze(JSON.parse(fs.readFileSync(args[1], 'utf8')));
    console.log(`Facade JSON passed: ${result.entries.length} entries, ${result.blocked_adapter_methods.length} inaccessible adapter methods.`);
  } else {
    assert(args.length === 0 || (args.length === 1 && args[0] === '--write'), 'usage: check-facade-api.js [--write | --self-test | --check-json FILE]');
    const actual = generate();
    const serialized = serialize(actual);
    if (args[0] === '--write') fs.writeFileSync(BASELINE, serialized);
    else assert.equal(fs.readFileSync(BASELINE, 'utf8'), serialized, 'facade API inventory changed; review it, then run node tools/check-facade-api.js --write');
    console.log(`Facade API inventory ${args[0] === '--write' ? 'updated' : 'verified'}; all configurations are free of callable engine types.`);
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
