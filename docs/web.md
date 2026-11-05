# Browser build (planned)

The goal is Remodel compiled to WebAssembly and running in a browser,
drawing with WebGPU. This page records what's ready and what's left.

## Ready

- **Renderer.** The GPU rasteriser (`crates/gpu`, see `docs/gpu.md`)
  uses wgpu, which runs on the browser's WebGPU. Its shader reads video
  RAM from storage buffers. WebGPU has those, but WebGL2 doesn't, so
  WebGL2 isn't a target.
- **Single thread.** The geometrizer can run inline instead of on a
  worker thread (`--single-thread`; the default on `wasm32`). The state
  hash is identical either way.
- **No clock needed.** Host timing in the board model goes through a
  stopwatch that measures nothing on `wasm32`, where
  `std::time::Instant` would panic.
- **Frame-sized steps.** `Machine::run_slice` returns at each frame
  boundary, which suits one emulated frame per `requestAnimationFrame`.

## Single-threaded speed (`make bench` run, native)

| build | geometrizer on a thread | inline |
|---|---|---|
| interpreters | 2.2x real time | 1.6x |
| recompiled (`RECOMP=1`) | 4.4x | 3.2x |

Same state hash in all four. WebAssembly usually runs at one half to two
thirds of native speed, so the recompiled build should manage roughly
1.6-2x real time in a browser. The interpreter build, the only one that
could be published (recompiled code is derived from the ROMs), would be
around 1x.

## Left to do

- The WebAssembly target: `rust-wasm` on Arch, or `rustup target add
  wasm32-unknown-unknown`.
- A front end:
  - a canvas surface;
  - a ROM picker that keeps ROMs in IndexedDB;
  - a frame loop on `requestAnimationFrame`;
  - audio through an AudioWorklet;
  - controllers through the Gamepad API.
- `make web`: build, generate the JS glue, serve on localhost.
- Optional: WebAssembly threads for the geometrizer. These need
  cross-origin isolation headers and a nightly `build-std`.
