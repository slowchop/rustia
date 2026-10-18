#!/usr/bin/env bash
# Execute the shared correctness test and reject a silent zero-test pass.
set -euo pipefail

args=(test --locked -p rustia --test platform_golden)
case "${1:-}" in
  native)
    case "${3:-}" in
      aarch64-apple-darwin|x86_64-unknown-linux-gnu) args+=(--target "$3") ;;
      *) echo "native execution requires an explicit supported target triple" >&2; exit 2 ;;
    esac
    ;;
  wasm) args+=(--target wasm32-unknown-unknown) ;;
  *) echo "usage: run-private-platform-golden.sh native|wasm default|bare [native-target]" >&2; exit 2 ;;
esac
case "${2:-}" in
  default) ;;
  bare) args+=(--no-default-features) ;;
  *) echo "expected default or bare feature mode" >&2; exit 2 ;;
esac
log=$(mktemp "${TMPDIR:-/tmp}/rustia-private-golden.XXXXXX")
trap 'rm -f "$log"' EXIT
cargo "${args[@]}" -- --nocapture 2>&1 | tee "$log"
count=$(grep -c '^RUSTIA_PRIVATE_PLATFORM_GOLDEN ' "$log" || true)
if [[ "$count" != 1 ]]; then
  echo "expected exactly one executed private golden transcript, observed $count" >&2
  exit 1
fi
