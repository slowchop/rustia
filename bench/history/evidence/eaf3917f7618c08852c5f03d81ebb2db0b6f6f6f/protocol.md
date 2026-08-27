# C2.5b dense_tables comparison — ci-bench

First rustia measurement taken on the dedicated `ci-bench` host rather than an
ephemeral cloud VM. DigitalOcean was retired on 2026-08-26 after six consecutive
`dense_tables` qualification failures; see `design/56-performance-research.md`.

## Comparison

| | |
|---|---|
| control | `4e5eb7048a7b566d357fa9947ca6a6524718ecbd` |
| candidate | `eaf3917f7618c08852c5f03d81ebb2db0b6f6f6f` |
| difference | the C2.5b append-lane admission, and nothing else |
| workload | `dense_tables`, fixed n=1500 |
| repetitions | 201 per leg, 4 legs per side, 804 samples per side |
| order | ABBA / BAAB counterbalanced |
| checksum | `number:193302491.00000000000000000`, identical on all 8 legs |

## Result

Pooled medians 39,898.435 -> 39,184.492 ns/unit: **-1.7894%**, conservative
nominal 95% interval **-2.1250% to -1.0770%**. The interval excludes zero, so
this establishes a speedup rather than a neutral result. Every candidate leg
median (39,109-39,243) is below every control leg median (39,579-40,028).

## Host

See `host.txt`. AMD Ryzen 5 3600X, SMT off, Ubuntu 24.04.4, kernel 6.8.0-138.
Measurement pinned to CPU 4, which the kernel command line isolates
(`isolcpus=4,5 nohz_full=4,5 rcu_nocbs=4,5`).

Measured between-leg spread on this host is **0.52%**, against 15.12% on the
DigitalOcean sfo3 pool and 8.65% on the developer Mac. The `schedutil` governor
was left as found: a controlled test against `performance` measured 0.52% vs
0.59% between-leg spread, so pinning frequency was not justified and the host's
configuration was not modified.

## Protocol deviation — read before citing

This run used a **simplified** protocol relative to the accepted cloud gates. It
has counterbalanced order, fixed work, retained raw samples, checksum equality
and the same `bench/analyze-fixed-samples.js` analyzer, but it does **not** have
the paired relative-stability qualification, the per-leg idle/steal gate, or the
60s cooldowns those gates used. Those exist to detect a contended host; this
host is uncontended by construction. They should still be wired into a runner
workflow before this protocol becomes the standing one, and this directory
should not be cited as if it had passed them.
