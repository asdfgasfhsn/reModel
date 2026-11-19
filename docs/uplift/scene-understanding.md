# Scene understanding

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. This spec underlies the other pass-two specs: models, lighting,
added elements, authoring and shader packs all draw on what it provides.

## The problem

To relight a scene, add grass to it or let someone point at a thing, the
renderer must know what is where. Today it knows a great deal on the CPU
side and passes little of it on.

**Observed.**

- **The game hands over view space.** Each object's transform already
  includes the camera: "a rotation and translation from model to view"
  (`crates/model2/src/photo.rs:13-17`). The game sends no camera matrix,
  and there is no world space anywhere.
- **Depth is exact only for some polygons.** Skies, flat shadows,
  checkerboard polygons and ground quads spanning more than eight times
  in depth keep the game's draw order and "test and write nothing"; the
  ID pass's 1/z for such ground is "only roughly the surface's", and it
  leaves translucent polygons out (`docs/enhancements.md`, "Depth
  buffer" and "Looks"). The ground is exactly where occlusion, shadows
  and grass matter most.
- **The state hash includes the geometrizer's output.** It covers all
  RAM and the last raster stream (`crates/model2/src/bus.rs:1413-1428`).
  Nothing draw-side may alter that stream.
- **View-space positions are thrown away.** A polygon is clipped in view
  space (`clip_view`, `crates/model2/src/raster.rs:1195`) and then keeps
  only screen position, 1/z and texture coordinates per corner
  (`crates/model2/src/raster.rs:38-48`).
- **A model has a name, on the CPU only.** The native geometrizers record
  each object's model address (`crates/model2/src/geo_native.rs:215`), and
  the catalogue keys objects by it (`crates/model2/src/catalogue.rs:379-381`).
  A polygon knows which object block it came from, but that is an ordinal
  within the frame (`crates/model2/src/raster.rs:116-119`) and never
  reaches the GPU.
- **A pixel knows its texture only.** The look's ID target holds a 23-bit
  fold of the texture key (`crates/gpu/src/look.rs:43-50`).
- **Objects are matched across frames by a heuristic.** Smooth motion pairs
  each transform with the previous frame's by model address and distance.
  It fails for models that swap with pose and for geometry the game
  rewrites each frame (`docs/hw/geometrizer.md`, "Smooth motion").
- **Surface kinds are guesses made per frame**: sky, flat black shadow,
  checkerboard, and polygons whose depth spreads more than eight times
  (`crates/model2/src/raster.rs:1853-1867`).
- **A camera spike exists and is unconfirmed.** `camera-scan` found
  candidate camera matrices in one game's RAM
  (`docs/photo-mode.md`, "A true camera").
- **Coverage is uneven.** Model addresses, normals and matching need a
  native geometrizer. `zerogun` and `pltkids` have none, and the chip path
  gives block facts without faces.

RTX Remix, the nearest outside comparison, has to rebuild identity from a
game's draw calls and documents its geometry hashes as fragile (its
documentation; not its code). reModel starts from a better place: a
model's address in ROM.

## Design

Four steps. Each is useful alone, and each is draw-side: it reads what the
geometrizer and the renderer already produce.

### 1. A model identity, and it reaches the pixel

- **Model key**: a 16-digit hash of the model's polygon data, written
  `model:<hash>` and `mdl_<hash>` in a file name, with the ROM address
  kept beside it. The hash survives a model being stored at two addresses;
  the address is what the game's manifest tables use. Today the catalogue
  keys an object by its address alone
  (`crates/model2/src/catalogue.rs:379-381`); changing that is part of
  this work. A model in polygon RAM, which the game can rewrite, is hashed
  again when that RAM changes and is marked `rewritten` if it does so
  while on screen. Polygons the game sends directly have no model.
- **In the ID target.** Each polygon's model, window and flags (checker,
  shadow, sky, drawn in the game's order) join the texture ID. They ride
  in the buffer that already carries per-polygon ID words, since a new
  storage buffer is not free.
- **In the tools.** `scene what` answers with the model as well as the
  artwork. The `ids` overlay can colour by model. A look tag or a draw
  rule can name a model. An artwork shared by a sandal sole and a banner
  can be split by which model it is on
  ([asset-identity.md](asset-identity.md)).

**Spiked on 2026-11-19** (Top Skater, software renderer; branch
`uplift-spikes`, about 190 lines):

- **It works.** The native 2C geometrizer hashes the polygon words each
  object reads and passes the key beside the existing lighting side
  channel. In 21 frames every 3D pixel had a model key. A race frame has
  20 to 71 models.
- **Draw-only holds.** The state hash over a 6,212-frame run is the same
  with and without it, and the key costs under 0.02 s over that run.
- **Keys are stable**: two runs gave identical output, and a model keeps
  its key as the camera moves.
- **A model is a part, not a thing.** The skater is about ten models
  (limbs, board), and the ground a few large ones. Picking "the skater"
  needs a step that groups models, which this spec did not have. Classes
  and the game's manifest are where groups would live.
