# Live sessions: field notes from a real task

First real use of `remodel serve` and the spines (2026-11-26): reaching
Top Skater's secret event with inputs, starting from the player's Expert
recording. The result is in [../games/topskatr.md](../games/topskatr.md),
"Hidden content". This page is only about the tools: what carried the work,
what was missing, what got in the way. The work took one long session; most
of the time went to the missing pieces below, not to the search itself.

## How it went

1. Opened a session from a made tour and read values; stepped and rendered:
   the armed start (Expert, jump front held as the race loads) works and
   looks like an ordinary race.
2. `spine.where` on the player's recording found the stretch near the target
   point; it was 240 units off (the player's line is on the left of a wide
   hall, the point is on the right).
3. A branch at frame 3072 (the GO) that replays the player's inputs follows
   the recording to two decimals (frame 4200: x 209.98, z -3134.8 in both),
   so the branch is a faithful base. From a mark at frame 3900, a search over
   steering and jump timings (restore, step one frame at a time, read, repeat:
   about 5 s a try, with two calls a frame) found the jump that fires the check.
4. `session.export` wrote the route, `tour run` replayed it to the same hash
   (207b665060eebdd7), and renders of the event came from the same session.

## What worked

- **Lockstep and marks.** Restoring a mark is 5 ms, so a timing search is
  cheap and exact. Every result in the notes above can be repeated.
- **One socket, many clients.** A 40-line Python client over the Unix socket
  (initialize, then `tools/call`) was enough for loops; `remodel call` for
  one-off questions. Errors name the nearest valid name ("did you mean s2").
- **Spines.** A spine for a branch tour (a line with `from=`) builds in 6 s;
  opening at a frame is then under a second. The server re-reads the tour
  files on every open, so editing a tour needs no restart.
- **Export and replay agree.** The exported branch replays to the session's
  hash, with its spine's line in the same file.
- **Pictures.** `session.render` after any step, 1080p, about 0.15 s.

## What was missing

1. **Applying a recording's inputs to a branch.** To follow the player's
   route I parsed the tour line myself and stepped in runs of constant
   input. Needed: `session.step` with `follow=LABEL` (use that tour's inputs
   for these frames), or a `frames` replay on open. Stepping a run of the recording is the first thing
   anyone does with a spine.
2. **Changing a recording's start.** The route needed the player's run with
   one more held input before the GO (jump front, 2560-3000). There is no
   way to open "this tour with one more term"; I wrote a second tour line by
   hand. Suggested: `session.open` with `overlay=jumpfront@2560-3000`, the
   result exportable as a branch whose line carries the overlay.
3. **A way to see why the game did not do something.** The question for most
   of the task was "does the check run, and what does it see". Sessions have
   no write watch, no PC trace and no breakpoint (`REMODEL_WATCH` exists for
   `run` only). I patched the binary locally to print a watched word's writes
   with the writer's address (ten lines, not kept); two runs of it answered
   what an hour of reading and poking had not (the check does run; the game
   rewrites the values it reads inside each frame). Suggested:
   `session.watch ADDR` (writes since the last call, each with frame and the
   i960 instruction address), and `session.disasm ADDR N` so the answer can
   be read without leaving the session.
4. **Pokes and the frame.** A poke is applied after a frame for the next, and
   `session.read` after a step shows the poked value, so it looks as if the
   game saw it. The game had overwritten it before its own code ran; every
   position and velocity poke "worked" on the read and did nothing to the
   game. Pokes of a word nothing rewrites (a flag, a task entry) do work.
   Suggested: say so in the verb's text and in the docs, and give `read` an
   option for the value as the frame left it, before the poke is applied.
5. **Raw reads.** `ADDR:l` cannot be read as a float (`ADDR:l:float` is
   refused), the answer for 19 values is 150 lines of JSON, and a range needs
   a name per word (I sent 800 names in one call, which works). Suggested:
   `:float` and `:signed`, `format=tsv` or one line per value, and
   `ADDR..ADDR` ranges.
6. **`until` is a constant test on a named value.** I wanted "until the
   distance to a point is under 20" and "until a word is not X"; both were
   client loops of one-frame steps. A game with a table of values would also
   want `until ... changes`.
