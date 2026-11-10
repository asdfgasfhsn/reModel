// Texture pack images (docs/web.md, "Texture packs"), fetched and decoded
// off the page's thread. The page posts:
//   {type: "init", pkg, module}   the package URL and the shared compiled
//                                 module (decoding is the module's
//                                 `decode_png`, the decoder the native
//                                 player uses)
//   {type: "get", name, url}      fetch and decode one image
// and gets back, per "get", {type: "image", name, width, height, rgba, ms}
// (straight alpha RGBA, its buffer transferred; ms: the decode's time) or
// {type: "missing", name, message}. Requests that arrive while the module
// loads wait for it.
let wasm = null;
let ready;

async function get(m) {
  try {
    await ready;
    const r = await fetch(m.url);
    if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
    const file = new Uint8Array(await r.arrayBuffer());
    const t = performance.now();
    const png = wasm.decode_png(file);
    const { width, height } = png;
    const rgba = png.rgba();
    postMessage({ type: "image", name: m.name, width, height, rgba, ms: performance.now() - t }, [rgba.buffer]);
  } catch (err) {
    postMessage({ type: "missing", name: m.name, message: String(err.message ?? err) });
  }
}

onmessage = (e) => {
  const m = e.data;
  if (m.type === "init") {
    ready = (async () => {
      wasm = await import(m.pkg + "remodel_web.js");
      await wasm.default({ module_or_path: m.module ?? m.pkg + "remodel_web_bg.wasm" });
    })();
  } else if (m.type === "get") {
    get(m);
  }
};
