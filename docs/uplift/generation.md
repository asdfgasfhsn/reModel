# Making art with any image model

Shared words, roles and rules: [README.md](README.md). Nothing is built
into reModel; what was tried is in "Evidence" (spike code is on unmerged
branches, not for merging as it stands). The batch runner that exists is
`studio draft` ([../uplift-studio.md](../uplift-studio.md)).

## The problem

Image models are good at reworking a picture in a style and unreliable at
everything a texture needs to be exact about.

**Observed in the showcase** (one model, through the Codex CLI).

- **Worked:** banners, logos, portraits, and character skins kept in their
  original UV layout, given the original, a style sheet and a brand list.
- **Sizes vary:** 1672x941 one time, 1920x1080 the next.
- **No seamless tiles and no exact alpha.** Ground, walls and lanes ended
  up made locally.
- **Designs drift:** a cap's "K" became "P"; shirts and shoes changed
  between redraws.
- **Refusals:** two of six calls refused because the prompt said to
  redraw the publisher's original artwork.
- **Cost:** 1 to 2 minutes an image and about 20,000 tokens of set-up
  each. Output lands in the working directory. One flag swallowed the
  prompt.

**Observed in the code.**

- `studio draft` runs a command template with the prompt on standard
  input, retries twice, and times out at 900 s
  (`crates/cli/src/studio.rs:1559-1614`). Success is "exit 0 and a PNG
  that decodes". The result is centre-cropped to the original's aspect and
  resized.
- It does not carry the original's alpha over, check for borders, or tell
  a refusal from a timeout.
- It records the command's first word, the brief, the date and a prompt
  file: no model version, seed or input hash.
- **It was mostly bypassed.** In the showcase's ledger
  (`out/topskatr/studio/cel-pack4/status/status.txt`, a local file) 30 of
  6,496 entries are generated and 5,591 are local recipes; the rest are
  typeset text, variants and imports. Most generation ran through
  hand-written scripts.

**What other sources say** (vendor documentation and model cards; some
pages read only in part). No hosted model produces a 64-pixel texture
natively. Mask-based editing may not keep to the mask. No vendor
guarantees consistency across a set. Tiling can be verified and repaired
but not requested. Resizing invalidates any content credential the
service attached.

So every guarantee has to be made after generation, by reModel.

## Design

`asset make ID --with BACKEND` runs five steps. The model only does the
second.

### 1. Prepare

From the asset's bundle: the original enlarged, the visible-texel mask,
the references (style sheet, a logo, approved neighbours), one context
crop, and the prompt built from the brief and the style rules.

- **Fit the backend's sizes.** The input is letterboxed to the nearest
  size the backend accepts, and the letterbox is undone afterwards from
  the rectangle recorded when it was made, so the UV layout survives.
  Bars are never stripped by colour. Cropping, as today, does not
  survive (E2).
- **Show cut-outs as a flat matte colour** (magenta in E2, where the
  model kept it flat on 3 of 3).
- **Lint the prompt.** A list of known refusal triggers is checked before
  spending a call: a publisher's name, a real brand, "redraw X's artwork".
  The lint proposes wording that describes the task as restyling the
  supplied image. The word "redraw" on its own belongs on the list (E2,
  E3), and the lint blocks the call, not only prints a warning: in E2 it
  flagged all ten prompts and the calls went ahead.

### 2. Generate

A backend is a command template, as today, plus what it can do:

```ini
[backend codex]
command  = codex exec {images} -
sizes    = 1024x1024 1536x1024 1024x1536
refs     = 4
alpha    = no
remote   = yes        # images leave this machine
parallel = 4
```

