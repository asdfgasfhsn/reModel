# Added elements: grass, props, decals, weather

Shared words, roles and rules: [README.md](README.md). Nothing is built
into reModel; what was tried is in "Evidence", and the spike code is on
an unmerged branch, not for merging as it stands. It rests on
[scene-understanding.md](scene-understanding.md) and
[model-interchange.md](model-interchange.md).

## The problem

Replacing what the game draws only goes so far. A flat green polygon is
still flat when its texture is better. The wish is to add things the game
never had: grass on verges, trees, a fuller crowd, debris, skid marks,
rain.

The difficulty is that added things must stay put. reModel has no world
space: the game hands over geometry already in the camera's view
(`crates/model2/src/photo.rs:13-17`). Anything placed in screen or view
space swims as the camera moves.

**Observed, and useful.**

- **Every object arrives with its own matrix**, and the native
  geometrizers know which model it is
  (`crates/model2/src/geo_native.rs:215`).
- **A model's polygons are fixed data**, keyed by a hash
  ([scene-understanding.md](scene-understanding.md)).
- **The geometrizer can expand a different mesh in a model's place**
  ([model-interchange.md](model-interchange.md)).
- **The games draw without a depth buffer**, far to near. Large ground
  polygons are drawn in the game's order with no reliable depth
  (`docs/enhancements.md`, "Depth buffer").
- **Scenes are small**: a few thousand polygons a frame.
- **A principle already stands**: "Gameplay stays authentic. A change
  that alters what the player can see in a way that could affect play ...
  is an optional patch, off by default" (`docs/enhancements.md`).

## Design

### The anchor: a model's own space

Every added element belongs to a model and is placed in that model's
coordinates. Each frame it is drawn with the object's own matrix, as if
it were part of the model. It is then steady by construction: through
camera moves, in-between frames, run-ahead and photo mode, with no camera
recovered and no per-game data.

This is the same mechanism as a replacement model, with the same
limits, and the machine's own output is never touched
([model-interchange.md](model-interchange.md), "Drawing it"). An added
element is extra geometry that travels with a model the game draws.


Two tiers of that mechanism are available. Extra polygons appended to a
model's data in a re-run of the game's own geometry code (tier 1) stay
attached, lit and clipped, but can only use the host's texture sheet and
flat lighting. That suits a decal or a simple prop, not a field of grass
with its own image (E1). Scatter and anything with its own texture use
the second tier, drawn by the renderer from the object's recorded matrix
and light. An anchored mesh holds steady through a moving camera and is
hidden by its host's own polygons where it should be (E2). Two rules
follow for any added element:

- **It needs a stated material.** Left alone it takes its host's, and a
  self-lit host gives full brightness (E2).
- **Draw order is worked out per triangle** (see "Drawing order").

### Scatter

Scatter is many small instances, each two crossed quads with a cut-out
image for grass, placed on the surface of a ground model (E3).

- **Instances stay glued down**, measured against the ground's own
  vertices.
- **Cost.** In software it is about 0.5 ms per thousand triangles. On the
  GPU, instances built on the CPU each frame are too slow: 13,800
  triangles took a 1080p frame from 31 to 44 ms. Real scatter needs
  instancing on the GPU.
- **Where.** Grass can be told from path by the polygon's texture, which
  gives the rule its "where". Palette colour alone does not.
- **Density follows the view.** Even spacing looks thin up close.
- **Every piece of the ground is covered by the rule.** On Daytona the
  road changes model along the course and scatter stops at the join. A
  fade hides the far edge; a cull at the screen's edge does not. Top
  Skater showed no pop-in, since the game never culled a ground piece in
  view.

For the protected classes, four things a check can measure: no instance
stands on a protected polygon; no tuft pixel lands where the plain
picture shows a protected class; the track's edge moves by under a
pixel; nothing lands on the HUD or a rival.

### Four kinds

