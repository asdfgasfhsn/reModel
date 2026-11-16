# Controller art

The pictures on the player's Controls page (`crates/play/src/menu/figure.rs`):
a gamepad (two layouts), a light gun, and a wheel with pedals. Original
images with no text, logos or brand marks, and not modelled on any real
product.

- `pad-1.webp`: a gamepad with offset sticks (an Xbox-style layout), shown
  for any pad that isn't PlayStation-like.
- `pad-2.webp`: a gamepad with its sticks side by side (a PlayStation-style
  layout), shown when gilrs's name or vendor for the connected pad looks
  like Sony's.
- `art-G.webp`: a light gun, side view, for light-gun games.
- `art-W.webp`: a wheel and pedals, for driving games.

**Provenance.** Made with Codex CLI image generation (`codex exec`,
1536x1024 landscape, 2026-11-16), then resized to 1024 wide and saved as
WebP (quality 90) with ImageMagick: about 25 to 32 kB each. They are drawn
on pure black; the player treats a pixel's brightness as its opacity, so the
black is see-through and the pictures sit on the page's cards. Where each
button, stick, trigger and pedal is was measured on the 1536x1024 originals
and is kept in `figure.rs` as fractions of the picture.

## Prompts

`pad-1`:

> Generate an image: A generic modern twin-stick gamepad: two analog sticks in an offset layout (left stick upper left, right stick lower right), a plain plus-shaped d-pad lower left, four plain round face buttons in a diamond upper right, two small plain oval menu buttons in the centre, and the two shoulder bumpers and two triggers visible along the top edge. Flat orthographic front view, perfectly symmetrical left to right, the whole controller centred with generous margin on a pure flat black background (#000000), 1536x1024 landscape. Crisp flat vector illustration with soft shading: dark graphite body, slightly lighter graphite buttons, a thin cyan neon edge highlight around the body outline, subtle electric-blue accent. All buttons plain and blank. Absolutely no text, letters, numbers, symbols, icons, logos or brand marks anywhere, not on the buttons either. A generic original design, not resembling any specific real controller. Save it as pad-1.png in the current directory. Reply with the file path only.

`pad-2`:

> Generate an image: A generic modern twin-stick gamepad: two analog sticks side by side in the lower centre (symmetric layout), a plain plus-shaped d-pad upper left, four plain round face buttons in a diamond upper right, two small plain oval menu buttons in the centre, a plain flat touch-pad-free centre, and the two shoulder bumpers and two triggers visible along the top edge. Flat orthographic front view, perfectly symmetrical left to right, the whole controller centred with generous margin on a pure flat black background (#000000), 1536x1024 landscape. Crisp flat vector illustration with soft shading: dark graphite body, slightly lighter graphite buttons, a thin cyan neon edge highlight around the body outline, subtle electric-blue accent. All buttons plain and blank. Absolutely no text, letters, numbers, symbols, icons, logos or brand marks anywhere, not on the buttons either. A generic original design, not resembling any specific real controller. Save it as pad-2.png in the current directory. Reply with the file path only.

`art-G`:

> Generate an image: A generic arcade-style light gun controller seen exactly from the side (profile view), pointing right: a chunky pistol-grip body with a plain trigger, a plain round thumb button on top behind the barrel, and a plain button on the side of the grip, no cable. Not a real firearm: toy-like rounded controller shapes. Centred with generous margin on a pure flat black background (#000000), 1536x1024 landscape. Crisp flat vector illustration with soft shading: dark graphite body, slightly lighter graphite buttons, a thin cyan neon edge highlight around the outline, subtle electric-blue accent. All buttons plain and blank. Absolutely no text, letters, numbers, symbols, icons, logos or brand marks anywhere. A generic original design, not resembling any specific real product. Save it as art-G.png in the current directory. Reply with the file path only.

`art-W`:

> Generate an image: A generic racing wheel controller seen from the front, slightly from above: a round steering wheel with two plain paddle shifters behind it and four plain round buttons on its spokes, on a compact base, with a small two-pedal set (accelerator and brake) placed below it. Centred with generous margin on a pure flat black background (#000000), 1536x1024 landscape. Crisp flat vector illustration with soft shading: dark graphite body, slightly lighter graphite buttons, a thin cyan neon edge highlight around the outline, subtle electric-blue accent. All buttons plain and blank. Absolutely no text, letters, numbers, symbols, icons, logos or brand marks anywhere. A generic original design, not resembling any specific real product. Save it as art-W.png in the current directory. Reply with the file path only.
