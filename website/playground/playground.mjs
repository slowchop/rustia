import {resizePanes} from './panes.mjs';
import {connectRepl} from './repl.mjs';
import {interactiveStarters} from './starters.mjs';
import {createGardenPlayer} from '../garden-player.mjs?v=garden-project';
import {drawGarden} from '../draw-garden.mjs';
import {luauEditorLanguage} from '../luau-editor-language.mjs';
import {editorIndentation} from '../editor-indent.mjs';
import {BrowserEditor} from '../editor.mjs';
import {sourceEditor} from '../source-editor.mjs?v=single-pane';

resizePanes(document.querySelector('.workspace'));

const $ = id => document.getElementById(id);
const params = new URLSearchParams(location.search);
const starters = {
  ...interactiveStarters,
  blank: {'main.luau':''},
  garden: {},
  hello: {
    'main.luau':'local greet = require("./greeting")\nprint(greet("Luau"))\n',
    'greeting.luau':'return function(name)\n    return "Hello, " .. name .. "!"\nend\n',
  },
  sprites: {'main.luau':`function update(time, mouse)
    sprite(mouse or vector.zero, Sprite.Ring)
end
`},
};
const requested = params.get('project');
const project = Object.hasOwn(starters, requested) ? requested : 'hello';
const garden = project === 'garden';
const entryFile = garden ? 'garden.luau' : 'main.luau';
const gardenPaths = {
  'garden.luau':'../examples/garden-studies/wandering-flowers.luau?v=flowerbed',
  'flowerbed.luau':'../flowerbed.luau?v=1',
  'drawing.luau':'../drawing.luau?v=garden-ring',
};
const gardenDrafts = 'rustia.garden.ribbons.';
const draftKey = file => garden && Object.hasOwn(gardenPaths, file)
  ? gardenDrafts + (file === entryFile ? 'silk-ribbons.luau' : file)
  : prefix + file;
const spriteMode = (params.get('mode') ?? ((project === 'sprites' || garden) ? 'sprites' : 'console')) === 'sprites';
document.querySelector('[data-project="' + project + '"]').setAttribute('aria-current', 'page');
const originals = starters[project];
if (garden) {
  try {
    await Promise.all(Object.entries(gardenPaths).map(async ([file, url]) => {
      const response = await fetch(url);
      if (!response.ok) throw Error('Could not load ' + file);
      return [file, await response.text()];
    })).then(entries => Object.assign(originals, Object.fromEntries(entries)));
  } catch (error) {
    $('state').textContent = error.message;
    $('state').dataset.tone = 'error';
    throw error;
  }
}

const mode = $('preview-mode');
mode.value = spriteMode ? 'sprites' : 'console';
mode.onchange = () => {
  params.set('mode', mode.value);
  location.search = params.toString();
};
const prefix = 'rustia.playground.v1.' + project + '.file.';
const selectionKey = 'rustia.playground.' + project + '.selection';
const validName = /^[A-Za-z_][A-Za-z0-9_-]*\.luau$/;
const files = new Map(Object.entries(originals));
let selected = entryFile;
let restored = false;
try {
  for (const file of files.keys()) {
    const draft = localStorage.getItem(draftKey(file));
    if (draft !== null) { files.set(file, draft); restored = true; }
  }
  for (const key of Object.keys(localStorage)) {
    const name = key.slice(prefix.length);
    if (key.startsWith(prefix) && validName.test(name)) {
      files.set(name, localStorage.getItem(key));
      restored = true;
    }
  }
  if (restored) $('draft-status').textContent = 'Draft restored';
  selected = params.get('file') ?? sessionStorage.getItem(selectionKey) ?? selected;
} catch { $('draft-status').textContent = 'Browser storage unavailable'; }

function saveFile(file, text) {
  try {
    const key = draftKey(file);
    if (text === originals[file]) localStorage.removeItem(key);
    else localStorage.setItem(key, text);
    $('draft-status').textContent = 'Saved in this browser';
  } catch { $('draft-status').textContent = 'Could not save in this browser'; }
}

