# Lighting

Shared words, roles and rules: [README.md](README.md). Nothing is built
into reModel; what was tried is in "Evidence", and the spike code is on
an unmerged branch, not for merging as it stands. It uses the buffers in
[scene-understanding.md](scene-understanding.md). What exists today is in
[../enhancements.md](../enhancements.md): smooth shading, material maps,
bloom.

## The problem

The games light each polygon with one brightness, from one direction and
an ambient level. That is the look of the hardware, and it is also why a
restyled or sharpened game still reads as flat: nothing casts a shadow,
and nothing sits in the scene.

**Observed.**

- **Lighting arrives already done.** The game's geometry program computes
  one brightness per polygon from a light direction, an ambient term, a
  diffuse term and sometimes a highlight, clamped to 0-255
  (`docs/hw/geometrizer.md`, "Transform, lighting, projection"). The renderer receives that
  number.
- **Much scenery is at full brightness.** Top Skater "lights most of it
  with ambient 255 and diffuse 255", so the sum is past the clamp at every
  angle (`docs/enhancements.md`, "Material maps"). There is no headroom
  to add light into.
- **The game's light can be re-evaluated.** With the native geometrizer's
  side channel, smooth shading recomputes the game's formula per corner
  or per pixel for any normal (`docs/enhancements.md`, "Smooth shading").
  Photo mode already turns the light direction with its camera
  (`crates/model2/src/smooth.rs:199`).
- **Shadows are polygons.** The games draw flat black shapes, which the
  renderer recognises (`Poly::shadow`,
  `crates/model2/src/raster.rs:127-131`).
- **Transparency is a checkerboard.** A `stipple` setting already exists
  (`blend` or `keep`), but it acts only under temporal anti-aliasing,
  where `blend`, a fixed 50%, is the default (`docs/enhancements.md`,
  "The checkerboard").
- **Depth is unreliable on large ground polygons**, which keep the
  game's draw order and write no depth (`docs/enhancements.md`, "Depth
  buffer"). Positions and normals must come from each polygon's plane
  ([scene-understanding.md](scene-understanding.md)).
- **There is no fog** in any game's data, and highlights matter in four
  games only (`docs/enhancements.md`, "Fog" and "Material maps").
- **Shadows and tone mapping are listed as ideas**, ray tracing as native
  only (`docs/enhancements.md`, "Lighting" and "Ray tracing").

The showcase's style sheet lists "shadows or lighting other than the
game's" as something a pack cannot do. Fan wishes gathered for other
games in the library point the same way: real transparency for shadows
and effects in the fighters and gun games (from public descriptions;
unverified).

## Design

One rule shapes everything: **new lighting takes light away or replaces
it; it never adds on top.** A multiplying term cannot double-count
shading the game already baked in and cannot push past the clamp.
Anything that would add light first re-evaluates the game's own formula
with headroom.

Where each stands: blended stipple is the one item in the first set.
Shadows are a design with an unfinished mechanism. Ambient occlusion is
set aside. Lights a pack adds are not yet designed.

### 1. Blended stipple: real transparency in place of the checkerboard

Make the existing `stipple` setting work without temporal anti-aliasing,
and let a pack set it per asset with an opacity. The checker flag is per
polygon and such polygons already keep the game's draw order, so
blending them in that order is well defined. Outside temporal
anti-aliasing the default stays `keep`, the original picture. This is
the most asked for across the library.

### 2. Shadows from the game's own light

A shadow map drawn from the light direction the game already uses, then
tested per pixel and multiplied in. The mechanism works and the scene's
light is usable, but the result is not yet worth shipping.

- **The light.** Each object block carries a light direction in view
  space. Most scenery shares one; that is the scene's light, and one
  light per scene holds on the three games tried (E1). Its vector is not
  unit length, and its sign was settled by eye.
- **The casters** are this frame's polygons, taken in view space before
  they are clipped to the screen, so a caster partly behind the camera
  still counts. They need no world space: the map is drawn from the
  light's direction in the same view space the camera uses. Skies, the
  game's own flat shadows, checkerboard polygons and HUD windows do not
  cast. A cut-out texture casts by its alpha, not as a solid rectangle.
- **One window at a time, with windows pooled.** A game's 3D windows
  have separate depth scales and may have separate cameras. Windows with
  the same light and lens share a map, so a fighter drawn in a window of
  its own still casts on the floor.
- **The game's own shadows.** Where the game draws a flat black shadow
  polygon under a car or a fighter, a real shadow would darken it twice.
  Blob shadows are named by model key or found by rule (flat black,
  checker, or near-black cut-out). A game's manifest says whether they
  are hidden when real shadows are on.
- **What it cannot see.** The game does not send what it culled: objects
  it judged out of view (its margin is wider than the picture:
  [scene-understanding.md](scene-understanding.md), E5), and the faces
  turned away from the camera. A shadow map
  made from what remains has holes, so a thing just out of view casts
  nothing and a thin object seen edge-on casts little. The native
  geometrizer already emits back faces and the raster build drops them,
  so collecting casters before that step fills holes cheaply: 72
  lines, adding 24% more shadow on Top Skater, 8% on `vf2` and under 1%
  on `daytona` (E1). What the game's own program never listed stays lost. Replaced
  models help, since the renderer has the whole mesh. A later step could
  ask the native geometrizer for the scene without culling, as it is
  already run a second time for motion vectors.
