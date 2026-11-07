// The geometrizer, on its own core. The page posts:
//   {type: "init", pkg, module, game, rom, native}
//                                       the package URL, the shared compiled
//                                       module, the ROM URL; native: the
//                                       native geometrizer (Model 2B) where
//                                       it implements the uploaded program
//   {type: "boot", program}             the uploaded program (u32 pairs)
//   {type: "frame", buffer, booted, readPtr, jumps}
//                                       one frame's job, at vblank
// and gets back {type: "frame", stream, points} per frame job, without
// stream and points for a frame overtaken by later ones it already holds
// (the page counts the replies: it lets the game run at most a couple of
// frames ahead of this worker, docs/web.md). Jobs that arrive
// while the ROMs load are kept and run in order: the engine's state carries
// from frame to frame.
// The page names its WebAssembly package (`pkg`, a URL ending in /); the
// worker imports the same one.
let wasm = null;

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
  // Only the newest frame is drawn: don't send ones already overtaken, but
  // answer every job.
  if (pending.length === 0) postMessage({ type: "frame", stream, points }, [stream.buffer, points.buffer]);
  else postMessage({ type: "frame" });
  if (++frames % 60 === 0) {
    postMessage({ type: "stats", msPerFrame: spent / 60 });
    spent = 0;
  }
}

onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    try {
      wasm = await import(m.pkg + "remodel_web.js");
      await wasm.default({ module_or_path: m.module ?? m.pkg + "remodel_web_bg.wasm" });
      const chips = await Promise.all(
        wasm.rom_files(m.game).map(async (f) => new Uint8Array(await (await fetch(`${m.rom}/${f}`)).arrayBuffer())),
      );
      engine = new wasm.GeoEngine(m.game, chips);
      if (m.native) engine.set_native(true);
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
