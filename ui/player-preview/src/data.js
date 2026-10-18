export const STORE_KEY = "ffxi.player-interface-preview.v1";

export const packs = [
  {
    id: "environments",
    name: "HD environments",
    description: "A little more detail in familiar places.",
    type: "Visual packs",
    source: "Sample folder",
    folder: "HD environments",
    files: ["ROM/12/34.DAT"],
    fields: [
      {
        id: "detail",
        label: "Texture detail",
        type: "choice",
        choices: ["Balanced", "Detailed"],
        default: "Balanced",
        help: "A sample pack option. Actual texture loading comes with native integration.",
      },
    ],
  },
  {
    id: "menus",
    name: "Clearer menus",
    description: "Sharper icons and more readable text.",
    type: "Visual packs",
    source: "Sample folder",
    folder: "Clearer menus",
    files: ["ROM/119/51.DAT", "ROM/119/57.DAT"],
    fields: [
      {
        id: "fontSize",
        label: "Menu text size",
        type: "range",
        min: 100,
        max: 160,
        step: 5,
        unit: "%",
        default: 120,
        help: "Choose the size you would like this pack to use in the game.",
      },
      {
        id: "icons",
        label: "Icon appearance",
        type: "choice",
        choices: ["Classic shapes", "Soft contrast"],
        default: "Classic shapes",
        help: "Keep familiar shapes or try a gentler palette.",
      },
    ],
  },
  {
    id: "faces",
    name: "Original character faces",
    description: "Keep the faces you know.",
    type: "Visual packs",
    source: "Sample folder",
    folder: "Original character faces",
    files: ["ROM/27/80.DAT"],
    fields: [],
  },
  {
    id: "warm",
    name: "Warm interface",
    description: "A softer gold palette for menus and icons.",
    type: "Visual packs",
    source: "Sample folder",
    folder: "Warm interface",
    files: ["ROM/119/51.DAT"],
    fields: [
      {
        id: "contrast",
        label: "Higher contrast icons",
        type: "toggle",
        default: true,
        help: "Make important icon outlines easier to distinguish.",
      },
    ],
  },
  {
    id: "shortcuts",
    name: "Chat shortcuts",
    description: "Keep your familiar commands close at hand.",
    type: "Addons",
    source: "Windower-style sample",
    fields: [
      {
        id: "showHints",
        label: "Show command hints",
        type: "toggle",
        default: true,
        help: "Display a brief reminder next to supported shortcuts.",
      },
      {
        id: "hintSize",
        label: "Hint text size",
        type: "range",
        min: 12,
        max: 32,
        step: 1,
        unit: "px",
        default: 18,
        help: "Use a larger size if you play farther from your monitor.",
      },
    ],
  },
  {
    id: "party",
    name: "Party notes",
    description: "A small, tidy space for your party reminders.",
    type: "Addons",
    source: "Ashita-style sample",
    fields: [
      {
        id: "opacity",
        label: "Panel opacity",
        type: "range",
        min: 40,
        max: 100,
        step: 5,
        unit: "%",
        default: 80,
        help: "Higher values make the panel easier to read over the game.",
      },
    ],
  },
  {
    id: "plugin",
    name: "Native plugin example",
    description: "Native plugin hosting is still being built.",
    type: "Plugins",
    source: "Unavailable in this preview",
    unavailable: true,
    fields: [],
  },
];

const profile = (
  id,
  name,
  description,
  enabled,
  resolution = "3840 × 2160",
) => ({
  id,
  name,
  description,
  character: "Mithra",
  server: "Local server",
  enabled,
  order: packs.filter((p) => p.type === "Visual packs").map((p) => p.id),
  addonSettings: {},
  graphics: {
    resolution,
    uiScale: 1.5,
    renderer: "Recommended",
    filtering: "Balanced",
    windowMode: "Borderless window",
  },
});

