// Browser screenshots of every screen and the main states, light and dark, at 1280x800 and
// 1920x1080. Needs `npm run dev` running (http://127.0.0.1:5199) and a dev-data snapshot.
// Usage: node scripts/shots.mjs [outDir] [snap]
// SCREENS=home,retail limits the list; SIZES=1280x800 limits the sizes.
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";

const out = process.argv[2] || "launcher-shots";
const snap = process.argv[3] || "snapshot";
mkdirSync(out, { recursive: true });

// [file name, query string]
const ALL = [
  ["home", "screen=home"],
  ["home-pol-running", "screen=home&pol=52140"],
  ["home-viewer-running", "screen=home&viewer=102"],
  ["home-current", "screen=home&client=current"],
  ["home-viewer-exited", "screen=home&exit=1"],
  ["retail", "screen=retail"],
  ["retail-pol-running", "screen=retail&pol=52140"],
  ["retail-missing", "screen=retail&client=missing"],
  ["retail-loading", "screen=retail&slow=1"],
  ["zones", "screen=zones"],
  ["zones-empty", "screen=zones&empty=favorites"],
  ["settings", "screen=settings"],
  ["mods", "screen=mods"],
];
const only = process.env.SCREENS?.split(",");
const shots = only ? ALL.filter(([n]) => only.includes(n)) : ALL;
const sizes = (process.env.SIZES || "1280x800,1920x1080").split(",").map((s) => s.split("x").map(Number));

const browser = await chromium.launch({ headless: true, args: ["--mute-audio"] });
const errors = [];
for (const [w, h] of sizes) {
  for (const theme of ["light", "dark"]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, colorScheme: theme, reducedMotion: "reduce" });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => errors.push(`${theme}: ${e.message}`));
    page.on("console", (m) => m.type() === "error" && errors.push(`${theme} console: ${m.text()}`));
    for (const [name, query] of shots) {
      await page.goto(`http://127.0.0.1:5199/?${query}&snap=${snap}&theme=${theme}`, { waitUntil: name.endsWith("loading") ? "load" : "networkidle" });
      if (query.includes("empty=favorites")) await page.getByRole("button", { name: "Favorites", exact: true }).click();
      await page.waitForTimeout(500);
      const file = `${out}/browser-${name}-${theme}-${w}.png`;
      await page.screenshot({ path: file });
      console.log("wrote", file);
    }
    await ctx.close();
  }
}
await browser.close();
if (errors.length) {
  console.log("page errors:\n" + errors.join("\n"));
  process.exitCode = 1;
}
