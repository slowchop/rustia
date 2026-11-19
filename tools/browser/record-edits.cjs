"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");

const editFrames = 240;

// The overlay listens to the same events as the editor. Its fade follows video frames.
function installInputOverlay() {
  document.body.insertAdjacentHTML("beforeend", `
    <style>
      #pointer, #keycap { pointer-events: none; position: fixed; z-index: 2 }
      #pointer { left: -16px; top: -27px; width: 60px; height: 84px;
        filter: drop-shadow(0 2px 2px rgb(0 0 0 / 55%)) }
      #pointer .text-cursor, #pointer.over-text .arrow { display: none }
      #pointer.over-text .text-cursor { display: inline }
      #keycap { left: 50%; bottom: 32px; transform: translateX(-50%); opacity: 0;
        min-width: 72px; padding: 6px 18px; text-align: center; color: #142826;
        background: #d6f5e5; border: 2px solid #89e7bb; border-radius: 12px;
        box-shadow: 0 5px #678e7c; font: 650 34px/1.2 system-ui, sans-serif }
      #keycap.pressed { background: #89e7bb; transform: translate(-50%, 3px); box-shadow: 0 2px #678e7c }
    </style>
    <svg id="pointer" aria-hidden="true" viewBox="-12 -24 60 84" fill="#171717" stroke="white" stroke-linejoin="round">
      <path class="arrow" stroke-width="2.6" d="M4 3 V42 L14.5 32 L23.5 53 L32 49.3 L23 29 H38 Z"/>
      <path class="text-cursor" stroke-width="1.5" d="M-4 -17 Q1 -17 4 -13 Q7 -17 12 -17 V-14
        Q5.5 -14 5.5 -9 V15 Q5.5 20 12 20 V23 Q7 23 4 19 Q1 23 -4 23 V20
        Q2.5 20 2.5 15 V-9 Q2.5 -14 -4 -14 Z"/>
    </svg>
    <kbd id="keycap" aria-hidden="true"></kbd>`);
  const pointer = document.querySelector("#pointer");
  const keycap = document.querySelector("#keycap");
  const source = document.querySelector("#source");
  document.addEventListener("pointermove", event => {
    pointer.style.transform = "translate(" + event.clientX + "px, " + event.clientY + "px)";
    pointer.classList.toggle("over-text", event.target === source);
  });
  let age = Infinity;
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

async function glide(mouse, from, control, to, progress) {
  // Ease time along a quadratic Bézier so the pointer comes to rest at the target.
  const t = progress * progress * (3 - 2 * progress);
  const u = 1 - t;
  await mouse.move(u * u * from.x + 2 * u * t * control.x + t * t * to.x,
    u * u * from.y + 2 * u * t * control.y + t * t * to.y);
}

async function captureEdits(page, work) {
  const editor = page.getByLabel("movement.luau");
  await page.waitForFunction(() => window.stepRace !== undefined);
  const original = await editor.inputValue();
  assert.ok(original.includes("return x + 1"), "unexpected starting movement rule");
  await page.evaluate(installInputOverlay);
  const digit = await editor.evaluate(digitBounds);
  const left = { x: digit.x + 1, y: digit.y };
  const right = { x: digit.x + digit.width - 1, y: digit.y };
  const start = { x: left.x + 410, y: left.y + 145 };
  const approach = { x: left.x + 65, y: start.y + 80 };
  const rest = { x: right.x + 355, y: right.y + 74 };
  const departure = { x: rest.x - 12, y: right.y + 20 };
  await page.mouse.move(start.x, start.y);
  const edits = new Map([
    [60, { key: "4", digit: "4", state: "applied", stride: 4 }],
    [140, { key: "Backspace", digit: "", state: "rejected", stride: 4 }],
    [200, { key: "2", digit: "2", state: "applied", stride: 2 }],
  ]);
  let position = 0;
  let stride = 1;
  for (let frame = 0; frame < editFrames; frame++) {
    if (frame >= 5 && frame <= 36) await glide(page.mouse, start, approach, left, (frame - 5) / 31);
    if (frame === 38) await page.mouse.down();
    if (frame >= 39 && frame <= 49) await page.mouse.move(left.x + (right.x - left.x) * (frame - 39) / 10, left.y);
    if (frame === 50) {
      await page.mouse.up();
      assert.equal(await editor.evaluate(el => el.value.slice(el.selectionStart, el.selectionEnd)), "1");
    }
    if (frame >= 64 && frame <= 87) await glide(page.mouse, right, departure, rest, (frame - 64) / 23);
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