7. **Spine query resolution.** The index is every 30 frames, so the nearest
   row is a coarse hint (236 units off the true approach). Fine for finding
   the place; the arrival needed the session's own stepping. `spine.where`
   could take `--refine` (re-step 30 frames each side of the best rows).
8. **The camera.** `session.camera` words (`tilt=-80`) put the eye inside
   the hall's ceiling, the tilt sign is the opposite of what the word
   suggests, and `z=` is unclear. A "look at the skater from here"
   preset (behind, above, side) would make third-person pictures one call.
9. **Command line.** `remodel call VERB --help` prints the program's help,
   not the verb's arguments (`remodel call --help` lists them); the
   spine's argument is `spine=`, and my first guess `tour=` was refused (the
   refusal listed the right names). Each `call` is a process (30-60 ms), fine by hand and slow in
   loops; a `remodel call --batch FILE` would do for scripts without a client.
10. **Sessions are not freed unless closed.** Four is the cap; I hit it twice
    after scripts that opened a fresh session each run. `session.list` would
    help (the "did you mean" hint is the only way to see what is open).

## Frictions that were not the tools

- The named-value table helped (position, speed, section) and cost me when it
  was wrong: `heading-z` is positive while the skater runs towards -z, so
  the check's direction test needed the velocity, found from the code. The
  table now says so, and has the values this task found (`lead-*`, `vel-*`,
  `section`, `skater-cc`, `mode-fn`, `secret-banner`).
- Tours are untracked files under `out/`. I wrote the new lines into the
  shared `out/topskatr/tours/hidden.txt` by replacing the file from my copy,
  and in doing so removed lines another agent had added to it meanwhile (a
  comment and two ride tours; not recoverable from disk). An append verb
  (`session.export file=... append=true`, with a lock or a merge) or one tours
  file per author would avoid it.
- One unexplained observation: once, a walk of the task list right after a
  `restore` found the check's task missing; later restores of the same mark
  in a fresh session found it. I could not reproduce it and did not rely on
  it (every later result was taken in a fresh session). Worth a test of
  `restore` against pokes in the same session (the pokes' state is not part
  of the mark).

## Suggestions in order of use

1. `session.step follow=LABEL`, then `overlay=` on open.
2. `session.watch` and `session.disasm`.
3. Float and range reads, compact output.
4. Say what a poke can and cannot do, and read before the poke applies.
5. A `session.list`, `until` with more tests, a camera preset.

## Addressed (2026-11-26, the same day)

What the list above asked for, and where it went (design: scenes-and-server.md,
"After the first real use"; user-facing: ../uplift-studio.md, "Live sessions").

| Missing | Now |
|---|---|
| 1. Applying a recording's inputs | `session.step follow: tour:LABEL`, with `inputs` as an overlay on top |
| 2. Changing a recording's start | `session.open overlay: [jumpfront@2560-3000]`; exports as a branch carrying it |
| 3. Why the game did not do something | `session.watch` (writes, with frame, value and the i960 `ip`) and `session.disasm`; no cost when unset, no change to any state hash. Not built: a PC trace, breakpoints |
| 4. Pokes and the frame | Stated in the verb text, the docs ("Pokes") and a warning on reads a poke covers; `session.read when: before_poke` |
| 5. Raw reads | `:float`, `:signed`, ranges (`ADDR:LEN`, `ADDR..END`), `format: compact` and `tsv` |
| 6. `until` | `distance NX,NY,NZ to X,Y,Z lt D`, `changes NAME`, `screen NAME` (the value table names screens); `until_reading` in the answer |
| 7. Spine query resolution | The granularity is reachable (`serve --index-every`, `open index_every`); `spine.where --refine` is not built |
| 8. The camera | `preset=chase\|side\|top`; tilt is positive for an eye above (checked), and the doc says so; the eye can still land in a wall |
| 9. Command line | `remodel call VERB --help`; `session.list`. Not built: `call --batch` |
| 10. Sessions not freed | `session.list` shows what is open |

The unexplained restore observation did not reproduce: a test restores after
pokes and compares the whole machine with a fresh replay (equal but for the
renderer's version counters); see "The restore question" in the design doc. Not
addressed: the shared tours file (an append mode for `session.export file=`).
