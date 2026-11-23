# The reModel website

The promo site for reModel: one scroll-driven page in the style of a 1996
arcade operator's sales flyer. Plain HTML, CSS and JavaScript, with GSAP
and ScrollTrigger from a CDN and fonts from Google Fonts; no build step
beyond assembling the files.

## Building it

```sh
make site-shots   # the game screenshots, made from your ROMs (ROM=, default rom/)
make site         # assemble site/ + the posters + the screenshots into out/site
make site-serve   # assemble and serve on http://127.0.0.1:8800
make site-deploy  # publish: out/site becomes the single commit of origin's gh-pages branch
```

The site is served by GitHub Pages from the `gh-pages` branch at
<https://asdfgasfhsn.github.io/reModel/>. `make site-deploy` force-pushes
the assembled site as that branch's only commit each time, so old
screenshots never accumulate in the repository's history.

## Social card

`social.html` is a 1200x630 page; the card the link previews show
(`social.png`, referenced by the `og:image` tag) is a screenshot of it:

```sh
chromium --headless --window-size=1200,630 --screenshot=site/social.png file://$PWD/out/site/social.html
```

## The screenshots the page expects

`shots/seq/{native-px.png,enhanced,bezel,wide,ultra}.webp` (the pinned
sequence; the game's name is `data-game` on `#stage` in `index.html`),
`shots/looks/<style>.webp` (the picture-style wipe; the style list is the
`LOOKS` array in `site.js`, and a style whose image is missing is left
out), `shots/aa/{off,on}.webp`, `shots/link/2p.webp`, `shots/hero.webp` and
`shots/gallery/<set>.webp`. Optional ones hide themselves when missing.

## Pages

`index.html` is the overview (what reModel is, one frame at three window
shapes, a card per page, the download). Behind it: `picture.html` (the
4:3-to-21:9 sequence, picture styles, motion), `link.html` (linked play),
`games.html` (the games and their versions), `packs.html` (texture packs)
and `about.html` (how it works, the specifications). The head, the header
with its navigation and the footer are shared parts, `_head.html`,
`_nav.html` and `_footer.html`: a page line `<!-- include _NAME.html -->` is
replaced by the part when `make site` assembles the site.

Motion stays cheap on purpose: nothing restyles the page on every scroll
frame except the sequence, whose stage is held by `position: sticky` in a
tall track (sized in `svh`, so a phone's toolbar moves nothing) and reads one
rectangle a frame. Reveals and the panels' one-time fades are driven by
IntersectionObserver, and the hero's ticker pauses off screen. GSAP is loaded
only on `picture.html`, for the sequence's timeline.

## What lives where

- `site/`: the page and its original art. Committed.
- `assets/posters/`: the game posters (original art, see their README),
  copied into `posters/` by `make site`.
- `out/site-shots/`: the game screenshots. They are taken from the games
  themselves, so they are made locally from your own ROMs by
  `make site-shots` (the list is in `tools/site-shots.sh`) and never
  committed. The captures run with the player's default settings, not
  your saved ones, so a saved picture style can't leak into them.
- `out/site/`: the assembled site (`tools/site-build.sh`). Screenshots
  become WebP, except `*-px.png`, which stay PNG so native pixels stay
  sharp.

## Art

The story art in `site/art/` is original, made with the Codex CLI's image
generation (generated at about 2048 wide and resized to 1920-wide WebP at
quality 82 with ImageMagick). Like the posters it contains no text,
logos, brands, game characters or artwork from the games, their cabinets
or their flyers. Each image's prompt was:

> Generate an image: <subject> <style> Save it as <name>.png in the
> current directory. Reply with the file path only.

Shared style (`board-reborn`, `link-up`):

> Style: 1990s arcade flyer art in the spirit of mid-1990s 3D arcade
> polygon graphics: flat-shaded low-poly models with light texturing, deep
> black background, cyan neon wireframe accents, bold accents in white,
> electric blue and red, cinematic, crisp and vivid. Absolutely no text,
> letters, numbers, logos, brand marks or signage anywhere in the image,
> including on cabinets, screens and marquees. Original generic subject,
> not based on any existing game, character, cabinet design or flyer.

- `board-reborn.webp`: Wide 16:9 landscape, 2048x1152. The same kind of
  generic arcade circuit board on black dissolving into a glowing cyan
  neon wireframe: the left side physical green board with chips, the right
  side the board reconstructed as luminous cyan wire-frame polygons and
  light-trail circuit paths flowing toward the right edge, particles of
  light lifting off, a sense of rebirth. No legible markings. (+ style)
- `link-up.webp`: Wide 21:9 ultrawide landscape, 2688x1152. Four generic
  unbranded sit-down driving arcade cabinets side by side in a row,
  connected by glowing cyan light cables running along the floor between
  them, each screen showing an abstract low-poly racing scene in a
  different colour, seen from a low dramatic three-quarter angle, deep
  black void around them with a cyan neon wireframe grid floor stretching
  to the horizon. No people. (+ style)
- `arcade-night.webp` (its own style text): Wide 16:9 landscape,
  2048x1152. A dim mid-1990s video arcade at night seen from the entrance:
  two long rows of generic unbranded upright and sit-down arcade cabinets
  receding into darkness, their blank glowing screens casting cool cyan
  and electric blue light, a pair of linked sit-down driving cabinets with
  red seats in the foreground, a coin changer silhouette, faint haze,
  reflections on a glossy dark floor. Colour palette strictly: deep black,
  cyan, electric blue, white and bold red accents; NO magenta, NO pink, NO
  purple. Style: mid-1990s 3D arcade polygon graphics, flat-shaded
  low-poly models, cinematic, crisp. Cabinet marquees are plain glowing
  white or blue panels with no writing. Absolutely no text, letters,
  numbers, logos, brand marks or signage anywhere. Original generic
  subject.
- `dying-board.webp` (its own style text): Wide 16:9 landscape,
  2048x1152. Pure macro product photograph, nothing else in frame: an
  aging generic green circuit board filling the whole frame, rows of
  unmarked black chips, bulging leaking capacitors with brown residue, a
  corroded copper trace, dust, a single thin wisp of smoke rising, one
  tiny red LED glowing, dramatic low raking cyan and magenta light from
  the side, the far edge fading into pure black, shallow depth of field.
  Photographic, moody, still life. Absolutely NO people, NO characters, NO
  figures, NO vehicles, NO robots, NO weapons, NO landscape or background
  scene, NO text, letters, numbers, logos or markings.

Effects are our own code, some inspired by techniques shown on
[React Bits](https://reactbits.dev) (no code from it is used).