const extensions = [
  ...luauEditorLanguage({definitions:spriteMode ? './garden.d.luau' : './playground/console.d.luau', currentFile:() => editor.current, projectFiles:() => files}),
  ...editorIndentation($('indent-tabs'), $('tab-help')),
];
const surface = sourceEditor($('source'), $('error'), extensions);
const editor = new BrowserEditor({
  source:$('source'), list:$('files'), restore:$('restore'),
  onSelect(file) {
    $('current-file').textContent = file;
    surface.select(file);
    surface.sync();
    try { sessionStorage.setItem(selectionKey, file); } catch { /* Selection is optional. */ }
  },
  onEdit(file, text) {
    files.set(file, text);
    saveFile(file, text);
    surface.sync();
    schedule(file);
  },
  onSaveDue:execute,
});
const pause = $('pause');
const preview = $('sprite-preview');
const sprites = $('preview-shapes');
const workingKey = gardenDrafts + 'silk-ribbons.luau.working';
const restoreWorking = $('restore-working');
let working = [];
if (garden) {
  try {
    const entries = JSON.parse(localStorage.getItem(workingKey));
    if (Array.isArray(entries)) working = entries.filter(item => typeof item?.source === 'string');
  } catch { /* A broken checkpoint leaves the draft intact. */ }
}
const entryForConsole = () => Array.from(files, ([name, text]) => [name === entryFile ? 'main.luau' : name, text]);
function gardenCheckpoint(projectFiles) {
  const edits = Object.fromEntries(projectFiles);
  return {source:edits[entryFile], drawing:edits['drawing.luau'], flowerbed:edits['flowerbed.luau'], files:projectFiles};
}
function sameCheckpoint(a, b) {
  return a?.source === b.source && a?.drawing === b.drawing && a?.flowerbed === b.flowerbed &&
    JSON.stringify(a?.files ?? b.files) === JSON.stringify(b.files);
}
function setWorking(entries) {
  working = entries;
  try { localStorage.setItem(workingKey, JSON.stringify(working)); } catch { /* Keep session recovery. */ }
}
restoreWorking.onclick = () => {
  const saved = working[0];
  if (!saved) return;
  const recovered = saved.files ?? [
    [entryFile, saved.source],
    ['drawing.luau', saved.drawing ?? originals['drawing.luau']],
    ['flowerbed.luau', saved.flowerbed ?? originals['flowerbed.luau']],
  ];
  for (const [file, text] of recovered) {
    files.set(file, text);
    saveFile(file, text);
    if (editor.drafts.has(file)) editor.setDraft(file, text);
  }
  refreshFiles();
  editor.select(editor.current ?? entryFile);
  surface.sync();
  surface.diagnostic(null);
  if (player?.paused) pausedEdit = true;
  else execute();
};
const output = $('output');
const programPanel = $('program-panel');
function selectConsole(repl) {
  programPanel.hidden = repl;
  $('repl-panel').hidden = !repl;
  $('show-program').setAttribute('aria-pressed', String(!repl));
  $('show-repl').setAttribute('aria-pressed', String(repl));
}
$('show-program').onclick = () => selectConsole(false);
$('show-repl').onclick = () => selectConsole(true);
connectRepl({form:$('repl-form'), command:$('repl-command'), result:$('repl-output'),
  reset:$('repl-reset'), files:entryForConsole});