- **Today's captures cannot answer.** A capture holds the finished
  stream and video memory, with no side channel. Identity must be
  recorded when the capture is made, because the polygon data is not in
  it. Recording only the per-object part is a few hundred bytes a frame.
- **The hash added nothing over the address** in this game: 129
  addresses, 129 keys. Its value (the same model at two addresses, ROM
  revisions, models in RAM) did not arise.
- **Repeated on `vf2` (2A) and `daytona` (original Model 2)** with no
  new hooks: both run the same native code in its fixed-function mode.
  Every 3D pixel had a model key on both; keys were stable between runs;
  state hashes were unchanged. A fighter is 15 to 20 parts and a car 3
  to 5.
- **Models in polygon RAM are real, and a hash cannot name them.**
  `vf2` draws two per frame, at fixed addresses, and rewrites them every
  pose: 17 and 14 different hashes over 21 frames. `daytona` draws up to
  five. Such a model must be named by its address and marked
  `rewritten`, as this section already proposed; the hash is useless for
  it.
- **Repeated on `gunblade` and `fvipers` (Model 2B)** on 2026-11-20.
  2B runs different native code, and it took the same four hooks there,
  43 lines. Every 3D pixel had a model key; keys were stable between
  runs; state hashes were unchanged.
  - `fvipers`' second DSP writes parts of the display list. Those parts
    are ordinary object commands, so they name models like any other.
  - `fvipers` has the same two rewritten RAM models as `vf2`, at the
    same addresses.
  - **A `gunblade` enemy changes model address from frame to frame.**
    The game animates it by drawing a different model per pose, so "this
    enemy" is a set of models, and grouping must cover poses as well as
    parts. An enemy is 8 to 10 models at a time; an explosion is hundreds
    of tiny ones.
  - On `gunblade` 15 hashes each appear at two or more addresses: the
    ROM holds the same model twice. Address names a model; the hash says
    two are the same.
