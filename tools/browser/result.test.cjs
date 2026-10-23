"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { requirePassed } = require("./result.cjs");
test("accept an executed passing suite", () => requirePassed("test result: ok. 3 passed; 0 failed; 0 ignored;"));
for (const [name, text] of Object.entries({
  missing: "Loading scripts...",
  zero: "test result: ok. 0 passed; 0 failed;",
  failed: "test result: FAILED. 1 passed; 1 failed;",
  contradictory: "test result: ok. 1 passed; 2 failed;",
  duplicate: "test result: ok. 1 passed; 0 failed;\ntest result: ok. 1 passed; 0 failed;",
})) test(`reject ${name}`, () => assert.throws(() => requirePassed(text)));
