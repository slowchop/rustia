import {tutorial, raceSteps} from './tutorial.mjs';
import {sourceEditor} from './source-editor.mjs?v=error-slide';
import {BrowserEditor} from './editor.mjs';

const $ = id => document.getElementById(id);
const source = $('race-source'), status = $('race-status'), errorBox = $('race-error');
const surface = sourceEditor(source, errorBox);
const output = $('race-stdout');
let current, lesson, originalStandings;
let frames = [];
let playbackTimer;
let inView = false;

function stopPlayback() {
  clearTimeout(playbackTimer);
  frames = [];
}

function play(frame = 0) {
  if (!inView || document.hidden || frames.length === 0) return;
  output.textContent = frames[frame];
  if (frame + 1 < frames.length) {
    if (frame === 0) status.textContent = 'Racing…';
    playbackTimer = setTimeout(() => play(frame + 1), 150);
  } else {
    status.textContent = 'Finished';
    playbackTimer = setTimeout(() => countdown(3), 1000);
  }
}

function countdown(seconds) {
  status.textContent = 'Restarting in ' + seconds + '…';
  playbackTimer = setTimeout(() => {
    if (seconds === 1) evaluate();
    else countdown(seconds - 1);
  }, 1000);
}

function syncPlayback() {
  clearTimeout(playbackTimer);
  play();
}

new IntersectionObserver(([entry]) => {
  inView = entry.isIntersecting;
  syncPlayback();
}).observe($('race-stdout').parentElement);
document.addEventListener('visibilitychange', syncPlayback);
function cancel() {
  if (!current) return;
  clearTimeout(current.timer);
  current.worker.terminate();
  current = undefined;
}
function showError(message) {
  surface.diagnostic(message, editor, 'standings.luau');
}
function evaluate(checkLesson = false) {
  cancel();
  stopPlayback();
  showError('');
  status.textContent = 'Running…';
  const worker = new Worker(new URL('./race-worker.mjs?lessons=1',import.meta.url), {type:'module'});
  function finish(text, error, repeatable) {
    if (worker !== current?.worker) return;
    cancel();
    // Form feed separates console screens. Ordinary print output stays static.
    const screens = text.split(/\f\r?\n?/);
    output.textContent = screens[0];
    if (error || screens.length === 1) {
      output.textContent = screens[screens.length - 1];
      showError(error || '');
      status.textContent = error ? 'Error' : 'Finished';
      return;
    }
    if (repeatable) lesson.accept('seed');
    frames = screens;
    status.textContent = 'Ready';
    syncPlayback();
  }
  const timer = setTimeout(() => finish('', 'The race timed out. Edit the script to try again.'), 15000);
  current = {worker, timer};
  worker.onmessage = ({data}) => finish(data.output, data.error, data.repeatable);
  worker.onerror = event => finish('', event.message || 'Could not start Rustia.');
  worker.postMessage({source: editor.drafts.get('standings.luau'), rules: editor.drafts.get('race.luau'),
    checkSeed:checkLesson && lesson.current === 'seed' && editor.drafts.get('standings.luau') !== originalStandings});
}
const editor = new BrowserEditor({
  source, list:$('race-files'), restore:$('race-restore'),
  onSelect(file) { surface.select(file); lesson?.refresh(); },
  onEdit(file) {
    surface.sync(); lesson?.refresh();
    showError('');
    cancel();
    stopPlayback();
    editor.clear();
    status.textContent = 'Editing…';
    editor.schedule(file);
  },
  onSaveDue:() => evaluate(true)
});
try {
  const files = await Promise.all(['standings.luau','race.luau'].map(async file => {
    const res = await fetch(new URL(file, import.meta.url));
    if (!res.ok) throw Error('Could not load ' + file);
    return {file, source:await res.text()};
  }));
  source.disabled = false;
  editor.setFiles(files);
  for (const link of document.querySelectorAll('[data-open-race-file]')) {
    link.addEventListener('click', () => {
      editor.select(link.dataset.openRaceFile);
      surface.focus();
    });
    link.disabled = false;
  }
  editor.select('standings.luau');
  originalStandings = editor.drafts.get('standings.luau');
  lesson = tutorial(document.querySelector('.race-pair'), raceSteps, surface, editor);
  evaluate();
} catch (error) {
  status.textContent = 'Error';
  showError(String(error));
}