- **On the GPU it needed no new target** (2026-11-20, about 40 lines).
  The look's ID target has spare bits in its tag; the object block's
  number went there, and the CPU maps a block to its model. Against the
  software result, every 4th pixel on 3 frames each:

  | | Game's resolution | 1080p |
  |---|---|---|
  | Top Skater | 99.3% to 99.6% | 99.4% to 99.8% |
  | `fvipers` | 95.8% to 98.5% | 98.1% to 99.3% |

  The misses are where the two renderers resolve overlap differently
  (the game's draw order against a depth test), mostly at edges between
  neighbouring models. Reading the whole target back costs 5 to 7 ms at
  the game's resolution and 55 to 71 ms at 1080p, so a click reads one
  pixel and `scene what` reads at the game's resolution.
- **The ID pass runs only with a look active** today. Picking needs it
  on its own.
- Not done: polygons sent directly (none occurred in any of the five
  games).

### 2. Instances that persist

A model drawn five times is five instances. The smooth-motion plan pairs
recorded transforms with the previous frame's. Where it has a pairing,
give the instance the same small ID from frame to frame, and a new one
when the pairing breaks or the camera cuts.

The limits are real and the design does not hide them. The plan pairs
transform commands, not objects, so an object that reuses the previous
matrix has no entry of its own. It exists only with a native geometrizer
and only while the recording that smooth motion and temporal
anti-aliasing use is on. An instance ID is therefore a hint that a
feature may use and must be able to do without.

### State across frames

Instance IDs, effects in the air, and a shader's feedback or history all
remember something from earlier frames. The picture can jump under them:
run-ahead switches machines, a save state loads, photo mode scrubs time,
the camera cuts. Temporal anti-aliasing already resets its history on
exactly these events. Every feature that keeps state across frames
resets on the same events, through the same call. None of that state
ever reaches the machine, so state hashes cannot be affected; only the
picture restarts.

### 3. A stated set of buffers

One contract, used by looks, lighting, shader packs and picking:

| Buffer | Holds | Exists today |
|---|---|---|
| Colour | The 3D layer | Yes |
| Depth | 1/z per pixel; exact for depth-tested polygons, rough for those drawn in the game's order | Yes, in the ID pass and with the depth option |
| IDs | Artwork, polygon, model, instance, window, flags | Artwork only |
| Normal | View-space, per face, from the polygon's plane | Only for lit polygons, on the CPU |
| Light | The game's brightness before colour tables | Yes, with smooth shading |
| Motion | Screen-space movement | Yes, with temporal anti-aliasing, native only |
| Position | View-space, from the pixel's ray and the polygon's plane | No |

**Position and normal do not come from depth.** Depth cannot be trusted
on the large polygons that matter most, as noted above. The reliable
source is the polygon itself. Before projection each polygon is flat and
has view-space corners. Keep its plane (a normal and a distance) per
polygon, computed where the polygon is built. A pixel that knows its
polygon then gets an exact view-space position, by meeting the pixel's
ray with that plane, and an exact face normal. This works for every
polygon the game draws, including ground quads drawn in the game's
order, and for every game, including those on the chip path, since it
needs only the stream.

The ID pass leaves translucent polygons out. They are see-through by
intent, so the buffers describe the surface behind them.

### 4. Space beyond one frame

Three levels, cheapest first. Most features need only the first.

- **Model space.** Anything attached to a model (a decal, grass on a
  ground piece, a lamp's light) is placed in that model's own coordinates
  and drawn with the object's matrix each frame. It is steady by
  construction, through camera moves, in-between frames and photo mode,
  and needs no camera at all.
- **Camera movement.** Static scenery shares one turn and shift between
  two frames: the camera's. The matrices of matched objects give it by
  agreement among them, generically. The games also shift the window's
  centre from frame to frame (the camera bob described under "Smooth
  motion" in `docs/hw/geometrizer.md`), which has to be added in. That is enough for effects that live
  in the air (rain, dust) to move correctly as the camera does.
- **A true world space.** The game's own camera, read from its RAM, as the
  photo-mode design proposes: per game, the address and format of the
  camera, held in the game's manifest. Reading RAM is draw-side. This is
  the only level that needs per-game data, and the only one that survives
  a camera cut.

### 5. What things are

A **class** says what a thing is in the game: `ground`, `track`, `wall`,
`sky`, `water`, `character`, `prop`, `vehicle`, `hud`. It is not the
project's `category`, which groups assets by how the author wants to
treat them (a recipe, a palette). A class is a fact; a category is a
choice.

Rules propose classes from what is known: sky and ground polygons by how
the game sorts them, HUD by window, characters by many small parts that
move together. A person confirms.

Classes live in two places. A pack's project holds its own in
`classes.tsv`, set with `asset set ID class=ground`; packs are per game,
so this travels with the pack. Classes by model address that have been
confirmed can also go in the game's manifest, where generic code already
reads model address ranges for smooth shading
(`crates/romset/src/lib.rs:385-390`). A project's entry wins. Whether to
keep such tables in the repository is a decision for the user, as scene
lists are.

## Interfaces

| Where | What |
|---|---|
| `scene what` | Adds `model`, `instance`, `window`, `class`, and view-space position and normal at a point |
| `scene render --overlay` | `models`, `instances`, `classes`, `normals`, `depth` |
| `asset list --kind model` | Models with their parts, artworks, scenes and class |
| `asset set ID class=ground` | Confirms a class in the project; `asset list --suggest-class` proposes them |
| `scene camera NAME` | The camera's movement over a clip, and whether a true camera is known for the game |

`classes.tsv` in the project: key, class, source (`rule` or `confirmed`),
note.

## The roles

- **Art agent.** `scene what` at a point returns "the left grandstand,
  class `prop`, drawn with these four artworks". Budget unchanged: numbers,
  no pictures.
- **Tool engineer.** Writes a lighting pass or a scatter rule against the
  buffer table, not against the renderer's internals.
- **Pack lead.** Confirms classes once per game from a sheet per class.
- **Tester.** Each buffer has an overlay, so "is the normal right" is a
  picture and a number.
- **Player.** Nothing changes unless a feature that uses a buffer is on.

## Testing

- **Without ROMs:** model keys frozen by test vectors; a synthetic scene,
  including a ground quad drawn in the game's order, whose IDs, positions
  and normals are known exactly; an instance that keeps its ID across
  frames and loses it on a cut, a state load and a machine switch.
- **With ROMs:** for the games with the lighting side channel, plane
  normals against the recorded ones; camera movement against the
  photo-mode spike's candidates; state hashes unchanged with every buffer
  on.

## Performance

The ID pass costs 0.7 ms at 1080p and 3.0 ms at 4K today
(`docs/enhancements.md`, "Looks"; documented, not re-measured). More words
per polygon should cost little on top; that is to be measured. Buffers are
made only when a feature asks for them, as today. Model hashing is done
once per model, not per frame.

## Without a native geometrizer

Model keys, instances and camera movement are absent. Depth, artwork IDs,
flags, and positions and normals from polygon planes remain. Features built on the first group say
they are off for those games; nothing guesses.

## Open questions

- **Do all objects in a frame share one camera?** Games draw several 3D
  windows, each with its own depth scale; some may have their own camera.
- **How often does matching break** in normal play, per game? It decides
  how much can rely on instances.
- **Is the camera in RAM findable for most games**, or only some? The
  spike covered one.
- **A plane per polygon is one more word of data per polygon** in a
  buffer that is already full of them. What does it cost?
- **Model-address class tables in the repository**: acceptable, as the
  shading tables are?

## MVP: a few days

1. Model, window and flags in the ID target; `scene what` and the `models`
   overlay.
2. A plane per polygon, with positions and face normals from it, and
   `normals` and `depth` overlays.
3. Class suggestions from the existing per-frame guesses, confirmed in
   the project's `classes.tsv`.
4. **Spike:** camera movement by agreement, checked by holding a marked
   ground point still over 120 frames of a moving scene.
