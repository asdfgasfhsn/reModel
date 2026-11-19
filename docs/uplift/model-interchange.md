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

The machine's own output is never touched: the state hash covers the
last stream (`crates/model2/src/bus.rs:1413-1428`). A replacement is
drawn by the renderer from a second pass that the machine never sees.

**Spiked on 2026-11-19** (Top Skater, software renderer, one model;
branch `uplift-spikes`, about 480 lines). The spike changed this section.
The first design had the renderer transform, light, cull and clip the new
mesh itself. The spike tried something cheaper and it worked.

**Tier 1: reshape, by re-running the game's own geometry code.** Smooth
motion already re-runs a frame's display list through the native
geometrizer to draw frames the machine never ran. The same re-run is
made with one model's polygon data swapped for the replacement's. The
game's own transform, lighting, culling, clipping and sort order are
reused, on every board that has a native geometrizer.

- Re-run with the model's own data, the result equals the machine's
  stream word for word, with no differing pixel, on 9 frames of 3 scenes.
- A skater's torso scaled to 1.5 times stayed attached, lit and culled
  through 7 frames of a moving camera.
- A box drawn in the torso's place did the same.
- State hashes were unchanged over 400 frames with it on.
- The re-run cost 0.5 to 1.1 ms a frame, and shares work with the one
  smooth motion makes.

**Repeated on `vf2` and `daytona`** with about 45 more lines: unchanged
data gave the identical frame on both, a fighter's part and a car's body
scaled to 1.5 times stayed attached over 5 frames, the re-run cost 0.3
to 0.5 ms, and state hashes were unchanged. Both use the standard
10-word polygon record, not Top Skater's compact one. Two gaps: the
new-mesh test was not repeated, because the spike's encoder writes the
compact record only; and the two models `vf2` keeps in polygon RAM and
rewrites every pose cannot be swapped by this hook, and should not be:
they are geometry the game animates.

**Repeated on `gunblade` and `fvipers` (Model 2B)** on 2026-11-20, 43
more lines in the 2B native code: unchanged data gave the identical
frame on 8 frames of each, state hashes were unchanged, and a part of a
fighter scaled to 1.5 times stayed attached. The re-run cost 1.9 to 2.7
ms a frame, four times the other boards' (rough: the machine was
shared). Both games use the standard record. On `gunblade` a scaled
enemy limb showed in only 2 of 6 frames, because the enemy is a
different model in each pose: replacing "the enemy" means replacing
every pose's model, which reshaping does not make practical.

**A new mesh in the standard record, RAM models and the GPU** (spiked
2026-11-20, about 480 more lines):

- **A box replaced a fighter's lower leg on `vf2` and the car body on
  `daytona`** (359 polygons swapped for 12), attached and culled
  correctly over 6 frames each.
- **The standard record's stored normal is used as given**: not
  recomputed and not normalised. It alone decides culling and shading.
  The games store it pointing inward. So a mesh can carry any normal,
  but one per polygon, never per corner: smooth shading still needs
  tier 2 or the renderer's own smooth-shading pass.
- **Textures are the hard limit.** Every record, hidden ones included,
  takes its UVs from the original's list in order, so a mesh with a
  different polygon count reads the wrong ones. The spike rewrote the
  list in the renderer's copy and could then choose among the original's
  UV sets. A new texture is not possible this way.
- **The two RAM models are cloth**: a flap of a fighter's garment on
  `vf2` and on `fvipers`, 2 to 10 polygons, moved by the game each pose.
  With the hook extended to RAM they re-run identically and can be
  scaled. Replacing one only makes sense as a rule on the slot that
  transforms the game's own words.
- **The GPU agrees.** The re-run stream drawn by the wgpu renderer
  equals the wgpu picture of the machine's stream on Top Skater and
  `vf2`, and the scaled models match the software result to within 1 or
  2 pixels at the edges.
- Not tested: an object whose normals the game computes (a mode bit),
  and a new mesh on 2B.

Tier 1 is bounded by the game's polygon format, and the bounds are
tight:

- **A mesh is strips** of triangles and quads with no index list.
  Anything that is not a chain needs a hidden polygon to restart. The
  box took 8 records for 6 faces.
