// The shared BrowserEditor owns drafts and restores. CodeMirror renders each file
// and Rustia's reported source range. Keeping a view per file also retains undo.
import {EditorView, basicSetup, setDiagnostics} from './codemirror.js?v=four-spaces';

function positionAtByte(source, byte) {
  const bytes = new TextEncoder().encode(source);
  return new TextDecoder().decode(bytes.subarray(0, byte)).length;
}

function readLocation(message, fallback, drafts) {
  const frames = [...message.matchAll(/([\w.-]+\.luau):(\d+):/g)];
  const last = frames.at(-1);
  const file = last?.[1] ?? fallback;
  const source = drafts.get(file);
  if (source === undefined) return;
  const span = message.match(/(?:parse|compile|lex) error at (\d+)\.\.(\d+)/i);
  if (span) {
    const from = positionAtByte(source, Number(span[1]));
    const to = positionAtByte(source, Number(span[2]));
    return {file, from, to};
  }
  if (last) {
    const lines = source.split('\n');
    const line = Number(last[2]);
    if (line < 1 || line > lines.length) return;
    const from = lines.slice(0, line - 1).reduce((n, s) => n + s.length + 1, 0);
    return {file, from, to:from + lines[line - 1].length};
  }
}

function errorArrow(host, panel, jump) {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('class', 'error-arrow');
  svg.setAttribute('aria-hidden', 'true');
  const shaft = document.createElementNS(ns, 'path');
  const head = document.createElementNS(ns, 'path');
  svg.append(shaft, head);
  panel.parentElement.append(svg);
  let target, scheduled;

  function place() {
    scheduled = undefined;
    const box = svg.getBoundingClientRect();
    const bounds = host.getBoundingClientRect();
    const point = target && host.contains(target.view.dom) && target.view.coordsAtPos(target.pos);
    const visible = point && !panel.hidden &&
      point.top < bounds.bottom && point.bottom > bounds.top &&
      point.left >= bounds.left && point.left <= bounds.right;
    svg.classList.toggle('is-visible', Boolean(visible));
    if (!visible) return;
    const x = point.left - box.left + 1;
    const y = point.bottom - box.top + 2;
    const start = jump.getBoundingClientRect();
    const startX = Math.max(start.left + 6, Math.min(start.right - 6, point.left)) - box.left;
    const startY = start.top - box.top;
    const rise = startY - y;
    if (rise < 6) {
      svg.classList.remove('is-visible');
      return;
    }
    const dx = x - startX;
    // When the badge is directly below the error, bow towards the left margin.
    const bow = Math.min(70, rise * 0.35, startX - 4);
    const controlX = startX + dx * 0.22 - bow;
    shaft.setAttribute('d', `M ${startX} ${startY} C ${controlX} ${startY - rise * 0.33} ${x} ${y + Math.min(42, rise * 0.45)} ${x} ${y}`);
    head.setAttribute('d', `M ${x - 2.8} ${y + 5} L ${x} ${y} l 2.8 5`);
  }
  function refresh() {
    if (target && scheduled === undefined) scheduled = requestAnimationFrame(place);
  }
  const observer = new ResizeObserver(refresh);
  observer.observe(host);
  observer.observe(panel);
  host.addEventListener('scroll', refresh, true);
  // CodeMirror virtualizes lines when the document itself scrolls.
  window.addEventListener('scroll', refresh, {passive:true});
  return {
    refresh,
    clear() { target = undefined; svg.classList.remove('is-visible'); },
    show(view, pos) { target = {view, pos}; refresh(); }
  };
}

