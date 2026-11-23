import {EditorView, indentUnit, indentWithTab, keymap} from './codemirror.js?v=four-spaces';

export function editorIndentation(checkbox, help) {
  // Keep the existing homepage preference across both editors.
  const key = 'rustia.garden.indentTabs';
  try { checkbox.checked = localStorage.getItem(key) === 'true'; }
  catch { /* The checkbox still works for this page. */ }
  help.hidden = !checkbox.checked;
  checkbox.addEventListener('change', () => {
    help.hidden = !checkbox.checked;
    try { localStorage.setItem(key, String(checkbox.checked)); }
    catch { /* The checkbox still works for this page. */ }
  });
  return [
    indentUnit.of('    '),
    EditorView.contentAttributes.of({'aria-describedby':help.id}),
    keymap.of([
      {key:'Tab', run:view => checkbox.checked && indentWithTab.run(view),
        shift:view => checkbox.checked && indentWithTab.shift(view)},
      {key:'Escape', run:() => {
        if (!checkbox.checked) return false;
        checkbox.focus();
        return true;
      }},
    ]),
  ];
}
