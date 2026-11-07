Remodel @VERSION@ (@TARGET@)
===========================

remodel-play runs Sega Model 2 arcade games in a window: the game's own
code on an emulated board, drawn on the GPU, with both sound boards live.
Supported so far: Top Skater (topskatr), Sega Ski Super G (skisuprg),
Sega Touring Car Championship (stcc), Sega Water Ski (segawski),
Wave Runner (waverunr), Over Rev (overrev), Super GT 24h (sgt24h), The
House of the Dead (hotd), Behind Enemy Lines (bel), Dynamite Baseball
'97 (dynabb97), Dead or Alive (doa), Indianapolis 500 (indy500),
Fighting Vipers (fvipers), Sonic Championship (schamp) and Virtua
Striker (vstriker).
This build runs the game code on the interpreter (no recompiled game code
is included).

Bring your own ROMs
-------------------
No game data is included. Put the sets as MAME keeps them, topskatr.zip or
a topskatr/ folder (likewise skisuprg, stcc, segawski, waverunr,
overrev, sgt24h, hotd, bel, dynabb97, doa, indy500, fvipers, schamp,
vstriker), in
the ROM folder:

    ~/.local/share/remodel/roms     ($XDG_DATA_HOME/remodel/roms)

or start the player, and in the menu type another folder under "ROM
folder" and press Scan. With no supported set found the player opens in
the menu. `remodel-play --rom DIR` sets the folder from the command line.

    ./remodel-play            play the last game (or open the menu)
    ./remodel-play --help     all options

Keys
----
Top Skater:
    C, Enter     coin, start
    arrows       steer (Down slides)
    Z, X         jump front, jump tail
    Q, E         select

Sega Ski Super G (two coins a credit):
    C            coin
    Enter or A   blue panel button (starts a game; left choice)
    S, D         red, green panel buttons (middle, right choice)
    arrows       Left/Right turn the skis, Up/Down push (unconfirmed)
    Q, E, W      select left, right, decide

Sega Touring Car Championship:
    C, Enter     coin, start
    arrows       Left/Right steer, Up gas, Down brake
    A, Z         shift up, shift down (manual gearbox: hold brake while
                 choosing the car)
    V, B         view buttons

Sega Water Ski (two coins a credit):
    C            coin
    Enter or S   set (starts a game, confirms)
    W, X         select up, down
    arrows       Left/Right slide (steer)
    Q, E         pitch left, right (unconfirmed)

Wave Runner (three coins a credit):
    C, Enter     coin, start
    arrows       Left/Right steer, Up throttle (Z too; it also picks
                 the course)
    V            view
    A, D         lean the body (roll; no effect seen yet)

Over Rev:
    C, Enter     coin, start
    arrows       Left/Right steer, Up gas (it also confirms on the
                 menus), Down brake
    A, Z         shift up, shift down (manual gearbox: pick MT with the
                 wheel on the gear select screen)
    V            view (chase, cockpit, bumper)

Super GT 24h (Model 2B: below full speed in races without recompiled code):
    C, Enter     coin, start
    arrows       Left/Right steer, Up gas (it also confirms on the
                 menus), Down brake
    A, Z         shift up, shift down (manual gearbox: pick MT with the
                 wheel on the gear select screen)
    V            view

The House of the Dead (two coins a credit; light gun):
    mouse        aim; left button fires, right button reloads (or
                 fire off the picture)
    C            coin
    Enter, 2     start player 1, player 2
    Space, R     fire, reload

Behind Enemy Lines (mounted guns):
    mouse        aim; left button fires, right button fires a missile
    C            coin
    Enter, 2     start player 1, player 2
    Space, X     fire, missile

Dead or Alive (Model 2B: below full speed on this build):
    C, Enter     coin, start (2 starts player 2)
    arrows       stick (picks the character)
    Z, X, V      hold, punch (it also confirms), kick

Indianapolis 500 (Model 2B: below full speed on this build; two coins
a credit):
    C, Enter     coin, start
    arrows       Left/Right steer, Up gas (it also confirms on the
                 menus), Down brake
    A, Z         shift up, shift down (manual gearbox: pick MT with the
                 wheel on the transmission screen)
    V, B         view in, view out

All games:
    Esc          menu (pauses): save slots, sound mix, display, game
    F5, F9       quick save, quick load
    F11          fullscreen

Gamepads work too; the one used last drives the game. Top Skater: left
stick steers, right trigger or B slides, A / X jump, Start, Back coins,
shoulders select. Ski Super G: left stick turns, triggers push, X / B / A
are the blue / red / green buttons (Start is blue too), Back coins,
shoulders and Y select and decide. Touring Car: left stick steers, right
and left triggers are gas and brake, shoulders (or B / X) shift up and
down, Y / A are the view buttons. Water Ski: left stick or d-pad
left/right slides, Start or A is set, d-pad up/down select, shoulders
pitch. Wave Runner: left stick steers, right
trigger or A is the throttle, right stick leans, Y is view. Over Rev:
left stick steers, right and left triggers are gas and brake, shoulders
(or B / X) shift up and down, Y is view; Super GT 24h the same, and
Indianapolis 500 too, with A the second view button. House of the
Dead: either stick moves a crosshair (player 2's once the mouse has
moved over the window), right trigger or A fires, left trigger or B
reloads. Behind Enemy Lines: the same, with the left trigger or B firing
a missile. Dead or Alive: d-pad is the stick, X / A / B hold, punch
and kick.

Files
-----
Settings, EEPROMs and save states go in $XDG_DATA_HOME/remodel (normally
~/.local/share/remodel):

    play-settings.txt           settings, as key = value lines
    <set>/eeprom.bin            the game's EEPROM (settings, records)
    <set>/saves/*.save          save states (quick, slot1 ... slot4)

Needs a GPU with Vulkan (or GL) drivers and a sound device.

Licence
-------
Remodel is MIT licensed (LICENSE). THIRD-PARTY-NOTICES.txt lists the
crates built into remodel-play and their licences. Game software and data
remain the property of their respective owners.