Backends are listed in the project's `backends.txt`. They can be an image
CLI, a script around any API, a local model, the existing upscaler path,
`recipe` (the local style processor with the project's `recipes.txt`),
or `hand`, which opens the bundle folder and
waits for a person to save a file. A `remote` backend asks once before
any game image is sent.

### 3. Normalise: the guarantees

All deterministic, all in reModel, no Python or ImageMagick needed.

| Guarantee | How |
|---|---|
| Exact size and aspect | Undo the letterbox from the recorded rectangle, resample to the asset's size times its scale. Bars and a trim of up to a tenth can be repaired; a stretched or recomposed image cannot be told apart reliably and is made again (E1) |
| Alpha | Take the original's cut-out, scaled by the renderer's own 0.5 rule, as a binary alpha (a smoothed one scored worse, E1). The generator's alpha is never trusted for a cut-out |
| Fit ("spill") | The share of drawn (non-background) pixels that fall where the original is cut out. Alpha from the original passes `alpha` by construction, so this test catches a subject drawn too large for the cut-out. First limits, from four real cases: up to 0.06 fits; 0.06 to 0.20 goes to a person; over 0.20 is `refit` (E1) |
| Only visible texels change | Composite over the mask; outside it keep the recipe's result or the original; pad island edges |
| Palette | Optional per category: snap to the style's palette in a perceptual colour space |
| Seamless | For each repeated axis: measure the seam first. Only if the seam gate fails, roll by half, repair the seam band and measure again. A repair on a tile that already passes can make it worse (E2). The `seamless` levelling in `crates/cli/src/studio_img.rs:417` is the start |
| Overall colour | Optional. For restyled art: one gain per channel that brings the mean colour back to the original's. For faithful upscales: the original's low-frequency colour, as the upscaler path does (`docs/enhancements.md`, "Batch texture upscaling"). The upscaler's rule spoils restyled flat art (E3) |

The registration score is not a gate: it read 0.02 to
0.15 on images that were right (E2).

### 4. Check and classify

Run the asset gates ([quality-gates.md](quality-gates.md)) and give one
outcome:

| Outcome | Next |
|---|---|
| `ok` | Written to `draft/<slug>.png`; the asset becomes `checked`, since the gates have run |
| `repaired` | The image passed only after a stated repair, named in the record |
| `refit` | The subject does not fit the original's cut-out (the fit test), or the aspect is wrong: make it again, do not ship a repair |
| `gate:<name>` | Retry, with the gate's fix line added to the prompt, up to a limit |
| `refused` | One retry with the lint's wording, then report. A refusal took 5 s in E2 and a reworded call went through (E3) |
| `timeout`, `rate-limited` | Retry later |
| `bad-output` | No image, a mockup with a border, or the wrong subject |

Passing the gates is not the same as being right: only the fit test
catches a subject that the cut-out chops up (E1). No gate in the first
set looks at colour (E2). Repeating surfaces are not worth a model:
"keep every shape in place" means nothing for a noise texture (E2).

### 5. Record

A provenance file per image, written by the tool and never by hand
([pack-project.md](pack-project.md)): backend,
model and version where the backend reports it, prompt, input hashes, the
post-steps applied, the gate numbers, time taken. The raw output is kept
in the cache so a post-step can be redone without a new call.

### Keeping a set consistent

- **Work by group.** An asset's partners (the textures of one skater, the
  banners of one wall) are made together with shared references.
- **Anchor on approved work.** Approved assets become reference images
  for later ones.
- **Typeset, do not generate, text.** Lettering is set locally from a font
  and composited, as `studio text` does. That is what stops "K" becoming
  "P".
- **Do not ask a model for what a recipe does better.** Flat, tiling
  surfaces come from recipes. The adapter says so when asked to generate a
  `flat` or tiling asset.

## Interfaces

| Verb | Does |
|---|---|
| `asset make ID --with BACKEND` | The five steps; `--group` for an asset and its partners; `--dry-run` shows the prompt, inputs and size without a call |
| `asset normalise ID FILE` | Steps 3 to 5 on an image made elsewhere: the artist's path |
| `backend list`, `backend test NAME` | What is configured; a one-image test that measures what the backend really returns |

`asset make --json` returns the outcome, the gate numbers, the time, and
the path of a contact sheet only when the outcome is not `ok`.

## The roles

- **Art agent.** One call per asset. It never handles sizes, alpha or
  seams, and never writes shell around a generator. Budget: 1 call and 0
  pictures on `ok`; one sheet on failure.
- **Artist.** Saves a painting from any tool and runs `asset normalise`,
  or drops it in the folder the `hand` backend watches.
- **Pack lead.** Writes the brief and the style rules once per category,
  and reviews a group as one sheet.
- **Tester.** Every generated image has a record that says what made it
  and what was done to it.

## Testing

Without ROMs or a model: a fake backend that returns the wrong size, an
opaque image for a cut-out, an image with a border, nothing, and a
refusal message. Normalise must repair what it can and classify the rest.
The seam repair is tested on synthetic tiles.

## Performance and cost

Generation dominates: 1 to 2 minutes an image for the backend used so
far, four at a time. Normalising and checking are under a second. Each
job records its time, so a pack's report can say what its art cost.

## Evidence

**E1. 2026-11-20, offline, 30 raw outputs.** The 30 raw image-model
outputs kept from the showcase, with the gates as calibrated the same
day ([quality-gates.md](quality-gates.md), E1 there). No model was
called.

| | Pass the gates |
|---|---|
| The 30 raw outputs as returned | 3 |
| After normalising (size, and alpha from the original) | 30 |
| The same 30 as the hand scripts finished them | 26 |

- **Size is the whole story for 23 of them**: the raw output is simply
  not at a whole scale of the original. Four more needed the original's
  cut-out as well. The hand scripts left those four opaque.
- **Normalised and hand-finished images are the same picture**: within
  3 levels on 28 of 29 comparable pairs.
- **Passing the gates is not the same as being right.** One of the four,
  a cloth with 64% cut-out, passes `alpha` by construction after its
  alpha is taken from the original, and is unusable: the model drew a
  larger subject and the cut-out chops it up. The fit test was 0.00 to
  0.04 for the three good repairs and 0.39 for the cloth. The limits
  0.06 and 0.20 come from four real cases and synthetic shifts.
- **Aspect was never wrong** in these 30, so undoing a letterbox was
  tested on synthetic cases only: bars and a trim of up to a tenth can
  be repaired; a stretched or recomposed image cannot be told apart
  reliably.
