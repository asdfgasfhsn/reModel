# Lighting

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. It uses the buffers in
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
it; it never adds on top.** Occlusion multiplies the picture, so it
cannot double-count shading the game already baked in and cannot push
past the clamp. Anything that would add light first re-evaluates the
game's own formula with headroom.

Three features to build, in the order they should be tried, and two
that are not yet designed.

### 1. Ambient occlusion

Darken creases and contact lines from position and normals. Both come
from each polygon's plane, so this works for every game, including those
without the lighting side channel, and on ground the depth buffer cannot
describe. It multiplies the lit colour. It
is the cheapest way to make objects sit on the ground.

### 2. Shadows from the game's own light

A shadow map drawn from the light direction the game already uses, then
tested per pixel and multiplied in.

- **The light.** Each object block carries a light direction in view
  space. Most scenery shares one; that is the scene's light.
- **The casters** are this frame's polygons, taken in view space before
  they are clipped to the screen, so a caster partly behind the camera
  still counts. They need no world space: the map is drawn from the
  light's direction in the same view space the camera uses. Skies, the
  game's own flat shadows, checkerboard polygons and HUD windows do not
  cast.
- **One window at a time.** A game's 3D windows have separate depth
  scales and may have separate cameras; each is shadowed on its own.
- **The game's own shadows.** Where the game draws a flat black shadow
  polygon under a car or a fighter, a real shadow would darken it twice.
  A game's manifest says whether its blob shadows are hidden when real
  shadows are on.
- **What it cannot see.** The game does not send what it culled: objects
  off screen, and the faces turned away from the camera. A shadow map
  made from what remains has holes, so a thing just out of view casts
  nothing and a thin object seen edge-on casts little. Replaced models
  help, since the renderer has the whole mesh. A later step could ask the
  native geometrizer for the scene without culling, as it is already run
  a second time for motion vectors.

### What the spike found (2026-11-20)

One frame each of Top Skater, `daytona` and `vf2`, worked out on the
CPU (branch `uplift-spikes-b`, about 770 lines). No GPU version was
made, so every GPU cost here is still an estimate.

**Ambient occlusion: safe, and faint.**

- Position and normal come from the plane of the polygon shown at each
  pixel, so ground drawn in the game's own order is no problem: it is
  17%, 43% and 27% of the three scenes' 3D pixels.
- Sky, HUD, 2D layers, the games' blob shadows and checker polygons
  stayed byte for byte the original.
- It darkens the contact under fighters, the skater and pillar bases,
  and did not make flat-shaded facets look worse.
- At a 40-pixel radius only 2% to 4% of pixels darken by more than a
  tenth. By eye it is hard to see at the game's resolution. Whether it
  is worth a pass is a judgement to make on a GPU version at 1080p.
- A radius in world units cannot be shared: the games' units differ by
  about 20 times. A radius in screen pixels suited all three.

**Shadows: not yet.** The mechanism works and the scene's light is
usable, but four things stand between it and a picture worth shipping.

- **One light per scene holds**: 99.8% to 100% of pixels share one
  direction. Its vector is not unit length, and its sign was settled by
  eye.
- **The culling holes are smaller than feared, and cheap to fill.** The
  native geometrizer already emits back faces; the raster build drops
  them. Collecting casters before that step took 72 lines and added 24%
  more shadow on Top Skater, 8% on `vf2` and under 1% on `daytona`. What
  the game's own program never listed stays lost.
- **Cut-out textures cast solid rectangles.** Trees and rails shadowed
  65% of Daytona's road; with them left out, 2.7%. The map must test
  the texture's alpha.
- **One map for the frame is wrong.** `vf2` draws its fighters in a
  window of their own, so they cast nothing on the floor; maps must be
  per window, and a fighter's shadow must cross windows.
- **Scenes are deep.** Top Skater's spans about 10,000 units, so the map
  needs a distance limit or cascades.
- **Blob shadows do double up**, and can be named by model key or found
  as flat black or checker polygons. On Daytona a real shadow would fall
  on 36% of the blob's pixels.
- The shadow lands correctly on ground drawn in the game's order.
- On Daytona's frame the light is behind the camera: the shadows fall
  out of view or under the car, where the blob already is. Top Skater is
  the best of the three for this.

### 3. Real transparency in place of the checkerboard

Make the existing `stipple` setting work without temporal anti-aliasing,
and let a pack set it per asset with an opacity. The checker flag is per
polygon and such polygons already keep the game's draw order, so
blending them in that order is well defined. Outside temporal
anti-aliasing the default stays `keep`, the original picture. This is the smallest of the five and
the one most asked for across the library.

### Not yet designed: lights a pack adds, and a different light

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

### Not proposed

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
| Player and CLI settings | `occlusion` and `shadows`, off by default as `bloom` is; the existing `stipple`, extended |
| `pack.txt` `material` line | `stipple=blend:0.5`, `shadow=none` for an asset that should not cast or receive |
| A game's manifest | Whether to hide its blob shadows; windows that take no lighting (HUD) |
| `scene render --overlay` | `occlusion`, `shadow`, `light`: each term alone |
| `make lighting-check` | Extended with the new terms, as it checks smooth shading today |

## The roles

- **Player.** Three switches in the Picture page. Each can be tried and
  left off.
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
  shadow; occlusion is zero on a flat plane; blending a checker polygon
  at 50% equals the average of drawing it and not drawing it; off is
  exact.
- **With ROMs:** `lighting-check` on every game's scenes: no pixel
  brighter than the original under occlusion or shadows; hue unchanged;
  software and GPU agree where the software renderer has the term.

## Performance

Nothing here is measured. The survey gives rough costs from other
engines, which are estimates for this renderer: a single shadow map of a
small scene under a millisecond at 1080p, and ambient occlusion at half
resolution in the range of 0.5 to 2 ms. A target to hold: occlusion and
shadows together within 2 ms at 1080p on the development GPU, since a
frame has 17.4 ms and the renderer uses 1 to 7 of them today. Each needs
a spike with a number before it is promised.

## Without a native geometrizer

Occlusion and blended stipple work, since they need only polygon planes
and the checker flag. Shadows need a light direction, which the chip
path's scan may give per block (unverified).

## Open questions

- **How bad are the culling holes** in practice? Less than feared for
  back faces (see the spike); unknown for what the game never lists.
- **One light per scene, or several?** Blocks can carry different
  directions.
- **Do occlusion and the game's flat shading fight?** Faceted surfaces
  may show their facets more.
- **Should a comic look and real shadows combine**, and which goes first?

## MVP: a few days each

1. Blended stipple without temporal anti-aliasing, with a per-asset
   opacity.
2. Ambient occlusion as a GPU pass, radius in screen pixels, judged at
   1080p before it is promised (spiked on the CPU: safe but faint).
3. Shadows wait for four things the spike named: alpha-tested casters,
   a map per window, a distance limit, and blob shadows named in the
   manifest. Casters are taken before the raster build's culls.
