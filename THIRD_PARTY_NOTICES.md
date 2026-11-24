# Windower/Fenestra reference signatures

The FFXI location signatures in `src/pattern.rs` are from Windower/Fenestra
commit `e5bfb6442f49bdb4d31859bff6218bbeb1bea620`,
`core/src/hooks/ffximain.cpp`.

The six display registry key paths and seven display value identifiers in
`runtime/src/display.rs` and `src/display.rs` are based on the same commit's
`core/src/hooks/advapi32.cpp`. This Rust implementation adds bounded result
buffer handling and remains an incomplete display adapter.

Copyright © Windower Dev Team

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.

# Other dependencies and ABI references

The optional local-test menu navigator references the menu layouts/signatures
in the pinned Ashita `autologin` addon version 1.2 and
`config/ashita/ashita.pointers.ini`. Source reference: https://github.com/AshitaXI/Ashita-v4beta
(SDK: plugins/sdk/Ashita.h). No original runtime is bundled.
Its header identifies Copyright (c) 2025 Ashita Development Team and GNU GPL
version 3 or later. The Rust diagnostic uses bounded own-process reads/writes
and an explicit local-only build/path gate. No Ashita DLL is used as its backend.

Cargo.lock identifies the pinned dependencies and their registry checksums.
Those crates retain their respective upstream licenses.

The controller discovery cache uses `arc-swap` 1.9.3 (MIT OR Apache-2.0)
for concurrent immutable snapshot publication. Its source and license files
are provided by the pinned crates.io package identified in Cargo.lock.

The D3D8 forwarding declarations are generated from public COM interface
signatures in the frozen Wine `d3d8.h` reference (SHA-256
`e03c863023535cdb80a7adf97f274da6d74c23ec78e20a3cc2447451c572f89b`).
The reference header is kept outside this Rust project in the tools directory.

## Ashita SDK reference

Copyright (c) 2025 Ashita Development Team. LGPL-3.0-or-later.
Frozen reference: https://github.com/AshitaXI/Ashita-v4beta/blob/4171c74c8ddb2ca2a31654f199e6c1cee40d7256/plugins/sdk/Ashita.h
SDK SHA-256: 6eef8612c51d49a59a3c8b7bfd81f56b297d585e791520eba3201a060d4ffae6.
The controlled C++ fixture and Rust ABI adapters study selected public interfaces;
the SDK and original runtime are not bundled.

## Wine D3D8 declaration reference

Copyright (C) 2002 Jason Edmeades. LGPL-2.1-or-later.
Source: https://github.com/wine-mirror/wine/blob/master/include/d3d8.h
The frozen header identity is recorded above; the link identifies the upstream project.
Generated Rust COM declarations preserve this attribution.
No original Wine header is bundled. Upstream LGPL-2.1 terms:
https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
The public export includes the LGPL-3.0 supplement under licenses/.

## Replacement client crates and launcher UI

Applies to `crates/signet-dat`, `crates/signet-render`, `crates/signet-launcher`,
`crates/signet-parity`, `crates/signet-patch` and `ui/launcher`. Their Rust dependencies are
pinned in Cargo.lock and the UI's in `ui/launcher/package-lock.json`; each keeps its own license
and the package files carry the full texts.

Direct Rust dependencies (licenses from `cargo metadata`):

- `ash` 0.38 (Vulkan bindings): MIT OR Apache-2.0.
- `naga` 25 (build script only; compiles the WGSL shader to SPIR-V): MIT OR Apache-2.0.
- `windows` 0.62.2 and `windows-sys` 0.61.2 (Microsoft Win32 bindings): MIT OR Apache-2.0.
- `wry` 0.57 (WebView2 host) and `tao` 0.37 (window): Apache-2.0 OR MIT (`tao`: Apache-2.0).
- `window-vibrancy` 0.8 (Mica backdrop): Apache-2.0 OR MIT.
- `md-5` 0.10 (patch file check values): MIT OR Apache-2.0.
- `miniz_oxide` 0.8 (patch file decompression): MIT OR Zlib OR Apache-2.0.
- `serde` 1 and `serde_json` 1: MIT OR Apache-2.0.

The Windows build of these crates pulls no MPL-2.0 packages; a few MPL-2.0 crates in
Cargo.lock (`cssparser`, `selectors`, `option-ext` and similar) come only from non-Windows
targets of `wry` and `tao`.

Launcher UI (bundled into `ui/launcher/dist` by Vite): `react` 19.3.1 and `react-dom` 19.3.1 (MIT).
Build and test tooling only, not shipped: `vite` 8.3.4 (MIT) and `playwright-core` 1.61.1 (Apache-2.0).

File-format and protocol sources. No code was copied from any of these; the decoders and the
patcher were written from scratch and checked against a local install:

- galkareeve/ffxi (2016 C++ map viewer, no license): read to understand the MMB/MZB
  obfuscation scheme and the zone mesh and placement layouts.
- jondwillis/kuluu-ffxi (GPL-3.0): read for ideas on the patch-server frame layout and
  catalog formats. No Kuluu code is used.
- GordianXI (MIT), VanaCore (GPL-3.0) and VanaView (MIT): design ideas only.
- Movement speed values follow the public bg-wiki "Movement Speed" page.

The 256-byte tables that de-obfuscate zone chunks are not stored in this repository.
`signet-dat` finds them in your own `FFXiMain.dll` at run time using a byte-sum and hash
fingerprint. FINAL FANTASY XI and its files belong to Square Enix; none are included.
