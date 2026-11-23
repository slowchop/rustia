// A lesson points at a range in the live editor. The host accepts an answer
// only after its worker observes the requested behaviour in the edited program.
export const gardenSteps = [
  { id:'blue-flower', file:'garden.luau', prompt:'Turn the ring at the origin into a blue flower.',
    target:/^(\s*sprite\(vector(?:\.zero|\(0, 0\)),\s*)([\w.]+)/m,
    value:'kinds.BlueFlower', hint:'sprite(vector.zero, kinds.BlueFlower)' }
];
export const ribbonSteps = [
  { id:'four-ribbons', file:'garden.luau', prompt:'Show 4 ribbons.',
    target:/^([ \t]*local\s+ribbons\s*=\s*)(.+?)(?=[ \t]*(?:--[^\n]*)?$)/m,
    value:'4', hint:'local ribbons = 4' },
  { id:'follow', file:'garden.luau', prompt:'The second ribbon shies away. Change its pull so its tip follows your pointer.',
    target:/^([ \t]*local\s+pull\s*=\s*)(.+?)(?=[ \t]*(?:--[^\n]*)?$)/m,
    value:'120', hint:'local pull = 120' }
];
export const raceSteps = [
  { id:'seed', file:'standings.luau', prompt:'Give the race a fixed seed so the same runners finish in the same order each time.',
    target:/^([ \t]*(?:local\s+)?seed\s*=\s*)(.+?)(?=[ \t]*(?:--[^\n]*)?$)/m,
    value:'7', hint:'seed = 7' }
];

export function tutorial(container, steps, surface, editor) {
  const card = document.createElement('aside');
  card.className = 'lesson-card';
  card.setAttribute('aria-label', container.dataset.lesson);
  card.innerHTML = '<div class="lesson-instruction" aria-live="polite"></div><div class="lesson-actions"><button type="button" class="lesson-hint" aria-expanded="false">Hint</button><button type="button" class="lesson-edit">Do it for me</button><button type="button" class="lesson-find" hidden>Show line</button></div><pre class="lesson-example" hidden><code></code></pre>';
  const arrow = document.createElementNS('http://www.w3.org/2000/svg','svg');
  arrow.setAttribute('class','lesson-arrow');
  arrow.setAttribute('aria-hidden','true');
  const path = document.createElementNS(arrow.namespaceURI,'path');
  arrow.append(path);
  container.append(card, arrow);
  const instruction = card.querySelector('.lesson-instruction'), actions = card.querySelector('.lesson-actions');
  const hint = card.querySelector('.lesson-hint'), apply = card.querySelector('.lesson-edit');
  const example = card.querySelector('pre'), find = card.querySelector('.lesson-find');
  let index = 0, phase = 'active', update;

  function range() {
    const step = steps[index];
    const match = step?.target.exec(editor.drafts.get(step.file) || '');
    if (!match) return;
    const from = match.index + match[1].length;
    return {from, to:from + match[2].length};
  }

  const editorPane = container.querySelector('.editor');
  const obstacles = [editorPane, ...container.querySelectorAll('.garden-caption, .race-output .bar, #race-stdout')];
  const compact = document.createElement('button');
  compact.type = 'button';
  compact.textContent = 'Tutorial';
  compact.className = 'lesson-launcher';
  compact.hidden = true;
  container.append(compact);
  let expanded = false;
  compact.onclick = () => { expanded = !expanded; refresh(); };

  function place(bounds, point) {
    const avoid = obstacles.map(el => el.getBoundingClientRect());
    const width = card.offsetWidth, height = card.offsetHeight + (example.hidden ? 0 : example.offsetHeight + 5);
    const edge = 8, pad = 12;
    const xMin = bounds.left + edge, xMax = bounds.right - width - edge;
    const yMin = bounds.top + edge, yMax = bounds.bottom - height - edge;
    const anchor = point || {x:avoid[0].left, y:avoid[0].top + 60};
    const xs = [anchor.x-width-pad, anchor.x+pad, xMin, xMax];
    const ys = [anchor.y-height/2, yMin, yMax];
    for (const r of avoid) {
      xs.push(r.left-width-pad, r.right+pad);
      ys.push(r.top-height-pad, r.bottom+pad);
    }
    let best, score = Infinity;
    for (const rawX of xs) for (const rawY of ys) {
      const x = Math.max(xMin, Math.min(rawX, xMax));
      const y = Math.max(yMin, Math.min(rawY, yMax));
      if (x > xMax || y > yMax || avoid.some(r =>
        x < r.right+4 && x+width > r.left-4 && y < r.bottom+4 && y+height > r.top-4)) continue;
      const distance = Math.hypot(anchor.x - Math.max(x, Math.min(anchor.x,x+width)),
        anchor.y - Math.max(y, Math.min(anchor.y,y+height)));
      if (distance < score) { best = {x,y}; score = distance; }
    }
    compact.hidden = !!best;
    if (best) expanded = false;
    compact.textContent = expanded ? 'Close tutorial' : 'Tutorial';
    compact.setAttribute('aria-expanded', String(expanded));
    card.style.visibility = best || expanded ? 'visible' : 'hidden';
    const spot = best || {x:xMax, y:bounds.bottom - height - 46};
    card.style.left = (spot.x - bounds.left) + 'px';
    card.style.top = (spot.y - bounds.top) + 'px';
    return best || expanded;
  }

  function layout() {
    update = null;
    path.removeAttribute('d');
    const bounds = container.getBoundingClientRect();
    if (phase !== 'active') { place(bounds); return; }
    const step = steps[index], target = range();
    apply.disabled = !target;
    const point = target && editor.current === step.file && surface.point(target.from);
    find.hidden = !target || !!point;
    if (!place(bounds, point) || !point) return;
    const box = card.getBoundingClientRect();
    const fromRight = box.left >= editorPane.getBoundingClientRect().right;
    const x = (fromRight ? point.textX + 4 : point.x - 4) - bounds.left;
    const y = point.y - bounds.top;
    const sx = Math.min(box.right, Math.max(box.left, bounds.left + x)) - bounds.left;
    const sy = Math.min(box.bottom, Math.max(box.top, point.y)) - bounds.top;
    const sign = sx > x ? -1 : 1;
    const elbow = fromRight ? editorPane.getBoundingClientRect().right - bounds.left + 4 : x-12;
    path.setAttribute('d', `M ${sx} ${sy} H ${elbow} V ${y} H ${x} m ${-6*sign} -4 ${6*sign} 4 ${-6*sign} 4`);
  }
  function refresh() { if (update == null) update = requestAnimationFrame(layout); }
  function reveal() {
    const pos = range();
    if (!pos) return;
    editor.select(steps[index].file);
    surface.reveal(pos.from);
    refresh();
  }
  find.onclick = reveal;
  hint.onclick = () => {
    example.hidden = !example.hidden;
    card.classList.toggle('has-hint', !example.hidden);
    hint.setAttribute('aria-expanded', String(!example.hidden));
    refresh();
  };
  apply.onclick = () => {
    const pos = range();
    if (!pos || phase !== 'active') return;
    reveal();
    surface.replace(pos, steps[index].value);
    // The normal edit pipeline verifies the change just like a hand edit.
  };
  function render() {
    const step = steps[index];
    phase = step ? 'active' : 'done';
    card.dataset.phase = phase;
    instruction.textContent = step ? step.prompt : 'Tutorial complete. Have fun!';
    actions.hidden = !step;
    example.hidden = true;
    hint.setAttribute('aria-expanded','false');
    example.firstElementChild.textContent = step?.hint || '';
    refresh();
  }
  function accept(id) {
    if (phase !== 'active' || steps[index].id !== id) return;
    phase = 'success';
    card.dataset.phase = phase;
    actions.hidden = example.hidden = true;
    card.classList.remove('has-hint');
    const praise = ['Nice one!', 'Got it!', 'That works!'];
    instruction.textContent = praise[Math.floor(Math.random() * praise.length)];
    refresh();
    setTimeout(() => {
      card.dataset.phase = 'leaving';
      setTimeout(() => { index++; render(); }, 220);
    }, 1050);
  }
  container.addEventListener('scroll', refresh, true);
  const observer = new ResizeObserver(refresh);
  observer.observe(container); observer.observe(card);
  for (const el of obstacles) observer.observe(el);
  render();
  return {get current() { return phase === 'active' ? steps[index].id : null; }, refresh, accept};
}
