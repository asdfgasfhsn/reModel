# Game-card posters

Poster art for the browser landing page's game cards, one per ROM set, plus
`placeholder.webp` for games without one.

**Art direction.** One series look that matches `assets/reModel-logo.png`:
black background; bold white, blue and red; cyan neon wireframe grid and
mountains; a 1990s arcade-flyer feel with flat-shaded, low-poly figures and
vehicles like Model 2-era 3D. Portrait 2:3, no text at all (the page renders
the title), and only generic subjects: no logos, brands, liveries, numbers or
game characters.

**Provenance.** Made with Codex CLI image generation (`codex exec`, generated
at 1024x1536 and resized to 600x900 WebP at quality 85 with ImageMagick).
They are original images and contain no artwork from the games, their
cabinets or their flyers.

## Prompts

Each image's prompt was its subject line followed by the shared style text:

> Generate an image: <subject> <style> Save it as <set>.png in the current
> directory. Reply with the file path only.

Style (shared):

> Portrait 2:3 poster, 1024x1536. Style: 1990s arcade flyer art in the spirit of mid-1990s 3D arcade polygon graphics: flat-shaded low-poly models with light texturing, deep black background, a cyan neon wireframe grid floor and cyan wireframe mountains on the horizon, bold accents in white, electric blue and red, dynamic diagonal composition, crisp and vivid. Absolutely no text, letters, numbers, logos, brand marks, sponsor decals or signage anywhere in the image, including on clothing, boards and vehicles. Original generic subject, not based on any existing game, character or flyer.

- `topskatr.webp` (Top Skater): A skateboarder in mid-air doing a kickflip high above a low-poly urban skate park at night, with quarter pipes, a handrail and concrete ledges, neon cyan wireframe city blocks in the background, plain unmarked clothing and a plain unmarked board.
- `skisuprg.webp` (Sega Ski Super G): A downhill ski racer in a tuck, carving hard past red and blue slalom gate flags on a steep low-poly snow mountain in bright daylight, spray of snow behind, faceted white and pale blue snow slopes, cyan wireframe mountain peaks in the distance against a black sky band at the top, plain unmarked race suit.
- `stcc.webp` (Sega Touring Car Championship): Two generic unbranded four-door touring cars, one white with blue stripes and one red with white stripes, racing door to door through a corner of a low-poly race circuit at dusk, red and white kerbs, sparks from the tarmac, orange-to-purple dusk sky fading into black with cyan wireframe hills, completely plain bodywork with no numbers, decals or badges.
- `waverunr.webp` (Wave Runner): A rider on a sit-down personal watercraft carving a hard turn and throwing a big arc of white spray across a low-poly faceted turquoise tropical sea, low-poly islands with palm trees, a cyan wireframe grid glowing under the water horizon, bright sun, plain unmarked watercraft hull and plain clothing.
- `segawski.webp` (Sega Water Ski): A water skier on a single slalom ski leaning hard into a cut, gripping a tow handle with the tow rope stretching off the frame edge, a tall white curtain of spray, low-poly faceted deep blue lake water at sunset with orange and magenta sky, a low-poly wooden jump ramp nearby, cyan wireframe mountains on the horizon, plain unmarked wetsuit and ski. No boat visible.
- `overrev.webp` (Over Rev): A generic unbranded silver two-door sports coupe drifting sideways through a tight hairpin on a low-poly elevated city expressway at night, white tyre smoke pouring off the rear wheels, a second generic coupe close behind leaving long red tail-light and white headlight streaks, low-poly skyscrapers with lit windows and a low-poly suspension bridge in the distance, completely plain bodywork with no numbers, decals or badges.
- `hotd.webp` (The House of the Dead): A moody low-poly haunted mansion on a hill at night, with dark gabled roofs, a crooked tower and a few windows glowing dim orange, bare twisted low-poly trees and an iron gate in the foreground, a pale full moon behind thin clouds, and a bright white-and-orange light-gun muzzle flash bursting at the lower edge of the frame with a thin red circular crosshair over one glowing window; no people, no creatures, no gore.
- `bel.webp` (Behind Enemy Lines): A generic unmarked open-top military jeep with a mounted gun on a roll bar, crew seen from behind as small faceless low-poly figures, driving fast down a dirt track through a low-poly pine forest at dusk, a wooden supply crate and a fuel drum by the roadside, an orange explosion flash among the trees ahead, a plain olive-drab vehicle with no markings, stars, insignia or numbers.
- `overrevb` (Over Rev, Model 2B) has no file of its own: it is the same
  game as `overrev`, so its card shows `overrev.webp`. A clone (a manifest
  with a `parent`) uses its parent's poster (`remodel web-index`); give it
  its own file here only if it should look different.
- `placeholder.webp` (games without a poster): An empty scene with no people or vehicles: a glowing cyan neon wireframe grid floor receding to the horizon, cyan wireframe low-poly mountains, a few floating flat-shaded low-poly geometric shapes (a cube, a pyramid, an icosahedron) in white, blue and red, a red horizon stripe and a blue gradient glow, deep black sky with sparse stars.
