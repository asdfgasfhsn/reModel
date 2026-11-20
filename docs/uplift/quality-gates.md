# Quality gates

Shared words, roles and rules: [README.md](README.md). Nothing here is
built.

## The problem

The showcase pack was checked mostly by eye, late, and by whoever came
next. Checks exist, but they are scattered, they print prose, and most
cannot fail.

**Observed today.**

| Check | Measures | Can it fail? |
|---|---|---|
| `studio seam` | Edge jump over typical; suspect above 2.5 times and 12 levels (`crates/cli/src/studio_img.rs:935`) | Only with `--strict` |
| `pack-lint` | Manifest, names, sizes, maps, memory | Yes, on errors; output is prose with a letter per check (`crates/cli/src/pack_lint.rs:4`) |
| `pack-flicker` | Replacement coming and going, ranked | No: exits 0 when flicker is found (`crates/cli/src/main.rs:1175-1200`) |
| `studio compare` | Share of pixels changed per frame | No threshold |
| `studio status` | Counts by state | No: always 0 |

Nothing checks what an image generator returns: alpha, borders, aspect or
whether the image changed at all. A generated draft for a cut-out texture
does not inherit the original's alpha (inferred: `finish_draft`,
`crates/cli/src/studio.rs:1761`, only crops and resizes; not run).

**What slipped through in the showcase.** A sand floor painted purple. An
ink seam across the ground. Halftone turning into a grid at a distance.
Skater textures that "barely change". A pack over the browser's memory
budget. 9,564 changes between replacement and original in one run, none of
them visible in a still. And repeatedly, a final report saying the last
fixes were "not seen in a render".

reModel already has the pattern to copy. `temporal-check` and
`lighting-check` state pass criteria as numbers and keep them as set
(`docs/enhancements.md`, "How good it is" and "How it is verified").

## Design

One command, `pack check`, runs gates at four levels and writes one report.
Each gate gives a number, a limit and a verdict: `error`, `warn`, `info` or
`review` (a person must look). Only `error` blocks and sets exit code 1.
A `review` item is settled when an approver accepts it with a reason.

### Asset gates: no ROM, no GPU, milliseconds each

| Gate | Passes when | Level |
|---|---|---|
| `file` | It decodes, has the original's aspect, a whole-number scale, and is within the size limit | error |
| `alpha` | Its cut-out at 0.5 agrees with the original's (overlap of at least 98%), and it has no part-transparent pixels where the original had none | error |
| `seam` | Each repeated or mirrored axis is under the seam limit; today's check, plus ink drawn on a wrapped edge | error |
| `stale` | The file's hash is the one that was approved | error |
| `changed` | Inside the visible-texel mask, enough of the image differs from the original | warn |
| `kept` | Its structure is still related to the original's (a similarity band, so UV layout and lettering positions survive) | warn |
| `palette` | Most pixels are near the style's palette for its category (CIEDE2000) | warn |
| `luma-order` | Dark stays dark: brightness correlates with the original's, so the game's lighting still reads | warn |
| `category` | Its *original* is not an outlier among its category's originals (an untested idea; not in the first set) | warn |
| `text` | Any readable text or logo found is on the project's allow-list | review |

**Calibrated on 2026-11-20** against the showcase (offline, in Python):
6,273 pairs of an approved image and its original (303 textures, 5,970
tiles), an older version of the pack, 30 raw image-model outputs, and
defects injected into approved images. No rejected images survive: the
project's drafts are the same files as the pack.

| Gate | First limit | Level | Wrongly flags, of approved | Catches |
|---|---|---|---|---|
| `file` | Aspect, a whole scale up to 8, 4 MB | error | 0% | Every wrong aspect and non-whole scale |
| `alpha`, textures | Over 2% of the cut-out disagrees | error | 2.3%, of which 4 are real | Every lost alpha, whole or a quarter |
| `alpha`, tiles | The same | warn | 14.6% | Every whole loss, 90% of quarter losses |
| `alpha`, part-transparent | Over 0.1% of pixels | warn | 0.7% | Every soft alpha |
| `seam` | 12 levels across the edge | warn | 4.2% | 31% of a 24-level step; none of a 6-level one |
| `changed`, textures | Under 5% differs | warn | 2.3% | 90% of unchanged copies |
| `noise` (new) | Over 16 | warn | 1.3% | Heavy noise only |

- **`alpha` earns its place at once.** It found four textures in the
  approved pack that the image model returned opaque. Three are plain
  defects by eye (hair, cloth and wool with their cut-out filled in);
  the fourth, a ticker, may be meant. The older pack still had their
  alpha.
- **`seam` is weak, and a warning at most.** It misses small steps, and
  most of what it flags in the pack is a seam the original has too. Ink
  on a wrapped edge cannot be a gate for cel art, which has it on
  purpose (13% to 23% wrongly flagged).
- **Dropped for restyled art:** `kept`, `luma-order`, `palette` and a
  halo test (26% to 65% wrongly flagged), and `changed` on tiles. None
  separates a good restyle from a bad one. They may suit a faithful
  pack; nothing here tests that.
- **`category` is dropped.** It scored the purple-sand originals as
  ordinary, the one case it was guessed for, and flagged 5% to 10% of
  originals.
- **Nothing catches a small offset or a blur**, at any limit.
- **`file` must run on the finished image.** 26 of the 30 raw
  image-model outputs fail it before normalising.
- Cost: 0.7 ms an image at the median, 33 ms at the 99th percentile;
  the whole pack in about 35 s on one core.
- Limits were set on the same images they are reported on. `stale`,
  `text` and the pack, scene and clip gates were not run.

So the first set is `file`, `alpha` and `stale` as errors, and `seam`,
`changed` (textures) and `noise` as warnings. A project states whether
it is a restyle or a faithful pack, and the style gates apply only to
the second.

