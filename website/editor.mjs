// SPDX-License-Identifier: MIT OR Apache-2.0

export const AUTOSAVE_DELAY_MS = 400;

/** An editor for one open set of files. The host owns saving and execution. */
export class BrowserEditor {
  #source;
  #list;
  #restore;
  #diagnostic;
  #onSelect;
  #onEdit;
  #onSaveDue;
  #delay;
  #files = new Map();
  #buttons = new Map();
  #selections = new Map();
  #timers = new Map();
  #current = null;
  drafts = new Map();

  constructor({source, list, restore, diagnostic, onSelect, onEdit, onSaveDue,
               delay = AUTOSAVE_DELAY_MS}) {
    this.#source = source;
    this.#list = list;
    this.#restore = restore;
    this.#diagnostic = diagnostic;
    this.#onSelect = onSelect;
    this.#onEdit = onEdit;
    this.#onSaveDue = onSaveDue;
    this.#delay = delay;

    restore.disabled = true;
    source.addEventListener('input', () => this.#edit(source.value, false));
    restore.addEventListener('click', () => {
      const entry = this.#files.get(this.current);
      if (entry) this.#edit(entry.original, true);
    });
  }

  get current() { return this.#current; }

  get canRestore() {
    return this.current !== null && !this.#source.disabled && !this.#source.readOnly
      && this.drafts.get(this.current) !== this.#files.get(this.current).original;
  }

  #edit(text, restored) {
    if (this.current === null || this.#source.disabled || this.#source.readOnly) return;
    this.setDraft(this.current, text);
    if (this.#onEdit) this.#onEdit(this.current, text, {restored});
    else this.schedule(this.current);
  }

  setFiles(entries, {preserveDrafts = false} = {}) {
    const next = new Map();
    for (const {file, source, original, group = 'Files'} of entries) {
      const previous = this.#files.get(file);
      const baseline = original ?? (preserveDrafts ? previous?.original : undefined) ?? source;
      next.set(file, {source, original: baseline, group});
      const unedited = !this.waiting(file) && this.drafts.get(file) === previous?.source;
      if (!preserveDrafts || !this.drafts.has(file) || unedited) {
        this.drafts.set(file, source);
        this.cancel(file);
      }
    }
    for (const file of this.#files.keys()) {
      if (!next.has(file) || !preserveDrafts) this.#selections.delete(file);
      if (!next.has(file)) {
        this.cancel(file);
        this.drafts.delete(file);
      }
    }
    this.#files = next;
    this.#buttons.clear();
    this.#list.replaceChildren();
    const groups = new Map();

    for (const [file, {group}] of next) {
      let section = groups.get(group);
      if (!section) {
        section = this.#list.ownerDocument.createElement('section');
        section.className = 'file-group';
        const heading = this.#list.ownerDocument.createElement('div');
        heading.className = 'file-group-title';
        heading.textContent = group;
        section.append(heading);
        this.#list.append(section);
        groups.set(group, section);
      }
      const button = this.#list.ownerDocument.createElement('button');
      button.type = 'button';
      button.className = 'file-button';
      button.dataset.file = file;
      button.innerHTML = '<span class="file-label"></span><span class="file-status" data-tone="grey"><i></i><span></span></span><span class="file-save-state" hidden></span>';
      button.querySelector('.file-label').textContent = file;
      button.setAttribute('aria-current', String(this.current === file));
      button.addEventListener('click', () => this.select(file));
      section.append(button);
      this.#buttons.set(file, button);
    }
    if (!next.has(this.current)) this.#current = null;
    this.#showDraft();
  }

  select(file) {
    if (!this.#files.has(file)) return;
    const previous = this.current;
    const source = this.#source;
    if (previous !== file) {
      if (previous !== null) {
        this.#selections.set(previous, {
          start: source.selectionStart, end: source.selectionEnd, direction: source.selectionDirection,
          top: source.scrollTop, left: source.scrollLeft,
        });
        this.#buttons.get(previous).setAttribute('aria-current', 'false');
      }
      this.#current = file;
      this.#showDraft();
      const selection = this.#selections.get(file);
      source.setSelectionRange(selection?.start ?? 0, selection?.end ?? 0, selection?.direction ?? 'none');
      source.scrollTop = selection?.top ?? 0;
      source.scrollLeft = selection?.left ?? 0;
    }
    this.#buttons.get(file).setAttribute('aria-current', 'true');
    this.#onSelect?.(file);
  }

  setDraft(file, text) {
    if (!this.#files.has(file)) return;
    this.drafts.set(file, text);
    if (file === this.current) this.#showDraft();
  }

  #showDraft() {
    const text = this.drafts.get(this.current) ?? '';
    if (this.#source.value !== text) this.#source.value = text;
    this.#restore.disabled = !this.canRestore;
  }

  setFileState(file, {tone = 'grey', text = '', saveState = 'saved', saveLabel = '', dirty = false}) {
    const button = this.#buttons.get(file);
    if (!button) return;
    button.dataset.dirty = String(dirty);
    button.dataset.saveState = saveState;
    const status = button.querySelector('.file-status');
    status.dataset.tone = tone;
    status.querySelector('span').textContent = text;
    const save = button.querySelector('.file-save-state');
    save.textContent = saveLabel;
    save.hidden = !saveLabel;
  }

  showDiagnostic(entry) {
    const {panel, kind, text, location} = this.#diagnostic;
    panel.hidden = !entry;
    location.onclick = null;
    location.hidden = !entry?.location;
    if (!entry) return;
    kind.textContent = `${entry.kind} error`;
    text.textContent = entry.text;
    if (!entry.location) return;
    const {file, line, column = 1} = entry.location;
    location.textContent = `${file}:${line}${column > 1 ? `:${column}` : ''}`;
    location.disabled = !this.#files.has(file);
    location.onclick = () => {
      if (!this.#files.has(file)) return;
      this.select(file);
      const lines = this.#source.value.split('\n');
      const index = Math.max(0, Math.min(line - 1, lines.length - 1));
      let offset = Math.max(0, Math.min(column - 1, lines[index].length));
      for (let i = 0; i < index; i++) offset += lines[i].length + 1;
      this.#source.focus();
      this.#source.setSelectionRange(offset, offset);
    };
  }

  schedule(file, {immediate = false} = {}) {
    this.cancel(file);
    if (!this.#files.has(file)) return;
    if (immediate) {
      this.#onSaveDue?.(file);
      return;
    }
    this.#timers.set(file, setTimeout(() => {
      this.#timers.delete(file);
      this.#onSaveDue?.(file);
    }, this.#delay));
  }

  waiting(file) { return this.#timers.has(file); }

  cancel(file) {
    clearTimeout(this.#timers.get(file));
    this.#timers.delete(file);
  }

  flush() {
    for (const file of [...this.#timers.keys()]) {
      if (this.waiting(file)) this.schedule(file, {immediate: true});
    }
  }

  clear() {
    for (const file of this.#timers.keys()) this.cancel(file);
  }
}