const inputForm = $('program-input');
const inputLine = $('input-line');
let pointer;
let pausedEdit = false;
let player;
if (spriteMode) {
  preview.removeAttribute('hidden');
  $('sprite-stage').hidden = false;
  $('sprite-layout-controls').hidden = false;
  try {
    const response = await fetch('../sprite-defs.html');
    if (!response.ok) throw Error('Could not load sprite shapes.');
    preview.insertAdjacentHTML('afterbegin', await response.text());
    const workspace = document.querySelector('.workspace');
    const stage = $('sprite-stage');
    const layout = $('sprite-layout');
    const layoutKey = 'rustia.playground.sprite-layout.' + (garden ? 'garden' : 'sprites');
    layout.value = garden ? 'garden' : 'compact';
    try {
      const savedLayout = localStorage.getItem(layoutKey) ?? (!garden && localStorage.getItem('rustia.playground.sprite-layout'));
      if (savedLayout === 'garden' || savedLayout === 'compact') layout.value = savedLayout;
    }
    catch { /* Use the compact layout when storage is unavailable. */ }
    let radius = 135;
    player = createGardenPlayer({
      inputs:() => ({...pointer, radius}),
      onReady(instance) {
        if (garden) {
          const saved = gardenCheckpoint(instance.files);
          if (!sameCheckpoint(working[0], saved)) setWorking([saved, ...working.slice(0, 1)]);
        }
        surface.diagnostic(null);
        setStatus(instance.animated ? (player.reducedMotion ? 'Reduced motion' : 'Live') : 'Finished',
          instance.animated ? 'live' : 'finished');
        pause.disabled = !instance.animated;
      },
      onFrame(_instance, frame) {
        drawGarden(sprites, frame.shapes);
        appendOutput(frame.output);
      },
      onError(instance, message, partialOutput) {
        if (garden) {
          if (sameCheckpoint(working[0], gardenCheckpoint(instance.files))) setWorking(working.slice(1));
          restoreWorking.hidden = !working.length;
        }
        appendOutput(partialOutput);
        const showingPrevious = Boolean(player.active?.animated);
        setStatus(showingPrevious ? 'Running the last working edit' : 'Error', showingPrevious ? 'warning' : 'error');
        pause.disabled = !showingPrevious;
        const helper = message.match(/In (drawing|flowerbed)\.luau:/);
        surface.diagnostic(message, editor, helper ? helper[1] + '.luau' : entryFile);
      },
    });
    function positionPreview() {
      if (layout.value === 'garden') {
        const area = stage.getBoundingClientRect();
        const box = preview.getBoundingClientRect();
        const scale = Math.min(area.width / 480, area.height / 270);
        if (!scale) return;
        const x = (box.left - area.left - area.width / 2) / scale;
        const y = (box.top - area.top - area.height / 2) / scale;
        preview.setAttribute('viewBox', [x, y, box.width / scale, box.height / scale].join(' '));
        radius = Math.min(area.width, area.height) / (2 * scale);
      } else {
        preview.setAttribute('viewBox', '-240 -135 480 270');
        radius = 135;
      }
      player.refresh();
    }
    const solid = $('garden-solid'), tracking = $('garden-pointer');
    if (garden) {
      $('garden-solid-option').hidden = false;
      $('garden-pointer-option').hidden = false;
      solid.checked = matchMedia('(prefers-reduced-transparency: reduce), (prefers-contrast: more), (forced-colors: active)').matches;
      tracking.checked = true;
      try {
        solid.checked = (localStorage.getItem('rustia.garden.solid') ?? String(solid.checked)) === 'true';
        tracking.checked = localStorage.getItem('rustia.garden.trackEditor') !== 'false';
      } catch { /* Use device and page defaults. */ }
      for (const [control, key] of [[solid, 'solid'], [tracking, 'trackEditor']]) {
        control.onchange = () => {
          try { localStorage.setItem('rustia.garden.' + key, String(control.checked)); } catch { /* Apply locally. */ }
          applyLayout();
        };
      }
    }
    function applyLayout() {
      workspace.classList.toggle('solid-editor', solid.checked);
      workspace.classList.toggle('garden-backdrop', layout.value === 'garden');
      pointer = undefined;
      positionPreview();
    }
    layout.onchange = () => {
      try { localStorage.setItem(layoutKey, layout.value); } catch { /* The switch still works. */ }
      applyLayout();
    };
    applyLayout();
    const observer = new ResizeObserver(positionPreview);
    observer.observe(workspace);
    observer.observe(stage);

    workspace.addEventListener('pointermove', event => {
      const matrix = preview.getScreenCTM();
      if (!matrix) return;
      const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(matrix.inverse());
      const overEditor = event.target.closest('.editor');
      pointer = (layout.value === 'garden' && (!garden || tracking.checked || !overEditor)) || (layout.value !== 'garden' && Math.abs(point.x) <= 240 && Math.abs(point.y) <= 135)
        ? {x:point.x, y:point.y} : undefined;
      player.refresh();
    }, {capture:true, passive:true});
    function clearPointer() { pointer = undefined; player.refresh(); }
    workspace.onpointerleave = workspace.onpointercancel = clearPointer;
    pause.onclick = () => {
      if (player.paused) {
        player.resume();
        pause.textContent = 'Pause';
        if (pausedEdit) execute();
        else setStatus(player.reducedMotion ? 'Reduced motion' : 'Live', 'live');
      } else {
        pausedEdit = player.checking || [...files.keys()].some(file => editor.waiting(file));
        editor.clear();
        player.pause();
        pause.textContent = 'Resume';
        setStatus('Paused', 'paused');
      }
    };
  } catch (error) {
    setStatus('Error', 'error');
    surface.diagnostic(error.message, editor, 'main.luau');
  }
}

