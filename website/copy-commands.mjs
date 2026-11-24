for (const copyButton of document.querySelectorAll('[data-copy]')) {
  const commands = document.getElementById(copyButton.dataset.copy);
  let resetCopyLabel;
  copyButton.onclick = async () => {
    clearTimeout(resetCopyLabel);
    let copied = false;
    try {
      if (!navigator.clipboard?.writeText) throw Error();
      await navigator.clipboard.writeText(commands.value);
      copied = true;
    } catch {
      // LAN previews use plain HTTP, where the clipboard API may be unavailable.
      commands.focus();
      commands.select();
      try { copied = document.execCommand('copy'); } catch { /* keep the text selected */ }
      if (copied) copyButton.focus({preventScroll:true});
    }
    copyButton.textContent = copied ? 'Copied' : 'Use copy shortcut';
    resetCopyLabel = setTimeout(() => { copyButton.textContent = 'Copy'; }, 1800);
  };
}
