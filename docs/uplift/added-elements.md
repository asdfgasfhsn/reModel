# Added elements: grass, props, decals, weather

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. It rests on [scene-understanding.md](scene-understanding.md) and
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
limits. The geometry is built in the renderer from what the native
geometrizer records on the side (the model, its matrix, its light), and
the machine's own output is never touched
([model-interchange.md](model-interchange.md), "Drawing it"). An added
element is extra geometry that travels with a model the game draws.

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

Added geometry is depth-tested among itself and against the model it
stands on. For a host polygon the game draws in its own order without
depth, such as a large ground quad, the added geometry is drawn directly
after its host, so it sits on the ground and under whatever the game
draws next. Where that order would be wrong (grass in front of a car that
is behind it), the depth option's rules decide, and the result needs
looking at per game.

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
  nothing is placed on a protected class.
- **With ROMs:** over a clip, each instance's position relative to its
  host polygon is constant to a fraction of a pixel; state hashes are
  unchanged; with the feature off, frames are exact.

## Performance

Unmeasured. The bounds are designed in: a cap on instances per scene, a
fade distance, and instanced drawing so a field of grass is a few draw
calls. Instance positions are generated on the GPU from the host
polygon's model data, not stored. A target to test: 20,000 scatter
instances within 1 ms at 1080p on the development GPU.

## Without a native geometrizer

Nothing is added. Models are not known, so there is nothing to anchor to.

## Open questions

- **Does anchoring hold in every game?** Scenery the game rebuilds each
  frame, or sends without a model, cannot host anything.
- **Grass on a painter's-order ground** is the common case and the
  hardest for draw order. It needs a spike before anything else.
- **Lighting added geometry.** With the game's formula for its host's
  light, or unlit and tinted from the surface under it?
- **Reflections and water** need the scene drawn a second time from a
  mirrored view. Possible, since the geometrizer can be run again, and
  not designed here.
- **Crowds** in these games are flat cards. Adding heads is scatter;
  making them move is animation the game does not have.

## MVP: a week

1. **Spike:** one prop anchored to one Top Skater scenery model, held
   through a moving clip, with the steadiness number.
2. **Spike:** scatter on one ground model, to settle draw order against
   painter's-order polygons.
3. `place.txt` with `prop` and `scatter`, `place check`, and the
   protected classes.
