// Keyboard check: tabs through Home and records which element has focus at each step,
// with a screenshot of the focus ring partway. Needs `npm run dev`.
// Usage: node scripts/focus-shot.mjs [outDir] [theme]
import { chromium } from "playwright-core";

const out = process.argv[2] || "launcher-shots";
const theme = process.argv[3] || "dark";
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, colorScheme: theme, reducedMotion: "reduce" });
await page.goto(`http://127.0.0.1:5199/?screen=home&snap=snapshot&theme=${theme}`, { waitUntil: "networkidle" });
const order = [];
for (let i = 0; i < 22; i++) {
  await page.keyboard.press("Tab");
  order.push(await page.evaluate(() => {
    const a = document.activeElement;
    return (a.getAttribute("aria-label") || a.textContent || a.tagName).trim().replace(/\s+/g, " ").slice(0, 40);
  }));
  if (i === 6) await page.screenshot({ path: `${out}/browser-home-focus-${theme}-1280.png` });
}
console.log(order.map((t, i) => `${i + 1}. ${t}`).join("\n"));
await browser.close();