export function defaults() {
  return {
    version: 1,
    selected: "classic-plus",
    preferences: { textScale: 100, opaque: false, checkUpdates: true },
    profiles: [
      profile(
        "classic-plus",
        "Classic Plus",
        "The familiar look, with a little more clarity.",
        ["environments", "menus", "shortcuts"],
      ),
      profile(
        "classic",
        "Clean & Classic",
        "A quiet starting point close to the original game.",
        ["faces"],
        "1920 × 1080",
      ),
      profile(
        "detail",
        "High fidelity",
        "A sample setup for exploring visual packs.",
        ["environments", "menus", "warm", "party"],
      ),
    ],
  };
}

export function cleanState(value) {
  const seed = defaults();
  if (
    !value ||
    value.version !== 1 ||
    !Array.isArray(value.profiles) ||
    !value.profiles.length ||
    value.profiles.length > 30
  )
    return seed;
  const ids = new Set();
  const allowed = new Set(packs.filter((p) => !p.unavailable).map((p) => p.id));
  for (const p of value.profiles) {
    if (
      !p ||
      typeof p.id !== "string" ||
      ids.has(p.id) ||
      typeof p.name !== "string" ||
      !p.name.trim() ||
      p.name.length > 60
    )
      return seed;
    ids.add(p.id);
    if (
      !Array.isArray(p.enabled) ||
      p.enabled.some((id) => !allowed.has(id)) ||
      new Set(p.enabled).size !== p.enabled.length
    )
      return seed;
    if (
      !Array.isArray(p.order) ||
      p.order.length !== 4 ||
      new Set(p.order).size !== 4 ||
      p.order.some((id) => !seed.profiles[0].order.includes(id))
    )
      return seed;
    if (
      typeof p.character !== "string" ||
      typeof p.server !== "string" ||
      typeof p.description !== "string" ||
      p.character.length > 60 ||
      p.server.length > 60 ||
      p.description.length > 180
    )
      return seed;
    if (
      !p.graphics ||
      !["3840 × 2160", "2560 × 1440", "1920 × 1080"].includes(
        p.graphics.resolution,
      ) ||
      !Number.isFinite(p.graphics.uiScale) ||
      p.graphics.uiScale < 1 ||
      p.graphics.uiScale > 2
    )
      return seed;
    if (
      ![
        "Recommended",
        "Original graphics",
        "Modern DirectX",
        "Vulkan",
      ].includes(p.graphics.renderer) ||
      !["Balanced", "Sharper textures", "Softer edges"].includes(
        p.graphics.filtering,
      ) ||
      !["Borderless window", "Windowed"].includes(p.graphics.windowMode)
    )
      return seed;
    if (
      !p.addonSettings ||
      typeof p.addonSettings !== "object" ||
      Array.isArray(p.addonSettings)
    )
      return seed;
    for (const pack of packs) {
      const saved = p.addonSettings[pack.id];
      if (saved === undefined) continue;
      if (!saved || typeof saved !== "object" || Array.isArray(saved))
        return seed;
      for (const field of pack.fields) {
        const v = saved[field.id];
        if (v === undefined) continue;
        if (field.type === "toggle" && typeof v !== "boolean") return seed;
        if (field.type === "choice" && !field.choices.includes(v)) return seed;
        if (
          field.type === "range" &&
          (!Number.isFinite(v) || v < field.min || v > field.max)
        )
          return seed;
      }
    }
  }
  if (!ids.has(value.selected)) value.selected = value.profiles[0].id;
  if (
    !value.preferences ||
    !Number.isFinite(value.preferences.textScale) ||
    value.preferences.textScale < 90 ||
    value.preferences.textScale > 125 ||
    typeof value.preferences.opaque !== "boolean" ||
    typeof value.preferences.checkUpdates !== "boolean"
  )
    value.preferences = seed.preferences;
  return value;
}

export function conflicts(profile) {
  const paths = new Map();
  for (const id of profile.order) {
    if (!profile.enabled.includes(id)) continue;
    const p = packs.find((pack) => pack.id === id);
    for (const path of p.files)
      paths.set(path, [...(paths.get(path) || []), p.name]);
  }
  return [...paths]
    .filter(([, names]) => names.length > 1)
    .map(([path, names]) => ({ path, winner: names[0], names }));
}
