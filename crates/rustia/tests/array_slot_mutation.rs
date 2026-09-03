//! End-to-end guards for the VM Core 2 C2.5a direct existing dense-array slot
//! write.
//!
//! The heap-level unit tests in `rustia-vm` pin the lane selection and the
//! accounting invariants; these run the same shapes through real Luau source
//! in both table layouts, so the guest-visible `__newindex`, readonly, length,
//! and iteration semantics are checked against the public API rather than
//! against internals.

use rustia::{Rustia, RustiaBuilder};

fn presets() -> Vec<(&'static str, Rustia)> {
    vec![
        ("standard", RustiaBuilder::standard().build().unwrap()),
        (
            "puc51_compat",
            RustiaBuilder::puc51_compat().build().unwrap(),
        ),
    ]
}

#[test]
fn existing_array_slots_are_written_without_consulting_newindex() {
    for (preset, mut rustia) in presets() {
        // The key is present, so Luau writes the slot directly and never
        // reaches __newindex.
        let report: String = rustia
            .eval(
                "local calls = 0 \
                 local t = setmetatable({10, 20, 30}, { \
                     __newindex = function(tbl, key, value) \
                         calls = calls + 1 \
                         rawset(tbl, key, value) \
                     end, \
                 }) \
                 t[2] = 99 \
                 t[1] = 11 \
                 t[3] = 33 \
                 return table.concat({t[1], t[2], t[3], calls, #t}, ',')",
            )
            .unwrap();
        assert_eq!(report, "11,99,33,0,3", "{preset}");
    }
}

#[test]
fn missing_and_nil_array_slots_still_defer_to_newindex() {
    for (preset, mut rustia) in presets() {
        // Slot 2 is present-but-nil and slot 9 is absent: both are missing
        // keys, so both must reach __newindex.
        let report: String = rustia
            .eval(
                "local seen = {} \
                 local t = setmetatable({10, nil, 30}, { \
                     __newindex = function(tbl, key, value) \
                         seen[#seen + 1] = key \
                         rawset(tbl, key, value) \
                     end, \
                 }) \
                 t[2] = 20 \
                 t[9] = 90 \
                 t[1] = 11 \
                 return table.concat({table.concat(seen, '/'), t[1], t[2], t[9]}, ',')",
            )
            .unwrap();
        assert_eq!(report, "2/9,11,20,90", "{preset}");
    }
}

#[test]
fn an_existing_array_slot_can_be_replaced_with_nil_in_place() {
    for (preset, mut rustia) in presets() {
        // Clearing a present key is still a present-key write, so __newindex
        // stays out of it and the surrounding slots keep their values.
        let report: String = rustia
            .eval(
                "local calls = 0 \
                 local t = setmetatable({10, 20, 30, 40}, { \
                     __newindex = function() calls = calls + 1 end, \
                 }) \
                 t[2] = nil \
                 local present = rawget(t, 2) == nil and rawget(t, 3) == 30 \
                 t[2] = 22 \
                 return table.concat( \
                     {tostring(present), calls, t[1], tostring(t[2]), t[3], t[4]}, ',')",
            )
            .unwrap();
        // Refilling the cleared slot is a missing-key write, so exactly one
        // __newindex call is expected.
        assert_eq!(report, "true,1,10,nil,30,40", "{preset}");
    }
}

#[test]
fn readonly_tables_reject_existing_array_slot_writes() {
    for (preset, mut rustia) in presets() {
        let message: String = rustia
            .eval(
                "local t = table.freeze({10, 20, 30}) \
                 local ok, err = pcall(function() t[2] = 99 end) \
                 return tostring(ok) .. '|' .. tostring(err) .. '|' .. tostring(t[2])",
            )
            .unwrap();
        assert!(
            message.starts_with("false|"),
            "{preset}: a frozen table must reject the write, got {message}"
        );
        assert!(
            message.contains("attempt to modify a readonly table"),
            "{preset}: expected the Luau readonly diagnostic, got {message}"
        );
        assert!(
            message.ends_with("|20"),
            "{preset}: the rejected write must not mutate the slot, got {message}"
        );
    }
}

#[test]
fn in_place_slot_writes_preserve_length_iteration_and_sort_visibility() {
    for (preset, mut rustia) in presets() {
        let report: String = rustia
            .eval(
                "local t = {5, 4, 3, 2, 1} \
                 t.tag = 'keep' \
                 for i = 1, #t do t[i] = t[i] * 10 end \
                 local order = {} \
                 for _, value in ipairs(t) do order[#order + 1] = value end \
                 table.sort(t) \
                 local sorted = {} \
                 for _, value in ipairs(t) do sorted[#sorted + 1] = value end \
                 return table.concat({ \
                     #t, \
                     table.concat(order, '/'), \
                     table.concat(sorted, '/'), \
                     tostring(t.tag), \
                     tostring(rawget(t, 6)), \
                 }, ',')",
            )
            .unwrap();
        assert_eq!(
            report, "5,50/40/30/20/10,10/20/30/40/50,keep,nil",
            "{preset}"
        );
    }
}

#[test]
fn repeated_in_place_writes_keep_the_table_sort_mutation_guard_armed() {
    for (preset, mut rustia) in presets() {
        // `table.sort` snapshots the array and rejects comparator-driven
        // mutation by watching the table's mutation version. The direct slot
        // lane must keep bumping that version.
        let message: String = rustia
            .eval(
                "local t = {3, 1, 2} \
                 local ok, err = pcall(table.sort, t, function(a, b) \
                     t[1] = 42 \
                     return a < b \
                 end) \
                 return tostring(ok) .. '|' .. tostring(err)",
            )
            .unwrap();
        assert!(
            message.starts_with("false|"),
            "{preset}: mutating inside the comparator must be detected, got {message}"
        );
    }
}
