import init, {Session} from './pkg/rustia_garden.js?v=garden-project';
let session, drawing;
function createSession(source, radius) {
  return drawing === undefined ? new Session(source, radius) : Session.withDrawing(source, drawing, radius);
}
let shapes = [], output = '';
self.playgroundOutput = {write(text) {
  if (output.length + text.length > 32_768) throw Error('Output exceeded 32 KiB for one frame.');
  output += text;
}};
self.gardenOutput = {
  sprite(kind, x, y, angle) {
    if (shapes.length >= 512) throw Error('A frame can draw up to 512 sprites.');
    if (![x,y,angle].every(Number.isFinite)) throw Error('A sprite needs finite coordinates.');
    shapes.push([kind, x, y, angle]);
  }
};
function tick(instance, time, x, y, radius) {
  if (radius === undefined) instance.tick(time, x, y);
  else instance.tickWithRadius(time, x, y, radius);
}
// Start each trial with the same knots. Only the pointer changes on the second tick.
function secondTip(source, direction, radius) {
  const probe = createSession(source, radius);
  try {
    shapes = [];
    tick(probe, 0, undefined, undefined, radius);
    const tip = shapes[12];
    if (!tip) return;
    shapes = [];
    tick(probe, 0, direction ? tip[1] + direction * 40 : undefined, direction ? tip[2] : undefined, radius);
    return shapes[12]?.[1];
  } finally { probe.free(); }
}
function followsPointer(source, radius) {
  const idle = secondTip(source, 0, radius);
  return secondTip(source, 1, radius) > idle && secondTip(source, -1, radius) < idle;
}
self.onmessage = async ({data}) => {
  try {
    if (data.type === 'snapshot') {
      try {
        data.port.postMessage({flowers:session.savedFlowers()});
      } catch (err) {
        data.port.postMessage({error:'In flowerbed.luau: ' + String(err)});
      } finally {
        data.port.close();
      }
      return;
    }
    shapes = [];
    output = '';
    if (data.type === 'load') {
      await init({module_or_path:new URL('./pkg/rustia_garden_bg.wasm?v=garden-project', import.meta.url)});
      drawing = data.drawing;
      let previous = new Float64Array();
      if (data.port) {
        const saved = await new Promise(resolve => { data.port.onmessage = event => resolve(event.data); });
        data.port.close();
        if (saved.error) throw Error(saved.error);
        previous = saved.flowers;
      }
      if (data.files) {
        const names = data.files.map(([name]) => name);
        const sources = data.files.map(([, text]) => text);
        session = data.garden
          ? Session.gardenProject(names, sources, previous, data.radius)
          : Session.spriteProject(names, sources, data.radius);
      } else {
        session = Session.fromEdit(data.source, drawing, data.flowerbed, previous, data.radius);
      }
    }
    if (session.animated()) tick(session, data.time, data.x, data.y, data.radius);
    const frame = shapes;
    let passed = false;
    if (data.lesson === 'follow') {
      try { passed = followsPointer(data.source, data.radius); } catch { /* Free-form edits can omit sprites. */ }
    }
    self.postMessage({shapes:frame, output, animated:session.animated(), passed:passed ? data.lesson : null});
  } catch (err) {
    self.postMessage({error: String(err), output});
  }
};
