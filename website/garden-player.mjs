// Owns the live Rustia worker and the worker testing the next edit.
// The page handles drafts, diagnostics and drawing.
export function createGardenPlayer({inputs, onReady, onFrame, onError}) {
  let active, candidate, paused = false, previousTime;
  let reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

  function discard(instance) {
    if (!instance) return;
    clearTimeout(instance.timer);
    instance.worker.terminate();
  }
  function cancelEdit() {
    discard(candidate);
    candidate = undefined;
  }
  function fail(instance, message, output) {
    if (instance !== active && instance !== candidate) return;
    discard(instance);
    if (instance === active) active = undefined;
    if (instance === candidate) candidate = undefined;
    onError(instance, message, output);
  }
  function request(instance, message) {
    const input = inputs();
    instance.radius = input.radius;
    instance.busy = true;
    instance.timer = setTimeout(() => fail(instance, 'The script did not respond.'),
      message.type === 'load' ? 20000 : 4000);
    instance.worker.postMessage({...message, ...input, time:instance.time}, message.port ? [message.port] : []);
  }
  function refresh() {
    if (!candidate && !paused && reducedMotion && active?.animated && !active.busy) request(active, {type:'frame'});
  }
  function load(code, {lesson, recovering = false, time = active?.time ?? 0} = {}) {
    cancelEdit();
    const worker = new Worker(new URL('./worker.mjs?v=garden-project', import.meta.url), {type:'module'});
    const instance = {...code, worker, recovering, time};
    candidate = instance;
    worker.onerror = event => fail(instance, event.message || 'Could not start the Wasm worker.');
    worker.onmessage = ({data}) => {
      if (instance !== active && instance !== candidate) return;
      clearTimeout(instance.timer);
      instance.busy = false;
      if ('error' in data) return fail(instance, data.error || 'Script failed.', data.output);
      instance.animated = data.animated;
      if (instance === candidate) {
        discard(active);
        active = instance;
        candidate = undefined;
        onReady(instance);
      }
      // A frame already in flight when Pause was clicked belongs to this VM.
      if (paused) instance.pendingFrame = data;
      else onFrame(instance, data);
      if (instance.radius !== inputs().radius) refresh();
    };
    // Snapshot after any in-flight frame. Hold further ticks until the new VM
    // either renders that state or fails, so flowers never jump backwards.
    let port;
    if (active && (!code.files || code.garden)) {
      const channel = new MessageChannel();
      port = channel.port2;
      active.worker.postMessage({type:'snapshot', port:channel.port1}, [channel.port1]);
    }
    request(instance, {type:'load', ...code, lesson, port});
  }
  function pause() {
    paused = true;
    cancelEdit();
  }
  function resume() {
    paused = false;
    previousTime = undefined;
    if (active?.pendingFrame) {
      onFrame(active, active.pendingFrame);
      active.pendingFrame = undefined;
    }
    refresh();
  }
  function restart() {
    discard(active);
    active = undefined;
    reducedMotion = false;
    resume();
  }
  function tick(now) {
    if (!candidate && !paused && active?.animated && !reducedMotion && !document.hidden) {
      active.time += Math.min((now - (previousTime ?? now)) / 1000, 0.05);
      if (!active.busy) request(active, {type:'frame'});
    }
    previousTime = now;
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);
  return {load, cancelEdit, pause, resume, restart, refresh,
    get active() { return active; }, get checking() { return Boolean(candidate); },
    get paused() { return paused; }, get reducedMotion() { return reducedMotion; }};
}
