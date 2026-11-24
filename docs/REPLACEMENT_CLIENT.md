# Signet replacement client

Signet is becoming a replacement FFXI client with Vulkan and Direct3D 12 renderers. The
retail-wrapper path (D3D8 hooks, Windower/Ashita adapters) stays as the compatibility route
until the replacement client can play.

Retail note: a third-party client on retail servers breaks Square Enix's terms of service and
can get an account banned. The warning lives in the README, not in every plan.

## Rules

- Original code only. GordianXI (MIT, C#), VanaCore and Kuluu (GPL-3, Rust +
  Bevy) are idea and spec references; their docs may inform formats and
  calibrations, and their output may cross-check Signet's decoders, but no code
  is copied or translated.
- Ships no game files. Reads the user's own install (DATs, ROM folders) read-only.
- Every milestone ends with a check that runs without a person at the keyboard: headless tests,
  image diffs, or a recorded capture.

## Technical decisions

| Area | Decision | Why |
|---|---|---|
| Language | Rust, new crates in this workspace | Signet's existing code, tests and DX12 base are Rust |
| GPU API | Own thin render layer with two backends: D3D12 (`windows` crate) and Vulkan (`ash`) | Both give ray tracing, DLSS, frame generation and HDR. `wgpu` has no stable ray tracing or DLSS path. D3D11 is dropped. |
| Backend order | D3D12 first, Vulkan second, then image parity between them. Vulkan parked 2026-11-23 (Windows only for now) | D3D12 base and NGX tests already pass on the RTX 4090 |
| Shaders | One HLSL source, compiled by DXC to DXIL and SPIR-V | One shader set for both backends |
| Upscaling | DLSS through NVIDIA Streamline (works on D3D12 and Vulkan); FSR as the non-NVIDIA fallback | Covers every GPU |
| Ray tracing | DXR 1.1 / `VK_KHR_ray_query` for shadows, AO, GI; off in Retail mode | Optional, never required to play |
| Render modes | Retail (matches the original client) and Remastered (RT, upscaling, HDR10, HD packs) | Parity first, upgrades opt-in |
| Memory | 64-bit only | Ends the 4 GB limit; HD packs stream freely |
| Mods | Signet's existing pack, profile and override system feeds the new asset loader | Mods work from day one |

## Crates

- `signet-dat`: VFS over the ROM folders, DAT table lookup, texture, mesh,
  skeleton, animation and zone decoders. No GPU code; fully testable headless.
- `signet-render`: render layer, D3D12 and Vulkan backends, frame graph, Retail
  and Remastered pipelines, upscaler and RT plug-ins.
- `signet-client`: window, input, audio, UI, multi-box shell; links the rest.
- `signet-parity`: the retail parity harness (read-only retail probe, parity CSV, comparator); see
  "Retail parity harness" below.
- `signet-launcher`: the desktop launcher (see `ui/launcher/README.md`). Retail first: its
  main action opens PlayOnline (`pol.exe`, no arguments) and it shows the installed client
  build and update state read only. `signet-render` is a secondary offline zone viewer there.
  It starts no server.
- `signet-patch`: the retail patcher (lib plus `signet-patch` CLI). Updates FINAL FANTASY XI from
  Square Enix's patch server without PlayOnline Viewer; see "Retail patcher" below.

## Milestones

1. **DAT reader**: VFS + zone mesh, texture and skeleton decoders, tested
   against the local install (skipped when no install is present).
2. **D3D12 zone viewer**: free camera over any zone, Retail lighting, screenshot
   command. Grows out of the existing model-inspector tools.
3. **Vulkan backend**: same viewer on Vulkan; per-pixel diff against D3D12.
   Done 2026-11-23 in screenshot mode (`--backend vk`, ash, WGSL compiled by naga in build.rs;
   within 1/255 of D3D12 on 3 views, `signet-render-pngdiff`). **Parked** (2026-11-23): Windows
   only for now, new features go to D3D12 only, no Vulkan SDK. Revisit for Linux, Steam Deck or Mac.
