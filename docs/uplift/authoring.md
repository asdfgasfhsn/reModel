# Authoring: the path with no terminal

Shared words, roles and rules: [README.md](README.md). Nothing is built
into reModel; what was tried is in "Evidence" (spike code is on
unmerged branches, not for merging as it stands). This is the spec the
artist role was missing in pass one. It adds no new abilities: it puts
the verbs of [interface.md](interface.md) behind a page in the player.

By decision (2026-11-19) agents are served first, so this follows the
slices that give agents their loop. It is written now so those slices
leave room for it.

## The problem

Every verb specified so far is a command. An artist with a released
build, their own ROM and Krita cannot run `asset bundle`, and has no
scenes or catalogue to begin with. An independent read of the pass-one
specs found the artist the worst-served role for exactly that reason.

**Observed.**

- **The player already has the parts of an interface.** It is built on
  egui (`crates/play/Cargo.toml:28`), with a side-panel Esc menu, a Picture page with live styles and a
  Compare divider, save slots with thumbnails, and a photo mode with a
  free camera and a time scrub (`docs/play.md`, `docs/photo-mode.md`).
- **It can already observe a game.** The Esc menu has a Catalogue setting
  that records what a game shows while it is played
  (`docs/uplift-studio.md`, "Observing").
- **People's tools today are two static pages**: the texmap viewer and
  the catalogue viewer, each a single HTML file that opens from disk.
- **The studio's own plan** lists what comes next: a before-and-after
  page, live reload, picking in the player, and editing in the viewer
  (`docs/uplift-studio.md`, "What is built, what comes later").
- **The player can be driven by script** for tests: `--nav`, `--input`,
  `--shot` (`docs/play.md`).
- **The player has no reload today.** It does not watch a pack; a change
  means switching the pack off and on, a cold load of about 0.3 s for
  Top Skater's race scene (E1), with original and pack art mixed for a
  quarter of a second. Reloading a whole pack resets the renderer's
  caches.
- **Opening the image editor exists nowhere in the code.** It needs the
  platform's opener, with its result checked.

