"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const editFrames = 240;

// The overlay listens to the same events as the editor. Its fade follows video frames.
function installInputOverlay() {
  document.body.insertAdjacentHTML("beforeend", `
    <style>
      #pointer, #keycap { pointer-events: none; position: fixed; z-index: 2 }
      #pointer { left: 0; top: 0; width: 28px; height: 34px; overflow: visible }
      #pointer circle { opacity: 0 }
      #pointer.pressed circle { opacity: .78 }
      #keycap { left: 50%; bottom: 32px; transform: translateX(-50%); opacity: 0;
        min-width: 72px; padding: 6px 18px; text-align: center; color: #142826;
        background: #d6f5e5; border: 2px solid #89e7bb; border-radius: 12px;
        box-shadow: 0 5px #678e7c; font: 650 34px/1.2 system-ui, sans-serif }
      #keycap.pressed { background: #89e7bb; transform: translate(-50%, 3px); box-shadow: 0 2px #678e7c }
    </style>
    <svg id="pointer" aria-hidden="true" viewBox="0 0 28 34">
      <circle cx="2" cy="2" r="16" fill="none" stroke="#89e7bb" stroke-width="3"/>
      <path d="M2 2 L3 26 L9 21 L15 32 L20 29 L14 19 L25 18 Z"
        fill="white" stroke="#142826" stroke-width="2" stroke-linejoin="round"/>
    </svg>
    <kbd id="keycap" aria-hidden="true"></kbd>`);
  const pointer = document.querySelector("#pointer");
  const keycap = document.querySelector("#keycap");
  document.addEventListener("pointermove", event => {
    pointer.style.transform = "translate(" + (event.clientX - 2) + "px, " + (event.clientY - 2) + "px)";
  });
  document.addEventListener("pointerdown", () => pointer.classList.add("pressed"));
  document.addEventListener("pointerup", () => pointer.classList.remove("pressed"));
  let age = Infinity;
  const source = document.querySelector("#source");
  source.addEventListener("keydown", event => {
    keycap.textContent = event.key === "Backspace" ? "⌫ Backspace" : event.key;
    keycap.classList.add("pressed");
    age = 0;
  });
  source.addEventListener("keyup", () => keycap.classList.remove("pressed"));
  window.updateInputOverlay = () => {
    keycap.style.opacity = String(Math.max(0, Math.min(1, (20 - age++) / 5)));
  };
}

function digitBounds(editor) {
  const digit = editor.value.indexOf("1");
  const lines = editor.value.slice(0, digit).split("\n");
  const style = getComputedStyle(editor);
  const rect = editor.getBoundingClientRect();
  const ctx = document.createElement("canvas").getContext("2d");
  ctx.font = style.fontSize + " " + style.fontFamily;
  const x = rect.left + editor.clientLeft + parseFloat(style.paddingLeft) - editor.scrollLeft
    + ctx.measureText(lines.at(-1)).width;
  const y = rect.top + editor.clientTop + parseFloat(style.paddingTop) - editor.scrollTop
    + (lines.length - 0.5) * parseFloat(style.lineHeight);
  return { x, y, width: ctx.measureText("1").width };
}

async function glide(mouse, from, to, progress) {
  const eased = progress * progress * (3 - 2 * progress);
  await mouse.move(from.x + (to.x - from.x) * eased, from.y + (to.y - from.y) * eased);
}

async function captureEdits(page, work) {
  const editor = page.getByLabel("movement.luau");
  await page.waitForFunction(() => window.stepRace !== undefined);
  const original = await editor.inputValue();
  assert.ok(original.includes("return x + 1"), "unexpected starting movement rule");
  await page.evaluate(installInputOverlay);
  const digit = await editor.evaluate(digitBounds);
  const start = { x: digit.x + 210, y: digit.y + 100 };
  const left = { x: digit.x + 1, y: digit.y };
  const right = { x: digit.x + digit.width - 1, y: digit.y };
  const rest = { x: right.x + 90, y: right.y + 45 };
  await page.mouse.move(start.x, start.y);
  const edits = new Map([
    [60, { key: "4", digit: "4", state: "applied", stride: 4 }],
    [140, { key: "Backspace", digit: "", state: "rejected", stride: 4 }],
    [200, { key: "2", digit: "2", state: "applied", stride: 2 }],
  ]);
  let position = 0;
  let stride = 1;
  for (let frame = 0; frame < editFrames; frame++) {
    if (frame >= 8 && frame <= 36) await glide(page.mouse, start, left, (frame - 8) / 28);
    if (frame === 38) await page.mouse.down();
    if (frame >= 39 && frame <= 49) await glide(page.mouse, left, right, (frame - 39) / 10);
    if (frame === 50) {
      await page.mouse.up();
      assert.equal(await editor.evaluate(el => el.value.slice(el.selectionStart, el.selectionEnd)), "1");
    }
    if (frame >= 64 && frame <= 80) await glide(page.mouse, right, rest, (frame - 64) / 16);
    const edit = edits.get(frame);
    if (edit) {
      await page.keyboard.down(edit.key);
      assert.equal(await editor.inputValue(), original.replace("return x + 1", "return x + " + edit.digit));
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
  console.log("Verified mouse selection, three keystrokes and every tick through both swaps and the rejected edit.");
}

module.exports = { captureEdits, editFrames };
