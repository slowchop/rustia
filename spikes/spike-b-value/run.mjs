// spike-B wasm runner (node/deno, both V8). Times each heap microbench + the IAUS kernel for
// NaN-box(8B) vs tagged(16B) vs fat(24B) inside wasm32. ns/op = elapsed / iters; us/decision =
// elapsed / (n*ticks). best-of-RUNS. Honest: wasm fn does the churned/black_box work, JS only times.
import { readFileSync } from "node:fs";

const WASM = new URL("./target/wasm32-unknown-unknown/release/spike_b_value.wasm", import.meta.url);
const { instance } = await WebAssembly.instantiate(readFileSync(WASM), {});
const ex = instance.exports;
const now = () => performance.now();

const rt = typeof Deno !== "undefined"
  ? "deno " + Deno.version.deno + " (v8 " + Deno.version.v8 + ")"
  : "node " + process.version;

function ns_per_op(fn, iters, runs = 7) {
  for (let w = 0; w < 3; w++) fn(Math.floor(iters / 10) + 1); // warm
  let best = Infinity, chk = 0n;
  for (let r = 0; r < runs; r++) {
    const t0 = now();
    chk ^= BigInt.asUintN(64, fn(iters));
    const t1 = now();
    const per = ((t1 - t0) * 1e6) / iters; // ms -> ns
    if (per < best) best = per;
  }
  return { ns: Number(best.toFixed(4)), chk };
}

function kernel_us(fn, n, ticks, runs = 7) {
  ex.bench_setup(n);
  for (let w = 0; w < 3; w++) fn(Math.max(1, ticks / 10));
  let best = Infinity, chk = 0n;
  for (let r = 0; r < runs; r++) {
    ex.bench_setup(n);
    fn(Math.max(1, Math.floor(ticks / 10)));
    const t0 = now();
    chk = BigInt.asUintN(64, fn(ticks));
    const t1 = now();
    const us = ((t1 - t0) * 1000) / (n * ticks);
    if (us < best) best = us;
  }
  return { us: Number(best.toFixed(4)), chk: "0x" + chk.toString(16) };
}

const IT_TBL = 20_000_000, IT_STR = 20_000_000, IT_MV = 100_000_000, IT_BULK = 1_000_000;

const out = { runtime: rt, sizeof: {}, soundness: {}, golden: {}, microbench_ns: {}, kernel_us: {} };

out.sizeof = {
  nanbox_8B: Number(ex.sizeof_nb()),
  tagged_16B: Number(ex.sizeof_tag()),
  fat_24B: Number(ex.sizeof_e24()),
};

// soundness: packed (tested<<40)|(before<<20)|after
{
  const p = BigInt.asUintN(64, ex.b_soundness());
  out.soundness = {
    tested: Number(p >> 40n),
    alias_before_canon: Number((p >> 20n) & 0xfffffn),
    alias_after_canon: Number(p & 0xfffffn),
  };
}

// golden
{
  const g = 0x31c6ec2734ea6fa2n;
  const nb = BigInt.asUintN(64, ex.bench_q16_golden_nb());
  const tg = BigInt.asUintN(64, ex.bench_q16_golden_tag());
  out.golden = {
    expected: "0x" + g.toString(16),
    nanbox: "0x" + nb.toString(16), nanbox_match: nb === g,
    tagged: "0x" + tg.toString(16), tagged_match: tg === g,
  };
}

const mb = out.microbench_ns;
mb["table get"]            = { nb: ns_per_op(ex.b_table_get_nb, IT_TBL).ns,   tag: ns_per_op(ex.b_table_get_tag, IT_TBL).ns,   e24: ns_per_op(ex.b_table_get_e24, IT_TBL).ns };
mb["table set"]            = { nb: ns_per_op(ex.b_table_set_nb, IT_TBL).ns,   tag: ns_per_op(ex.b_table_set_tag, IT_TBL).ns,   e24: ns_per_op(ex.b_table_set_e24, IT_TBL).ns };
mb["table insert/grow"]    = { nb: ns_per_op(ex.b_table_insert_nb, IT_TBL).ns, tag: ns_per_op(ex.b_table_insert_tag, IT_TBL).ns };
mb["string intern (hit)"]  = { nb: ns_per_op(ex.b_string_intern_nb, IT_STR).ns, tag: ns_per_op(ex.b_string_intern_tag, IT_STR).ns };
mb["string eq (interned)"] = { nb: ns_per_op(ex.b_string_eq_nb, IT_STR).ns, tag: ns_per_op(ex.b_string_eq_tag, IT_STR).ns };
mb["value move (reg)"]     = { nb: ns_per_op(ex.b_clone_move_nb, IT_MV).ns,  tag: ns_per_op(ex.b_clone_move_tag, IT_MV).ns,  e24: ns_per_op(ex.b_clone_move_e24, IT_MV).ns };
mb["bulk clone [256]"]     = { nb: ns_per_op(ex.b_bulk_clone_nb, IT_BULK).ns, tag: ns_per_op(ex.b_bulk_clone_tag, IT_BULK).ns, e24: ns_per_op(ex.b_bulk_clone_e24, IT_BULK).ns };
mb["value move (Arc 24B)"] = { arc: ns_per_op(ex.b_clone_arc, IT_MV).ns };

// kernel
const KN = 4000, KT = 200;
out.kernel_us = {
  nanbox_canon:    kernel_us(ex.kernel_nb_canon, KN, KT),
  nanbox_nocanon:  kernel_us(ex.kernel_nb_nocanon, KN, KT),
  tagged_canon:    kernel_us(ex.kernel_tag_canon, KN, KT),
  tagged_nocanon:  kernel_us(ex.kernel_tag_nocanon, KN, KT),
  fat_canon:       kernel_us(ex.kernel_e24_canon, KN, KT),
};

console.log(JSON.stringify(out, null, 2));
