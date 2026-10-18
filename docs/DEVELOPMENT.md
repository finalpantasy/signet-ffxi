# Development and verification

The workspace contains the main library/CLI and `runtime/`. The optional
`providers/luajit/` workspace is deliberately separate. Cargo package and DLL
names retain the internal Windower-compatible naming for protocol and build
stability. The player-facing application name is Signet. Keep protocol/ABI names stable when changing presentation.

## Checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --target i686-pc-windows-msvc --features windower-runtime/game-thread-probe,windower-runtime/client-windowing,windower-runtime/isolated-probe -- -D warnings
cargo test --workspace --locked --target i686-pc-windows-msvc --features windower-runtime/game-thread-probe,windower-runtime/client-windowing,windower-runtime/isolated-probe
cargo test --locked --manifest-path providers/luajit/Cargo.toml --target i686-pc-windows-msvc
```

Synthetic fixtures live in `tests/fixtures/` and `lab/`. The lab's Lua server
bridge is a diagnostic fixture with a fixed local path and controlled test
character. It is not an installed or generally configured server service.
Private controllers, preparation scripts, reports and server/account material
are intentionally absent from this source export. Do not expect a turnkey live
game trial or the original-source replay snapshots from this checkout.

The native workbench is `launcher_gui.exe`. After building, an owned, offline
profile-editor diagnostic can be run against a fresh output directory:

```powershell
# Start from the repository root; choose a new output directory each time.
$demoDirectory = (New-Item -ItemType Directory local/editor-example).FullName
$demoWorkspace = (New-Item -ItemType Directory (Join-Path $demoDirectory workspace)).FullName
$fixtureRoot = (Resolve-Path tests/fixtures/runtime-root).Path
& target/i686-pc-windows-msvc/debug/launcher_gui.exe --self-test $demoWorkspace $fixtureRoot (Join-Path $demoDirectory report.json) (Join-Path $demoDirectory editor.bmp)
```

This diagnostic exercises prototype controls; it is not the planned player UI.
It starts no game. Preserve failed outputs rather than relabeling them as passes.
Read a diagnostic's CLI and feature requirements before running other probes.

## Contribution boundaries

Use fresh evidence for compatibility claims. Real game tests must identify the
owned client/session, build, hooks, command effects and clean exit. Do not attach
to another person's process or mutate shared servers/assets during a trial.

Do not commit game assets, commercial binaries, credentials, accounts, local
server databases, player chat, memory dumps or personal machine paths. Keep
third-party notices and provenance with derived code. DLL hashes establish
file identity; they do not establish publisher authenticity or sandboxing.
