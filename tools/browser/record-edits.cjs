"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const editFrames = 240;

// Use the recording clock for the caret blink and keycap fade in headless screenshots.
function installInputOverlay() {
  document.body.insertAdjacentHTML("beforeend", `
    <style>
      #caret, #keycap { pointer-events: none; position: fixed; z-index: 2 }
      #caret { width: 3px; background: #abf3cf }
      #keycap { left: 50%; bottom: 32px; transform: translateX(-50%); opacity: 0;
        min-width: 72px; padding: 6px 18px; text-align: center; color: #142826;
        background: #d6f5e5; border: 2px solid #89e7bb; border-radius: 12px;
        box-shadow: 0 5px #678e7c; font: 650 34px/1.2 system-ui, sans-serif }
      #keycap.pressed { background: #89e7bb; transform: translate(-50%, 3px); box-shadow: 0 2px #678e7c }
    </style>
    <span id="caret" aria-hidden="true"></span>
    <kbd id="keycap" aria-hidden="true"></kbd>`);
  const source = document.querySelector("#source");
  const caret = document.querySelector("#caret");
  const keycap = document.querySelector("#keycap");
  const ctx = document.createElement("canvas").getContext("2d");
  const style = getComputedStyle(source);
  ctx.font = style.fontSize + " " + style.fontFamily;
  caret.style.height = style.fontSize;
  let blinkFrame = 0;
  let keyAge = Infinity;

  function placeCaret() {
    const rect = source.getBoundingClientRect();
    const lines = source.value.slice(0, source.selectionEnd).split("\n");
    const lineHeight = parseFloat(style.lineHeight);
    const x = rect.left + source.clientLeft + parseFloat(style.paddingLeft) - source.scrollLeft
      + ctx.measureText(lines.at(-1)).width;
    const y = rect.top + source.clientTop + parseFloat(style.paddingTop) - source.scrollTop
      + (lines.length - 1) * lineHeight + (lineHeight - parseFloat(style.fontSize)) / 2;
    caret.style.left = Math.round(x) + "px";
    caret.style.top = Math.round(y) + "px";
    blinkFrame = 0;
  }
  placeCaret();
  source.addEventListener("input", placeCaret);
  source.addEventListener("keydown", event => {
    keycap.textContent = event.key === "Backspace" ? "⌫ Backspace" : event.key;
    keycap.classList.add("pressed");
    keyAge = 0;
  });
  source.addEventListener("keyup", () => keycap.classList.remove("pressed"));
  window.updateInputOverlay = () => {
    caret.hidden = blinkFrame++ % 20 >= 10;
    keycap.style.opacity = String(Math.max(0, Math.min(1, (20 - keyAge++) / 5)));
  };
}

async function captureEdits(page, work) {
  const editor = page.getByLabel("movement.luau");
  await page.waitForFunction(() => window.stepRace !== undefined);
  const original = await editor.inputValue();
  const rule = "return x + 2";
  const ruleStart = original.indexOf(rule);
  assert.ok(ruleStart >= 0, "unexpected starting movement rule");
  const afterDigit = ruleStart + rule.length;
  await editor.evaluate((el, offset) => {
    el.focus();
    el.setSelectionRange(offset, offset);
  }, afterDigit);
  await page.evaluate(installInputOverlay);

  const edits = new Map([
    [40, { key: "Backspace", digit: "", state: "rejected", stride: 2 }],
    [48, { key: "2", digit: "2", state: "applied", stride: 2 }],
    [120, { key: "Backspace", digit: "", state: "rejected", stride: 2 }],
    [170, { key: "4", digit: "4", state: "applied", stride: 4 }],
  ]);
  let position = 0;
  let stride = 2;
  for (let frame = 0; frame < editFrames; frame++) {
    const edit = edits.get(frame);
    if (edit) {
      await page.keyboard.down(edit.key);
      assert.equal(await editor.inputValue(), original.replace(rule, "return x + " + edit.digit));
      assert.equal(await editor.evaluate(el => el.selectionEnd), afterDigit - 1 + edit.digit.length);
      assert.equal(await page.locator("#status").getAttribute("data-state"), edit.state);
      if (edit.state === "rejected") assert.match(await page.locator("#detail").innerText(), /movement\.luau/);
      assert.equal(await page.locator("#keycap").textContent(), edit.key === "Backspace" ? "⌫ Backspace" : edit.key);
      stride = edit.stride;
    }
    const release = edits.get(frame - 3);
    if (release) await page.keyboard.up(release.key);
    await page.evaluate(() => window.updateInputOverlay());
    if (frame % 3 === 0) {
      position += stride;
      assert.equal(await page.evaluate(() => window.stepRace()), position);
      assert.equal(await page.locator("#distance").innerText(), position + " m");
    }
    await page.screenshot({ path: path.join(work, String(frame).padStart(3, "0") + ".png"), caret: "hide" });
  }
  console.log("Verified all four keystrokes, caret positions and race ticks through both rejected edits.");
}

module.exports = { captureEdits, editFrames };
