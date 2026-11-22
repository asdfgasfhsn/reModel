# Quality gates

Shared words, roles and rules: [README.md](README.md). **Status: the
first part is built** (2026-11-22, roadmap item 4): the asset gates
`file`, `alpha`, `alpha.soft`, `seam`, `changed`, `noise` and `stale`
(`remodel asset check`), the pack gates `lint`, `budget`, `known` and
`selected`, the scene gate `coverage` (`remodel pack check`, with
`gates.tsv`, `report.md` and `report.json`), limit overrides in
`gates.txt`, and the clip gate `flicker` (`remodel pack-flicker` exits 1
over its limit). How to use them, their limits and their output are in
[../uplift-studio.md](../uplift-studio.md), "Gates"; this page keeps what
is not built, and the evidence the limits come from.

## What the re-check found (rule 1, 2026-11-22)

The spec's "Observed" claims, against the code at `7695599`:

- Held, with line numbers moved: the seam test is `seam_axis` in
  `studio_img.rs` (2.5 times and 12 levels); `pack-lint`'s output is
  prose with a letter per check; `pack-flicker` exited 0 when it found
  flicker; `studio status` always returns 0; `studio compare` has no
  threshold.
- Changed: the seam limits were literals; they are now named constants in
  `gates.rs` that the seam check reads. `finish_draft` (`studio.rs`)
  now keeps a cut-out's transparency (fixed in slice A), so "a draft does
  not inherit alpha" is true only of an image from outside, which
  `alpha` catches.
- Wrong or missing: nothing recorded what was approved, so `stale` had
  nothing to compare with; `studio approve` now writes
  `status/approved.tsv`. A pack's showcase files approved before that have
  no record, and `stale` says `info` for them. E1's `noise` measure was
  not specified: the built one is Immerkaer's estimate. "Scene coverage as a
  share of pixels drawn from replaced assets" needs per-pixel identity,
  which `scene what` (not built) would give; the built `coverage` counts
  pixels that differ from the original, a lower bound.
- The project the spec calls a project is, today, a studio project
  (`project.txt`, `pack/`, `draft/`); `project approve` and the pack
  project (item 6) are not built, so the gates work on the studio layout.

## Not built

| Gate | Passes when | Level | Why it waits |
|---|---|---|---|
| `kept`, `palette`, `luma-order`, halo | Structure, palette and brightness order stay related to the original's. Faithful packs only | warn | E1: they cannot judge a restyle; a project must say it is a faithful pack (item 6), and none exists to test them on |
| `text` | Any readable text or logo is on the project's allow-list (`[words]` of `style.txt`) | review | Needs an outside tool; it can only flag |
| `browser` | A pack that claims the browser uses nothing the browser cannot show | warn | `budget` already warns over the browser's 192 MB |
| `misses` | No pixel misses for `colour`, `sibling` or `pending` in a scene | warn | Needs `--overlay miss` (item 10) |
| `distance` | Replaced surfaces drawn at 1x are close to a supersampled reference, as `pack-check` measures | warn | Untested idea; needs calibration |
| `steady` | Still pixels change from frame to frame no more than 1.5 times what the original's do | warn | Untested idea; needs a clip |

Also not built: `pack report` (write the report again from the last
results); `project approve` calling `studio::refusals` and refusing on an
error unless given `--override "reason"` (item 6; the function is there and
tested); a `review` verdict and its recorded decisions; the sheet made on
a failure (source, candidate rolled by half, difference map); the
project's list of required scenes and each scene's own coverage target (a
`gates.txt` limit sets one for all); coverage as a share of pixels drawn
from replaced assets, with the largest unreplaced assets listed; the
player's pack page showing the gates to an artist; `asset check` on a tile's
cut-out as anything but a warning, which E1 settled.

Nothing catches a small offset or a blur at any limit (E1). Whether a
scene looks right is the pack lead's call, made from one sheet per scene;
the gates exist so that call is about taste and not about defects.

### What each would have caught

| Showcase problem | Gate | Confidence |
|---|---|---|
| Opaque draft for a cut-out | `alpha` | High: found 7 of 303 textures in the approved pack |
| Ink seam on the ground | `seam` | Low (E1): ink on an edge is normal in cel art |
| Skaters that barely changed | `changed` | Unproven at the built limit (E2) |
| Over the browser budget | `budget` | High: 263 MB against 192 MB |
| 9,564 replacement changes | `flicker` | High, but the limit is a guess (E2) |
| Assets missing past frame 6212 | `coverage` on later scenes | Needs those scenes |
| Halftone grid at a distance | `distance`, `steady` | Medium: not built |
| Purple sand floor | None | The `category` guess failed (E1) |
| Stray outlines | None | Renderer defects, fixed in the renderer |

## Evidence

**E1. 2026-11-20, calibration** against the showcase (offline, in
Python): 6,273 pairs of an approved image and its original (303
textures, 5,970 tiles), an older version of the pack, 30 raw
image-model outputs, and defects injected into approved images. No
rejected images survive: the project's drafts are the same files as the
pack.

| Gate | First limit | Level | Wrongly flags, of approved | Catches |
|---|---|---|---|---|
| `file` | Aspect, a whole scale up to 8, 4 MB | error | 0% | Every wrong aspect and non-whole scale |
| `alpha`, textures | Over 2% of the cut-out disagrees | error | 2.3%, of which 4 are real | Every lost alpha, whole or a quarter |
| `alpha`, tiles | The same | warn | 14.6% | Every whole loss, 90% of quarter losses |
| `alpha`, part-transparent | Over 0.1% of pixels | warn | 0.7% | Every soft alpha |
| `seam` | 12 levels across the edge | warn | 4.2% | 31% of a 24-level step; none of a 6-level one |
| `changed`, textures | Under 5% differs | warn | 2.3% | 90% of unchanged copies |
| `noise` (new) | Over 16 | warn | 1.3% | Heavy noise only |

