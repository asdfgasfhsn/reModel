# Distribution: finding and installing packs

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. The direction is the user's (2026-11-18 and 2026-11-19): no
curated list, packs found by a convention on GitHub, and easy to find.

## The problem

A player with a working game has no way to learn that a pack exists for
it. An author has no way to publish one except by telling people. Nobody
wants to keep a list.

**Observed.**

- **reModel has no network code** at commit `7d8f099`: no crate opens a
  connection, and the browser build fetches only its own files. (Linked
  play for Sega Rally is being explored separately.)
- **A pack is a folder of data**: PNG images and text
  (`docs/texture-packs.md`). Pass two would add glTF meshes and WGSL
  shaders. None of it is code that runs on the CPU.
- **The showcase pack is 197 MB built**, and megabytes as recipes plus
  original art ([pack-project.md](pack-project.md)).
- **GitHub's search API** (its documentation, read 2026-11-19) allows
  10 requests a minute without signing in, returns up to 1,000 results
  at up to 100 a page, sorts by stars or last update, and returns each
  repository's topics, description, owner and star count in the result.
  Other API calls are limited to 60 an hour for each address without
  signing in.

## Design

### The convention

A pack is a repository that follows four rules. Following them is all it
takes to be listed; nobody approves anything.

| Rule | Why |
|---|---|
| Topics `remodel-pack` and `remodel-<set>` (for example `remodel-topskatr`). A shader pack has `remodel-shader` instead | The search |
| `pack.txt` at the root, with `name`, `game`, `version`, `author` | The check that this is a pack and not only a topic |
| The repository's description is the pack's one-line summary; `preview.png` at the root is its picture | The listing, with no extra requests |
| The newest release holds one file `<name>-<version>.remodel-pack` (a zip) and its `.sha256` | The download |

One pack per repository. Stars, the update date, issues and releases then
all mean "this pack", and a search result is a pack.

`pack build --archive` writes the two release files. `pack publish
--check` reads a repository's address and says which of the four rules it
does not yet meet. It changes nothing; publishing is `git push` and a
release, done by the author.

### Finding

The launcher's page for a game has a "Find packs" action. Nothing is
requested until it is used.

1. **One search request**: repositories with both topics, sorted by
   stars. The result already holds what a list needs: name, owner,
   summary, stars, last update.
2. **For each result, `pack.txt` and `preview.png`** are fetched as plain
   files. A repository without a readable `pack.txt` for this game is
   dropped, which is what keeps the topic from being a spam list.
3. **Opening a pack** asks for its newest release: one API request.

The list is cached for a day. At most one search per game per day, and
one request per pack opened, keeps an ordinary user far inside the
limits with no sign-in. When a limit is hit or there is no network, the
cached list is shown with its age.

**Order, with nobody choosing.** Packs made against the user's own ROM
revision first (the manifest's `program` hashes), then by stars, then by
update date. A pack that `needs` something this build or this game's
path cannot show is listed greyed, with the reason. A user can hide a
pack or an author; that is local.

**Other ways in**, all the same install path: paste a repository
address, paste a link to an archive, or open a file. An extra source can
be added as the address of an index file (below), for packs hosted
elsewhere.

### A generated index, later

If lists grow or limits bite, a scheduled job in reModel's own
organisation runs the same search for every game, validates each
`pack.txt`, and publishes the result as one static JSON file. No person
edits it. The launcher reads the index when it can and falls back to
searching directly. This is the only part that costs reModel upkeep, so
it waits until it is needed.

### Installing

1. Download the archive and check it against the `.sha256`. This proves
   the download is whole. It says nothing about the author.
2. Unpack with limits: a size cap, a file count cap, no path leaving the
   pack's folder, and only the file kinds a pack may hold.
3. Run `pack check` on it. A pack that fails to parse is not enabled.
4. Bake any recipes from the user's ROM, with a progress bar.
5. Add it as a layer, switched on, above what is there
   ([pack-project.md](pack-project.md), "Layers").

Updates are offered, never taken silently, and the old version stays
until the new one has passed step 3.

### What the listing shows

Name, author, the game, version, stars, last update, the preview, the
licence and AI label if the author gave them, the download size, and
whether it was made against this ROM revision. One fixed line says that
packs come from their authors and are not reviewed by reModel.

### Safety

A pack cannot run code, but its files are read by decoders. Images are
decoded with a cap on size and memory, as the pack budget already
requires. Meshes and shaders, when they exist, are validated before use
([model-interchange.md](model-interchange.md),
[shader-packs.md](shader-packs.md)). Recipes are steps from a fixed list,
never scripts. The request to GitHub comes from the user's address; the
page says so the first time.

## Interfaces

| Where | What |
|---|---|
| Launcher, a game's page | Installed packs with switches and order; "Find packs" |
| `pack find [--game SET] --json` | The same list as data |
| `pack install OWNER/REPO`, a URL or a file | Download, check, unpack, bake, enable |
| `pack update [NAME]` | Offers newer releases |
| `pack publish --check ADDRESS` | Which rules a repository does not meet |
| `pack build --archive` | The release file and its hash |

## Data formats

The listing fields come from `pack.txt`
([pack-project.md](pack-project.md), "The shipped manifest"): `name`,
`game`, `version`, `author`, `licence`, `ai`, `program`, `needs`,
`depends`. The index file is a JSON list of the same fields plus the
repository, stars, update date and the release's address and hash.

## The roles

- **Player.** Opens a game, presses "Find packs", picks one, waits for a
  bar. No account and no terminal.
- **Pack lead.** Adds two topics and makes a release. `pack publish
  --check` says what is missing.
- **Art agent.** `pack find --json` to see what exists before starting a
  pack; `pack install` to study one. Budget: 1 call each.
- **Tester.** The search and the files are behind one trait, so every
  step is tested against recorded answers with no network.
- **Tool engineer.** One small HTTP client, native only. The browser is
  not a target.

## Testing

- **Without ROMs or network:** recorded search results and files; a
  repository with the topic and no `pack.txt` is dropped; an archive with
  a wrong hash, a path that escapes, or too many files is refused; an
  update that fails its check leaves the old version in place.
- **With ROMs:** install the showcase from a local archive of recipes and
  compare the baked pack with today's.

## Performance

One search, then two small files per pack listed. A list of fifty packs
is about a hundred requests to plain file hosting and one to the API.
Baking is the slow step ([pack-project.md](pack-project.md),
"Performance").

## Open questions

- **A new dependency.** reModel has no HTTP client. Which one, and is it
  worth it before a second pack exists? Until then, paste-an-address and
  open-a-file can be built with none of the search.
- **Forks and copies.** Does a fork of a pack list beside the original?
  How GitHub's search treats forks here was not checked.
- **Other hosts.** The convention is GitHub's. The index file is the
  bridge for anything else.
- **Signed releases**: later, or never?
- **Shader packs** list on every game's page. Is that too noisy once
  there are many?

## MVP: a few days

1. `pack build --archive` and `pack install FILE`, with the unpack limits
   and the check.
2. `pack install OWNER/REPO` and `pack publish --check`.
3. `pack find` and the launcher's list, cached.
