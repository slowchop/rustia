# script-bench-harness Results

| engine | workload | ns/unit min | ns/unit median | speed vs rustia | units/sec | n | checksum |
|---|---|---:|---:|---:|---:|---:|---|
| rustia | dense_tables | 123198.74 | 131966.67 | 1.00x | 8117 | 2093 | `number:269739486.00000000000000000` |
| rustia | sparse_tables | 100517.09 | 106785.63 | 1.00x | 9949 | 2535 | `number:6691183200.00000000000000000` |
| rustia | closures_upvalues | 381.52 | 384.15 | 1.00x | 2621093 | 642825 | `number:116223382734.00000000000000000` |
| rustia | generic_iteration | 15711.84 | 15926.44 | 1.00x | 63646 | 15712 | `number:359726240.00000000000000000` |
| rustia | stdlib_math_table | 1108.61 | 1205.68 | 1.00x | 902028 | 236614 | `number:2852189.57414873503148556` |
| koto | dense_tables | 95478.00 | 95947.30 | 1.29x | 10474 | 2618 | `i64:337344558` |
| koto | sparse_tables | 53284.77 | 53931.45 | 1.89x | 18767 | 4654 | `i64:22530944800` |
| koto | closures_upvalues | 118.61 | 119.48 | 3.22x | 8430769 | 2088619 | `i64:18797556` |
| koto | generic_iteration | 2259.97 | 2275.63 | 6.95x | 442484 | 109357 | `i64:2503728515` |
| koto | stdlib_math_table | 587.65 | 590.91 | 1.89x | 1701703 | 420691 | `f64:6201069.48319827299565077` |
| mlua-luau | dense_tables | 9338.79 | 9449.35 | 13.19x | 107080 | 26340 | `i64:3394474020` |
| mlua-luau | sparse_tables | 8770.84 | 8856.70 | 11.46x | 114014 | 28282 | `i64:831954624800` |
| mlua-luau | closures_upvalues | 20.70 | 20.93 | 18.43x | 48318193 | 12019066 | `i64:40628872857009` |
| mlua-luau | generic_iteration | 1960.10 | 1984.70 | 8.02x | 510179 | 126792 | `i64:2902902840` |
| mlua-luau | stdlib_math_table | 84.66 | 86.26 | 13.09x | 11811344 | 2908323 | `f64:105452487.88045325875282288` |
