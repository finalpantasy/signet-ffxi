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