- **Deep scenes need cascades and a distance limit.** One map loses a
  distant rider's shadow entirely.

**Solved:** cut-out casters, windows with a shared light, back-face
holes. **Blocks shipping:** shadows flicker, because the map cannot be
snapped to a grid when nothing here has world space; casters shadow
themselves in blotches (a bias problem); hiding the game's blob shadows
leaves the ground looking unshaded, because the game's own formula gives
only a pale shadow term; and cascades are needed. Top Skater with two
cascades is the nearest to usable. None is shippable, and no GPU version
exists, so every GPU cost is an estimate (E1, E2).

### 3. Not yet designed: lights a pack adds, and a different light

A pack placing a lamp, or changing the time of day, means adding light.
That collides with the rule above, and the collision is not solved here.
Scenery already clamped at full brightness has no room for more, so a
lighting layer with headroom is needed: the game's sum before the clamp,
which the side channel can give, then some curve to bring the result
back into range. That curve changes the base picture everywhere, not
only under the new light. It is a different picture style, to be
designed and judged as one, not an add-on.

What is known: a lamp can be anchored in a model's own space, as added
elements are, and photo mode already turns the game's light direction
through the game's own formula. What is not known is the curve, and
whether the result still reads as the game.

### Set aside

- **Ambient occlusion.** It darkens creases and contact lines from
  position and normals taken from each polygon's plane, so it works for
  every game and on ground the depth buffer cannot describe, and it
  multiplies the lit colour. Built as real GPU passes it matches the
  CPU result and touches nothing it should not, but a player would not
  see it: it shows only when the difference is multiplied by four, and
  is no clearer at 1080p than at the game's resolution. These games'
  scenes are open ground and convex parts, with few creases to darken.
  It is not cheap: over 10 ms at 1080p. The two passes took 13 to 17 ms
  and the wider ID pass 9 to 20 ms more, against a frame of 17.4 ms. Half
  resolution might bring it to 3 or 4 ms (inferred), still too much for
  an effect nobody sees. A stronger setting, or occlusion only at contact
  with the ground, would be a new idea to try, not this one tuned. If it
  returns, its radius should be in screen pixels, which suited all
  three games: a radius in world units
  cannot be shared, as the games' units differ by about 20 times (E1,
  E2).
- **Global illumination by screen-space methods.** The survey found the
  gain on mostly diffuse arcade scenes small for the cost and the
  artefacts.
- **Ray tracing as a requirement.** wgpu's ray queries are experimental
  and native only. With scenes of a few thousand polygons it may one day
  be the simplest way to get exact shadows and occlusion, and it would
  solve the culling holes only if given unculled geometry.
- **Tone mapping.** The games' output is not high dynamic range
  (`docs/enhancements.md`).

## Interfaces

| Where | What |
|---|---|
| Player and CLI settings | `shadows`, off by default as `bloom` is; the existing `stipple`, extended. `occlusion` only if it is revived |
| `pack.txt` `material` line | `stipple=blend:0.5`, `shadow=none` for an asset that should not cast or receive |
| A game's manifest | Whether to hide its blob shadows; windows that take no lighting (HUD) |
| `scene render --overlay` | `shadow`, `light`: each term alone (`occlusion` if revived) |
| `make lighting-check` | Extended with the new terms, as it checks smooth shading today |

## The roles

- **Player.** Two switches in the Picture page, shadows and stipple.
  Each can be tried and left off.
- **Pack lead.** Chooses whether a pack asks for shadows and which
  assets blend.
- **Art agent.** Sets `stipple=blend` on a shadow texture and reads the
  overlay's numbers to confirm it took.
- **Tool engineer.** Each term is a pass over the stated buffers, with
  its own overlay and its own check.
- **Tester.** With everything off the frame is byte for byte the
  original. With it on, state hashes are unchanged.

## Testing

- **Without ROMs:** a synthetic scene with a known occluder gives a known
  shadow; a cut-out caster shadows only where its alpha is set;
  occlusion, if revived, is zero on a flat plane; blending a checker polygon
  at 50% equals the average of drawing it and not drawing it; off is
  exact.
- **With ROMs:** `lighting-check` on every game's scenes: no pixel
  brighter than the original under occlusion or shadows; hue unchanged;
  software and GPU agree where the software renderer has the term.

## Performance

Occlusion's measured cost is under "Set aside": too much. Shadows have
no GPU version, so their cost is still an estimate. The survey gives
rough costs from other engines, which are estimates for this renderer: a
single shadow map of a small scene under a millisecond at 1080p. A target
to hold: shadows within 2 ms at 1080p on the development GPU, since a
frame has 17.4 ms and the renderer uses 1 to 7 of them today. A shadow
GPU spike with a number comes before it is promised.

