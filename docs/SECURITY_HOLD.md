# Native verification hold

Native build and launch acceptance is paused pending security review. Source
hardening does not establish that a blocked artifact is safe or a false positive.
No native binary release accompanies these changes.

## Current source policy

- Play remains unavailable in the player service.
- The legacy `launch-client` command is refused before input or process work.
- `launch-local` is also refused in this public snapshot because the separate
  owner/private-USER/mute integration is not included here. It must not be used
  as a substitute route.
- Every low-level startup helper requires an explicit runtime path and SHA-256
  identity. The CLI diagnostic argument requires a reviewed pin JSON rather than
  a raw DLL path. Old calls must be deliberately migrated; there is no fallback.
- The common startup boundary checks identity and i686 DLL shape before process
  creation. File and ancestor handles are retained through bootstrap to refuse
  writes/replacement. Access or security denial is a hard refusal.

Pins are externally reviewed expectations. Do not create approval by hashing a
selected artifact or silently update a mismatched pin. Equality to a pin does
not establish publisher trust or security clearance. The runtime's dependency
DLLs and privileged mutation are outside this check.

## Evidence limits

The new hardening patch has passed syntax/format and source-application checks.
Its native compilation, library tests, Windows sharing behavior, and launch
acceptance remain unverified. The four included regression tests use synthetic
header bytes and never load a DLL or create a child; they have not been run for
this public patch. Earlier build/test results apply only to their exact commits.

CI currently runs a static source-contract checker, including seven deliberate
source mutations. This can catch certain accidental policy regressions. It is
not Rust type checking, behavioral testing, or a security audit. The normal
Windows build/test job is intentionally disabled; a green source-only workflow
does not establish that the native code builds or that the launcher is safe.

## Resuming verification

An explicit reviewed decision must identify the permitted exact source/test
scope before any test-only compilation. Production builds or client trials need
separate security and artifact review, owner-bound isolation, lifetime private
USER routing, pre-start mute, and any required network approval. Enabling the CI
native job or Play requires that review; neither happens automatically here.

Do not restore, rename, repack, rebuild or run a substitute to evade a security
block. Do not weaken endpoint protection or change firewall rules to make a
test pass. Sample retrieval and external submission are separate decisions.
Private security receipts and machine/account details do not belong in Git.
