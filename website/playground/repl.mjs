export function connectRepl({form, command, result, reset, files}) {
  let worker, timer;
  let history = [], position = 0;
  let busy = false;

  function append(text) {
    result.textContent = (result.textContent + text).slice(-32_768);
    result.scrollTop = result.scrollHeight;
  }
  function finish(text, error) {
    clearTimeout(timer);
    append(text || '');
    if (error) append(error + '\n');
    busy = false;
    form.querySelector('[type=submit]').disabled = false;
  }
  function close() {
    worker?.terminate();
    worker = undefined;
  }
  reset.onclick = () => {
    close();
    result.textContent = '';
    command.value = '';
    position = history.length;
    finish();
    command.focus();
  };
  form.onsubmit = event => {
    event.preventDefault();
    const source = command.value.trim();
    if (busy || !source) return;
    if (!worker) {
      worker = new Worker(new URL('./worker.mjs?v=console', import.meta.url), {type:'module'});
      const current = worker;
      worker.onmessage = ({data}) => {
        if (worker === current) finish(data.output, data.error);
      };
      worker.onerror = event => {
        if (worker !== current) return;
        close();
        finish('', event.message || 'Could not start Rustia.');
      };
    }
    busy = true;
    form.querySelector('[type=submit]').disabled = true;
    command.value = '';
    history = [...history.slice(-99), source];
    position = history.length;
    append('> ' + source + '\n');
    timer = setTimeout(() => {
      close();
      finish('', 'Command timed out. The console session was reset.');
    }, 15000);
    worker.postMessage({type:'command', source, files:[...files()]});
  };
  command.onkeydown = event => {
    if (event.isComposing) return;
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      form.requestSubmit();
    } else if (!command.value.includes('\n') && ['ArrowUp', 'ArrowDown'].includes(event.key)) {
      event.preventDefault();
      const direction = event.key === 'ArrowUp' ? -1 : 1;
      position = Math.max(0, Math.min(history.length, position + direction));
      command.value = history[position] ?? '';
    }
  };
}
