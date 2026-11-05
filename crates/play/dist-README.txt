Remodel @VERSION@ (@TARGET@)
===========================

remodel-play runs Sega Model 2 arcade games in a window: the game's own
code on an emulated board, drawn on the GPU, with both sound boards live.
Supported so far: Top Skater (topskatr). This build runs the game code on
the interpreter (no recompiled game code is included).

Bring your own ROMs
-------------------
No game data is included. Put the sets as MAME keeps them, topskatr.zip or
a topskatr/ folder, in the ROM folder:

    ~/.local/share/remodel/roms     ($XDG_DATA_HOME/remodel/roms)

or start the player, and in the menu type another folder under "ROM
folder" and press Scan. With no supported set found the player opens in
the menu. `remodel-play --rom DIR` sets the folder from the command line.

    ./remodel-play            play the last game (or open the menu)
    ./remodel-play --help     all options

Keys
----
    C, Enter     coin, start
    arrows       steer (Down slides)
    Z, X         jump front, jump tail
    Q, E         select
    Esc          menu (pauses): save slots, sound mix, display, game
    F5, F9       quick save, quick load
    F11          fullscreen

Gamepads work too; the one used last drives the game.

Files
-----
Settings, EEPROMs and save states go in $XDG_DATA_HOME/remodel (normally
~/.local/share/remodel):

    play-settings.txt           settings, as key = value lines
    <set>/eeprom.bin            the game's EEPROM (settings, records)
    <set>/saves/*.save          save states (quick, slot1 ... slot4)

Needs a GPU with Vulkan (or GL) drivers and a sound device.
