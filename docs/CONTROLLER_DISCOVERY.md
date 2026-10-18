# Background controller discovery

The x86 runtime now caches standard DirectInput controller discovery outside
the caller thread. **The [native verification hold](SECURITY_HOLD.md) remains
in effect.** This public source integration has not been compiled or launched;
earlier standalone results below apply to the matching controller implementation
in a separate development workspace. No native binary accompanies this change.

## Supported behavior

The runtime intercepts ANSI and Unicode DirectInput 8 `EnumDevices` calls for
`DI8DEVCLASS_GAMECTRL` (class 4), with flags `0`, `DIEDFL_ATTACHEDONLY` (`1`),
`DIEDFL_FORCEFEEDBACK` (`0x100`), or their combination (`0x101`).

One worker owns separate native DirectInput objects and runs below normal
priority. It prepares all eight encoding/flag snapshots before the suspended
application is resumed. This preserves a connected controller on the first
discovery call, including variants that are first queried later. Preparation
has a 15-second budget within the existing 20-second bootstrap timeout; failure
refuses startup instead of exposing an unprepared cache.

Game calls read immutable snapshots through `arc-swap` 1.9.3, without a caller
mutex or native enumeration. Refreshes retain old snapshots for active readers.
Callbacks execute on their original caller thread with their original context,
order and stop behavior. Game COM objects never cross to the worker.

Every supported variant refreshes continuously, including unused variants.
The worker pauses one second between full batches. Discovery freshness includes
that pause **plus the eight native scan durations**; a stalled OS call delays
publication. Button/stick polling, mappings, device creation, acquisition and
rumble are unchanged.

Keyboard/mouse classes, mixed `ALL` queries, concrete device types, uncommon
alias/phantom/hidden flags, and invalid or null-callback requests retain their
native path. This is a bounded discovery cache, not a universal asynchronous
DirectInput replacement. In-game evidence must establish whether FFXI uses
these supported queries and whether the original hitch is removed.

Hooks, trampolines and the worker remain mapped for the process lifetime;
this is not an unloadable addon. Installation refuses an already initialized
game or an original Windower/Ashita host. It uses the system DirectInput DLL
and refuses a provider with ambiguous shared ANSI/Unicode hook targets.

## Options and diagnostics

`RuntimeOptions.controller_discovery` defaults to `true`, including options
documents that omit the field. Setting it to `false` selects the original path
for an approved comparison. This option changes no registry value or installed
game file and does not lift any launch/security gate.

Client diagnostics include `controller_discovery`. The native export
`windower_rs_controller_discovery_counter(u32)` reports:

| Index | Meaning |
| --- | --- |
| 0 | Installed (0/1) |
| 1 | Cached caller queries |
| 2 | Published worker scans, including preparation |
| 3 | Failed worker scans |
| 4 | Maximum cache lookup microseconds, excluding caller callbacks |
| 5 | Maximum worker scan microseconds |
| 6 | Distinct query slots used |
| 7 | Startup preparation milliseconds |

Counters establish interception rather than frame-time improvement. Diagnostic
output deliberately keeps `native_hitch_fix_verified` false.

## Evidence and limits

The matching controller module and tests passed nine standalone x86 Windows
tests in the development workspace. They cover real system DirectInput A/W
interception, initial and refreshed results, native keyboard/error forwarding,
synthetic connection transitions, dormant variants, blocked searches, concurrent
publication, callback reentrancy/context/stop behavior, descriptor layouts and
bounded collection. Selected-slot publication generations establish readiness.

In the final standalone run, the longest native worker scan was 422,191
microseconds, while the longest cache lookup was 22 microseconds. Preparing
all eight variants took 1,174 milliseconds. These are one-machine diagnostic
measurements, not an in-game performance benchmark or a general latency bound.
The development workspace also passed 65 launcher-library tests, runtime builds
and Clippy. That workspace contains separate private integration code, so those
results do **not** establish that this public revision builds or is security
cleared. Public source checks and formatting are recorded separately in the PR.

After security review permits an exact test/build scope, verify the public
integration without enabling game launch. Then perform an owner-isolated,
muted game trial with controller support enabled, equivalent cache-off/on
frame-time captures, and increasing interception counters. Physical-controller
acceptance must cover absent/connected startup, disconnect/reconnect, mappings,
focus changes and zoning. If FFXI bypasses the supported queries, extend the
interception with matching evidence instead of inferring coverage.

Do not re-enable native CI, release binaries, restore a blocked artifact, or
use a substitute launcher as part of this source integration. Follow the
existing verification hold for those separate decisions.
