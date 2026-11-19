# Authoring: the path with no terminal

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. This is the spec the artist role was missing in pass one. It adds
no new abilities: it puts the verbs of [interface.md](interface.md) behind
a page in the player.

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
  egui (`crates/play/Cargo.toml:28`), with a side-panel Esc menu, a Picture page with live styles and a Compare divider, save slots with
  thumbnails, and a photo mode with a free camera and a time scrub
  (`docs/play.md`, `docs/photo-mode.md`).
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

The ID pass that picking needs costs 0.7 ms at 1080p and 3.0 ms at 4K as
documented (`docs/enhancements.md`, "Looks"), and runs only while the
page is open. The watch is a check a few times a second, as the look's
is. Reloading a whole pack resets the renderer's caches; if that is
slower than a second on a large pack, single files must be reloaded
instead. Not measured.

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