- **One normal per polygon.** Top Skater's compact record works it out
  from the strip; the standard record stores it. Either way lighting is
  flat per polygon.
- **UVs are not in the model.** They sit in a list the object points to,
  two per corner in strip order, and each polygon can only step a short
  way through the texture headers. A replacement uses the original's
  texture sheet, and its polygons must be matched to the original's UV
  entries.

So tier 1 covers moving, scaling and re-cutting a model, and new shapes
that reuse the original's textures. Those textures can themselves be
replaced by the pack. It does not cover a free mesh with its own UVs,
its own images and smooth normals.

**Tier 2: a free mesh, built in the renderer.** Spiked on 2026-11-20
(software renderer; Top Skater and `daytona`; branch `uplift-spikes`,
about 1,000 lines). It works.

A sphere and a torus with their own vertices, a normal per vertex, their
own UVs and a generated 256x256 image with a cut-out were drawn in place
of the skater's torso and Daytona's car body, and anchored beside a
piece of Top Skater's scenery, over 37 frames of moving camera. The
original was removed with tier 1. The side channel gained one thing: the
object block's matrices.

- **It does not drift.** The original model's own vertices, put through
  the recorded matrix, land on the game's own picture to within 0.003
  of a pixel (4,033 vertices on Top Skater, 24,938 on Daytona). The
  anchored torus moved at most 0.009 of a pixel against its scenery.
- **Smooth shading works**, per pixel, with the game's own lighting
  formula and the block's light.
- **Its own image works**, at any size, with nothing from the game's
  sheets. A cut-out works; blended translucency was not tried.
- **Clipping works** at the near plane and the window's edges.
- **Draw order has to be per triangle.** The game sorts polygons and
  does not depth-test most of them. Giving each new triangle the game's
  own kind of key (its nearest corner) put it right to within 0.1 pixel
  on the torso. One position for the whole mesh was wrong by 47 pixels
  there and by thousands on Daytona's car, and even per triangle the car
  was off by 211 pixels. Big objects next to big polygons need a better
  rule.
- **It costs nothing measurable**: 0.06 ms a frame to build the
  polygons, and the software draw time did not change.
- **State hashes were unchanged.**

What it found missing:

- **Which material?** A removed model leaves no polygon to copy a
  lighting entry and colour table from. Taken from the wrong neighbour,
  the anchored torus came out self-lit at full brightness. The block
  must record them, or the mesh must name them.
- **Parts with two matrices.** Top Skater's torso bends between two; the
  spike used the first only. A mesh for such a part needs a weight per
  vertex.
- **The GPU renderer** has no way to draw a polygon with an image that
  is not a game texture or a pack's replacement of one. That path is the
  main work left.
- **Model 2B** does not record the matrices yet.
- **Names for instances**: one mesh per model address is not enough
  where a model is drawn several times, or where a thing changes model
  each pose ([scene-understanding.md](scene-understanding.md)).

**What a still check needs.** A capture cannot be re-run: it holds only
the finished stream. A scene render with a replaced model must start
from the scene's save state and run one frame, a few hundred
milliseconds, or the capture must also hold the geometrizer's inputs.

Two things need rules.

- **Order.** The games draw without a depth buffer, far to near by a sort
  key per polygon. In tier 1 the replacement's polygons take the game's
  own sort keys; nothing sorted wrongly in the spike's frames, by eye. In
  tier 2 a replacement takes the original object's place in that order
  and is depth-tested within itself, the way the depth option already
  treats a group of polygons the game sorted together
  (`docs/enhancements.md`, "Depth buffer").
- **Distance models.** A pack can name several keys for one replacement,
  with an optional coarser mesh. Without that, a replaced car would turn
  back into the original at a distance.

**A character is many models.** The skater is about ten: limbs, torso,
board. Replacing a character is replacing its parts, each rigid. One
smooth-skinned mesh over a moving character is animation the game does
not supply.

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
4. Tier 1 from the spike made real: `mdl_` files encoded to the game's
   records, with `model check` reporting what will not encode.
5. Tier 2 (spiked in software): the block's matrices, light and
   material on the side channel; a glTF loader; per-triangle sort keys;
   then the GPU path.
