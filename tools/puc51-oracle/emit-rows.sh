#!/bin/sh
# Emit the MANIFEST.tsv and REFS.tsv rows for the checked-in puc51-compat
# fixtures, with expectations captured from the pinned PUC 5.1.5 oracle.
# Usage: tools/puc51-oracle/emit-rows.sh <manifest-out> <refs-out>
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST_OUT="$1"
REFS_OUT="$2"
EXPECT="$("$ROOT/tools/puc51-oracle/capture-expectations.sh")"

row() {
    # row <file> <id> <category> <ref> <summary>
    file="$1"
    id="$2"
    category="$3"
    ref="$4"
    summary="$5"
    expect="$(printf '%s\n' "$EXPECT" | grep "^$file	" | cut -f2-)"
    path="conformance/fixtures/puc51-compat/$file"
    printf '%s\t%s\t%s\tPuc51Compat\tpuc-lua-5.1.5\t%s\t%s\n' \
        "$id" "$category" "$ref" "$expect" "$path" >>"$MANIFEST_OUT"
    printf '%s\t%s\t%s\tpuc-lua-5.1.5\t%s\n' \
        "$ref" "$category" "$path" "$summary" >>"$REFS_OUT"
}

: >"$MANIFEST_OUT"
: >"$REFS_OUT"

row table_iter_mixed_scalars.luau puc51.table.iter_mixed_scalars runtime puc51.table.iter-mixed-scalars \
    "PUC 5.1.5 next() enumeration order and border under mixed scalar keys with insert/delete churn."
row table_iter_delete_churn.luau puc51.table.iter_delete_churn runtime puc51.table.iter-delete-churn \
    "PUC 5.1.5 next() order after mass-deletion tombstones and second-wave inserts across rehash boundaries."
row table_border_rehash.luau puc51.table.border_rehash runtime puc51.table.border-rehash \
    "PUC 5.1.5 length-border evolution while sequential inserts cross array-part rehash boundaries."
row table_iter_sparse_jumps.luau puc51.table.iter_sparse_jumps runtime puc51.table.iter-sparse-jumps \
    "PUC 5.1.5 next() order and border for jumping-k sparse integer writes with hole filling."
row table_delete_during_next.luau puc51.table.delete_during_next runtime puc51.table.delete-during-next \
    "PUC 5.1.5 deletion of the current key during next() traversal plus tombstone resurrection."
row table_iter_string_heavy.luau puc51.table.iter_string_heavy runtime puc51.table.iter-string-heavy \
    "PUC 5.1.5 next() order for string-key-dominated tables driven through hash-part rehashes."
row table_iter_bool_keys.luau puc51.table.iter_bool_keys runtime puc51.table.iter-bool-keys \
    "PUC 5.1.5 boolean keys alongside distinct number/string keys, with tombstone and resurrection."
row table_ctor_presizing.luau puc51.table.ctor_presizing runtime puc51.table.ctor-presizing \
    "PUC 5.1.5 table constructors: positional runs, int2fb round-up, multi-flush SETLIST, keyed fields, call/vararg tails."
row table_iter_frac_negative_keys.luau puc51.table.iter_frac_negative_keys runtime puc51.table.iter-frac-negative-keys \
    "PUC 5.1.5 next() order for negative and fractional number keys living in the hash part."
row table_next_walk.luau puc51.table.next_walk runtime puc51.table.next-walk \
    "PUC 5.1.5 next() walks from explicit middle keys, the invalid-key error, and empty-table nil."
row table_insert_remove.luau puc51.table.insert_remove stdlib puc51.table.insert-remove \
    "PUC 5.1.5 table.insert/table.remove sequences resolved through the luaH_getn border."
row format_tostring_edges.luau puc51.format.tostring_edges stdlib puc51.format.tostring-edges \
    "PUC 5.1.5 %.14g formatting edges through tostring, concatenation, table.concat, and string.format."
row format_tostring_prng.luau puc51.format.tostring_prng stdlib puc51.format.tostring-prng \
    "PUC 5.1.5 %.14g formatting of deterministic PRNG-generated doubles."
row sort_plain_duplicates.luau puc51.sort.plain_duplicates stdlib puc51.sort.plain-duplicates \
    "PUC 5.1.5 auxsort default-order output over arrays with engineered duplicate keys."
row sort_comparator_ties.luau puc51.sort.comparator_ties stdlib puc51.sort.comparator-ties \
    "PUC 5.1.5 auxsort ascending-comparator tie order pinned by record ids."
row sort_reverse_comparator.luau puc51.sort.reverse_comparator stdlib puc51.sort.reverse-comparator \
    "PUC 5.1.5 auxsort descending-comparator tie order pinned by record ids."
row sort_shapes.luau puc51.sort.shapes stdlib puc51.sort.shapes \
    "PUC 5.1.5 auxsort over engineered array shapes: ascending, descending, organ-pipe, all-equal, sawtooth."
