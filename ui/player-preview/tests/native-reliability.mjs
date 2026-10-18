import assert from "node:assert/strict";
import { chromium } from "playwright-core";
import { mkdir, writeFile } from "node:fs/promises";
const [url, output] = process.argv.slice(2);
await mkdir(output, { recursive: false });
const browser = await chromium.launch({
  headless: true,
  args: ["--mute-audio"],
});
const page = await browser.newPage();
const checks = [];
const nav = (title) =>
  page
    .getByRole("navigation")
    .getByRole("button", { name: title, exact: true })
    .click();
try {
  await page.goto(url);
  await page.getByText("Native workspace", { exact: true }).waitFor();
  await nav("Profiles");
  await page.getByLabel("Width", { exact: true }).fill("900");
  await nav("Addons & Mods");
  await page
    .getByRole("switch", { name: "Enable example", exact: true })
    .click();
  await page.getByText("Saved to native profile", { exact: true }).waitFor();
  let snapshot = await (await page.request.get(url + "api/v1/snapshot")).json();
  assert.equal(
    snapshot.profiles[0].document.width,
    1280,
    "addon toggle must not commit a display draft",
  );
  await nav("Profiles");
  assert.equal(
    await page.getByLabel("Width", { exact: true }).inputValue(),
    "900",
    "display draft remains editable",
  );
  checks.push("addon toggle preserves persisted display and unsaved draft");
  await nav("Addons & Mods");
  await page
    .getByRole("switch", { name: "Enable example", exact: true })
    .click();
  await page.getByText("Saved to native profile", { exact: true }).waitFor();
  await nav("Play");
  await page.locator(".advanced > summary").click();
  await page.getByRole("button", { name: "Test addons", exact: true }).click();
  await page.getByText("Loaded in offline runtime", { exact: true }).waitFor();
  await page.route("**/api/v1/rpc", (route) => route.abort("failed"));
  await page.route("**/api/v1/snapshot", (route) => route.abort("failed"));
  await page
    .getByRole("button", { name: "Refresh runtime", exact: true })
    .click();
  await page.getByText("Runtime status unavailable", { exact: true }).waitFor();
  assert.equal(
    await page.getByText("Loaded in offline runtime", { exact: true }).count(),
    0,
  );
  assert.equal(
    await page
      .getByText("Running · offline addon test", { exact: true })
      .count(),
    0,
  );
  checks.push("transport failure invalidates active and loaded UI claims");
  await page.unroute("**/api/v1/rpc");
  await page.unroute("**/api/v1/snapshot");
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByText("Loaded in offline runtime", { exact: true }).waitFor();
  checks.push("explicit refresh reconciles actual owned session");
  await page.getByRole("button", { name: "Stop test", exact: true }).click();
  await page.getByText("Offline test stopped", { exact: true }).waitFor();
  const stale = await browser.newPage();
  await stale.goto(url);
  await stale.getByText("Native workspace", { exact: true }).waitFor();
  await stale.locator(".advanced > summary").click();
  await nav("Profiles");
  await page.getByRole("button", { name: "Save profile", exact: true }).click();
  await page.getByText("Saved to native profile", { exact: true }).waitFor();
  await stale.getByRole("button", { name: "Test addons", exact: true }).click();
  await stale
    .getByRole("alert")
    .filter({ hasText: "Profile changed" })
    .waitFor();
  snapshot = await (await page.request.get(url + "api/v1/snapshot")).json();
  assert.equal(snapshot.session, null);
  checks.push("stale browser cannot start historical profile");
  await writeFile(
    output + "/report.json",
    JSON.stringify({ checks_passed: true, checks }, null, 2),
  );
} finally {
  await browser.close();
}
