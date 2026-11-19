#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { webkit } = require("playwright");

const root = path.resolve(__dirname, "../..");
const demo = path.join(root, "demos/web-live");
const seeds = [7, 11];
const fps = 20;
const framesPerHeat = 150;

function run(command, args) {
  execFileSync(command, args, { cwd: root, stdio: "inherit" });
}

function winners(trace) {
  assert.ok(trace.length % 3 === 0 && trace.length >= 6, "expected three distances per tick");
  assert.deepEqual(trace.slice(0, 3), [0, 0, 0]);
  assert.deepEqual(trace.slice(-3), [100, 100, 100]);
  for (let i = 3; i < trace.length; i++) {
    assert.ok(Number.isInteger(trace[i]) && trace[i] >= trace[i - 3] && trace[i] <= 100,
      "invalid distance at sample " + i);
  }
  const firstFinish = Math.floor(trace.indexOf(100) / 3) * 3;
  return [0, 1, 2].filter(i => trace[firstFinish + i] === 100);
}

async function capture(target, work) {
  const manifest = ["--locked", "--manifest-path", path.join(demo, "Cargo.toml"), "--target-dir", target];
  run("cargo", ["build", ...manifest, "--example", "race-native"]);
  run("cargo", ["build", ...manifest, "--lib", "--release", "--target", "wasm32-unknown-unknown"]);
  run("wasm-bindgen", ["--target", "web", "--out-dir", work, "--out-name", "race",
    path.join(target, "wasm32-unknown-unknown/release/rustia_web_live_example.wasm")]);

  const native = seeds.map(seed => execFileSync(path.join(target, "debug/examples/race-native"),
    [String(seed)], { encoding: "utf8" }).trim().split(/\s+/).map(Number));

  const browser = await webkit.launch({ headless: true });
  try {
    const page = await browser.newPage();
    // Serve exactly these three files inside the browser. No port or network access needed.
    const origin = "http://race.localhost";
    const files = new Map([
      [origin + "/", [path.join(__dirname, "race-frame.html"), "text/html"]],
      [origin + "/race.js", [path.join(work, "race.js"), "application/javascript"]],
      [origin + "/race_bg.wasm", [path.join(work, "race_bg.wasm"), "application/wasm"]],
    ]);
    await page.route("**/*", route => {
      const file = files.get(route.request().url());
      return file ? route.fulfill({ path: file[0], contentType: file[1] }) : route.abort();
    });
    await page.goto(origin);
    const wasm = await page.evaluate(async seeds => {
      const { default: init, race_frames } = await import("./race.js");
      await init();
      return seeds.map(seed => Array.from(race_frames(seed)));
    }, seeds);
    const heats = seeds.map((seed, index) => {
      assert.deepEqual(wasm[index], native[index], "Native and WebKit/Wasm disagree for seed " + seed);
      const result = { seed, native: native[index], wasm: wasm[index], winners: winners(native[index]) };
      console.log("Seed %d: %d matching ticks", seed, native[index].length / 3 - 1);
      return result;
    });
    assert.notDeepEqual(native[0], native[1], "Changing the seed must change the race");
    await page.evaluate(heats => { window.heats = heats; }, heats);
    for (let frame = 0; frame < framesPerHeat * heats.length; frame++) {
      const png = await page.evaluate(([frame, count]) => {
        window.drawFrame(window.heats[Math.floor(frame / count)], frame % count, count);
        return document.querySelector("canvas").toDataURL("image/png").split(",")[1];
      }, [frame, framesPerHeat]);
      fs.writeFileSync(path.join(work, String(frame).padStart(3, "0") + ".png"), Buffer.from(png, "base64"));
    }
  } finally {
    await browser.close();
  }
}

async function main() {
  const output = path.resolve(process.argv[2] || "target/demo/rustia-replay.mp4");
  const target = path.resolve(process.env.CARGO_TARGET_DIR || path.join(demo, "target"));
  fs.mkdirSync(target, { recursive: true });
  const work = fs.mkdtempSync(path.join(target, "record-"));
  try {
    await capture(target, work);
    fs.mkdirSync(path.dirname(output), { recursive: true });
    const staging = fs.mkdtempSync(path.join(path.dirname(output), ".rustia-race-"));
    try {
      const video = path.join(staging, "race.mp4");
      run("ffmpeg", ["-hide_banner", "-loglevel", "error", "-nostdin", "-framerate", String(fps),
        "-i", path.join(work, "%03d.png"), "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p",
        "-movflags", "+faststart", video]);
      const probe = JSON.parse(execFileSync("ffprobe", ["-v", "error", "-count_frames", "-select_streams", "v:0",
        "-show_entries", "stream=codec_name,width,height,nb_read_frames,avg_frame_rate", "-of", "json", video]));
      const [stream] = probe.streams;
      assert.equal(stream.codec_name, "h264");
      assert.deepEqual([stream.width, stream.height], [960, 448]);
      assert.equal(Number(stream.nb_read_frames), framesPerHeat * seeds.length);
      assert.equal(stream.avg_frame_rate, fps + "/1");
      fs.renameSync(video, output);
      console.log("Saved " + output);
    } finally {
      fs.rmSync(staging, { recursive: true, force: true });
    }
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
