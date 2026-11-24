# Retail measurement routine

About 2 minutes, done once on the patched retail client. Nothing here changes the game: the probe only
reads memory and key state.

**Where:** a quiet, flat, open spot (no walls within 40 yalms, no other players or monsters moving
nearby, not in a town crowd). Standing character, no movement-speed gear or buffs, not engaged, default
camera (do not touch it before step 1). Run the game in a window or full screen, but keep it the active
window the whole time.

## Before

1. Log in to the character and stand in the spot. Do not move the camera.
2. In a terminal (same Windows user as the game, not elevated unless the game is):

   ```
   target\debug\signet-retail-probe.exe --out parity\retail.csv --seconds 150
   ```

   It prints a self-check. Continue only when the last check line says `SELF-CHECK PASS` and it starts
   `logging 60 Hz`. If it says FAIL, stop and keep the output for a bug report.
3. Click the game window so it is in front. The probe keeps running behind it.

## Steps (watch a clock; each is under 10 seconds)

1. **Stand still, 3 s.** Hands off the mouse and keyboard.
2. **Run forward, 5 s.** Hold W for 5 s, release, then stand still 3 s.
3. **Camera left, 3 s.** Hold the left arrow for 3 s (the camera swings around the character). Release.
   Then hold the right arrow for 3 s. Release. If the arrows move the character instead of the camera
   in your key setup, use numpad 4 and 6.
4. **Camera pitch.** Hold the up arrow (numpad 8) 2 s, to the top limit. Release. Hold the down arrow
   (numpad 2) 3 s, to the bottom limit. Release. Stand still 1 s.
5. **Mouse orbit.** Hold the right mouse button and drag the mouse slowly to the right for about 1.5 s,
   then up and down for about 1 s. Release. If the right button does not orbit the camera, repeat with
   the left button.
6. **Zoom in.** One wheel notch per second, scrolling toward you or away as needed to zoom in, 6 notches
   (past the minimum distance). Wait 1 s.
7. **Zoom out.** The other direction, one notch per second, 6 notches (past the maximum distance).
   Wait 1 s.
8. **Walk a square.** Hold W 2 s. Hold D 1 s (turn). Hold W 2 s. Hold D 1 s. Stand still 2 s.

Then press Ctrl+C in the terminal (or wait until the probe stops by itself at 150 s). It prints
`wrote N rows`. The file is `parity\retail.csv` (the `parity` folder must exist).

## After

Signet side and comparison, from the repository root:

```
target\debug\signet-render.exe --play --script crates\signet-render\scripts\parity-routine.txt --parity-log parity\signet.csv --actor -92,auto,77,90
target\debug\signet-parity-compare.exe parity\retail.csv parity\signet.csv
```

The table lists retail value, Signet value and difference per behavior, and the value to put in each
UNVERIFIED constant in `crates/signet-render/src/play.rs`.

## If the game changes (after a patch)

Re-check that the probe still finds everything, offline, before the next recording (unpack your own
`FFXiMain.dll` into a memory image first; the unpacking script is not part of this repository):

```
target\debug\signet-retail-probe.exe --static <unpacked image of your FFXiMain.dll> --expect none
```

Every signature must say `ok` and the last line must be `SELF-CHECK PASS`.