## Without a native geometrizer

Blended stipple works, as would occlusion, since they need only polygon
planes and the checker flag. Shadows need a light direction, which the chip
path's scan may give per block (unverified).

## Evidence

Spike code is on the unmerged branch `uplift-spikes-b`, not for merging
as it stands. Entries are in date order.

**E1. Occlusion and shadows on the CPU (2026-11-20).** One frame each of
Top Skater, `daytona` and `vf2`, worked out on the CPU (about 770
lines). No GPU version was made, so every GPU cost is still an estimate.

Ambient occlusion, observed: position and normal come from the plane of
the polygon shown at each pixel, so ground drawn in the game's own order
is no problem: it is 17%, 43% and 27% of the three scenes' 3D pixels.
Sky, HUD, 2D layers, the games' blob shadows and checker polygons stayed
byte for byte the original. It darkens the contact under fighters, the
skater and pillar bases, and did not make flat-shaded facets look worse.
At a 40-pixel radius only 2% to 4% of pixels darken by more than a
tenth, and by eye it is hard to see at the game's resolution. A
screen-pixel radius suited all three scenes. Not tested: judgement at
1080p on a GPU version.

Shadows, observed:

- 99.8% to 100% of pixels share one light direction.
- The native geometrizer already emits back faces; the raster build
  drops them. Collecting casters before that step took 72 lines and added
  24% more shadow on Top Skater, 8% on `vf2` and under 1% on `daytona`.
- Cut-out textures cast solid rectangles: trees and rails shadowed 65%
  of Daytona's road; with them left out, 2.7%.
- `vf2` draws its fighters in a window of their own, so they cast
  nothing on the floor with one map for the frame.
- Top Skater's scene spans about 10,000 units.
- Blob shadows double up; on Daytona a real shadow would fall on 36% of
  the blob's pixels.
- The shadow lands correctly on ground drawn in the game's order.
- On Daytona's frame the light is behind the camera: the shadows fall
  out of view or under the car, where the blob already is. Top Skater is
  the best of the three for this.

**E2. Occlusion on the GPU, shadows again (2026-11-20).** About 1,400
more lines on `uplift-spikes-b`. Occlusion was built as real GPU passes
and shadows were taken further on the CPU. Observed:

- Occlusion: the ID pass carries a polygon number per pixel, a table
  gives each polygon's plane, and two passes (16 samples, then a blur)
  darken the 3D layer before the 2D layers are added. It matches the CPU
  result to 0.002 and changes no pixel of the HUD, sky, 2D layers or
  blob shadows. Looked at before and after at 1080p on all three scenes,
  it shows only when the difference is multiplied by four. At 1080p the
  two passes took 13 to 17 ms and the wider ID pass 9 to 20 ms more,
  most of that on the CPU preparing polygons a second time. Half
  resolution might bring it to 3 or 4 ms (inferred).
- Shadows, blocker by blocker:

  | Blocker | Result |
  |---|---|
  | Cut-outs cast rectangles | Fixed by testing the caster's texture: Daytona's road is 14.8% shadowed, from 66%, and trees throw tree shapes |
  | One map for all windows | Windows with the same light and lens are pooled; `vf2`'s fighters cast on the floor. A decal lying on the floor then shadows itself (94%), a bias problem not fixed |
  | Deep scenes | Top Skater needs two cascades and a distance limit, Daytona four. One map loses the rider's shadow entirely |
  | Blob shadows | Found by rule (flat black, checker, or near-black cut-out). Hiding them leaves the ground looking unshaded, because the game's own formula gives only a pale shadow |
  | Stability, new | 2% to 3% of shadowed pixels flicker over 12 frames on Top Skater and `vf2`; 9% to 25% on Daytona. The map cannot be snapped to a grid, since nothing here has world space |
  | Self-shadowing, new | Blotches on the rider |
  | The light's sign, new | Settled by eye; the evidence on `vf2` is weak |

  Top Skater with two cascades is the nearest to usable. None is
  shippable. No GPU version was made.

## Open questions

- **How bad are the culling holes** in practice? Back faces are
  cheap to fill (E1); what the game never lists is unknown.
- **One light per scene on every game?** It held on three, but blocks can
  carry different directions.
- **Can shadows be steadied without world space?** Flicker over 12
  frames, and the map cannot be snapped to a grid (E2).
- **Can self-shadowing be fixed**, and the pale shadow term made darker
  so blob shadows can be hidden? A decal lying on the floor shadowed
  itself (E2).
- **Is the light's sign right?** Settled by eye; weak evidence on `vf2`
  (E1, E2).
- **Is there a stronger occlusion worth a pass**, such as contact with
  the ground only? Untried (E2).
- **Should a comic look and real shadows combine**, and which goes first?

## MVP: a few days each

1. Blended stipple without temporal anti-aliasing, with a per-asset
   opacity.
2. Shadows, continued on Top Skater, the best case: stability,
   self-shadowing, the pale term and cascades, on a GPU version with a
   measured cost. Occlusion is not in the MVP.
