# Native player contract v1

The compact React surface now has two explicit modes. The static preview remains
sample-only. `player_server` serves the same built interface with a native mode
marker; a native connection failure never loads sample profiles as a fallback.

## Run

Build the existing i686 workspace with `windower-runtime/isolated-probe` to include
`profile-addon-host.exe`, and build `ui/player-preview` with its pinned dependencies.
Use a dedicated workspace with existing native `Document` JSON files in `profiles/`.
Do not point this development adapter at another task's running workspace.

```text
player_server WORKSPACE DIST PORT [OFFLINE_HOST SHA256]
```

`PORT=0` allocates a free loopback port. The first stdout line is a JSON identity
receipt with the bound address and this server's PID. Open that address. No browser,
FFXI client or server service starts automatically. Offline addon tests appear
under Advanced when the exact host executable and SHA-256 were supplied.

## Contract

| Route/action | Actual behavior |
| --- | --- |
| `GET /api/v1/snapshot` | Native documents, saved selections, history, capability limits and last owned runtime reply |
| `POST /api/v1/rpc`, `profile` | Wrap a version 1 native profile command in `request` |
| `save_profile` | Existing document `id`, unchanged `name`, width, height, UI scale and enabled addon names; append via `Document.save_new` |
| `copy_profile` | Existing document `id` plus a new native name |
| `restore_profile` | Current `id` plus a historical `version` from the same profile; append restored values |
| `packages` | Existing `package_editor::Inspect`; active native package generation and slots |
| `start_offline` | Document `id`; start only the configured SHA-pinned capture-only Lua host |
| `runtime` | Typed `status`, `load`, `reload` or `unload`; optional selected addon `name` |
| `stop_runtime` | Shutdown and reap only the host this server created |
| `shutdown` | Close the owned runtime and this server; release this workspace's lock |

Every RPC has `schema_version: 1`, JSON content type, exact same Origin and
`X-Signet-Request: 1`. Host must match the bound loopback address. Unknown fields,
unsupported versions and foreign/stale profile IDs are refused. There is no CORS,
remote bind, arbitrary filesystem endpoint, PID attach or browser-selected executable.
The server serves only its built HTML/JS/CSS/SVG directory, with a restrictive CSP.
Requests, headers, native profiles and host replies have byte limits and deadlines.
This is a local development adapter, not an OS sandbox for trusted Lua or plugins.

## Persistence and session scope

Native `Document` files are the source of truth. Every save runs the existing
file/provider validators and appends a new immutable version. Original files remain.
One player server holds a workspace lock. A stale browser revision cannot overwrite
a newer version. A crashed server's lock requires identity inspection before manual
removal; it is never silently taken over. Other native tools can append documents
independently; the lock serializes player adapters, not arbitrary external editors.

The available addon library is derived from actual document history sharing each
native runtime root. A profile's own engine/ecosystem declarations win. Enable
controls change the next native selection. The running host keeps the exact launch
document; profile edits or switching profiles do not change that host. Loaded state
comes from its real lifecycle reply and source digests, distinct from saved selection.
A failed Lua reload retains the previously loaded addon and reports the error.

Startup rejects historical profile IDs. Snapshot replies check owned-process
liveness; the browser polls during an active test and reconciles state after
failed or uncertain operations. Disconnection invalidates active/loaded claims
until a successful refresh. Normal stop waits for the child's exit and returns
`exit_code: 0`; a shutdown reply alone is not considered a normal exit.

Addon-only saves use persisted display values, preserving unfinished display
edits until Save profile is explicitly selected. Profile appends are refused at
4,096 JSON versions before writing. Completed bytes are flushed to a `.pending`
file, then atomically published without replacing an existing JSON version.
Sequence allocation starts above the highest existing version, including damaged
files, so archived gaps cannot hide new saves. Old interrupted staging files are
ignored. Damaged JSON versions are preserved and reported in snapshot `warnings`;
`requires_restore` identifies a readable profile with an unreadable newer revision.
The Profiles Advanced section offers explicit restore, including the latest
readable version in that recovery case. Runtime startup and ordinary saves refuse
the damaged head until recovery. Imported files must keep native `name-NNNN.json`
version names.

Compatibility labels remain partial for Windower/Ashita. Offline host success only
establishes the tested Lua APIs and lifecycle; it does not establish game-thread
behavior, packet delivery or universal addon compatibility. Package installation,
activation and prepare remain in the existing native workbench/CLI in this batch.

## Verification and remaining work

`scripts/verify-player-native.py` provisions only an owned fixture workspace and
package, uses a muted fresh browser, verifies native saves/reload/copy, lifecycle,
syntax-error rollback, origins/paths/versions, exact source digest, neighbor
preservation and normal cleanup. `tests/player_service.rs` covers stale writes,
invalid paths/settings/selections, workspace ownership, copying disabled addons,
restore isolation and per-profile compatibility declarations. The static sample
preview has its own regression suite.

The recovery driver adds lost-response reconciliation, damaged-version restore,
addon-only draft isolation, stable profile selection, prototype-like names,
concurrent operations and case-insensitive addon catalog checks. Its browser
cases use real native responses and fresh muted browser sessions. Test receipts,
fixture profiles and compiled binaries are generated locally and are not included
in the source checkout.

Game Play remains unavailable in this adapter. Connecting the local client still
requires verified process-lifetime USER isolation, loader order and independently
owned endpoints, followed by exact-build local-game acceptance.

Native asset/UI/font overrides, named visual folders, generated addon settings,
graphics-provider installation/recovery, measured pacing, updates and native plugin
hosting remain unsupported by this player adapter. 64-bit remains deferred.
## UI state follow-up

Profile selection follows its unchanged native name across revision refresh,
while every operation still sends the displayed document's exact native ID. Draft
and loaded-source lookup uses own properties, so names such as `constructor` and
`__proto__` have ordinary behavior. A failed package inspection clears the old view;
return to Addons & Mods to retry. These browser checks do not establish live-game acceptance.

## Concurrent browser operations

The UI keeps mutation actions disabled until every pending browser operation and
its error reconciliation completes. A package read finishing first cannot unlock
a pending profile mutation. Navigation and input remain available; save/restore
completion clears the submitted draft only if it has not since changed. Late
save/restore replies preserve the current selected profile. Copy selects the new
profile only while its source is still selected, and clears the copy-name input
only if it still contains the submitted name. Newer edits remain pending for an
explicit save. Five real-operation response-delay cases are in
`ui/player-preview/tests/native-concurrency.mjs`.

## Addon catalog identity

Shared and historical catalog entries use the runtime's lowercase addon identity.
Case aliases such as example/Example produce one toggle. A profile's own latest
historical declaration still supplies the displayed spelling, ecosystem and engine,
including after disabling and re-enabling the addon. The case-alias regression is covered by
`tests/player_service.rs` and `ui/player-preview/tests/native-addon-catalog.mjs`.
