# Making art with any image model

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. The batch runner that exists is `studio draft`
([../uplift-studio.md](../uplift-studio.md)).

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
  size the backend accepts, and the same letterbox is undone afterwards,
  so the UV layout survives. Cropping, as today, does not.
- **Lint the prompt.** A list of known refusal triggers is checked before
  spending a call: a publisher's name, a real brand, "redraw X's artwork".
  The lint proposes wording that describes the task as restyling the
  supplied image.

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
| Exact size and aspect | Undo the letterbox, resample to the asset's size times its scale |
| Alpha | Take the original's cut-out, scaled by the renderer's own 0.5 rule. The generator's alpha is never trusted for a cut-out |
| Only visible texels change | Composite over the mask; outside it keep the recipe's result or the original; pad island edges |
| Palette | Optional per category: snap to the style's palette in a perceptual colour space |
| Seamless | For each repeated axis: roll by half, repair the seam band, measure again. The `seamless` levelling in `crates/cli/src/studio_img.rs:417` is the start |
| Overall colour | Optional: put back the original's low-frequency colour, as the upscaler path does (`docs/enhancements.md`, "Batch texture upscaling") |

### 4. Check and classify

Run the asset gates ([quality-gates.md](quality-gates.md)) and give one
outcome:

| Outcome | Next |
|---|---|
| `ok` | Written to `draft/<slug>.png`; the asset becomes `checked`, since the gates have run |
| `gate:<name>` | Retry, with the gate's fix line added to the prompt, up to a limit |
| `refused` | No retry; report the lint's suggestion |
| `timeout`, `rate-limited` | Retry later |
| `bad-output` | No image, a mockup with a border, or the wrong subject |

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

## Open questions

- **Which backends honour transparent output**, and what are their terms
  on outputs? Only third-party summaries were read.
- **The refusal list** is three entries from one model. It will need
  feeding from real refusals.
- **Material maps from a model.** Nothing found evaluates normal or
  roughness estimation on stylised low-resolution art. `make
  pack-materials` reads height from brightness today, with known flaws.
- **Is a model the right tool for small tiles at all?** The showcase used
  recipes for 5,591 of its images.

## MVP: a few days

1. Steps 3 to 5 wrapped around today's `studio draft`: exact size by
   letterbox, alpha from the original, mask composite, outcome classes.
2. `backends.txt` with the Codex CLI and a fake backend.
3. `asset normalise` for hand-made images.
4. **Spike:** ten showcase assets through the adapter, counting how many
   pass the gates on the first call, against the same ten through the
   hand scripts.
