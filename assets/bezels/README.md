# Bezels

Cabinet-style artwork drawn beside a game's 4:3 picture when the window is
wider than it, in the native player (`docs/play.md`) and the browser
(`docs/web.md`), one per ROM set. Each suits its game's theme, like its
poster (`assets/posters/README.md`), in the same series look.

**Format.** WebP, 1080 pixels tall: a plain black 1440x1080 middle, where
the picture goes (it covers it), between two side panels about 275 pixels
wide, so each is about 1990x1080 (a little wider than 16:9). The art is
scaled to the picture's height and centred on it; a wider window shows
black beyond it, and a window 4:3 or taller has no room for it. The
player embeds every file here at build time (`crates/play/build.rs`); the
browser build copies them to `out/web/bezels/` and `games.json` names each
game's (`remodel web-index`). Adding a game's bezel is adding its file.

**Rules.** As for the posters: original images with no text, letters,
numbers, logos, brands, insignia or game characters (no people at all
here), and nothing from the games, their cabinets or their flyers.

**Provenance.** Made with Codex CLI image generation (`codex exec`,
generated at 1672x941). Each side panel, up to the plain opening the
image leaves in its middle, was scaled to 1080 tall (squeezed to at most
276 wide), put either side of a black 1440x1080 middle and saved as WebP at
quality 80 with ImageMagick: 55-105 KB each.

## Prompts

Each image's prompt was the shared text with the game's theme in it:

> Generate an image: Wide landscape 16:9 image (generate it as wide as you
> can, e.g. 1536x1024 or wider). It shows the front of a 1990s arcade
> cabinet's monitor surround (bezel), seen perfectly straight on, flat,
> symmetrical left to right, filling the whole image. The central
> rectangle, covering the middle three quarters of the width and the full
> height, is the screen opening: plain flat pure black, completely empty.
> All decoration is in the left and right side panels outside it and a
> thin moulded trim around the opening. Theme: <theme>. Style: 1990s arcade
> flyer art in the spirit of mid-1990s 3D arcade polygon graphics:
> flat-shaded low-poly shapes, deep black background, cyan neon wireframe
> grid, bold accents in white, electric blue and red, crisp and vivid.
> Absolutely no text, letters, numbers, logos, brand marks, signage or
> people anywhere in the image. Original generic design, not based on any
> existing cabinet, game or artwork. Save it as <set>.png in the current
> directory. Reply with the file path only.

Themes:

