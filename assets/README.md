# Artwork

- `reModel-logo.png`, `reModel-logo-1086.png`: the logo (the player's
  splash and launcher, the browser pages).
- `reModel-icon-1024.png`: the app icon, 1024x1024 on Apple's icon grid
  (an 824-pixel rounded square, corner radius 185, with a soft shadow, on
  a transparent canvas): the macOS `.app`'s `remodel.icns` (`make app`).
- `reModel-icon.png`: the same at 512x512: the Linux AppImage and `make
  install-desktop`'s menu entry.
- `posters/`: the launcher's game cards (`posters/README.md`).
- `bezels/`: the browser build's cabinet bezels.

**The icon's provenance.** Made with Codex CLI image generation (`codex
exec`, 1024x1024), then cut to the rounded square with its shadow with
ImageMagick. An original image: no text, logos or brand marks. Prompt:

> Generate an image: A glowing cyan neon wireframe grid floor in strong
> perspective filling the lower half, with a large faceted electric-blue
> and white low-poly mountain peak in the centre whose edges are outlined
> in cyan neon, a red sun disc with horizontal black slits low behind the
> peak, and three short white speed stripes on the left. Square app icon
> artwork, 1024x1024, full bleed edge to edge (no rounded corners, no
> border, no frame, no drop shadow; the corners will be masked later, so
> keep the subject inside the central 80%). Style matching a 1990s arcade
> logo: deep black background, a cyan neon wireframe perspective grid
> floor and cyan neon wireframe low-poly mountain peaks, bold accents in
> white, electric blue and red, crisp flat vector-like rendering with
> clean thick shapes that stay readable at 32x32 pixels. Absolutely no
> text, letters, numbers, logos or brand marks. Original, not based on any
> existing game, console or company icon. Save it as icon-C.png in the
> current directory. Reply with the file path only.
