# Signet launcher

One native Windows window for retail FINAL FANTASY XI. Its main action opens PlayOnline the
normal way; it also shows whether the installed client is up to date. Signet's own renderer
is a secondary, offline zone viewer ("Explore"). The launcher starts no server and has no
private-server or local-server path.

## Build and run

```powershell
cd ui\launcher
npm install          # once
npm run build        # writes dist/, which the exe embeds
cd ..\..
cargo build --release -p signet-launcher -p signet-render
.\target\release\signet-launcher.exe
```

`cargo build -p signet-launcher` embeds whatever is in `ui/launcher/dist` at build time. Rebuild
the exe after `npm run build`. Without `dist`, the window shows a short "UI not built" page.

The launcher looks for `signet-render.exe` next to itself, then in `target\release`, then in
`target\debug`.

### UI development

```powershell
npm run dev                                           # http://127.0.0.1:5199, browser preview
..\..\target\release\signet-launcher.exe --snapshot dev-data\snapshot.json   # real data for the preview
..\..\target\release\signet-launcher.exe --dev-url http://127.0.0.1:5199/    # native window on the dev server
npm run shots                                         # every screen and state, light and dark, 1280x800 and 1920x1080
```

The browser preview reads `dev-data/<name>.json` (`?snap=<name>`), so it shows real zones
and client state. It never starts anything; actions say they run only in the window. Query
overrides for screenshots: `pol=<pid>` (PlayOnline running), `viewer=<zone id>`,
`client=current|outdated|missing|unknown`, `exit=<code>`, `slow=1` (loading state).
`dev-data/` is not part of the build.

`copy-strings.txt` holds every user-facing string, grouped by source file, for the copy scans
(`banned_phrase_scan.py` and `structure_scan.py --genre docs` from the unslop skill). Regenerate
it when copy changes.

### Zone art

`scripts/render-zone-art.ps1` renders every entry in `scripts/zone-art.json` with
`signet-render --screenshot`, converts it to JPEG and writes `public/zones/*.jpg` plus
`public/zones/index.json`. `zone-art.json` is the only list: `hero: true` marks the default
Home image (East Sarutabaruta), `alt: true` a second view of a zone that is kept but not used
as that zone's art (the Gusgen Mines cart tunnel). Re-run it after renderer changes (sky, fog, lighting), then
`npm run build` and rebuild the exe. `-Only <slug>` re-renders one image.

The images are renders of your own install, so they are not stored in this repository
(`public/zones/` is git-ignored). Without them the launcher shows its plain backgrounds. Run the
script with `-Root <your FFXI install>` (or set `SIGNET_FFXI_ROOT`) to make them locally.

Camera spots came from `signet-launcher-scout <zone.DAT>`, which ranks open vistas in the
zone's collision mesh, followed by test renders. An entry's `spawn` (x, z, yaw) is also where
Explore places the character for that zone.

## Architecture

- `crates/signet-launcher` (Rust, bin `signet-launcher`): a `tao` window with a Windows 11
  Mica backdrop (`window-vibrancy`), native title bar (snap, resize, min/max/close all native)
  and a transparent WebView2 (`wry`). Light, dark or system theme sets the title bar, Mica
  tint and WebView color scheme together.
- `ui/launcher` (React + Vite): the screens. Built files are embedded into the exe by
  `build.rs` and served from the `signet://` custom protocol (`http://signet.localhost/`).
- Bridge: WebView2 `postMessage` only, no HTTP server and no open port. Each message is
  `{id, cmd, args}`; replies come back through `window.__signetReply`.
  - Navigation and IPC are accepted only from the app origin (or the explicit `--dev-url`).
  - Commands are a fixed list (`init`, `zones`, `settings.set`, `status`, `explore`,
    `explore.stop`, `retail.info`, `retail.play`, `open`). Arguments are typed with
    `deny_unknown_fields`, and every value checked before use: zone ids against the
    install's zone table, speed within -50..100, resolution from a fixed list, folders from
    fixed names. No path or free text from the page reaches a command line.
  - A Content-Security-Policy limits the page to its own scripts and images.
- Settings: `%APPDATA%\Signet\launcher.json` (zone, camera mode, speed-mod, theme,
  resolution, favorites, recent), written through a temp file and rename.

### Retail

- Play FINAL FANTASY XI runs `pol.exe` with no arguments through ShellExecute, like its
  shortcut. pol.exe asks Windows for approval itself (its manifest needs administrator
  rights). The folder comes from `HKLM\SOFTWARE\WOW6432Node\PlayOnlineUS\InstallFolder`
  value `1000`, else `PlayOnlineViewer` next to the FFXI install. No profile, ini or
  server arguments.
- PlayOnline is found running by full image path (FFXI runs inside pol.exe), whether the
  launcher started it or not. Play is disabled while it runs, so it cannot start twice.
- Read only: `FFXiMain.dll` size, date and SHA-256, and the installed version (highest
  `NNNNNNNN_N` in the install's `patch.cfg`; `30190430_0` is the 2019-04-30 build). The
  client counts as out of date when its build is more than 31 days older than the PlayOnline
  viewer's own `patch.cfg` version, or more than 62 days old. Then Home and Retail say
  "Update through PlayOnline: Play, then FINAL FANTASY XI, then Play."

### Zone viewer

- Explore runs `signet-render --play` for the selected zone, offline, with the camera, speed
  and resolution settings. One viewer at a time; Close viewer sends WM_CLOSE to its window
  and ends that one process only if it is still open after 15 seconds (the window appears
  only once the zone has loaded, so WM_CLOSE is retried until it exists).

## Limits

- Windows 11 only for Mica; elsewhere the page paints its own opaque backdrop (not tested on
  Windows 10). The window has no GDI redirection bitmap, so Mica shows through WebView2.
- Window mode for Signet is windowed only (signet-render has no fullscreen flag yet).
- Mods is a placeholder; mods still run in the 32-bit runtime.
- Explore runs `signet-render --play`. Zones without decodable collision exit at once;
  the Home screen shows the exit code and the last log line
  (`%APPDATA%\Signet\logs\signet-render.log`).
- The PlayOnline launch was built but not triggered during development (it needs a person to try it).
- "Out of date" is inferred from dates; the launcher cannot ask Square Enix for the current
  version. PlayOnline itself checks when it starts.
