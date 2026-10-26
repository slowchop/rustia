#!/usr/bin/env bash
# Correctness only: actual Firefox/Safari or explicitly labelled headless WebKit; never a timing authority.
set -euo pipefail
fail() { echo "browser correctness: $*" >&2; exit 1; }
browser=${1:?expected firefox, safari or webkit}
expected_browser=${2:?expected exact browser version}
expected_driver=${3:-}
# Reject inherited alternative engines/drivers rather than silently falling back.
for var in NO_HEADLESS WASM_BINDGEN_USE_NODE_EXPERIMENTAL WASM_BINDGEN_USE_DENO WASM_BINDGEN_USE_DEDICATED_WORKER WASM_BINDGEN_USE_SHARED_WORKER WASM_BINDGEN_USE_SERVICE_WORKER WASM_BINDGEN_USE_EMSCRIPTEN WASM_BINDGEN_TEST_ONLY_NODE GECKODRIVER_REMOTE CHROMEDRIVER_REMOTE SAFARIDRIVER_REMOTE MSEDGEDRIVER_REMOTE; do
  if declare -p "$var" >/dev/null 2>&1; then echo "unexpected browser override: $var" >&2; exit 2; fi
done
unset GECKODRIVER CHROMEDRIVER SAFARIDRIVER MSEDGEDRIVER
unset GECKODRIVER_ARGS CHROMEDRIVER_ARGS SAFARIDRIVER_ARGS MSEDGEDRIVER_ARGS
unset WASM_BINDGEN_TEST_WEBDRIVER_JSON
case "$browser" in
  webkit)
    tool_dir=$(cd "$(dirname "$0")/browser" && pwd)
    [[ "$(node -p "require('$tool_dir/node_modules/playwright/package.json').version")" == "$expected_browser" ]] || fail "Playwright version does not match"
    config=
    ;;
  firefox)
    case "$(uname -s)/$(uname -m)" in
      Linux/x86_64|Darwin/arm64) ;;
      *) fail "unsupported Firefox correctness platform" ;;
    esac
    firefox_bin=$(command -v firefox || true)
    if [[ -z "$firefox_bin" && "$(uname -s)" == Darwin ]]; then
      firefox_bin=/Applications/Firefox.app/Contents/MacOS/firefox
    fi
    [[ -x "$firefox_bin" ]] || fail "Firefox is missing"
    GECKODRIVER=$(command -v geckodriver) || fail "geckodriver is missing"
    export GECKODRIVER
    [[ "$("$firefox_bin" --version)" == "Mozilla Firefox $expected_browser" ]] || fail "required platform or pinned tool version does not match"
    [[ -n "$expected_driver" ]] || fail "required platform or pinned tool version does not match"
    driver_version=$("$GECKODRIVER" --version)
    [[ "$driver_version" == "geckodriver $expected_driver" || "$driver_version" == "geckodriver $expected_driver "* || "$driver_version" == "geckodriver $expected_driver"$'\n'* ]] || fail "required platform or pinned tool version does not match"
    config=$(mktemp "${TMPDIR:-/tmp}/rustia-firefox.XXXXXX")
    # Tell geckodriver exactly which binary was version-checked.
    python3 -c 'import json,sys; print(json.dumps({"moz:firefoxOptions":{"binary":sys.argv[1]}}))' "$firefox_bin" > "$config"
    export WASM_BINDGEN_TEST_WEBDRIVER_JSON="$config"
    ;;
  safari)
    [[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || fail "required platform or pinned tool version does not match"
    [[ "$(defaults read /Applications/Safari.app/Contents/Info CFBundleShortVersionString)" == "$expected_browser" ]] || fail "required platform or pinned tool version does not match"
    export SAFARIDRIVER=/usr/bin/safaridriver
    "$SAFARIDRIVER" --version
    config=
    ;;
  *) echo "expected firefox, safari or webkit" >&2; exit 2 ;;
esac
log=$(mktemp "${TMPDIR:-/tmp}/rustia-browser-test.XXXXXX")
cleanup() {
  status=$?
  if [[ "$status" != 0 && "$browser" == safari ]]; then
    echo 'Safari run did not finish. Check the WebDriver/session error above and Allow remote automation in Safari Settings > Developer.' >&2
    echo 'This script does not change Safari permissions. A driver SIGKILL can be test-runner cleanup, not the original failure.' >&2
  fi
  rm -f "$log"
  if [[ -n "$config" ]]; then rm -f "$config"; fi
}
trap cleanup EXIT
[[ "$(wasm-bindgen --version)" == 'wasm-bindgen 0.2.126' ]] || fail "required platform or pinned tool version does not match"
[[ "$(wasm-bindgen-test-runner --version)" == 'wasm-bindgen-test-runner 0.2.126' ]] || fail "Wasm test runner version does not match"
[[ "$(rustc --version)" == 'rustc 1.95.0 '* ]] || fail "required platform or pinned tool version does not match"
printf 'Browser correctness: %s %s\n' "$browser" "$expected_browser"
git rev-parse HEAD
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
if [[ "$browser" == webkit ]]; then
  export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="node $tool_dir/webkit-runner.cjs"
fi
export WASM_BINDGEN_USE_BROWSER=1
export WASM_BINDGEN_TEST_TIMEOUT=120
export WASM_BINDGEN_TEST_DRIVER_TIMEOUT=30
# Build both modes before Safari starts taking focus for its visible windows.
# NO_HEADLESS selects wasm-bindgen's manual server, not a Safari display mode.
for mode in bare default; do
  args=(test --locked -p rustia --target wasm32-unknown-unknown --no-run)
  if [[ "$mode" == bare ]]; then args+=(--no-default-features); adapter=bare_wasm_live; else adapter=web_live; fi
  cargo "${args[@]}" --test platform_golden --test "$adapter" --test host_functions \
    --test typed_objects --test live_value_contract --test yieldable_require \
    --test callable_continuations --test numeric_index_contract
done
if [[ "$browser" == safari ]]; then
  echo 'Safari uses visible automation windows; it has no headless mode.'
  echo 'The pinned test runner labels its WebDriver mode "headless" even for Safari.'
  echo 'Leave automation enabled and avoid interacting with Safari until both modes finish.'
fi
for mode in bare default; do
  bash tools/run-private-platform-golden.sh wasm "$mode"
  args=(test --locked -p rustia --target wasm32-unknown-unknown)
  if [[ "$mode" == bare ]]; then args+=(--no-default-features); adapter=bare_wasm_live; else adapter=web_live; fi
  for suite in "$adapter" host_functions typed_objects live_value_contract yieldable_require callable_continuations numeric_index_contract; do
    cargo "${args[@]}" --test "$suite" -- --nocapture 2>&1 | tee "$log"
    if ! grep -Eq 'test result: ok[.] [1-9][0-9]* passed; 0 failed;' "$log"; then
      echo "missing nonzero successful test result for $browser/$mode/$suite" >&2
      exit 1
    fi
  done
done
printf 'Browser correctness complete: %s; bare and default suites passed.\n' "$browser"
