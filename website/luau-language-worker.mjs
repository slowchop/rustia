import createAnalyzer from './analysis/analyzer.mjs?v=garden-hover';

let engine, setSource, diagnostics, complete, hover;
let output;
async function initialize(definitionsFile) {
  const [module, definitions] = await Promise.all([
    createAnalyzer({locateFile: name => new URL('./analysis/' + name + '?v=garden-hover', import.meta.url).href}),
    definitionsFile ? fetch(new URL(definitionsFile, import.meta.url)).then(response => {
      if (!response.ok) throw Error('Could not load Luau API definitions.');
      return response.text();
    }) : '',
  ]);
  engine = module;
  engine.onIssue = (line, message) => output.diagnostics.push({range: {start: {line}}, message});
  engine.onLabel = label => output.labels.push(label);
  output = {diagnostics: [], labels: []};
  if (!engine.cwrap('init_analysis', 'number', ['string'])(definitions))
    throw Error(output.diagnostics[0]?.message || 'Could not load Luau API definitions.');
  setSource = engine.cwrap('set_source', null, ['string', 'string']);
  diagnostics = engine.cwrap('diagnostics', null, ['string']);
  complete = engine.cwrap('complete', null, ['string', 'number', 'number']);
  hover = engine.cwrap('hover', 'string', ['string', 'number', 'number']);
}

self.onmessage = async ({data}) => {
  const {id, definitions, sources, file, text, position} = data;
  try {
    if (!file) {
      await initialize(definitions);
      self.postMessage({id, ready: true});
      return;
    }
    output = {diagnostics: [], labels: []};
    for (const [name, source] of sources) {
      if (name !== file) setSource(name, source);
    }
    setSource(file, text);
    if (position) {
      const before = text.split('\n')[position.line].slice(0, position.character);
      const byteColumn = new TextEncoder().encode(before).length;
      if (data.operation === 'hover') {
        output.signature = hover(file, position.line, byteColumn);
      } else {
        complete(file, position.line, byteColumn);
        output.labels = output.labels.filter(label => /^[A-Za-z_][A-Za-z_0-9]*$/.test(label)).sort();
      }
    } else {
      diagnostics(file);
    }
    self.postMessage({id, ...output});
  } catch (error) {
    self.postMessage({id, error: error instanceof WebAssembly.Exception ? engine.getExceptionMessage(error).join(": ") : String(error.message || error)});
  }
};
