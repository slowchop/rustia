const defaults = {files:170, output:0.34};
const key = 'rustia.playground.pane-widths';

export function resizePanes(workspace) {
  const [fileDivider, outputDivider] = workspace.querySelectorAll('.pane-divider');
  const desktop = matchMedia('(min-width:851px)');
  const dividerWidth = 12;
  const minimum = {files:130, editor:300, output:235};
  let preference = {...defaults};
  let widths = {};
  let available;
  try {
    const saved = JSON.parse(localStorage.getItem(key));
    if (saved && Number.isFinite(saved.files) && Number.isFinite(saved.output)) preference = saved;
  } catch { /* Fall back to the default widths. */ }

  const clamp = (value, low, high) => Math.max(low, Math.min(value, high));
  function layout() {
    if (!desktop.matches) return;
    available = workspace.clientWidth - 2 * dividerWidth;
    widths.files = Math.round(clamp(preference.files, minimum.files, available - minimum.editor - minimum.output));
    widths.output = Math.round(clamp(preference.output * available, minimum.output, available - minimum.editor - widths.files));
    for (const [pane, divider, other] of [
      ['files', fileDivider, 'output'], ['output', outputDivider, 'files'],
    ]) {
      workspace.style.setProperty('--' + pane + '-width', widths[pane] + 'px');
      divider.setAttribute('aria-valuemin', minimum[pane]);
      divider.setAttribute('aria-valuemax', available - minimum.editor - widths[other]);
      divider.setAttribute('aria-valuenow', widths[pane]);
      divider.setAttribute('aria-valuetext', widths[pane] + ' pixels');
    }
  }
  function save() {
    try { localStorage.setItem(key, JSON.stringify(preference)); }
    catch { /* Resizing still works for this visit. */ }
  }
  function resize(pane, other, width) {
    const next = clamp(width, minimum[pane], available - minimum.editor - widths[other]);
    if (pane === 'files') preference.files = next;
    else preference.output = next / available;
    layout();
  }
  function bind(divider, pane, other, direction) {
    let drag;
    divider.onpointerdown = event => {
      if (event.button !== 0 || drag) return;
      drag = {id:event.pointerId, start:event.clientX, width:widths[pane]};
      divider.setPointerCapture(event.pointerId);
      divider.dataset.dragging = '';
      divider.focus();
      event.preventDefault();
    };
    divider.onpointermove = event => {
      if (drag?.id === event.pointerId) resize(pane, other, drag.width + (event.clientX - drag.start) * direction);
    };
    divider.onlostpointercapture = () => {
      drag = undefined;
      delete divider.dataset.dragging;
      save();
    };
    divider.onkeydown = event => {
      const step = event.shiftKey ? 48 : 16;
      let width;
      switch (event.key) {
        case 'ArrowLeft': width = widths[pane] - step * direction; break;
        case 'ArrowRight': width = widths[pane] + step * direction; break;
        case 'Home': width = minimum[pane]; break;
        case 'End': width = available - minimum.editor - widths[other]; break;
        default: return;
      }
      event.preventDefault();
      resize(pane, other, width);
      save();
    };
    divider.ondblclick = () => {
      resize(pane, other, pane === 'files' ? defaults.files : defaults.output * available);
      save();
    };
  }
  bind(fileDivider, 'files', 'output', 1);
  bind(outputDivider, 'output', 'files', -1);
  new ResizeObserver(layout).observe(workspace);
  layout();
}
