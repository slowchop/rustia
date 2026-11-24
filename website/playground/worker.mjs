import init, {ConsoleProgram, ConsoleRepl} from '../pkg/rustia_garden.js?v=console';

let program, repl;
let output = '';
self.playgroundOutput = {write(text) {
  if (output.length + text.length > 32_768) throw Error('Output exceeded 32 KiB.');
  output += text;
}};

self.onmessage = async ({data}) => {
  output = '';
  try {
    if (data.type === 'command') {
      if (!repl) {
        await init({module_or_path:new URL('../pkg/rustia_garden_bg.wasm?v=console', import.meta.url)});
        repl = new ConsoleRepl(data.files.map(([name]) => name), data.files.map(([, text]) => text));
      }
      repl.evaluate(data.source);
      self.postMessage({output});
      return;
    }
    if (data.type === 'start') {
      await init({module_or_path:new URL('../pkg/rustia_garden_bg.wasm?v=console', import.meta.url)});
      const [seed] = crypto.getRandomValues(new Uint32Array(1));
      program = new ConsoleProgram(data.files.map(([name]) => name), data.files.map(([, text]) => text), seed);
    }
    const waiting = program.advance(data.line);
    self.postMessage({output, waiting});
  } catch (error) {
    self.postMessage({output, error:String(error)});
  }
};
