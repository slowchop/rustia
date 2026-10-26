"use strict";
exports.requirePassed = output => {
  const results = [...output.matchAll(/test result: (\w+)\. (\d+) passed; (\d+) failed;/g)];
  if (results.length !== 1 || results[0][1] !== "ok" || Number(results[0][2]) < 1 || Number(results[0][3]) !== 0) {
    throw new Error("Expected exactly one nonzero successful Wasm test result");
  }
};
