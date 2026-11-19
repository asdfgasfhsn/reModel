# Replacement models through glTF

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. It rests on model identity from
[scene-understanding.md](scene-understanding.md).

## The problem

A texture pack cannot change a shape: "a rounder cap, a different board
shape" is on the showcase's list of what a pack can't do. The user wants
replacement models, with one rule about tools: define what goes out and
what comes back, and let people use whatever they like in between.

**Observed.**

- **The hook is known.** `docs/enhancements.md` ("Model replacement")
  plans to map a model's ROM address and a hash of its polygon data to a
  replacement mesh, drawn with the object's matrix, "where a model is
  expanded into polygons". It lists what is unsettled: the new mesh's
  textures and UVs, models built from several pieces, and collision
  staying the game's.
- **The pack format has reserved the names**: the `mdl_` file prefix
  (`crates/model2/src/pack.rs:393-395`) and a `model` keyword in the
  manifest (`docs/texture-packs.md:266`).
- **Models are already found.** The catalogue lists objects by model
  address, with the artworks each shows: 419 for Top Skater (the local
  catalogue's report).
- **Lighting can follow new normals.** Smooth shading lights each corner
  with the game's own formula for a given normal
  (`docs/enhancements.md`, "Smooth shading").
- **Characters are rigid parts.** A fighter's limbs are separate models,
  each with its own transform. Nothing is skinned.
- **Some geometry cannot be keyed.** `lastbrnx` rewrites joint pieces in
  polygon RAM every frame, and games send some polygons directly, already
  in view space, with no model at all.
- **Games swap models with distance.** A far car is a different, coarser
  model with its own address.
- **No command exists** to export or import a model, and reModel has no
  glTF dependency.

The recompilation projects surveyed either replace models through
game-specific asset archives or, in RTX Remix's case, through hashes of
captured geometry that its own documentation calls unstable. A ROM
address plus a content hash is sturdier than both.

## Design

### Out: one glTF per model

`remodel model export` writes standard glTF for a model, a scene, or
everything the catalogue has seen.

| In the file | Holds |
|---|---|
| One mesh per model | Positions in the game's model space, the game's per-face normals, UVs, one material per artwork |
| `extras` on the mesh | The model key, its address, the game, polygon count, and the scale and axis convention |
| Materials | Named by artwork, pointing at the artwork's image by its pack name |
| Polygons | As the game has them: quads and triangles, not re-triangulated |
| A preview | A picture of the model as the game draws it |

`model export --scene NAME` writes a single glTF of the scene with every
drawn model placed by its transform for that frame, each node carrying
its model key. This is for seeing a character or a car whole. Only the
meshes come back; the placement is the game's.

Exported files are ROM-derived and stay local, like texture dumps.

### Back: any glTF with the same keys

A replacement is a glTF file in the pack named `mdl_<hash>.glb`, the
reserved prefix and the model's 16-digit hash, or a mesh inside any glTF
whose `extras` carry a model key. The rules:

- **One rigid part per key.** No skinning, no animation of its own. The
  game moves it.
- **Same space.** The origin, axes and scale are the original's. That is
  what puts a wheel on its axle.
- **Materials** point at an artwork (so the game's texture, or the pack's
  replacement for it, is used) or at an image the pack supplies under the
  reserved `img_` prefix, with its own UVs.
- **Normals** are the file's. They are lit with the game's formula, as
  smooth shading does.

`model check` reports, per part: key matched, bounds against the
original's (a part that grew or moved its pivot is flagged), polygon
count against a budget, materials resolved, and a before-and-after
picture from each scene the model appears in.

### Drawing it

A replacement cannot be put into the geometrizer's output. That stream
has a fixed form (a polygon count per block, and texture headers that
point into texture RAM), which cannot carry new UVs, a pack's own images
or per-corner normals. It is also part of the machine: the state hash
covers the last stream (`crates/model2/src/bus.rs:1413-1428`). So the
machine's stream is never touched.

The replacement is made in the renderer, where the stream becomes
polygons (`raster::build_with`). For a block whose model has a
replacement, the renderer builds the new polygons itself and drops the
block's own. It needs three things the native geometrizer already
records on the side: the model's address for the block, the object's
matrix, and the block's light. With those it transforms the mesh, lights
each corner with the game's formula, culls, clips and projects, as the
game's program would have.

This path is the one smooth motion and photo mode already use to draw a
frame the machine never ran. It only exists while that recording is on,
and with a native geometrizer.

Two things need rules.

- **Order.** The games draw without a depth buffer, far to near by a sort
  key per polygon. A replacement's polygons take the original object's
  place in that order and are depth-tested among themselves, the way the
  depth option already treats a group of polygons the game sorted
  together (`docs/enhancements.md`, "Depth buffer").
- **Distance models.** A pack can name several keys for one replacement,
  with an optional coarser mesh. Without that, a replaced car would turn
  back into the original at a distance.

### What it does not cover

- Geometry the game rewrites each frame, or sends without a model.
- An object the game culled. The replacement is drawn only where the
  game drew the original, so a replacement larger than the original can
  vanish at the screen's edge a moment early.
- Games on the chip path, and the two with no native geometrizer.
- Skinned characters. A smooth elbow would need the renderer to know two
  parts are joined and blend between their transforms. The joins are not
  recorded anywhere; proposing them from parts that move together is
  possible future work, not this spec.
- Collision, which is the game's and does not change. A much larger
  replacement will look as if it clips or floats.

## Interfaces

| Verb | Does |
|---|---|
| `model export KEY`, `--scene NAME`, or `--all` | Writes glTF and previews into the project's `cache/models/` |
| `model check FILE` | The import report |
| `asset list --kind model` | Models, parts, artworks, scenes, class |
| `asset same KEY KEY` | Declares distance siblings |
| `scene render --asset MODEL` | The model in its scenes, before and after |

Reading and writing glTF uses a library, not hand-written code.

Tools in between are the author's choice: Blender by hand, Blender
through its own automation, a generative model, anything that keeps the
`extras`.

## The roles

- **Artist.** Exports a scene, opens it in Blender, rounds off the cap,
  exports. `model check` says the pivot moved by two units before it ever
  reaches the game.
- **Art agent.** Works from the same files and report. Budget: 3 calls
  and 1 picture per part.
- **Pack lead.** Sets the polygon budget and decides which distance
  models share a replacement.
- **Tester.** Bounds, pivots and budgets are numbers. State hashes are
  unchanged.
- **Player.** Replaced models can be turned off apart from textures.
- **Tool engineer.** One hook where the renderer builds polygons, shared
  by every game with a native geometrizer.

## Testing

- **Without ROMs:** export then import of a synthetic model gives the
  same polygons; a part with a moved pivot, a missing key, or a blown
  budget is reported; ordering against a synthetic scene.
- **With ROMs:** exporting a model and importing it unchanged draws the
  frame byte for byte as before. That identity test is the strongest
  check of the whole path. State hashes equal with replacements on.

## Performance

Scenes are a few thousand polygons a frame, so there is room: a
replacement ten times as detailed is still small for a GPU. The cost to
watch is on the CPU, where polygons are built, sorted and lit each frame
(1 to 2 ms for the shading pass today, `docs/enhancements.md`). A budget
per scene, enforced by `model check` and reported by `pack check`, keeps
a pack inside a frame. None of this is measured.

## Open questions

- **How are per-face attributes kept through glTF?** The games colour and
  light per polygon; glTF is per vertex. Duplicated vertices per face
  work and inflate files.
- **Untextured polygons** have a colour, not an artwork. A material per
  palette colour, or vertex colours?
- **Which artwork variant** does a replacement's material mean when the
  original polygons used several colours of one artwork?
- **Does every game store models the same way?** The native geometrizers
  cover three board generations.

## MVP: a few days

1. `model export` for one model and for a scene, with previews.
2. The identity test: export, import unchanged, draw, compare.
3. `model check` with keys, bounds and budget.
4. **Spike**, as already planned: one Top Skater prop and one skater's
   parts exported, the prop modified, drawn before and after.
