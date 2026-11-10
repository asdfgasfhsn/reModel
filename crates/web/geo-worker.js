// The geometrizer, on its own core. The page posts:
//   {type: "init", pkg, module, game, rom, native, smooth, light}
//                                       the package URL, the shared compiled
//                                       module, the ROM URL; native: the
//                                       native geometrizer where it
//                                       implements the uploaded program;
//                                       smooth: smooth motion's in-betweens;
//                                       light: smooth shading's side channel
//   {type: "set", light}                the side channel on or off, from
//                                       the next frame
//   {type: "boot", program}             the uploaded program (u32 pairs)
//   {type: "frame", buffer, booted, readPtr, jumps, steps}
//                                       one frame's job, at vblank; steps:
//                                       in-betweens wanted (smooth motion)
// and gets back {type: "frame", stream, points, between, light,
// betweenLight} per frame job (between: with smooth motion and steps over
// 1, the frame's in-betweens at k / steps of the way from the frame
// before, GeoEngine::between; light: with the side channel on, the frame's
// lighting side channel as words, GeoEngine::light, and betweenLight each
// in-between's, GeoEngine::between_light; all transferred), without
// stream and points for a frame overtaken by later ones it already holds
// (the page counts the replies: it lets the game run at most a couple of
// frames ahead of this worker, docs/web.md). Jobs that arrive
// while the ROMs load are kept and run in order: the engine's state carries
// from frame to frame.
// The page names its WebAssembly package (`pkg`, a URL ending in /); the
// worker imports the same one.
let wasm = null;

let engine = null;
let light = false;
const pending = [];
let spent = 0;
let frames = 0;

function run(m) {
  if (m.type === "boot") {
    engine.boot(m.program);
    return;
  }
  if (m.type === "set") {
    light = !!m.light;
    engine.set_light(light);
    return;
  }
  const t = performance.now();
  engine.frame(m.buffer, m.booted, m.readPtr, m.jumps);
  const stream = engine.stream();
  const points = engine.points();
  spent += performance.now() - t;
  // Only the newest frame is drawn: don't send ones already overtaken, but
  // answer every job.
  if (!pending.some((p) => p.type === "frame")) {
    const t1 = performance.now();
    const between = m.steps > 1 ? engine.between(m.steps) : undefined;
    const lit = light ? engine.light() : undefined;
    const betweenLight = light && between ? engine.between_light() : undefined;
    spent += performance.now() - t1;
    const buffers = [stream.buffer, points.buffer, ...[...(between ?? []), ...(betweenLight ?? [])].map((b) => b.buffer)];
    if (lit) buffers.push(lit.buffer);
    postMessage({ type: "frame", stream, points, between, light: lit, betweenLight }, buffers);
  } else postMessage({ type: "frame" });
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
      if (m.smooth) engine.set_smooth(true);
      light = !!m.light;
      if (light) engine.set_light(true);
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
