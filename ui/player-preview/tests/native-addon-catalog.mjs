import assert from "node:assert/strict";
import { chromium } from "playwright-core";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
const [url, output] = process.argv.slice(2);
assert.equal(new URL(url).hostname, "127.0.0.1");
await mkdir(output, { recursive: false });
const browser = await chromium.launch({
  headless: true,
  args: ["--mute-audio"],
});
const page = await browser.newPage();
const errors = [],
  checks = [];
page.on("pageerror", (e) => errors.push(e.message));
const snapshot = () =>
  page.evaluate(async () => (await fetch("/api/v1/snapshot")).json());
const profile = async (name) =>
  (await snapshot()).profiles.find((p) => p.document.name === name);
try {
  await page.goto(url);
  await page.getByText("Native workspace", { exact: true }).waitFor();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Addons & Mods", exact: true })
    .click();
  for (const [name, spelling, ecosystem, initiallyEnabled] of [
    ["Classic", "example", "native", false],
    ["Evening", "Example", "windower4", true],
  ]) {
    await page.locator(".launch-toolbar select").selectOption({ label: name });
    const before = await profile(name);
    assert.equal(
      before.available_addons.length,
      1,
      `${name} catalog exposes the same addon twice with conflicting capitalization`,
    );
    assert.equal(before.available_addons[0].name, spelling);
    assert.equal(before.available_addons[0].ecosystem, ecosystem);
    const toggle = page.getByRole("switch", {
      name: `Enable ${spelling}`,
      exact: true,
    });
    assert.equal(await page.getByRole("switch").count(), 1);
    if (initiallyEnabled) {
      await toggle.click();
      await page
        .getByText("Saved to native profile", { exact: true })
        .waitFor();
      assert.equal((await profile(name)).document.profile.addons.length, 0);
    }
    await toggle.click();
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    const saved = await profile(name);
    assert.deepEqual(saved.document.profile.addons, [
      { name: spelling, ecosystem, engine: "lua51" },
    ]);
    assert.equal(saved.document.width, before.document.width);
    assert.equal(saved.document.height, before.document.height);
    checks.push(
      `${name}: one case-insensitive toggle preserves own spelling/ecosystem through disable and enable`,
    );
  }
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(output, "report.json"),
    JSON.stringify(
      { checks_passed: true, checks, errors, muted: true, game_started: false },
      null,
      2,
    ),
  );
} finally {
  await browser.close();
}