Other projects' lesson, from their documentation: the ones people enjoy
modding show identity in the running game (RT64's texture tab, RTX
Remix's hash views), and no emulator surveyed offers "point at this and
edit it" for textures.

## Design

A **Studio page** in the player's Esc menu, for mouse and keyboard on a
desktop. Each control calls a verb in-process and records the person as
the actor. Nothing here can do what a command cannot.

### The loop

1. **Open or start a project.** Choose a folder, or "New pack for this
   game", which runs `project init`.
2. **Fill the library, once.** With no scenes or catalogue the page says
   so and offers one action: "Play for a few minutes with capture on".
   Playing with the catalogue recording fills the asset list, and each
   screen it sees becomes a scene. A game whose manifest lists scenes can
   make those without playing.
3. **Go to a scene.** A grid of thumbnails. Choosing one loads its state;
   the game runs from there, or freezes as photo mode does.
4. **Point at a thing.** With picking on, the cursor names what is under
   it: the artwork, its model, or the 2D region, with its status. A click
   selects it. This is the ID buffer of
   [scene-understanding.md](scene-understanding.md), drawn only while the
   page is open.
5. **Take it out.** "Edit" writes the asset's bundle, puts a starting
   image at `draft/<name>.png`, and opens it in the system's default
   image program. "Show folder" opens the bundle in the file manager.
6. **Save, and see it.** The player watches the project. On a save it
   normalises the image (size, alpha, the visible-texel mask), reloads,
   and runs the asset's gates. A small chip beside the asset shows the
   result in plain words: "seam shows on the left and right edges".
7. **Compare.** The Picture page's Compare, a divider dragged across the
   paused picture, shows the original beside the pack.
8. **Approve and build.** A review list steps through everything checked,
   original beside new, with approve and reject. "Build pack" writes the
   pack; "Make archive" writes the file to share.

### What the renderer must gain

The page needs three things from the renderer, and a reload at all
(there is none today):

- **A one-image swap**: replace one decoded image under a list of keys,
  not a whole-pack reload. The one-second target holds with room as long
  as only the saved image is swapped (E1).
- **Decoding and mipmaps on a worker thread**, so a large image does not
  stall the frame.
- **The ID pass without a look**, so picking works with no look active
  (E1).

The watch checks the scene's files about ten times a second and the
whole folder once or twice. A file is accepted when it is unchanged for
100 ms and decodes; otherwise the old image stays, which is safe for a
half-written file (E1). With a 4096 image shrunk on the CPU and the
folder checked four times a second, the target is missed (E1); so
check more often than that (inferred).

### Other kinds of work, same page

- **2D.** Drag a rectangle on a menu or HUD to capture a region; the page
  shows which tiles are shared and what will be cleared.
- **Models.** "Export this scene" writes glTF and opens the folder.
  Saving a `mdl_` file back into the project reloads it, with the import
  report as the chip.
- **Placing things.** In photo mode's free camera, point at a surface and
  drop a prop or a decal ([added-elements.md](added-elements.md)).
- **Looks.** The look's parameters are sliders, as the Comic look's are
  on the Advanced page today, with the overlays one key away.

### People and agents in one project

The project folder is the meeting point. When an agent writes drafts, the
page shows "6 new drafts to review" because it watches the same files.
When a person approves, the agent's next `project status` sees it.
Nothing else connects them.

### Review by several people

A project is a folder in git. `pack diff` and `project review` write
self-contained HTML pages of before-and-after sheets, as the catalogue
viewer is written today, which can be attached to a pull request or sent
as a file. Comments happen wherever the project is hosted. No server and
no accounts.

### Tools outside

Nothing is specific to one program. Krita, Photoshop, Aseprite and Blender
all read and write the files, and the watch does the rest. Automating
Blender or an image model is the author's choice and uses the same
folders.

## Interfaces

No new verbs. The page uses `project init/status/review/approve/reject`,
`scene list/make/what`, `asset bundle/normalise/check`, `region capture`,
`model export/check`, `pack build`. Two new abilities in the player:
opening a file or folder with the system's default program, and watching
a project folder.

## The roles

- **Artist.** The whole path, from "New pack" to "Make archive", with a
  mouse. Target: under a second from saving in Krita to seeing it in the
  scene.
- **Pack lead.** The review list and the before-and-after pages.
- **Art agent.** Unaffected; it shares the project.
- **Player.** A pack made this way installs like any other.
- **Tester.** The page runs the same gates and writes the same log.
- **Tool engineer.** A new kind of uplift appears on the page by adding
  its verb and its overlay; the page has no logic of its own.

## Testing

- **Without ROMs:** each control's verb is tested as a command. The page
  is driven by the player's scripted navigation to check that a control
  calls the verb it claims, and that a failing gate shows its message.
- **With ROMs:** a scripted session: open a project, jump to a scene,
  pick a known pixel, replace a file on disk, and compare the next shot.
  State hashes equal with the page open.
- **Draw-only:** jumping to a scene loads a state, which is the user's
  action, as loading a save slot is. Picking and reloading read only.

## Performance

Save to picture is 0.17 to 0.22 s for a 512x512 texture and 0.55 to
0.6 s for a 2048x2048 one (E1). The ID pass that picking needs adds 1.2
ms at 1080p and 3.8 ms at 4K as measured (E1; documented at 0.7 ms and
3.0 ms, `docs/enhancements.md`, "Looks"), and runs only while the page
is open. The watch is a check a few times a second, as the look's is.
Pointing is free (E1). Jumping to a scene takes about 0.1 s.

## Evidence

**E1. 2026-11-20, measured** (branch `uplift-spikes-b`, about 940 lines;
Top Skater's race scene at 1080p with the showcase pack; each step timed
on its own, not one end-to-end run).

| Step | A 512x512 texture | A 2048x2048 texture |
|---|---|---|
| Notice the save (10 checks a second, then 100 ms for the file to settle) | 0.14 to 0.19 s | The same |
| Read and decode | 9 to 12 ms | 100 to 191 ms |
| Alpha and mask | 1 ms | 19 ms |
| Gates | About 10 ms | About 190 ms (from the gate spike) |
| Upload and next frame | 3 to 5 ms | 8 to 42 ms |
| **Save to picture** | **0.17 to 0.22 s** | **0.55 to 0.6 s** |

- **The one-second target holds**, with room, as long as only the saved
  image is swapped. It fails only when a 4096 image must be shrunk on
  the CPU and the folder is checked just four times a second (1.2 to
  1.3 s).
- **Observed:** the cold load that stands in for a reload is about 0.3
  s for this scene.
- **Swapping one image works**: 2 ms at 64x64, 12 to 15 ms at 512, 108
  to 200 ms at 2048, and the picture returns exactly when the original
  is put back. One image under 12 colourings is 3 ms of uploads after
  one decode, but over a second if each colouring is reloaded as its own
  file.
- **Checking 6,290 files costs 10.5 ms**; checking the 40 a scene uses,
  0.04 ms. So: the scene's files ten times a second, the whole folder
  once or twice.
- **A half-written file is safe to refuse.** The rule "unchanged for 100
  ms and decodes, else keep the old image" passed five ways editors
  write files, with none accepted early or missed.
- **Pointing is free.** Reading one pixel of the ID target takes 0.05 to
  0.07 ms at any resolution. The ID pass adds 1.2 ms at 1080p and 3.8 ms
  at 4K, and today runs only with a look.
- **Jumping to a scene takes about 0.1 s**: 82 ms on Top Skater, 105 ms
  on Daytona, from loading the state to the picture.
- **Correction:** the documented ID-pass figures (0.7 ms at 1080p, 3.0
  ms at 4K) differ from the 1.2 ms and 3.8 ms measured here; the
  measured ones are used in Performance.
- **Not measured:** one end-to-end run, and whether reloading a whole
  pack takes over a second on a large pack (then single files must be
  reloaded instead).

## Open questions

- **Is a page in the player enough**, or does review want a separate
  window beside the game?
- **Opening the system's image program** differs on Linux, Windows and
  macOS and can fail silently. What does the page do then?
- **How much of the first-run step can be skipped** by shipping scene
  recipes with reModel?
- **A browser studio** was set aside with the rest of the browser work.
  The static review pages are the only web output proposed.

## MVP: after slices A and C

1. The Studio page with the scene grid and `--scene` jumps.
2. Picking with a read-out, "Show folder" and "Edit".
3. The project watch, with normalise, reload and the gate chip.
4. The review list with approve and reject.
