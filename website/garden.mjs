import {createGardenPlayer} from './garden-player.mjs?v=garden-project';
import {luauEditorLanguage} from './luau-editor-language.mjs';
import {drawGarden} from './draw-garden.mjs';
import {tutorial, gardenSteps, ribbonSteps} from './tutorial.mjs?v=vector-completion';
import {sourceEditor} from './source-editor.mjs?v=active-file';
import {BrowserEditor} from './editor.mjs';
import {editorIndentation} from './editor-indent.mjs';

const $ = s => document.querySelector(s);
// Both pages mount the same artwork, controls, and editor.
const mount = $('[data-garden]');
try {
  const [fragment, palette] = await Promise.all([
    read('garden-fragment.html?v=mobile-garden'), read('sprite-defs.html'),
  ]);
  mount.innerHTML = fragment;
  mount.querySelector('#bed').insertAdjacentHTML('afterbegin', palette);
  const links = $('#garden-home-links');
  if (links) mount.querySelector('.garden-caption').append(links.content.cloneNode(true));
} catch (err) {
  mount.textContent = 'Could not load the garden. Refresh to try again.';
  throw err;
}
const choices = [...document.querySelectorAll('.study-links a')];
const selected = new URLSearchParams(location.search).get('garden');
const choice = choices.find(a => new URL(a.href).searchParams.get('garden') === selected) || choices[0];
const study = choice && new URL(choice.href).searchParams.get('garden');
const file = (study || 'garden') + '.luau';
const drawingFile = 'drawing.luau';
const flowerFile = 'flowerbed.luau';
const workspace = study ? 'study.' + study : 'ribbons';
// Keep drafts saved under the original homepage filename.
const keyFor = name => 'rustia.garden.' + workspace + '.' +
  (!study && name === file ? 'silk-ribbons.luau' : name);
const originals = new Map();
const draftStatus = $('#draft-status');
let storageAvailable = true;
function readLocal(key) {
  try { return localStorage.getItem(key); }
  catch { storageAvailable = false; return null; }
}
function writeLocal(key, value) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
    return true;
  } catch { return false; }
}
// Save the source files together, only after Rustia has rendered a frame.
const workingKey = keyFor(file) + '.working';
let working = [];
try {
  const saved = JSON.parse(readLocal(workingKey));
  if (Array.isArray(saved)) working = saved.filter(entry =>
    entry && typeof entry.source === 'string' &&
    (study ? entry.drawing === undefined : typeof entry.drawing === 'string')).slice(0, 2);
} catch { /* A missing or unreadable checkpoint leaves the draft intact. */ }
function sameCode(a, b) {
  return a && b && a.source === b.source && a.drawing === b.drawing && a.flowerbed === b.flowerbed;
}
function saveWorking(entries) {
  working = entries;
  writeLocal(workingKey, JSON.stringify(entries));
}
const restoreWorking = $('#restore-working');
const solid = $('#solid-editor');
const appearanceKey = 'rustia.garden.solid';
const savedAppearance = readLocal(appearanceKey);
solid.checked = savedAppearance === null
  ? matchMedia('(prefers-reduced-transparency: reduce), (prefers-contrast: more), (forced-colors: active)').matches
  : savedAppearance === 'true';
