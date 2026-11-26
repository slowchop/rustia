"use strict";

const previewVersion = "0.2.0";
const previewRegistry = "crates-io";
const previewPackages = Object.freeze([
  "rustia-lexer",
  "rustia-parser",
  "rustia-vm",
  "rustia-compiler",
  "rustia",
  "rustia-cli",
]);
const previewRegistries = Object.freeze([previewRegistry]);

function expectedPublication(name, version) {
  if (version === previewVersion && previewPackages.includes(name)) {
    return previewRegistries;
  }
  return false;
}

module.exports = { expectedPublication, previewPackages, previewRegistry, previewVersion };
