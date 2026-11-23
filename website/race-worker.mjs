import init, {run_race} from './pkg/rustia_garden.js';

self.onmessage = async ({data: {source, rules, checkSeed}}) => {
  let output = '';
  self.raceOutput = {
    write(text) {
      if (output.length + text.length > 32_768) throw Error('Console output exceeded 32 KiB.');
      output += text;
    }
  };
  try {
    await init();
    const [initialSeed] = crypto.getRandomValues(new Uint32Array(1));
    run_race(source, rules, initialSeed);
    const firstRace = output;
    let repeatable = false;
    if (checkSeed && /^Seed -?\d+/m.test(firstRace) && firstRace.includes('\f')) {
      // Change the host entropy, then see whether the script reproduces its seed and every frame.
      try {
        repeatable = [0, 7, 12345, ~initialSeed >>> 0].every(entropy => {
          output = '';
          run_race(source, rules, entropy);
          return output === firstRace;
        });
      } catch { /* A failed second run does not establish repeatability. */ }
    }
    self.postMessage({output:firstRace, repeatable});
  } catch (error) {
    self.postMessage({output, error: String(error)});
  }
};