function appendOutput(text) {
  if (text) {
    output.textContent = (output.textContent + text).slice(-32_768);
    output.scrollTop = output.scrollHeight;
  }
}

let worker, timeout, waitingForInput = false, refocusInput = false;
function setStatus(text, tone) {
  const status = $('state');
  status.dataset.tone = tone;
  status.textContent = text;
}
function cancelRun() {
  clearTimeout(timeout);
  worker?.terminate();
  worker = undefined;
  waitingForInput = false;
  inputForm.hidden = true;
  inputLine.value = '';
}
function schedule(file) {
  cancelRun();
  editor.clear();
  surface.diagnostic(null);
  if (player?.paused) {
    pausedEdit = true;
    return;
  }
  player?.cancelEdit();
  output.textContent = '';
  setStatus('Editing…', 'pending');
  editor.schedule(file);
}
function execute() {
  editor.clear();
  cancelRun();
  surface.diagnostic(null);
  output.textContent = '';
  if (spriteMode) {
    if (!player) return;
    pausedEdit = false;
    setStatus('Checking…', 'pending');
    restoreWorking.hidden = true;
    player.load({files:[...files], garden});
    return;
  }
  const trial = new Worker(new URL('./worker.mjs?v=console', import.meta.url), {type:'module'});
  worker = trial;
  trial.onmessage = ({data}) => {
    if (worker !== trial) return;
    clearTimeout(timeout);
    appendOutput(data.output);
    if (data.waiting) {
      waitingForInput = true;
      inputForm.hidden = false;
      inputLine.disabled = false;
      setStatus('Waiting for input', 'pending');
      if (refocusInput && !programPanel.hidden) inputLine.focus();
    } else {
      finishConsole(data.error);
    }
  };
  trial.onerror = event => {
    if (worker === trial) finishConsole(event.message || 'Could not start Rustia.');
  };
  refocusInput = false;
  requestConsole({type:'start', files:entryForConsole()});
}

function finishConsole(error) {
  cancelRun();
  setStatus(error ? 'Error' : 'Finished', error ? 'error' : 'finished');
  surface.diagnostic(garden ? error?.replaceAll('main.luau', entryFile) : error, editor, entryFile);
}
function requestConsole(message) {
  waitingForInput = false;
  inputLine.disabled = true;
  setStatus('Running…', 'live');
  const trial = worker;
  timeout = setTimeout(() => {
    if (worker === trial) finishConsole('The script timed out. Edit it to try again.');
  }, 15000);
  trial.postMessage(message);
}
inputForm.onsubmit = event => {
  event.preventDefault();
  if (!waitingForInput) return;
  const line = inputLine.value;
  inputLine.value = '';
  appendOutput('> ' + line + '\n');
  refocusInput = true;
  requestConsole({type:'input', line});
};
function refreshFiles() {
  editor.setFiles([...files].map(([file, source]) => ({file, source, original:originals[file] ?? ''})));
}
function showForm(open) {
  $('new-file').hidden = !open;
  $('add-file').setAttribute('aria-expanded', String(open));
  $('name-error').textContent = '';
  if (open) $('filename').focus();
  else $('add-file').focus();
}
$('add-file').onclick = () => showForm($('new-file').hidden);
$('cancel-add').onclick = () => showForm(false);
$('new-file').onsubmit = event => {
  event.preventDefault();
  const stem = $('filename').value.trim();
  const file = stem.endsWith('.luau') ? stem : stem + '.luau';
  if (!validName.test(file) || files.has(file) || (garden && file === 'main.luau')) {
    $('name-error').textContent = files.has(file) ? 'A file with this name already exists.' : garden && file === 'main.luau' ? 'Use another name. The console uses main.luau for the entry script.' :
      'Start with a letter or underscore. Use letters, numbers, underscores or hyphens.';
    $('filename').focus();
    return;
  }
  files.set(file, '');
  saveFile(file, '');
  refreshFiles();
  editor.select(file);
  $('new-file').reset();
  showForm(false);
  surface.focus();
  schedule(file);
};
$('source').disabled = false;
refreshFiles();
editor.select(files.has(selected) ? selected : entryFile);
$('restart').onclick = () => {
  if (player) {
    player.restart();
    pause.textContent = 'Pause';
    pause.disabled = true;
  }
  execute();
};
$('restart').disabled = false;
execute();
