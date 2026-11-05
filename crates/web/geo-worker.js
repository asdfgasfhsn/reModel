// The geometrizer, on its own core. The page posts:
//   {type: "init", module, game, rom}       the shared compiled module, ROM URL
//   {type: "boot", program}             the uploaded program (u32 pairs)
//   {type: "frame", buffer, booted, readPtr, jumps}
//                                       one frame's job, at vblank
// and gets back {type: "frame", stream, points} per frame. Jobs that arrive
// while the ROMs load are kept and run in order: the engine's state carries
// from frame to frame.
import init, { GeoEngine, rom_files } from "./pkg/remodel_web.js";

let engine = null;
const pending = [];
let spent = 0;
let frames = 0;

function run(m) {
  if (m.type === "boot") {
    engine.boot(m.program);
    return;
  }
  const t = performance.now();
  engine.frame(m.buffer, m.booted, m.readPtr, m.jumps);
  const stream = engine.stream();
  const points = engine.points();
  spent += performance.now() - t;
  // Only the newest frame is drawn: skip posting ones already overtaken.
  if (pending.length === 0) postMessage({ type: "frame", stream, points }, [stream.buffer, points.buffer]);
  if (++frames % 60 === 0) {
    postMessage({ type: "stats", msPerFrame: spent / 60 });
    spent = 0;
  }
}

onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    try {
      await init({ module_or_path: m.module ?? "./pkg/remodel_web_bg.wasm" });
      const chips = await Promise.all(
        rom_files(m.game).map(async (f) => new Uint8Array(await (await fetch(`${m.rom}/${f}`)).arrayBuffer())),
      );
      engine = new GeoEngine(m.game, chips);
      postMessage({ type: "ready" });
      while (pending.length) run(pending.shift());
    } catch (err) {
      postMessage({ type: "error", message: String(err.message ?? err) });
    }
  } else if (engine && pending.length === 0) {
    run(m);
  } else {
    pending.push(m);
  }
};