function setAppearance() { mount.classList.toggle('translucent', !study && !solid.checked); }
setAppearance();
solid.onchange = () => { setAppearance(); writeLocal(appearanceKey, String(solid.checked)); };
if (study) {
  choice.setAttribute('aria-current','page');
  $('.garden-caption').prepend($('#bed'));
  $('.garden-caption h2').textContent = choice.textContent;
  $('.garden-caption p').textContent = {
    'wandering-flowers':'Ten sprites alternate between wandering and following your pointer.',
    'follow-the-leader':'Ten little trains wander. Bring the pointer near a leader to guide its followers.',
    'spring-pairs':'Each pair shares a spring. Sweep through the pairs to scatter them and watch them rebound.',
    'stepping-stones':'Creatures hop towards nine gathering spots. Move the pointer to carry one spot around.',
    'seed-drift':'A slow breeze carries the seeds. Rest your pointer among them and they gently drift apart.',
    'minnow-school':'Each minnow eases into its neighbour’s direction. Hold the pointer nearby to bend the school.',
    'silk-ribbons':'Bring your pointer near the tips. Five ribbons follow, three shy away, and both react faster as you get closer.',
    'meadow-breath':'A breeze passes across rooted flowers. Brush them aside and they slowly settle back.',
    'slow-confluence':'Two streams of seeds meet downstream. Slip your pointer into one to open a wake.',
    'lantern-bearers':'Little lights wander towards their own destinations. Hold still to gather nearby lights, then move away.',
    'pollen-relay':'The bee visits the ringed flower, then picks another. Get close to lead it astray.'
  }[study];
} else if (mount.dataset.starter) {
  $('.garden-caption h2').textContent = 'Wandering flowers';
  $('.garden-caption p').textContent = 'Move your pointer to lead the flowers.';
}
const bed = $('#bed'), layer = $('#sprites'), state = $('#state');
const pauseButton = $('#pause'), restartButton = $('#restart');
const extensions = [...luauEditorLanguage({
  definitions:'./garden.d.luau',
  currentFile:() => editor.current,
  projectFiles:() => editor.drafts,
}), ...editorIndentation($('#indent-tabs'), $('#tab-help'))];
const surface = sourceEditor($('#source'), $('#error'), extensions);
let pointer;
let radius;
let lesson, editor, drawingSource, flowerSource;
const player = createGardenPlayer({inputs:() => ({...pointer, radius}), onFrame, onReady, onError});