export function sourceEditor(textarea, panel, extensions = []) {
  const host = document.createElement('div');
  host.className = 'source-views';
  textarea.after(host);
  textarea.hidden = true;
  const views = new Map();
  const messageText = panel.querySelector('pre');
  const jump = panel.querySelector('button');
  const arrow = errorArrow(host, panel, jump);
  const slide = document.createElement('div');
  slide.className = 'error-slide';
  panel.before(slide);
  slide.append(panel);
  let slideAnimation, panelOpen = false;

  function showPanel(open) {
    if (open === panelOpen) return;
    panelOpen = open;
    const from = slide.getBoundingClientRect().height;
    slideAnimation?.cancel();
    panel.inert = !open;
    panel.hidden = false;
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      panel.hidden = !open;
      return;
    }
    const to = open ? panel.offsetHeight : 0;
    const animation = slide.animate([{height:from + 'px'}, {height:to + 'px'}],
      {duration:120, easing:'ease-out'});
    slideAnimation = animation;
    animation.onfinish = () => {
      if (slideAnimation !== animation) return;
      panel.hidden = !panelOpen;
      slideAnimation = undefined;
      arrow.refresh();
    };
  }

  let current, syncing = false, marked, pendingMark;

  function select(file) {
    current = file;
    if (!views.has(file)) {
      const view = new EditorView({
        doc:textarea.value,
        extensions:[basicSetup, ...extensions, EditorView.lineWrapping, EditorView.contentAttributes.of({'aria-label':'Edit ' + file, spellcheck:'false'}),
          EditorView.updateListener.of(update => {
            if (!syncing && update.docChanged && current === file) {
              textarea.value = update.state.doc.toString();
              textarea.dispatchEvent(new Event('input'));
            }
          })]
      });
      views.set(file, view);
    }
    // Keep each file's undo history; only mount the selected editor in the pane.
    const view = views.get(file);
    if (host.firstElementChild !== view.dom) {
      host.replaceChildren(view.dom);
      view.requestMeasure();
    }
    if (pendingMark?.file === file && !marked) pendingMark.apply();
    arrow.refresh();
  }

  function sync() {
    const view = views.get(current);
    if (view && textarea.value !== view.state.doc.toString()) {
      syncing = true;
      try { view.dispatch({changes:{from:0, to:view.state.doc.length, insert:textarea.value}}); }
      finally { syncing = false; }
    }
  }

  function diagnostic(message, editor, fallback) {
    pendingMark = undefined;
    arrow.clear();
    if (marked) {
      const previous = marked;
      marked = undefined;
      // Input listeners run inside CodeMirror's edit transaction.
      queueMicrotask(() => {
        if (marked !== previous) previous.dispatch(setDiagnostics(previous.state, []));
      });
    }
    if (!message) { showPanel(false); return; }
    jump.hidden = true;
    jump.onclick = null;
    messageText.textContent = message;
    const range = readLocation(message, fallback, editor.drafts);
    if (!range) { showPanel(true); return; }
    // Materialize a file only when selected. Never move the caret while typing.
    function underline() {
      const view = views.get(range.file);
      if (!view) return;
      view.dispatch(setDiagnostics(view.state, [{from:range.from, to:range.to, severity:'error', message}]));
      marked = view;
      const line = view.state.doc.lineAt(range.from);
      const indent = range.from === line.from ? line.text.match(/^\s*/)[0].length : 0;
      arrow.show(view, Math.min(range.to, range.from + indent));
    }
    pendingMark = {file:range.file, apply:underline};
    underline();
    const prefix = editor.drafts.get(range.file).slice(0, range.from).split('\n');
    jump.hidden = false;
    jump.textContent = range.file + ':' + prefix.length + ':' + (prefix.at(-1).length + 1);
    showPanel(true);
    jump.onclick = () => {
      editor.select(range.file);
      marked.dispatch({selection:{anchor:range.from, head:range.to}, scrollIntoView:true});
      marked.focus();
    };
  }
  function point(pos) {
    const view = views.get(current);
    const line = view?.coordsAtPos(pos);
    if (!line) return;
    const bounds = view.scrollDOM.getBoundingClientRect();
    const y = (line.top + line.bottom) / 2;
    if (y < bounds.top || y > bounds.bottom) return;
    return {x:view.contentDOM.getBoundingClientRect().left, textX:line.left, y};
  }
  function reveal(pos) {
    const view = views.get(current);
    view.dispatch({effects:EditorView.scrollIntoView(pos, {y:'nearest'})});
  }
  function replace(range, insert) {
    views.get(current).dispatch({changes:{...range, insert}, userEvent:'input'});
  }
  return {select, sync, diagnostic, point, reveal, replace, focus:() => views.get(current)?.focus()};
}