- **Binary alpha was better than a smoothed one** by the gate.
- Of the 30: 26 `ok`, 3 `repaired`, 1 `refit`.
- Not tested offline: the seam and palette steps, and the
  visible-texel mask (none is on disk; the original's cut-out stood in).

**E2. 2026-11-20, ten real calls.** Ten Top Skater textures, none among
the 30, went through steps 1 to 4 with one call each: the Codex CLI
(version 0.171.0, model `gpt-6.2-sol`), the original enlarged and
letterboxed, one approved image as a style reference, and a prompt from
one template. The ten were three cut-outs (3%, 43% and 57% cut out), a
4:1 strip, a 1:2 plank, two repeating surfaces, a "CHECK POINT" sign, a
face and a striped cone.

| | Observed |
|---|---|
| Pass the gates on the first call | 9 of 10. No retry was needed |
| The tenth | Refused in 5 s: the face. No image |
| Size returned | Exactly the size asked, 9 of 9 |
| Letterbox bars | Returned flat and in place, 5 of 5, so undoing them was exact |
| Cut-outs shown as flat magenta | Kept flat, 3 of 3; fit test 0.01 to 0.02 against a limit of 0.06 |
| Repeating surfaces | Both already tiled as returned |
| Cost | 45 to 108 s and 14,000 to 29,000 tokens a call; 188,000 tokens in all |

- **Observed:** letterbox in, undo after, and alpha from the original
  all held. Stripping bars by colour was never needed.
- **Observed:** the refusal reply was "I can't redraw an existing game
  asset with the same exact shapes and layout". The template said "only
  redraw them in the new style" in all ten prompts, and nine were not
  refused. Which of the wording and the face caused it was not tested.
- **Observed:** run on a tile that already passed, the roll-and-blend
  seam repair did no harm to one and made the other worse (seam measure
  0.0 to 8.9) and ghosted it.
- **Observed, by eye on one contact sheet:** shapes held; colour did
  not. Folds, planks, rails, the tree's outline and the sign's letters
  stayed where they were. Red cloth came back salmon, pale wood orange,
  and grey-green ground mint: the colours of the reference image. No
  gate in the first set looks at colour, so all of these pass.
- **Observed:** the two repeating surfaces came back as new blob
  patterns. They tile and pass, and a recipe would have made them for
  nothing.
- **Observed:** the registration score read 0.02 to 0.15 on images that
  were right.
- Not tested: a retry after a gate failure, timeouts and rate limits,
  the palette step, more than one reference, more than two calls at
  once, and whether a second call gives the same size. Ten assets is a
  count, not a rate.

**E3. 2026-11-20, two follow-ups.**

- **The face went through with one word changed.** The same call with
  "only change their style" in place of "only redraw them in the new
  style" returned an image in 49 s that passes the gates, with brows,
  nose and mouth where the original's are. One call each way does not
  prove the word was the cause.
- **The colour step.** Tried on the nine results and the face, with no
  calls. Drift is the largest difference in mean colour, in levels of
  255.

| Colour step | Worst drift of the ten | By eye |
|---|---|---|
| None | 89 | Skin orange, cloth salmon, ground mint |
| The upscaler's low-pass difference, 2 texels wide | 11 | Colour returns, and so do the original's soft shading and blotches: halos round ink lines, gradients on flat areas |
| One added shift per channel | 14 | Flat stays flat, but dark ink is tinted: the face's black lines turn blue |
| One gain per channel | 10 | Flat stays flat and ink stays dark. Bright areas clip a little |

  Every variant still passes the gates. One gain cannot fix a texture
  of several colours: the cone's white stripe turned pink. A gain per
  colour region was not tried.

## Open questions

- **Which backends honour transparent output**, and what are their terms
  on outputs? Only third-party summaries were read.
- **The refusal list** is three entries from one model, plus "redraw"
  (E2, E3). It will need feeding from real refusals.
- **How much colour may a restyle change?** The ten calls'
  images took their colours from the reference (E2). Whether that is
  wanted is the pack lead's call per category; the adapter needs a
  switch and a number to report. A `colour` warning in the gates (mean
  drift over some limit) is not tried.
- **A gain per colour region** for textures of several colours (E3).
- **Material maps from a model.** Nothing found evaluates normal or
  roughness estimation on stylised low-resolution art. `make
  pack-materials` reads height from brightness today, with known flaws.
- **Is a model the right tool for small tiles at all?** The showcase used
  recipes for 5,591 of its images.

## MVP: a few days

1. Steps 3 to 5 wrapped around today's `studio draft`: exact size by
   letterbox, alpha from the original, mask composite, outcome classes
   (`ok`, `repaired`, `refit`, `gate:<name>`, `refused`, `timeout`,
   `rate-limited`, `bad-output`).
2. `backends.txt` with the Codex CLI and a fake backend.
3. `asset normalise` for hand-made images.
4. The fit test, with first limits 0.06 and 0.20, the prompt lint that
   blocks the call, and the colour step as one gain per channel.
5. Then a retry after a gate failure, and more assets through a real
   image model. Any such run needs the user's
   yes, since images leave the machine.