| Kind | What it is | Placed by |
|---|---|---|
| **Scatter** | Many small instances (grass blades, flowers, pebbles, crowd heads) | A rule over a model's polygons |
| **Prop** | One glTF mesh (a tree, a sign, a barrier) | A position and turn in a model's space |
| **Decal** | A flat image laid on a surface (skid marks, paint, a logo) | A polygon and a rectangle on it |
| **Air** | Things not attached to anything (rain, dust, leaves) | View space, moved by the camera's movement |

**Scatter rules** say where and how much, not each position:

```
scatter grass on class=ground art=verge-grass density=40 height=0.3..0.6 mesh=blade tint=from-surface
```

Positions come from a hash of the model key, the polygon and a cell on
it, taken from the model's own data and not from the clipped polygon on
screen. The same polygon always grows the same grass, on every frame and
every machine, and nothing is stored per instance. `class` and `art` come
from scene understanding and asset identity, so a rule written once
applies wherever that surface appears in the game.

**Air** is the one kind that needs the camera's movement between frames,
which is found generically from the objects' matrices. It needs no true
world space. It keeps state between frames, so it restarts on a camera
cut, a state load and a switch of machine, as every such feature does
([scene-understanding.md](scene-understanding.md), "State across
frames").

### Drawing order

The games draw without a depth buffer, far to near, so added geometry
needs a rule. **One key per triangle, or the GPU's depth test; one
position per ground model is wrong (E3).** Each added triangle takes the
game's own kind of key, its nearest corner, so it is sorted with the
polygons the game drew. Where the GPU's depth test is on, the ground
stays in the game's order and the added geometry is tested against it.
Which suits a game needs looking at per game, since the two differ on
Daytona (E3).

#### Set aside

- **One position for the whole ground model**: tufts were cut by ground
  drawn later (1,222 wrong pixels a frame on average on Top Skater).
- **Each tuft drawn straight after the polygon it stands on**: 7 wrong
  pixels on Top Skater and 241 on Daytona.
- **The ground's plane used as depth**: 246 and 213, and the only rule
  that ever drew a tuft over the skater, a rail or the skater's shadow.

### When the host is not drawn

An element is drawn only when the game draws its host. When the game
culls a model at the screen's edge, what stands on it goes too, a moment
before it would have left the picture. Tall or wide additions show this
most. Polygons the game sends directly, with no model, cannot host
anything.

### Distance

The games swap coarser models in at a distance. An element anchored to
the near model vanishes when the far one appears. Two answers: a rule
can name the far model as a sibling
([model-interchange.md](model-interchange.md)), and scatter fades out
with distance before the swap, which also bounds its cost.

### Play stays honest

Added elements are draw-only: they have no collision and the game does
not know they exist. Two rules follow.

- **Off by default**, and switched separately from textures.
- **A class of surfaces is protected.** By default nothing may be added
  on `track` or in HUD windows, where it could hide the road, a rival or
  a target. A pack that overrides this is labelled as changing what the
  player can see.

### Editing

Rules are text, so an agent can write them. Placing a prop or a decal by
hand is the photo mode camera plus picking: fly to the spot, point at a
surface, and the tool records the model, the polygon and the position on
it. That is part of [authoring.md](authoring.md).

## Interfaces

| Where | What |
|---|---|
| `place.txt` in a pack | `scatter`, `prop`, `decal` and `air` lines, each naming a model, class or artwork |
| `mdl_<name>.glb` | Meshes for props and scatter instances |
| `scene render --overlay added` | Added elements alone, coloured by rule, with counts in `--json` |
| `place check` | Per scene: instances drawn, polygons added, anything on a protected class, anything anchored to a model no scene shows |
| `scene clip` | The steadiness check: the same instance's screen position against its host's over a clip |

## The roles

- **Art agent.** Writes a scatter rule from a brief ("grass on the verges
  of the Novice course") and reads `place check` for counts and
  violations. Budget: 3 calls and 1 picture.
- **Artist.** Flies the photo camera to a corner and drops a tree.
- **Pack lead.** Decides what may be added, and reviews in motion, since
  a still cannot show swimming or popping.
- **Tester.** Steadiness and polygon budget are numbers. State hashes are
  unchanged.
- **Player.** One switch, off by default.
- **Tool engineer.** One mechanism, shared with replacement models.

## Testing

- **Without ROMs:** the same rule gives the same instances twice; an
  instance stays fixed to a synthetic model through a moving camera;
  nothing is placed on a protected class; a triangle's key is its own
  nearest corner, not its host's.
- **With ROMs:** over a clip, each instance's position relative to its
  host polygon is constant to a fraction of a pixel; state hashes are
  unchanged; with the feature off, frames are exact.

## Performance

The bounds are designed in: a cap on instances per scene, a fade
distance, and instanced drawing so a field of grass is a few draw
calls. Instance positions are generated on the GPU from the host
polygon's model data, not stored. Measured so far: about 0.5 ms per
thousand triangles in software, and 13,800 CPU-built triangles raising a
1080p GPU frame from 31 to 44 ms, which is why instancing is needed
(E3). A target to test: 20,000 scatter instances within 1 ms at 1080p on
the development GPU.

## Without a native geometrizer

Nothing is added. Models are not known, so there is nothing to anchor to.

## Evidence

Spike code is on the unmerged branch `uplift-spikes`, not for merging as
it stands. Entries are in date order.

**E1. Appended polygons, tier 1 (2026-11-19).** Observed: extra polygons
appended to a model's data in a re-run of the game's own geometry code
stayed attached, lit and clipped. Not tested: anything with its own
texture, which tier 1 cannot draw.

**E2. An anchored prop, tier 2 (2026-11-20).** Observed: a torus with
its own image, anchored at an offset to a Top Skater speed-limit sign,
held to within 0.009 of a pixel over 37 frames of moving camera, and
was hidden by the sign's own polygons where it should be. It took a
self-lit material from its host by accident and came out at full
brightness. Draw order had to be worked out per triangle.

**E3. Scatter (2026-11-20).** About 1,300 lines on `uplift-spikes`: 200
to 2,000 grass tufts, each two crossed quads with a cut-out image,
placed on the surface of Top Skater's grass verge and of a Daytona
roadside, over 65 frames in software and 26 on the GPU. Observed:

- A tuft's foot against the ground's own vertices: 0.002 of a pixel on
  average, 0.7 at worst (Top Skater); 0.17 at worst on Daytona.
- Draw order, measured against a depth test of everything, as wrong
  pixels a frame:

  | Rule | Top Skater, of about 3,980 | Daytona |
  |---|---|---|
  | One position for the whole ground model | 1,222 on average; tufts cut by ground drawn later | 456 |
  | Each triangle keyed by its nearest corner | 0 | 13 |
  | Each tuft drawn straight after the polygon it stands on | 7 | 241 |
  | GPU depth test, ground in the game's order | 0 to 4 | 4 to 71 |
  | The ground's plane used as depth | 246 | 213 |

  No rule but the last ever drew a tuft over the skater, a rail or the
  skater's shadow.
- No pop-in on Top Skater; on Daytona scatter stopped at the join where
  the road changes model.
- Cost: about 0.5 ms per thousand triangles in software; on the GPU
  13,800 triangles took a 1080p frame from 31 to 44 ms.
- Grass told from path by the polygon's texture, not by palette colour.
  Even spacing looked thin up close.
- State hashes were unchanged in software; the GPU runs' were not
  compared.

## Open questions

- **Does anchoring hold in every game?** Scenery the game rebuilds each
  frame, or sends without a model, cannot host anything.
- **Lighting added geometry.** With the game's formula for its host's
  light, or unlit and tinted from the surface under it? A prop needs a
  stated material either way (E2).
- **Are state hashes unchanged with scatter on the GPU path?** Not
  compared (E3).
- **Reflections and water** need the scene drawn a second time from a
  mirrored view. Possible, since the geometrizer can be run again, and
  not designed here.
- **Crowds** in these games are flat cards. Adding heads is scatter;
  making them move is animation the game does not have.


## MVP: a week

1. One prop anchored to one Top Skater scenery model, with a stated
   material.
2. Scatter on a ground model: a key per triangle or the GPU's depth
   test, GPU instancing, and view-dependent density.
3. `place.txt` with `prop` and `scatter`, `place check`, and the
   protected classes.
