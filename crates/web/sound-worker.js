// The sound and music boards, on their own core. The page posts:
//   {type: "init", pkg, module, game, rom, mix}
//                                 the WebAssembly package's URL, the
//                                 compiled module (shared with the page),
//                                 the ROM directory URL and
//                                 optionally [sound board dB, music dB]
//   {type: "link", cycles, bytes, until, ratio}
//                                 each frame's sound link bytes, and the
//                                 resampling ratio (the game's speed)
//   {type: "port", port}          a MessagePort into the AudioWorklet
// Link messages that arrive while the ROMs load are kept and replayed, so the
// boards always follow the game from boot. Audio is dropped until there's a
// port (browsers start audio only after a tap or key press).
// The page names its WebAssembly package (`pkg`, a URL ending in /); the
// worker imports the same one.
let wasm = null;

let engine = null;
let out = null;
const pending = [];
let spent = 0;
let fed = 0;

function feed(m) {
  const t = performance.now();
  const pcm = engine.feed(m.cycles, m.bytes, m.until, m.ratio ?? 1);
  spent += performance.now() - t;
  if (out && pcm.length) out.postMessage(pcm, [pcm.buffer]);
  if (++fed % 60 === 0) {
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
      engine = new wasm.SoundEngine(m.game, chips);
      if (m.mix) engine.set_mix(m.mix[0], m.mix[1]);
      postMessage({ type: "ready" });
      for (const p of pending.splice(0)) feed(p);
    } catch (err) {
      postMessage({ type: "error", message: String(err.message ?? err) });
    }
  } else if (m.type === "port") {
    out = m.port;
  } else if (m.type === "link") {
    if (engine) feed(m);
    else pending.push(m);
  }
};