- `topskatr.webp` (Top Skater): night-time urban skate park: the side panels show low-poly concrete quarter pipes, handrails and ledges, cyan neon wireframe city blocks, spray-paint style abstract shapes (no letters), with chrome trim and glowing blue and red neon tube accents along the edges
- `skisuprg.webp` (Sega Ski Super G): alpine downhill ski race: the side panels show steep low-poly faceted white and pale blue snow slopes, red and blue slalom gate flags, low-poly pine trees dusted with snow, sprays of powder snow, cyan wireframe mountain peaks, with brushed aluminium trim and icy blue and white neon tube accents along the edges
- `stcc.webp` (Sega Touring Car Championship): touring car racing at dusk: the side panels show a low-poly race circuit corner with red and white kerbs, stacked tyre barriers, a chequered pattern band, an orange-to-purple dusk sky, motion streaks, cyan wireframe hills, with chrome trim and red and white neon tube accents along the edges
- `segawski.webp` (Sega Water Ski): water skiing on a lake at sunset: the side panels show low-poly faceted deep blue lake water, tall white curtains of spray, a low-poly wooden jump ramp, floating orange slalom buoys, an orange and magenta sunset sky, cyan wireframe mountains, with chrome trim and orange and blue neon tube accents along the edges
- `waverunr.webp` (Wave Runner): tropical personal watercraft racing: the side panels show low-poly faceted turquoise sea, white wave crests and spray, low-poly islands with palm trees, floating red and yellow course buoys, a bright sun, a cyan wireframe grid under the water horizon, with chrome trim and turquoise and red neon tube accents along the edges
- `overrev.webp` (Over Rev): street racing on a city expressway at night: the side panels show low-poly skyscrapers with lit windows, elevated expressway ramps, a low-poly suspension bridge, long red tail-light and white headlight streaks, a generic unbranded silver two-door coupe with plain bodywork drifting in white tyre smoke, its shape simple and invented, not resembling any real car model, with chrome trim and magenta and blue neon tube accents along the edges. (The first attempt, without the coupe's description, drew a red car too close to a real model, so it was made again.)
- `hotd.webp` (The House of the Dead): haunted mansion horror: the side panels show a moody low-poly gothic mansion on a hill at night, bare twisted low-poly trees, a wrought iron fence and gate, a pale full moon behind thin clouds, a few windows glowing dim orange, carved weathered stone pillars as the frame, with dim red and pale green glow accents; no people, no creatures, no skulls, no gore
- `bel.webp` (Behind Enemy Lines): military vehicle combat in a forest at dusk: the side panels show riveted olive-drab steel plates, camouflage netting, low-poly pine forest, wooden supply crates, fuel drums, sandbags, orange explosion flashes among the trees, with hazard-stripe trim in black and yellow and amber warning-lamp accents; no insignia, stars, flags, markings or numbers
- `sgt24h.webp` (Super GT 24h): 24 hour endurance sports car racing at night: the side panels show a floodlit low-poly circuit straight, a row of brightly lit open pit garages, a tall low-poly grandstand, glowing orange brake discs, long headlight streaks, the first pale blue light of dawn behind cyan wireframe hills, with chrome trim and yellow and blue neon tube accents along the edges
- `dynabb97.webp` (Dynamite Baseball '97): night baseball: the side panels show a low-poly night stadium with towering floodlight banks, a sweeping tiered grandstand, an outfield of faceted green stripes, a white baseball streaking upward with a motion trail, crossed wooden bats, cyan wireframe hills beyond the stadium rim, with chrome trim and white and red neon tube accents along the edges
- `doa.webp` (Dead or Alive): martial arts tournament: the side panels show a raised low-poly stone fighting ring with glowing blue edges, a low-poly red temple gate, paper lanterns without writing, falling pink blossom petals, orange energy blasts erupting from low-poly floor tiles, cyan wireframe mountains, with lacquered red and gold trim and blue neon tube accents along the edges; no people
- `indy500.webp` (Indianapolis 500): open-wheel oval speedway racing on a sunny afternoon: the side panels show a steeply banked low-poly oval track turn with a white concrete retaining wall and catch fence, a long tiered grandstand, a generic unbranded open-wheel single-seater race car with exposed wheels, a low nose, front and rear wings and plain solid metallic teal bodywork with no stripes, numbers or decals, its shape simple and invented, not resembling any real car, make or team livery, speeding through the banking with motion streaks, a chequered pattern band, cyan wireframe sky grid above, with chrome trim and blue and orange neon tube accents along the edges. (The first attempt asked for plain solid colour bodywork and drew an all-red single-seater too close to a well-known team's livery, so it was made again in teal.)
- `fvipers.webp` (Fighting Vipers): caged street-fighting arena on a city rooftop at night: the side panels show tall steel cage fencing with dented riveted metal wall panels, one panel bursting apart in sparks and flying twisted bars, glaring floodlights, low-poly rooftops and water towers, abstract spray-paint splashes with no letters, with diamond-plate steel trim and red and blue neon tube accents along the edges; no people
- `schamp.webp` (Sonic Championship): cartoon tropical island fighting tournament: the side panels show bright low-poly palm trees, a tumbling waterfall, faceted turquoise sea, the corner posts and thick red and blue ropes of a boxing ring, floating faceted crystal gems in bright colours, puffy white clouds and cartoon starbursts, with glossy yellow and red trim and blue neon tube accents along the edges; no people, no animals, no gold rings, no checkerboard patterns
- `vstriker.webp` (Virtua Striker): football (soccer) under floodlights at night: the side panels show towering low-poly stadium floodlight banks, a sweeping tiered grandstand, a pitch of faceted green stripes with white lines, a white goal frame with its net, a faceted black and white football streaking upward with a motion trail, cyan wireframe hills beyond the stadium rim, with chrome trim and green and blue neon tube accents along the edges; no people, no flags, banners, scarves or national colours
- `lastbrnx.webp` (Last Bronx): street fighting with weapons at dusk in a city: the side panels show tall low-poly skyscrapers with lit windows, a low-poly traffic light, painted white crosswalk stripes, a plain wooden staff and a plain short metal baton crossed like swords with a bright white clash flash, orange and purple dusk sky, cyan wireframe skyline, with dented riveted steel trim and red and blue neon tube accents along the edges; no people, no signs or lettering
- `srallyc.webp` (Sega Rally Championship): rally racing on a dirt road through a forest: the side panels show a low-poly faceted tan dirt road with tyre tracks, drifting dust and flying gravel, low-poly pine trees and brown rock cuttings, wooden barrier posts, hay bales, a generic unbranded rally car with plain solid white bodywork and a boxy shape, simple and invented, not resembling any real car model, sliding through a bend with a plume of dust, a blue sky with puffy white clouds, cyan wireframe hills, with chrome trim and orange and blue neon tube accents along the edges. (Each panel is the image's outer 262 pixels, squeezed to 276 wide.)
- `overrevb` (Over Rev, Model 2B) has no file of its own: a clone (a
  manifest with a `parent`) uses its parent's bezel, as with posters.
