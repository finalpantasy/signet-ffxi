# Windower 4 native ABI evidence

Target Hook SHA-256:
`68d059e8a9e332b274464d6b744a0732bc4a09a49c67d9c07c301c06122ccaa7`.
Interface version is **0x04070300**, independently of file version 4.7.9.3.

The loader (0x1006eb70) calls factories without arguments. After a non-null
instance, it pushes self and calls vtable slots 0 and 1 for author/name. At
0x1006f9aa it pushes the host, pushes self, and calls slot 2. Config's slot 2
adapter ends in `ret 8`; its slot 3 shutdown adapter ends in `ret 4`. Explicit
stdcall self is required. Internal virtual methods reached by these adapters
are thiscall and must not be confused with the public boundary.

| Instance slot | Observed behavior | Config adapter |
| --- | --- | --- |
| 0 | Author string | 0x1004d270 |
| 1 | Name string | 0x1004d260 |
| 2 | Initialize(self, host) | 0x1004d150 |
| 3 | Shutdown and destruction(self) | 0x1004d130 |
| 4–16 | Callback adapters; types/order need validation | 0x1004d120 onward |

Config's initialization calls host slot 3, stores the result at instance+8,
then invokes its own path setup. That setup calls host slot 0 and reads narrow
strings at offsets 0x150, 0x1150 and 0x2150 in the returned object. This is a
different representation from the launcher's 42,180-byte UTF-16 packet.
Passing that packet as the plugin settings object would be incorrect.

Host table prefix at **0x10185d70**, observed in the shipped Hook:

| Slot | Function | Observed return |
| --- | --- | --- |
| 0 | 0x10072a80 | `*[0x101cdf64] + 0x10` |
| 1 | 0x10072a70 | `*(*[0x101cdf64] + 4)` |
| 2 | 0x10072a90 | `*(self + 0xa14b4)` |
| 3 | 0x10072aa0 | `*(*[0x101cdf64] + 8)` |
| 4 | 0x10072ab0 | `*(*[0x101cdf64] + 0x3190)` |
| 5 | 0x10072ac0 | `*(*[0x101cdf64] + 0x3194)` |
| 6 | 0x10072ad0 | `self + 8` |
| 7 | 0x10072ae0 | Singleton access; full type unresolved |

Slots 0–6 end in `ret 4`. The following pointer is a constructor, not evidence
of another public slot. Manager names, settings layout, ownership, thread
requirements, callback types/order and slot 7 still need recovery/validation.
No complete host vtable is implemented and native plugin initialization remains
disabled. Config also scans game code and starts a thread during initialization;
factory/metadata success does not establish that initialization can run without
FFXI present.

Ghidra evidence IDs:

- Loader assembly: `ev_3911cec21810195a4e097880d63a8b8a147ff61c4b8f86ccf4563c8c76bbb0b1`
- Startup callsite: `ev_441094296b21d7a655895964809a18c3e54627b4db3898211811f0ffd6a55f4f`
- Host settings getter: `ev_3881c49bb67099d7afb79c07a7718ae993e8fd8f9c9c843748ae38ada23ed986`

Other listed instruction addresses were read using pefile/Capstone against the
same Hook and the local Config.dll. They are static observations, not a live
callback trace. Existing metadata probes intentionally retain uninitialized
objects/modules until their dedicated process exits: WinControl crashes when
shutdown is invoked before initialization.