function draftCode() {
  return {source:editor.drafts.get(file), drawing:editor.drafts.get(drawingFile) ?? drawingSource,
    flowerbed:editor.drafts.get(flowerFile) ?? flowerSource};
}
function setStatus(text, tone) {
  state.dataset.tone = tone;
  state.textContent = text;
}
function liveStatus() {
  setStatus(player.reducedMotion ? 'Reduced motion' : 'Live', 'live');
}
function onError(instance, message) {
  if (sameCode(working[0], instance)) saveWorking(working.slice(1));
  if (player.paused) setStatus('Paused', 'paused');
  else if (player.active) setStatus('Current code has errors. Running the last working edit.', 'warning');
  else setStatus('Current code has errors. Waiting for a fix.', 'error');
  if (instance.recovering) {
    restoreWorking.hidden = !working.length;
    return;
  }
  const inDrawing = message.includes('In drawing.luau:') || /drawing\.luau:\d+:/.test(message);
  if (inDrawing && !editor.drafts.has(drawingFile)) message += ' Open the standalone garden to edit drawing.luau.';
  const inFlowerbed = /flowerbed\.luau:/.test(message);
  restoreWorking.hidden = !working.length || sameCode(working[0], draftCode());
  surface.diagnostic(message.replaceAll('garden.luau', file), editor, inFlowerbed ? flowerFile : inDrawing ? drawingFile : file);
  if (!player.paused && !player.active && !player.checking && working[0] && !sameCode(working[0], draftCode())) {
    start(working[0], {recovering:true});
  }
}
function start(code = draftCode(), options = {}) {
  if (!options.recovering) setStatus('Checking…', 'pending');
  player.load(code, {...options, lesson:code.source === originalGarden ? null : lesson?.current});
}
function onReady(instance) {
  const {source, drawing, flowerbed} = instance;
  if (!sameCode(working[0], instance)) saveWorking([{source, drawing, flowerbed}, ...working.slice(0, 1)]);
  if (instance.recovering) {
    setStatus('Current code has errors. Running the last working edit.', 'warning');
  } else {
    liveStatus();
    surface.diagnostic(null);
  }
}
function onFrame(instance, data) {
  drawGarden(layer, data.shapes);
  if (lesson && editor.drafts.get(file) === instance.source && instance.source !== originalGarden) {
    if (data.passed) lesson.accept(data.passed);
    if (lesson.current === 'blue-flower' && data.shapes.some(([kind, x, y]) =>
      kind === 4 && x === 0 && y === 0)) lesson.accept('blue-flower');
    if (lesson.current === 'four-ribbons' && data.shapes.filter((shape, i, shapes) =>
      i === 0 || shape[0] !== shapes[i - 1][0]).length === 4) {
      lesson.accept('four-ribbons');
    }
  }
}
restoreWorking.onclick = () => {
  const saved = working[0];
  if (!saved) return;
  editor.clear();
  player.cancelEdit();
  let persisted = true;
  for (const [name, text] of [[file, saved.source], [drawingFile, saved.drawing], [flowerFile, saved.flowerbed ?? originals.get(flowerFile)]]) {
    if (text === undefined) continue;
    if (name === drawingFile) drawingSource = text;
    if (name === flowerFile) flowerSource = text;
    editor.setDraft(name, text);
    if (!writeLocal(keyFor(name), text === originals.get(name) ? null : text)) persisted = false;
  }
  surface.sync();
  surface.diagnostic(null);
  surface.focus();
  lesson?.refresh();
  draftStatus.textContent = persisted ? 'Last working version restored' : 'Restored for this session';
  if (!player.paused) applyDraft();
};
function applyDraft() {
  if (sameCode(player.active, draftCode())) {
    player.active.recovering = false;
    surface.diagnostic(null);
    liveStatus();
  } else {
    start(draftCode(), {time:0});
  }
}
pauseButton.onclick = () => {
  editor.clear();
  if (player.paused) {
    player.resume();
    pauseButton.textContent = 'Pause';
    applyDraft();
  } else {
    player.pause();
    pauseButton.textContent = 'Resume';
    setStatus('Paused', 'paused');
  }
};
restartButton.onclick = () => {
  editor.clear();
  player.restart();
  pauseButton.textContent = 'Pause';
  start(draftCode(), {time:0});
};
const hero = $('.hero');
const narrow = matchMedia('(max-width: 820px)');
const openArea = $('.garden-origin-area');
const trackEditor = $('#track-editor');
const trackingKey = 'rustia.garden.trackEditor';
const editorPane = mount.querySelector('.editor');
if (mount.hasAttribute('data-mobile-editor')) {
  const toggle = mount.querySelector('.garden-editor-toggle');
  toggle.addEventListener('click', () => {
    const expanded = toggle.getAttribute('aria-expanded') === 'false';
    toggle.setAttribute('aria-expanded', String(expanded));
    toggle.textContent = expanded ? 'Hide editor' : 'Edit the garden';
  });
}
trackEditor.checked = readLocal(trackingKey) !== 'false';
trackEditor.onchange = () => {
  writeLocal(trackingKey, String(trackEditor.checked));
  updatePointer();
};
let pointerClient;
function updatePointer() {
  if (!pointerClient) return;
  const matrix = bed.getScreenCTM();
  if (!matrix) return;
  const p = pointerClient.matrixTransform(matrix.inverse());
  const box = bed.viewBox.baseVal;
  let outside = study && (p.x < box.x || p.x > box.x + box.width || p.y < box.y || p.y > box.y + box.height);
  if (!study && !trackEditor.checked) {
    const {left, right, top, bottom} = editorPane.getBoundingClientRect();
    outside = pointerClient.x >= left && pointerClient.x < right &&
              pointerClient.y >= top && pointerClient.y < bottom;
  }
  pointer = outside ? undefined : {x:p.x, y:p.y};
  player.refresh();
}
function fitGarden() {
  if (study) {
    const centered = choice.hasAttribute('data-centered');
    bed.setAttribute('viewBox', centered ? '-490 -280 980 560' : '0 0 980 560');
    if (centered) radius = bed.viewBox.baseVal.height / 2;
  } else {
    const svg = bed.getBoundingClientRect();
    const area = openArea.getBoundingClientRect();
    const scale = $('.bench').clientWidth / (narrow.matches ? 520 : 1050);
    if (!scale || !svg.height) return;
    const nextRadius = Math.min(area.width, area.height) / (2 * scale);
    const resized = nextRadius !== radius;
    radius = nextRadius;
    const left = (svg.left - area.left - area.width / 2) / scale;
    const top = (svg.top - area.top - area.height / 2) / scale;
    bed.setAttribute('viewBox', [left, top, svg.width / scale, svg.height / scale].join(' '));
    if (resized && !pointerClient) player.refresh();
  }
  updatePointer();
}
fitGarden();
const resize = new ResizeObserver(fitGarden);
for (const element of [bed, openArea, $('.bench')]) resize.observe(element);
narrow.addEventListener('change', fitGarden);
function trackPointer(event) {
  pointerClient = new DOMPoint(event.clientX, event.clientY);
  updatePointer();
}
// Capture before the text editor handles selection and drag events.
hero.addEventListener('pointermove', trackPointer, {capture:true, passive:true});
hero.addEventListener('pointerdown', trackPointer, {capture:true, passive:true});
window.addEventListener('scroll', updatePointer, {passive:true});
hero.onpointerleave = hero.onpointercancel = () => {
  pointerClient = undefined;
  pointer = undefined;
  player.refresh();
};
let originalGarden;
async function read(path) {
  const res = await fetch(path);
  if (!res.ok) throw Error('Could not load ' + path);
  return res.text();
}
try {
  const starter = mount.dataset.starter || (study ? 'examples/garden-studies/' + file : 'garden.luau');
  originalGarden = await read(starter + '?v=flowerbed');
  originals.set(file, originalGarden);
  if (!study) originals.set(flowerFile, await read('flowerbed.luau?v=1'));
  if (!study) originals.set(drawingFile, await read('drawing.luau?v=garden-ring'));
  let resumed = false;
  const entries = [];
  for (const [name, original] of originals) {
    const saved = readLocal(keyFor(name));
    resumed ||= saved !== null;
    const source = saved ?? original;
    if (name === drawingFile) drawingSource = source;
    if (name === flowerFile) flowerSource = source;
    if (name === file || mount.dataset.files === 'all') entries.push({file:name, source, original});
  }
  draftStatus.textContent = !storageAvailable ? 'Browser storage unavailable'
    : resumed ? 'Draft restored' : 'Autosaves in this browser';
  if ($('#rust')) $('#rust').textContent = await read('garden.rs');
  editor = new BrowserEditor({
    source:$('#source'), list:$('#files'), restore:$('#restore'),
    onSelect(file) { surface.select(file); surface.sync(); lesson?.refresh(); },
    onEdit(file, text) {
      const saved = writeLocal(keyFor(file), text === originals.get(file) ? null : text);
      draftStatus.textContent = saved ? 'Saved in this browser' : 'Could not save in this browser';
      surface.sync(); lesson?.refresh();
      player.cancelEdit();
      surface.diagnostic(null);
      editor.clear();
      if (!player.paused) { setStatus('Editing…', 'pending'); editor.schedule(file); }
    },
    onSaveDue() { if (!player.paused) start(); }
  });
  $('#source').disabled = false;
  editor.setFiles(entries);
  pauseButton.disabled = restartButton.disabled = false;
  const requested = new URLSearchParams(location.search).get('file');
  editor.select(editor.drafts.has(requested) ? requested : file);
  if (!study && mount.dataset.tutorial !== 'off') {
    const steps = ribbonSteps[0].target.test(editor.drafts.get(file)) ? ribbonSteps : gardenSteps;
    lesson = tutorial($('.bench'), steps, surface, editor);
  }
  start();
} catch (err) {
  setStatus(String(err), 'error');
}
