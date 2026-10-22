//! Deterministic string index policy; see design/95-numeric-index-contract.md.
use rustia::{Profile, RustiaBuilder};

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn substring_indices_are_checked_and_truncate_toward_zero() {
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard()
            .profile(profile)
            .fuel(100_000)
            .memory_limit_bytes(8 * 1024 * 1024)
            .build()
            .unwrap();
        for after_reset in [false, true] {
            if after_reset {
                vm.reset_decision().unwrap();
            }
            for callee in ["string.sub", "alias"] {
                for index in [
                    "0/0",
                    "math.huge",
                    "-math.huge",
                    "9223372036854775808",
                    "-9223372036854777856",
                ] {
                    for args in [
                        format!("'abcdef', {index}"),
                        format!("'abcdef', 1, {index}"),
                    ] {
                        let source = format!(
                            "local alias=string.sub; local ok,err=pcall(function() return {callee}({args}) end); return tostring(ok)..':'..tostring(err)"
                        );
                        let actual: String = vm
                            .load(&source)
                            .source_name("numeric_index_contract.luau")
                            .eval()
                            .unwrap();
                        assert_eq!(actual,
                            "false:numeric_index_contract.luau:1: type error: expected integer string library index",
                            "{profile:?} {callee}({args}) after_reset={after_reset}");
                        assert_eq!(vm.eval::<i32>("return 6*7").unwrap(), 42);
                    }
                }
                for (args, expected) in [
                    ("'abcdef', 2.9", "bcdef"),
                    ("'abcdef', -2.9", "ef"),
                    ("'abcdef', 1, 3.9", "abc"),
                    ("'abcdef', 1, -2.9", "abcde"),
                    ("'abcdef', -0.9", "abcdef"),
                    ("'abcdef', 1, -0.9", ""),
                    ("'abcdef', '2.9', '4.9'", "bcd"),
                    ("'abcdef', 2.9, nil", "bcdef"),
                    ("12, 1.9", "12"),
                    ("'abcdef', 9223372036854774784", ""),
                    ("'abcdef', 1, 9223372036854774784", "abcdef"),
                    ("'abcdef', -9223372036854775808", "abcdef"),
                    ("'abcdef', 1, -9223372036854775808", ""),
                ] {
                    let source = format!("local alias=string.sub; return {callee}({args})");
                    let actual: String = vm.load(&source).eval().unwrap();
                    assert_eq!(
                        actual, expected,
                        "{profile:?} {callee}({args}) after_reset={after_reset}"
                    );
                }
            }
        }
    }
}