### Pack gates

| Gate | Passes when | Level |
|---|---|---|
| `lint` | Today's `pack-lint` checks | error |
| `budget` | Images and maps fit 512 MB native; 192 MB for a pack that claims the browser | error or warn |
| `known` | Every file resolves to an asset the catalogue has seen; unused files are listed | warn |
| `selected` | Every selected asset is approved | error for a release |
| `browser` | A pack that claims the browser uses nothing the browser cannot show | warn |

### Scene gates: from captures, about 2 s a scene

| Gate | Passes when | Level |
|---|---|---|
| `coverage` | The share of the scene's pixels drawn from replaced assets meets the project's target; the largest unreplaced assets are listed | error for a release |
| `misses` | No pixels miss for reason `colour`, `sibling` or `pending` in the frame | warn |
| `distance` | Replaced surfaces drawn at 1x are close to a supersampled reference, as `pack-check` measures | warn |

### Clip gates: need the ROM

| Gate | Passes when | Level |
|---|---|---|
| `flicker` | Pixel-weighted changes between replacement and original per 100 frames are under the limit | error |
| `steady` | Still pixels change from frame to frame no more than 1.5 times what the original's do | warn |

### What each would have caught

| Showcase problem | Gate | Confidence |
|---|---|---|
| Opaque draft for a cut-out | `alpha` | High |
| Ink seam on the ground | `seam` | Low after the spike: ink on an edge is normal in cel art, and the file was not there to test |
| Skaters that barely changed | `changed` | High |
| Over the browser budget | `budget` | High (lint already notes it) |
| 9,564 replacement changes | `flicker` | High |
| Assets missing past frame 6212 | `coverage` on later scenes | High, given the scenes exist |
| Halftone grid at a distance | `distance`, `steady` | Medium: needs calibration |
| Purple sand floor | None | The `category` guess was tested and failed |
| Stray outlines | None | These were renderer defects, fixed in the renderer |

Not everything has a number. Whether a scene looks right is the pack lead's
call, made from one sheet per scene. The gates exist so that call is about
taste and not about defects.

### Limits and overrides

Defaults ship with reModel. A project overrides one in `gates.txt` with a
reason, and the report shows the override. The first defaults come from
measuring the showcase's approved assets: see the MVP.

### What "done" means

A pack revision is done when: no gate is at `error`; every selected asset
is approved and not stale; `coverage` meets the target on every scene the
project lists as required; `flicker` passes on every clip; and each
`review` item has a recorded decision.

## Interfaces

| Verb | Does |
|---|---|
| `asset check ID` | Asset gates for one asset, in under a second. The art agent's check |
| `pack check` | Everything. `--level asset`, `pack`, `scene` or `clip` narrows it; `--scene NAME` and `--asset ID` too |
| `pack report` | Writes the report again from the last results |

An asset that passes becomes `checked`. `project approve` runs the asset
gates again and refuses on an error unless given `--override "reason"`,
which is recorded. The `text` gate's allow-list is the `[words]` section
of the project's `style.txt`.

Output files in the project's `reports/<date>-<commit>/`: `report.json` (all
results), `gates.tsv` (one row per asset and gate: value, limit, verdict)
and `report.md` (for people: totals, failures first, the sheets).

One result:

```json
{"asset":"wall-base","gate":"seam","value":3.1,"limit":2.5,
 "verdict":"error","axis":"x",
 "fix":"set seamless for this asset in recipes.txt, or repaint the left and right edges",
 "sheet":"cache/sheets/wall-base-seam.png"}
```

The `fix` line matters for a mid-tier model: it turns a failure into a
next step. The sheet is made only on failure: source, candidate, the
candidate rolled by half so the seam is in the middle, and a difference
map.

## The roles

- **Art agent.** `asset check` after each write. On a pass it reads no
  picture. Budget: 1 call per attempt, 1 small sheet per failure.
- **Tester.** `pack check --json` is the sign-off. Nothing needs to be
  looked at to know whether a revision is complete.
- **Pack lead.** Reads `report.md`: what changed since the last revision,
  what failed, what needs a decision.
- **Artist.** The same gates show in the player's pack page as a short
  list, in plain words.
- **Tool engineer.** A new uplift kind adds its gates to the same table,
  with synthetic tests.

## Testing

- **Without ROMs:** each asset gate on synthetic images made to pass and to
  fail (the seam check's tests are the model). The report formats.
- **With ROMs:** the scene and clip gates on the showcase pack, with
  numbers recorded in `docs/games/topskatr.md`. No images enter the
  repository.

## Performance

Asset gates are simple image arithmetic. Target: the whole showcase (6,290
files) in under a minute on all cores; one asset in under 100 ms. Scene
gates cost a scene render each. Clip gates run the game and are the slow
part, so they run on request and before a release, not on every edit.

## Open questions

- **Limits.** All of them need calibrating. `distance`, `steady`, `kept`
  and `category` are untested ideas.
- **Text and logo detection** would need an outside tool. Nothing was
  found on its accuracy at 32 to 64 pixels. It can only flag, never pass.
- **Learned measures** such as LPIPS need model weights and are left out.
  A plain structural similarity is enough for a band.
- **Trademark checks on invented brand names** remain a web search by a
  person. The showcase's were checked "by reasoning only".

## MVP: a few days

1. `asset check` with `file`, `alpha` and `stale` as errors and `seam`,
   `changed` and `noise` as warnings, and `project approve` refusing on
   errors.
2. `pack check --json` adding `lint`, `budget`, `known`, `selected` and
   scene `coverage`, with `gates.tsv` and `report.md`.
3. `pack-flicker` exits 1 over a limit.
4. Limits from the spike of 2026-11-20, re-set once a second pack
   exists to check them on.
