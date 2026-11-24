// One browser worker resolves types for both completion and hover.
import {EditorView, StreamLanguage, hoverTooltip, lua} from './codemirror.js?v=four-spaces';

export function luauEditorLanguage({definitions, currentFile, projectFiles}) {
  const language = StreamLanguage.define(lua);
  let worker, ready, nextId = 0;
  const pending = new Map();

  function request(payload) {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      pending.set(id, {resolve, reject});
      worker.postMessage({id, ...payload});
    });
  }

  async function analyze(doc, pos, operation) {
    const file = currentFile();
    const sources = [...projectFiles()];
    if (!ready) {
      worker = new Worker(new URL('./luau-language-worker.mjs?v=shared', import.meta.url), {type:'module'});
      worker.onmessage = ({data}) => {
        const entry = pending.get(data.id);
        if (!entry) return;
        pending.delete(data.id);
        if (data.error) entry.reject(Error(data.error));
        else entry.resolve(data);
      };
      worker.onerror = () => {
        for (const entry of pending.values()) entry.reject(Error('Luau analyzer unavailable'));
        pending.clear();
        worker.terminate();
        ready = undefined;
      };
      ready = request({definitions}).catch(error => {
        worker.terminate();
        ready = undefined;
        throw error;
      });
    }
    await ready;
    const line = doc.lineAt(pos);
    return request({operation, file, sources, text:doc.toString(),
      position:{line:line.number - 1, character:pos - line.from}});
  }

  async function complete(context) {
    const member = context.matchBefore(/\.\w*$/);
    if (!member || context.tokenBefore(['string', 'comment'])) return null;
    try {
      const {labels} = await analyze(context.state.doc, context.pos, 'complete');
      if (context.aborted || !labels.length) return null;
      return {from:member.from + 1, validFor:/^\w*$/,
        options:labels.map(label => ({label, type:'property'}))};
    } catch (error) {
      console.warn('Luau completions:', error.message);
      return null;
    }
  }

  const hover = hoverTooltip(async (view, pos, side) => {
    const doc = view.state.doc;
    const line = doc.lineAt(pos);
    const at = pos - line.from - (side < 0 ? 1 : 0);
    if (at < 0 || !/\w/.test(line.text.charAt(at))) return null;
    const from = line.from + at - line.text.slice(0, at).match(/\w*$/)[0].length;
    const to = pos + (side < 0 ? -1 : 0) + line.text.slice(at).match(/^\w+/)[0].length;
    const qualified = line.text.slice(0, to - line.from).match(/[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*$/);
    if (!qualified) return null;
    try {
      const {signature} = await analyze(doc, line.from + at, 'hover');
      if (!signature || view.state.doc !== doc) return null;
      const type = signature.replace(/^@checked /, '');
      const display = qualified[0] + (type.startsWith('(') ? '' : ': ') + type;
      return {pos:from, end:to, above:true, create() {
        const dom = document.createElement('code');
        dom.className = 'luau-signature';
        dom.textContent = display;
        return {dom};
      }};
    } catch (error) {
      console.warn('Luau hover:', error.message);
      return null;
    }
  }, {hideOnChange:true});
  return [language, language.data.of({autocomplete:complete}), hover, EditorView.baseTheme({
    '.luau-signature': {display:'block', padding:'5px 8px', maxWidth:'min(75vw, 56ch)', whiteSpace:'pre-wrap', overflowWrap:'anywhere'}
  })];
}