4. **Actors**: player and NPC models, skeletal animation, equipment swaps.
5. **Retail parity harness**: render a zone at a set spot, hour and weather and
   diff it against a retail capture; tracked per zone.
6. **Network**: not part of this repository.
7. **Remastered**: RT shadows/AO/GI, DLSS/FSR, frame generation, HDR10.
8. **Multi-box**: several characters in one process, background ones headless,
   picture-in-picture and pop-out windows.
9. **Addons**: sandboxed Lua with Windower, Ashita and native flavors, reusing
   Signet's existing compatibility work.
10. **Retail login**: PlayOnline authentication, then retail play.

Ideas taken from GordianXI's public design (not its code): headless engine core,
single-process multi-boxing, retail-calibrated lighting with plain-retail default,
addon flavor chosen by install folder, signed releases with an SBOM.

Ideas taken from VanaView 1.74-2.1 (By-T0r's Ashita D3D8 plugin, MIT; ideas only, no code),
for milestones 2 and 7:

- Generate the mip chains the retail textures lack, in two classes: ground (opaque or
  non-cutout alpha) and foliage (alpha cutout). Foliage mips keep alpha coverage (scale
  alpha per level so the share of texels passing the cutout stays the same), so thin leaves
  and grass don't vanish at distance.
- Before downsampling cutout textures, fill the RGB of fully transparent texels from the
  nearest opaque texels. Alpha stays identical. This removes dark fringes, because the
  transparent texels in retail data are black.
- Cache generated mips on disk with a size cap, and upload them in small staged batches.
- View settings: draw distance, object range multiplier, map LoD (always the highest-detail
  model, or the game's own switching), and keep distant terrain drawn when turning the camera.
- Two fog modes: the game's weather fog distance, or fog that follows the draw distance.
- Optional per-zone profiles for draw distance, LoD and fog.
- A debug view that tints every material a given feature touched (VanaView tints its
  mipmapped scenery green).
- A one-click diagnostic report: recent frame times, zone id, settings and cache counters.
- Regression tests that rebuild mip chains of captured textures and compare them byte for byte.

## Progress

Milestone 1 (crate `signet-dat`, checked against a Steam retail install, 2026-11-23):

| Part | Result |
| --- | --- |
| File tables (VTABLE/FTABLE, ROM 1-9) | 109,401 ids, 48,259 files, 0 missing |
| Chunk stream | 45,154 files (93.6%) parse cleanly; the rest are other formats |
| Textures (kind 0x20) | 112,295 of 112,341 decode (99.96%): DXT1/3/5, palette 0xB1 and 0x81 |
| Zone meshes (MMB, kind 0x2E) | 66,266 de-obfuscated, 1,121 stored plain ("MMB" magic) |
| Zone layouts (MZB, kind 0x1C) | 596 de-obfuscated, 29 plain, 1.29M placements; every placement in the 3 zones checked names a mesh in the same file |
| Zone mesh geometry (MMB) | 67,385 of 67,387 parse (2 empty stubs): 25.1M vertices, 19.2M triangles, 0 vertices outside their bounding box |
| End-to-end | `signet-dat-zoneshot` software-renders a top-down textured zone; ROM\0\101 and 102 render complete |

Milestone 2 (crate `signet-render`, 2026-11-23): D3D12 zone viewer. `signet-render [DAT] [--screenshot out.png --camera x,y,z,yaw,pitch]` draws every MZB placement with textures, depth and directional light, free-fly camera in the window; ROM\0\101.DAT renders in about 0.8 ms/frame at 1280x720 (RTX 4090). No water or sky yet.

Milestone 2 materials (2026-11-23): the alpha rule, from a census of 539 zone files and the reference viewer:
- `blend & 0x8000`: alpha blend (texture alpha times vertex alpha), sorted back to front, depth write off.
- Mesh name starts with `_` and the texture has clear texels: cutout (`clip(a - 0.5)`).
- Everything else: opaque, and texture alpha is ignored (ground atlases carry alpha that is not a mask).

DXT3 alpha is PS2 range (0x80 = opaque) and is doubled in `texture.rs`.
Generated mips (`signet-render/src/mips.rs`): box filter for opaque textures; for cutouts, edge-color fill plus alpha scaled to keep mip 0 coverage. 16x anisotropic sampling.
Env switches: `SIGNET_ALPHA=off`, `SIGNET_MIPS=box`, `SIGNET_GAIN=<f>` (light gain override), `SIGNET_DUMP_SUBMESHES=1` (table of mesh, texture, blend, pass).

Foliage and ground fix (2026-11-23). Decoded: DXT block rows are stored top-down; palette (0x81/0xB1) rows are bottom-up like a Windows bitmap. The decoder had flipped DXT rows too, so every DXT texture was upside down. The leaf cards (blend 0x2000, V 0.51..0.99 of `ron_w01c`) therefore sampled the bark half and rendered as dark opaque shards; the trunks are 0x0000. Evidence: Hume face `hm_h11_1` upside down, ground UV bands picking rock instead of grass, and the reference viewer maps block row i to image row 4i. Fixed in `texture.rs`; trees and ground in West Ronfaure now match retail. The alpha rule above is unchanged and still the best known. Tuned: nothing here.

Open questions:
- What the 0x1000/0x2000/0x4000 blend bits mean. Seen: 0x2000 on leaf and branch cards, 0xA000 on blended ground and trunk overlays. The viewer's `(blend & 0xF000) >> 12` multiplier is not understood.
- Left-right mirroring against retail is unresolved: the D3D12 and software renders agree with each other, but no retail reference has been compared.
- Plant drop-out and dark fronds were reported before the DXT fix; West Ronfaure no longer shows them, other zones are not re-audited.

Lighting (decoded versus tuned, 2026-11-23):
- Decoded: the 0x2F environment key has a flag byte; dungeons have flag 1. Colors: block A at 0x0c (diffuse, c1 unknown, ambient, fog), block B at 0x2c is about 0.88 of A. Gusgen Mines: diffuse 4a5b53, ambient 4e5f5f, fog black, fog 6.5..74.3.
- Shader: `texture * saturate((ambient + diffuse*|N.L|) * gain)`, then linear fog. Vertex colors are not applied by default (Gusgen mean is 105/255; applying `saturate(vc*2)` roughly halves brightness; `SIGNET_VCOL` turns it on).
- Tuned guesses: `OUTDOOR_GAIN` 0.62 (was 1.3 for everything) and `INDOOR_GAIN` 1.0 (flag 1 keys). Gusgen now reads green-gray and dim with visible rails and timbers, like the retail thumbnail. Unknown: the c1 color (043204 in Gusgen), the f32 at 0x24 (about 1.5), point lights, emissive.

Zone collision (`signet-dat/src/collision.rs`, 2026-11-23):
- **Where it lives:** the u32 at MZB byte 8 points to a 32-byte collision header.
- **Instances:** one 4x4 matrix per placement. Entries are 192 bytes, or 112 in older plain "MZB" zones.
- **Meshes:** vertices, normals, and 8-byte triangles. The top 2 bits of each index are flags of unknown meaning.
- **Grid:** 4-yalm cells, each listing (instance, mesh) pairs.
- **Results:** 597 of 645 MZB chunks decode, giving 100.9M triangles. In ROM\0\101, `ground_height` sits within 0.5 yalm of the visual mesh at 362 of 362 test points.
- **API:** `ground_height`, `ground_height_below`, `raycast`.
- **Unknown:** the flag bits and the exact grid origin.

Zone lookup (`signet-dat/src/zones.rs`, 2026-11-23, ids 256+ added 2026-11-23):
- **Names 0-255:** the XISTRING table in file id 65 (index = zone id). It has exactly 256 entries; a scan of every file id found no XISTRING table with names above 255.
- **Names 256+:** a static array inside the user's `FFXiMain.dll` (read at runtime, nothing stored): 12-byte entries `[long name ptr, 0, short name ptr]`, indexed by zone id (index 100 is West Ronfaure, 230 Southern San d'Oria, 256 Western Adoulin, 280 Mog Garden, 298 Walk of Echoes [P]). Pointers are virtual addresses into the 32-bit image. The array is found by the entry that points at "Southern San d'Oria" / "SSandOria" (index 230). Ids 278, 279, 283, 286 and 287 have no name (null entries) but may still have a DAT, so they open by id only. The auto-translate "Place Names 2" table (`ROM\168\25.DAT`) only holds a subset in its own order and is not used.
- **Model DAT file id:** `100 + id` for 0-255 and **`83635 + id`** above (256 is file 83891, 7 to 13 MB zone files). The earlier `83635 + (id - 256)` rule was wrong for this install: ids 83635 to 83890 are 592-byte placeholders and small unrelated files. Dialog is `6420 + id`, then `85335 + id` (256 is 85591); the NPC list for 256+ is not found (`86490 + id` has no files past 86746), so it stays unverified.
- **`--zone <name|id>`:** `Zones::resolve` takes an id, an exact name, a unique prefix, or a unique substring, case-insensitive. An ambiguous query returns the candidates and `signet-render` prints them (for example `--zone port` lists Port San d'Oria, Port Bastok, Port Windurst, Port Jeuno). Works with `--play`, `--screenshot` and the window path.
- **Default spawn:** with `--zone` and no `--actor`, `--play` searches outward from the centroid of the zone geometry for the nearest collision ground (the bounding-box middle put towns in the sky). Without `--zone` it is the bounding-box middle as before.
- **Not recounted:** earlier totals (467 zone DATs, 314 with an MZB, 299 with collision) were taken before the 256+ rule changed. `signet-dat-zonelist` prints them all and renders towns (into the folder you pass, else the temp folder).
- **New tools:** `signet-dat-xistrings` (finds string tables), `signet-dat-chunks <zone id> [--tree] [--env] [--kind 0x.. --name ..]` (chunk census and environment keys).

Zone environment (`signet-dat/src/environment.rs`, `signet-render/src/env.rs`, 2026-11-23, D3D12 only):
- **Where it lives:** kind `0x2f` chunks (176 bytes) in the zone DAT, named `HHMM` (`0000`, `0500`, `0600`, `1200`, `1700`, `1800`, `2100`). The chunk stream is a folder tree (kind `0x01` opens, kind `0x00` closes); the keys sit under `weat/<weather>/` with weather names `suny`, `fine`, `clod`, `mist`, `bliz`, `thdr`, `rain`, `dust`, `wind` and others. 300 or so zones have them, including ids 256+. Event keys under other folders (`ev01`) are ignored.
- **Decoded by value pattern (not from a spec):** per key, four RGB light colors (alpha is always 0x80): sun/diffuse, a second diffuse that is bright only at night and dawn (read as moon), ambient, and fog/background color; then a fog end distance (mist 69 to 99, clear 340 to 450, dungeons 64 to 89) and a fog start (0 outdoors, 2 to 6 in dungeons); a horizon color; and eight sky gradient colors with eight stops. In event keys the whole sky collapses to the fog color, which is why color 3 is read as the fog color.
- **Not decoded:** sun and moon direction (not in the key, so the old fixed light direction stays), the second 32-byte light block (differs slightly; unused), several constants (56.4, 252.5, 2029.5 and a flags word), and cloud and star layers. Which block retail applies to actors versus terrain is unknown, so one set lights both.
- **What the renderer does with it:** clear color and a sky gradient from the horizon colors (drawn as a full-screen pass using the camera's horizon height; the stop-to-angle mapping is approximate); linear fog from start to end using the fog color; lighting `texture * saturate((ambient + diffuse * |N.L|) * gain)`. Gain is a tuning guess: 0.62 outdoors, 1.0 for flag-1 (dungeon) keys. Vertex colors stay off as before.
- **Time and weather:** default is 12:00 and the first of `suny`, `fine`, else the first weather in the file. Keys are interpolated by hour and wrap past midnight. `--hour H`, `--weather NAME`, `--env off` (old flat look: blue clear color, no fog, neutral shading).
- **Fallback:** a zone DAT without keys (for example zone 0) keeps the old look and the log says so. It is not a per-zone table; it is the same fixed blue sky, no fog and neutral shading everywhere.
- **Shots:** before (`--env off`) and after (before and after folders): West Ronfaure, East Sarutabaruta, Southern San d'Oria, Gusgen Mines (dungeon, black fog) and Rala Waterways (id 258, via the new name lookup).
- **Not compared to retail:** no retail screenshot was available, so the fog start, the gain and the sky mapping are not verified against the retail client. DXT3 alpha is now converted in `texture.rs` for every consumer (`texture::ps2_alpha`); the render crate no longer rewrites it.

Walk mode (`signet-render --play`, 2026-11-23):
- **Movement:** WASD relative to the camera at the retail base run speed, 5.0 yalms/s (server speed byte 50 x 0.1). `--speed-mod N` adds N percent (even values, as retail gear does). The run animation plays at speed / stride (1.194 yalms per cycle).
- **Simulation:** fixed 60 Hz step; ground following through collision; walls block below a 50-degree max slope and the character slides along them.
- **Animation:** blends idle to run over 0.15 s.
- **Camera:** `--camera-mode retail` (default) or `free`. Retail mode uses values read statically from FFXiMain.dll: distance 3.0 to 6.0 yalms (starts at 6.0) and a 57.5-degree vertical FOV (2 x atan(192/350)). Both modes pull the camera in front of walls. Pitch limits (-60/+15), look-at height (1.5), orbit rate and zoom step are UNVERIFIED placeholders until the retail client is measured on the local server. `free` keeps the older modern orbit camera (zoom to 8.0).
- **Command line:** options accept both `--opt value` and `--opt=value` (needed for negative numbers such as `--actor=-92,...`). `--zone <name|id>` replaces the DAT path (see Zone lookup), for example `signet-render --zone "west ronfaure" --play`.
- **Headless proof:** `--script scripts/walk.txt` writes screenshots and a position log to `--shot-dir` (default: a `signet-render-shots` folder in the temp directory).
- **Collision gaps fixed (2026-11-23):** the file lists each collision patch in one cell only, but patches spill past cell borders. `collision.rs` now builds a lookup index from the geometry itself. Across 24 zones the gap rate fell from 0.11% to 0.0015% of samples (`signet-dat-gaps`). The sideways probes in `play.rs` are no longer needed and can be removed.

Milestone 3 Vulkan backend is parked (see Milestones). The D3D12 viewer now also draws a GPU-skinned, animated Hume (`--actor x,y,z,yaw --anim NAME --anim-time S`).

Milestone 4 groundwork (crate `signet-dat`, 2026-11-23): actor formats decode install-wide, none obfuscated.

| Part | Result |
| --- | --- |
| Skeletons (kind 0x29) | 2,804 of 2,805 decode; 30-byte bones (parent, quaternion, translation) |
| Actor meshes (kind 0x2A) | 36,128 of 36,133 decode; 24-byte single-bone and 56-byte dual-bone vertices, weights always sum to 1 |
| Animations (kind 0x2B) | 121,825 of 121,825 decode; per-bone deltas on the bind pose (`rot = anim * bind`, `trans = bind + anim`) |
| End-to-end | `signet-dat-actorshot` renders a textured Hume in bind pose and mid-run |

Not done:
- Skeleton reference points.
- LOD polygon sections.
- Applying the scale channels.
- The "SC"/"C" colored polygon runs.
- 82 vertices with bad values, in meshes named `hf_*`/`hh_w`.
- One skeleton with a different layout (id 5849).

Tools: `signet-dat-scan`, `signet-dat-textures` (DAT to PNG), `signet-dat-texcensus`,
`signet-dat-zones` (`--show <DAT>` prints placements), `signet-dat-zoneshot <DAT> <png> [size]`. The zone key tables are read from the
user's own `FFXiMain.dll` by fingerprint, so Signet stores no client data.

Not done yet: MZB collision data, the 1 MB kind-0x1C blocks in ROM\167-177 (not zone
layouts), skeletons (0x29), actor meshes (0x2A), animation (0x2B), and which zone textures
use alpha as a cutout (ground textures carry alpha that is not a mask; the blend flags on
each sub-mesh are the likely switch).

## Entity models

Mapping (`signet-dat/src/modelmap.rs`, `modeltable.rs`), derived by listing chunk kinds (0x29
skeleton, 0x2A mesh, 0x2B animation, 0x20 texture) across the install and checking anchor models;
not copied from any source. Standard look (model `m`): `0..=1483` is DAT file `1300+m`;
`1500..=2999` is file `50295+m`; Trusts `3000..=3152` are file `99907+(m-3000)` (checked: 3010 is
a Galka, 3086 has texture "zeid"). Models 3153 and up and 1484..1499 are unmapped.

Equipped look (PC or humanoid NPC): race `1..=8` is block `race-1`; race 29 is the Mithra
mannequin and 30 the Hume F mannequin (guessed skeletons). Per race the skeleton is
`[7072, 10248, 13424, 16600, 19776, 19776, 23176, 26352]`. Face, head, body, hands, legs, feet,
main, sub and ranged files are NOT linear: the install's own resource table (FFXiMain.dll,
offset 215128, 432 bytes per race block, 48 bytes per slot, six `(first file id, count)`
extents per slot) is baked into `modeltable.rs`. Model id `n` of a slot walks the extents in
order, so ids of 256 and up land in the later extents (head 320+ starts at extent 71247; id 580
Herculean head is file 98791; body ids 576-588 are files 98819-98831; main weapons 512-639 are
the 128 files of extent 63643). Faces have 32 entries per race (Taru F has its own base); a
face byte with no file falls back to face 0. Weapons are 0x2A meshes (wep0..wep4) with bit 7
set, so vertices go through the mesh's bone table (skeleton bone indices, for example main 5, 15,
6, 3, 13, 10; sub 24, 87; ranged 89, 91); the existing mesh loader already resolves it, so a
weapon is drawn by adding its file to the model. Weapon model 0 means empty hand and is skipped.
The skeleton's trailing reference-point block is a u16 count (128) at offset 4 of the block,
then 26-byte records (u16 bone plus six floats); not used yet.

`signet-dat-idinfo` lists the chunks of one file id; `signet-dat-walkpath` plans a walkable
route over zone collision.

Not done or guessed: the chocobo file (39275) and the misc and automaton model-to-file rules are
guesses and mostly fail to resolve; race 29/30 skeletons are guesses; Trust models 3153+ have no
mapping; engaged weapon position uses the right-hand grip bone 78 and was only checked on the Mithra; which "1" animation variant belongs to which weapon type is unknown; NPC-specific
animation set is not verified; run animation rate comes from a measured stride per model.

Weapon attachment (2026-11-23). Decoded: weapon meshes are skinned to one attachment bone (81..107, children of the pelvis, zero bind translation; by type: claw 104, knife 97, sword/club/rapier 84, axe 103/106, scythe/wand 88, staff 106). The base animations `idl0`, `run0`, `wlk0` do not key those bones, so the weapon sat at the pelvis origin with the blade pointing down: the hip dagger and the thin line below the feet. The weapon animation set is the DAT after the skeleton DAT (Mithra 23177; `wlk1 mvb1 mvl1 mvr1 run1 idl1 gdm1`), and `idl1` puts bone 83/84 at the sheath mount. `ModelRef::anim_files()` adds it for armed humanoids, and armed models default to `idl1`/`run1`. Engaged (`--engaged`, `ActorModel::engaged`): attachment bones take bone 78's world pose, the right-hand grip frame seen in the skeleton tail reference-point records (26-byte records, u16 bone plus 6 f32; refs on bone 78 carry z offsets such as -0.93). Inferred, not verified against retail: that bone 78 is the grip and the blade direction in the engaged pose (it points sideways in the idle pose). Offline test flag: `--look race,face,head,body,hands,legs,feet,main,sub,ranged`. Dev tools in `signet-dat`: `signet-dat-envdump`, `-weapon`, `-skeltail`, `-animbones`, `-animnames`.

## Retail parity harness (`signet-parity`)

Measures the real retail client while it is played normally, then compares it with Signet's `--play`
walk. Retail first: nothing here starts a server, and nothing writes to the game.

- **`signet-retail-probe`** (`crates/signet-parity`). Live: opens the client with
  `PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ` only and samples at 60 Hz (QueryPerformanceCounter)
  into a CSV: player position, heading, effective and server speed, camera eye and look-at, derived
  distance, yaw and pitch, FOV, a quality flag, and passively read keys (GetAsyncKeyState), mouse buttons,
  wheel notches and raw mouse counts (Raw Input on its own hidden window, no hooks). No writes, injection,
  hooks, debugger, packets or synthesized input. It prints a self-check first and refuses to log if a
  signature does not resolve or the values look wrong.
- **Signatures, not addresses.** FFXiMain.dll is found by module name inside the process (pol.exe), and
  the camera object, the player entity, its field offsets, the FOV constants and the camera distance
  constants are found by wildcarded byte patterns (`crates/signet-parity/src/layout.rs`), each unique
  or cross-checked against a second code site. Resolved on the unpacked 2019 image they give the
  offsets below: camera `[[base+0x4444e4]+0x50]`
  with eye +0x44 and look-at +0x50; entity table +0x46e5f8 indexed by the word at +0x473a72, position
  +0x04, heading +0x18, effective speed +0x98, server speed +0x158.
- **Offline check, rerun after every patch.** `signet-retail-probe --static <unpacked image>` runs the
  same resolver on a file. Unpack
  your own `FFXiMain.dll` into a memory image (the unpacking script is not part of this repository),
  then `signet-retail-probe --static <out.bin> --expect none`. The `layout.rs` test
  `real_images_resolve_when_present` reads images named by `SIGNET_PARITY_IMAGE_2019` and
  `SIGNET_PARITY_IMAGE_CURRENT` and skips when they are unset.
  `--expect 2019` (the default) also lists which values moved from the 2019 build.
- **Build 30261102_1 (FOV is now a runtime value).** The projection setup no longer loads a constant
  denominator: FOV = `2*atan(192 / den)` where 192.0 is still a constant double and `den` is a float32 at
  `[[base+<camera global>]+0x2f4]`, so retail FOV is adjustable. `layout.rs` has two projection signatures
  (constant pair, 2019; field, 30261102_1; the field's global must equal the camera global) and
  `Layout::fov_den` is `FovDen::Const(rva)` or `FovDen::Field { global_rva, off }`. Live mode reads the field
  each sample; `--static` prints the numerator RVA, the field location and the SetTransform slot (0x94) but
  cannot show a FOV value, since the denominator exists only in a running client. Use `--expect none` there.
- **`--parity-log <csv>`** on `signet-render --play` writes the same columns from Signet's own simulation
  at 60 Hz, windowed or with `--script`. Scripts take `hold` keys `w a s d` and camera arrows `< > ^ v`, plus
  `wheel <notches>` and `mouse <dx> <dy> [ticks]`; `scripts/parity-routine.txt` is the Signet side of the
  retail routine. The arrow keys now orbit the camera (`ARROW_ORBIT_DEG_PER_S`, UNVERIFIED).
- **`signet-parity-compare <retail.csv> <signet.csv>`** prints a table of retail value, Signet value and
  difference, plus the retail value to use for each UNVERIFIED constant in `play.rs`: run speed and
  acceleration, turn rate, camera follow lag, distance range, zoom step per notch, pitch limits and
  start pitch, orbit rate by key and by mouse count, look-at height, FOV.
  `--make-synthetic <dir>` writes made-up recordings to try it on.
- **Measurement routine:** `docs/RETAIL_MEASUREMENT_ROUTINE.md` (about two minutes in a quiet, flat spot).

Not verified until the patched live run: every offset above is from the 2019 build; the live camera
and entity reads, heading units and axis conventions, and the wheel and mouse-count capture have only
been exercised offline. Mouse sign conventions and the retail key bindings (arrows versus numpad) are
logged raw and decided from the data.

## Retail patcher (`signet-patch`)

Does what PlayOnline Viewer does to update FINAL FANTASY XI: version check, file list
(catalog), compare with the install, fetch each changed file as a whole image or a chain of
deltas, check every result against its catalog signature, install.

- **Server:** `pc001.pol.com`, TCP 53001 (`pc%03d%s.pol.com`, port 53000 + title). Frames are
  `u32 length, 4-byte check, "POLP", u32 operation, payload`; the check is the first four bytes
  of MD5 over everything after the check field. Operations: 1 hello (answered by the catalog,
  2), 3 read (answered by 4 chunk, 64 KB at most), 5 refused, 6 bye, 7 version (answered by 8).
  Client frames carry the tags `W20\0` and `0001`.
- **Catalog:** the same text as `FINAL FANTASY XI\patch.cfg`: `file <path> {`, one line per
  version (`stamp size bytesum md5head <stamp>/Direct/<path>.slc len [<stamp>/Indirect/<path>.olc len]`),
  `}`, `end`. Signature = size, sum of bytes as signed 8-bit, first four MD5 bytes as LE i32.
- **Codings:** `.slc` and `.olc` are LSB-first bit streams after a u32 bit count (`.slc`: literal
  or 16-bit distance/8-bit length copy; `.olc` adds copies from the old file at a signed 16-bit
  offset). Whole-file downloads start with a flag byte: 1 stored, 2 `.slc`, 3 zlib.
- **Decision rule:** missing or unknown file: whole image. Matches an older version: deltas, if
  every later version has one and they total fewer bytes than the image. Otherwise the image.
- **Retail-only guard:** code (.exe/.dll) and data recognition rates must agree within 8
  points, so private-server clients are refused. An install with no `patch.cfg` is allowed.
  The server prunes old version lines (on 2026-11-24 it lists FFXiMain.dll only at 30020917_0
  and from 30210706_0 on), so an old retail install's programs match nothing there. The
  install's own `patch.cfg` therefore counts as a second witness for versions the server no
  longer lists, but only when it shares at least 100 version lines with the server and none of
  them disagree (the 2019 E: install: 3065 shared, 0 disagree).
- **`FINAL FANTASY XI\file.txt`** is local only: `hash:size:path`, where hash is the full
  128-bit MD5 in base64 bit order with the alphabet
  `TSG8IncW3HFKokOg79qzeCmZs2yBYEQVAUxR5rbwi4P@jMDLtpvad0f_J1hlN6uX`. It is the same MD5
  whose first four bytes `patch.cfg` keeps; the patcher does not use it.

Safety: `plan` never writes to the install and downloads no game files. `apply` builds and checks
every file in the staging folder first (`%LOCALAPPDATA%\Signet\patch-staging\<version>\` by default), refuses while `pol.exe`,
`polboot.exe` or anything under the SquareEnix folder is running, then backs up each original
to `%LOCALAPPDATA%\Signet\patch-backup\<timestamp>\` (with a journal) before replacing it
through a checked temp file and a rename. `patch.cfg` goes last. `signet-patch restore
<backup dir>` undoes it. Without `--yes`, `apply` stops after the plan.

Lib API for the launcher: `plan()`, `apply()` (progress callback, `Cancel` token, cancel works
until the install step), `restore()`. The network is behind the `Source` trait; tests use an
in-memory server and a 127.0.0.1 server speaking the real frames.

## Sources

Format knowledge (no code copied):

- galkareeve/ffxi on GitHub (2016 C++ map viewer, no license): the MMB/MZB obfuscation
  scheme, the 100-byte MZB placement record and the MMB piece/sub-mesh/vertex layout. Read for the format only (the entity model-id mapping above was derived empirically from the
  install); Signet's code is
  written from scratch and checked against the local install.
- jondwillis/kuluu-ffxi on GitHub (GPL-3.0), `ffxi-install/src`: the patch-server frame layout,
  operation ids and check value, the catalog and `.slc`/`.olc` formats, the whole-file flag
  byte, the server host and port, the patch decision rule and the code-versus-data lineage
  test. Read for the format only; `signet-patch` is written from scratch. Frame layouts, the
  MD5 check, the host format, the port base, the tags and the catalog line format were checked
  against the local `polcore.dll`/`app.dll`, and the signature arithmetic against the local
  install's files.
