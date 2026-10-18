# Signet for FFXI — player interface preview

A compact, frosted-white utility with Play, Profiles, Addons & Mods, and Settings.
Graphics and app options share Settings; technical choices are under Advanced.
Application name: Signet. Repository: `finalpantasy/signet-ffxi`.

This is **sample data saved in the browser**. Play opens a setup summary; it does
not start FFXI. Mod installation, game settings, native providers, and update
checking are not connected. No audio, media playback, or external requests.

## Try it

The owned preview runs at http://127.0.0.1:58473/ while its local server is running.

- Create, copy, rename, remove, and switch independent profiles.
- Enable named mod folders, reorder priority, and inspect file conflicts.
- Search and filter addons, mods, and plugins.
- Configure typed fields with sliders, numbers, choices, and validation.
- Save separate graphics preferences for each profile.
- Adjust the app's actual text size and transparency.
- Reset only this preview's saved state.

## Develop

Verified with Node.js 26.5.0 and npm 11.17.0. Install dependencies on a local,
non-synced filesystem. Versions are pinned in package.json and package-lock.json.

```powershell
npm ci --ignore-scripts
npm run format:check
npm run build
npm run preview -- --port 58473
```

Use an unused loopback port if this one is occupied. Do not stop another process.

## Verification

```powershell
npm run test:browser -- --url http://127.0.0.1:58473/ --output local/new-acceptance
```

The output directory must be new. An isolated headless Chromium runs muted,
checks profile isolation, generated forms, persistence, filters, priority,
keyboard focus, scoped reset, and narrow layouts. Nine axe scans cover desktop,
dialogs, mobile, and enlarged text. Browser binaries are not included; a matching
Playwright Chromium installation is needed. Automated checks do not establish
full accessibility compliance or game integration.

## Design

The previous dark illustrated prototype was rejected. The current interface uses
the user's compact Windows utility reference: pale frosted surfaces, dark
sans-serif labels, teal actions, small radii, and minimal copy. It loads no scene
artwork. The prior concept files remain historical and are not the current design.
See design/DESIGN.md for the built system. CSS translucency is a browser preview;
native Windows backdrop integration is not implemented by this frontend.
