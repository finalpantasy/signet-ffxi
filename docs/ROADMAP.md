A browser prototype now demonstrates sample profile, library, and settings flows; native integration remains planned. See `ui/player-preview/README.md`.

# Player experience roadmap

Everything below is planned unless the README explicitly identifies it as
currently working. The goal is a single understandable FFXI application for
players and addon/mod authors, with a Rust runtime and compatibility adapters.

## Everyday use

- A clear home screen, selected profile and prominent Play action.
- Readable frosted glass, independent UI/text scale, keyboard navigation,
  reduced motion/transparency and an opaque high-contrast option.
- Named character/server profiles with duplicate, backup, restore and readable changes.
- Import supported Windower/Ashita settings without changing the original setup.
- Search settings by intent, such as “make text bigger.”

## Addons, plugins and visual packs

- One searchable library with descriptions, authors, categories and specific
  compatibility status: tested, partial, untested or unsupported.
- Dependencies, favorites, profile membership and reversible configuration.
- A native asset override manager with named folders and checkboxes, optional
  pack components, load priorities and per-file conflict explanations.
- UI, font, texture, icon and model replacements without original-file overwrites
  where validated interception supports them. Trace unsupported loading paths
  rather than claiming every package works.
- Show requested versus confirmed-loaded state and restart requirements.
- Keep concurrent clients' profile and asset overrides independent.

## Configuration and updates

- English labels, descriptions, toggles, sliders, paired number entries,
  units, validation, defaults and Reset.
- A versioned declarative author settings schema to generate configuration forms.
- Reviewed adapters for popular legacy addons; arbitrary old settings cannot
  automatically become a safe generated form.
- Automatic update checking, readable changelogs, pinned versions, dependency-aware
  staging and rollback. Unattended installation is a separate preference.
- Offline operation for installed components without a mandatory launcher account.

## Graphics and tools

- Detect hardware and recommend a tested modern translation provider.
- Evaluate supported D3D11/12 or Vulkan paths by correctness and measurements,
  not merely the highest API number. Respect provider licensing.
- Configure display choices and verify the provider/settings actually loaded.
- Revert failed tests; preserve a working setup.
- Measure frame times and validate game timing separately from FPS targets.
- Resource diagnostics, recovery launch, controller setup and supported HUD layout editing.
- Matching original/candidate asset previews and optional capture tools.
- Author SDK, isolated reload tests and an evidence-based compatibility catalog.

## Suggested sequence

1. Clear player UI prototype: home, profiles, library, mod folders and settings.
2. Native override foundation: real resource traces, early font/UI loading,
   priorities, conflicts, no-overwrite verification and client isolation.
3. Graphics setup with verified settings, measurements and failure recovery.
4. Everyday import/configuration/update workflows; expand from real compatibility errors.

Full native plugin hosting and broad game APIs remain separate implementation
work. True 64-bit game conversion is deferred.