- **Observed:** `alpha` found four textures in the approved pack that
  the image model returned opaque. Three are plain defects by eye (hair,
  cloth and wool with their cut-out filled in); the fourth, a ticker,
  may be meant. The older pack still had their alpha.
- **Observed:** `seam` misses small steps, and most of what it flags in
  the pack is a seam the original has too. Ink on a wrapped edge is on
  purpose in cel art (13% to 23% wrongly flagged).
- **Observed:** `kept`, `luma-order`, `palette` and a halo test were
  wrongly flagged on 26% to 65% of approved restyled art, and `changed`
  on tiles; none separates a good restyle from a bad one. Whether they
  suit a faithful pack was not tested.
- **Observed:** `category` scored the purple-sand originals as ordinary,
  the one case it was guessed for, and flagged 5% to 10% of originals.
  Dropped.
- **Observed:** `file` must run on the finished image: 26 of the 30 raw
  image-model outputs fail it before normalising.
- **Observed:** cost 0.7 ms an image at the median, 33 ms at the 99th
  percentile; the whole pack in about 35 s on one core.
- **Thin:** limits were set on the same images they are reported on.

**E2. 2026-11-22, the built gates on two games.** The Top Skater
showcase's last pack (6,286 files, 6,500 selected) and, for `vf2`, a
throwaway pack made for the purpose: 40 images made by script from 54 real
`vf2` textures (28 restyled and good; the rest deliberately bad: two
opaque for a cut-out, a wrong aspect, a 2.5x scale, a 9x scale, a ramp
across a repeated edge, two unchanged copies, two noisy, one edited after
approval). The second pack tests that each gate fails when it should and
that restyles are not flagged; it cannot say how often real art is wrongly
flagged.

| Gate (limit) | Top Skater showcase, 6,286 files | `vf2`, 40 images, 8 gates demonstrated both ways |
|---|---|---|
| `file` (scale 1 to 8, 4 MB) | 6,286 pass | 3 error (aspect, 2.5x, 9x), 37 pass |
| `alpha` (2%) | 7 error of 303 textures (2.3%); 552 warn of 5,970 tiles (9.2%) | 2 error, the 2 made; 35 pass |
| `alpha.soft` (0.1%) | 34 warn (0.54%) | 0 |
| `seam` (12 levels, 2.5x) | 1 warn of 219 checked | 1 warn of the 2 made (the other's original has the same seam, so it is not flagged) |
| `changed` (5%, 12 levels) | 0 of 303 (E1: 2.3%); the lowest is 11.5% | 2 warn, the 2 copies |
| `noise` (16) | 167 warn (2.7%; E1: 1.3%) | 2 warn, the 2 made; 27 on the noisy ones |
| `stale` | `info`: no approval record | 1 error, the one edited |
| `lint` | 0 errors, 0 warnings | 0 errors |
| `budget` (512 MB; 192 MB browser) | 263.1 MB: warn | 25.6 MB: pass (fail shown with a 20 MB override) |
| `known` | 1,401 warn: not in the project's one map | pass (a stray file warns) |
| `selected` | 227 of 6,500 not approved: warn | 14 of 54: warn |
| `coverage` (10%) | 0.0, 0.0, 6.8, 88.0, 90.9 over five scenes | 4.0, 54.7, 53.1 over three |
| `flicker` (5,000 px per 100 frames) | 10,342: error (993 changes, 1,725 frames) | 6,502: error (2 changes) |

- **Observed:** the asset gates over the 6,286 files take 2.2 s on all
  cores; `pack check` of the whole showcase, 5 s with five scenes drawn.
- **Observed:** E1's `alpha` figures reproduce (textures 2.3%, soft 0.54%
  against 0.7%); tiles come out lower (9.2%, E1 14.6%) and `noise`
  higher (2.7%, E1 1.3%), since E1's noise measure is not recorded.
- **Observed:** `changed` at 12 levels flags none of the 303 textures, where
  E1 flagged 2.3%; "differs" had no level in E1. The "skaters that barely
  changed" case is not caught at this level.
- **Observed:** the flicker limit is wrong in both directions. Top Skater's
  attract run (993 changes) and `vf2`'s (2 changes, one the size of
  the screen) both fail it. A pixel-weighted total counts a screen-sized
  fade as flicker; the limit is a guess, and two logs do not settle it.
- **Observed:** `coverage` reads 0.0% on Top Skater's two 2D scenes
  (coin-up, course-select) with a pack that has regions and tile images; it
  is not checked whether the 2D layer draws the pack in `scene render`, or
  whether those scenes show none of it.
- **Observed:** `known` resolves against the project's `map/` only; a pack
  made from several runs warns for the textures of the runs that are not
  linked.
- **Thin:** the second pack is synthetic. Limits are still set on the
  showcase; a second real pack is needed to check the false-flag rates.

## Open questions

- **Limits.** The flicker and coverage limits need real clips and scenes.
  `distance`, `steady`, `kept` and `category` are untested ideas.
- **Text and logo detection** would need an outside tool. Nothing was
  found on its accuracy at 32 to 64 pixels. It can only flag, never pass.
- **Learned measures** such as LPIPS need model weights and are left out.
  A plain structural similarity is enough for a band.
- **Trademark checks on invented brand names** remain a web search by a
  person. The showcase's were checked "by reasoning only".

## What is left of the MVP

The MVP's four steps are built, except that `project approve` does not
exist: item 6 calls `studio::refusals`. Next in this spec: the `review`
verdict with recorded decisions, `pack report`, the failure sheet, and
re-setting the limits above from a second real pack.
