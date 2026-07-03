# script-bench-harness Results

| engine | workload | ns/unit min | ns/unit median | speed vs rustia | units/sec | n | checksum |
|---|---|---:|---:|---:|---:|---:|---|
| rustia | dense_tables | 125464.15 | 133403.90 | 1.00x | 7970 | 2083 | `number:268449376.00000000000000000` |
| rustia | sparse_tables | 103165.12 | 105889.33 | 1.00x | 9693 | 2510 | `number:6559935200.00000000000000000` |
| rustia | closures_upvalues | 383.49 | 389.48 | 1.00x | 2607615 | 638890 | `number:114804739587.00000000000000000` |
| rustia | generic_iteration | 15804.47 | 16072.10 | 1.00x | 63273 | 15603 | `number:357230685.00000000000000000` |
| rustia | stdlib_math_table | 1125.11 | 1218.01 | 1.00x | 888804 | 233163 | `number:2798701.03174196090549231` |
| koto | dense_tables | 96682.44 | 97623.19 | 1.30x | 10343 | 2551 | `i64:328721501` |
| koto | sparse_tables | 53983.09 | 54655.25 | 1.91x | 18524 | 4553 | `i64:21563736480` |
| koto | closures_upvalues | 118.72 | 119.46 | 3.23x | 8423344 | 2074181 | `i64:18667614` |
| koto | generic_iteration | 2298.18 | 2306.29 | 6.88x | 435127 | 108318 | `i64:2479940610` |
| koto | stdlib_math_table | 591.07 | 596.50 | 1.90x | 1691841 | 409879 | `f64:5979709.58692069258540869` |
