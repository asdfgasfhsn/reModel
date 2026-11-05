Remodel @VERSION@ (@TARGET@)
===========================

remodel-play runs Sega Model 2 arcade games in a window: the game's own
code on an emulated board, drawn on the GPU, with both sound boards live.
Supported so far: Top Skater (topskatr), Sega Ski Super G (skisuprg),
Sega Touring Car Championship (stcc), Sega Water Ski (segawski) and
Wave Runner (waverunr).
This build runs the game code on the interpreter (no recompiled game code
is included).

Bring your own ROMs
-------------------
No game data is included. Put the sets as MAME keeps them, topskatr.zip or
a topskatr/ folder (likewise skisuprg, stcc, segawski, waverunr), in the ROM folder:

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
trigger or A is the throttle, right stick leans, Y is view.

Files
-----
Settings, EEPROMs and save states go in $XDG_DATA_HOME/remodel (normally
~/.local/share/remodel):

    play-settings.txt           settings, as key = value lines
    <set>/eeprom.bin            the game's EEPROM (settings, records)
    <set>/saves/*.save          save states (quick, slot1 ... slot4)

Needs a GPU with Vulkan (or GL) drivers and a sound device.
