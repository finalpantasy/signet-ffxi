import assert from "node:assert/strict";
import { chromium } from "playwright-core";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
const [url, output, mode = "all"] = process.argv.slice(2);
assert.equal(new URL(url).hostname, "127.0.0.1");
await mkdir(output, { recursive: false });
const browser = await chromium.launch({
  headless: true,
  args: ["--mute-audio"],
});
const page = await browser.newPage();
const errors = [],
  checks = [];
page.on("pageerror", (error) => errors.push(error.message));
const nav = (name) =>
  page
    .getByRole("navigation")
    .getByRole("button", { name, exact: true })
    .click();
const snapshot = () =>
  page.evaluate(async () => (await fetch("/api/v1/snapshot")).json());
const rpc = (request) =>
  page.evaluate(async (request) => {
    const response = await fetch("/api/v1/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Signet-Request": "1" },
      body: JSON.stringify({ schema_version: 1, action: "profile", request }),
    });
    const reply = await response.json();
    if (!response.ok || !reply.ok) throw Error(reply.error);
    return reply.value;
  }, request);
const profile = async (name) =>
  (await snapshot()).profiles.find((p) => p.document.name === name);
const select = async (name) =>
  page.locator(".launch-toolbar select").selectOption({ label: name });
const copy = async (name) => {
  const existing = await profile(name);
  if (!existing)
    await rpc({
      schema_version: 1,
      action: "copy_profile",
      id: (await profile("Classic")).id,
      name,
    });
};
try {
  await page.goto(url);
  await page.getByText("Native workspace", { exact: true }).waitFor();
  if (mode === "names" || mode === "all") {
    for (const name of ["constructor", "__proto__", "toString"])
      await copy(name);
    await page.reload();
    await page.getByText("Native workspace", { exact: true }).waitFor();
    await nav("Profiles");
    for (const name of ["constructor", "__proto__", "toString"]) {
      await select(name);
      assert.equal(
        await page.getByLabel("Width", { exact: true }).inputValue(),
        "1280",
        `valid profile ${name} inherited a non-draft object property`,
      );
      await page.getByLabel("Width", { exact: true }).fill("1300");
      await page
        .getByRole("button", { name: "Save profile", exact: true })
        .click();
      await page
        .getByText("Saved to native profile", { exact: true })
        .waitFor();
      assert.equal((await profile(name)).document.width, 1300);
    }
    checks.push(
      "all valid prototype-like profile names load and save independently",
    );
  }
  if (mode === "addons" || mode === "all") {
    await select("Classic");
    await nav("Play");
    await page.locator(".advanced > summary").click();
    await page
      .getByRole("button", { name: "Test addons", exact: true })
      .click();
    for (const name of ["constructor", "__proto__"]) {
      await page
        .getByRole("button", { name: `Unload ${name}`, exact: true })
        .click();
      await page
        .getByRole("button", { name: `Load ${name}`, exact: true })
        .waitFor({ timeout: 5000 });
      assert.equal(
        Object.hasOwn(
          (await snapshot()).session.reply.status.loaded_sources,
          name,
        ),
        false,
      );
      await page
        .getByRole("button", { name: `Load ${name}`, exact: true })
        .click();
      await page
        .getByRole("button", { name: `Unload ${name}`, exact: true })
        .waitFor();
    }
    await page.getByRole("button", { name: "Stop test", exact: true }).click();
    await page.getByText("Offline test stopped", { exact: true }).waitFor();
    checks.push("prototype-like addon names report real unload and load state");
  }
  if (mode === "selection" || mode === "all") {
    await copy("Evening");
    await page.reload();
    await page.getByText("Native workspace", { exact: true }).waitFor();
    await nav("Profiles");
    await select("Evening");
    await page.getByLabel("Width", { exact: true }).fill("900");
    const old = await profile("Evening");
    await rpc({
      schema_version: 1,
      action: "save_profile",
      id: old.id,
      name: "Evening",
      width: 1600,
      height: 720,
      ui_scale: 1.0,
      enabled: ["example"],
    });
    await page
      .getByRole("button", { name: "Save profile", exact: true })
      .click();
    await page
      .getByRole("alert")
      .filter({ hasText: "Profile changed" })
      .waitFor();
    // Both controls are disabled until error reconciliation completes.
    await page.waitForFunction(
      () =>
        ![...document.querySelectorAll("button")].find(
          (b) => b.textContent.trim() === "Save profile",
        )?.disabled,
    );
    assert.equal(
      await page.locator(".launch-toolbar select option:checked").innerText(),
      "Evening",
      "stale refresh selected an unrelated profile",
    );
    assert.equal(
      await page.getByLabel("Width", { exact: true }).inputValue(),
      "900",
    );
    await page
      .getByRole("button", { name: "Save profile", exact: true })
      .click();
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    assert.equal((await profile("Evening")).document.width, 900);
    assert.equal((await profile("Classic")).document.width, 1280);
    checks.push(
      "stale-revision refresh preserves selected profile and unfinished draft",
    );
  }
  if (mode === "packages" || mode === "all") {
    await nav("Addons & Mods");
    await page
      .getByText("Active generation: none", { exact: true })
      .waitFor({ state: "attached" });
    await nav("Play");
    await page.route("**/api/v1/rpc", (route) =>
      route.request().postDataJSON()?.action === "packages"
        ? route.abort("failed")
        : route.continue(),
    );
    await nav("Addons & Mods");
    await page.getByRole("alert").waitFor();
    await page
      .getByText("Package store is unavailable.", { exact: true })
      .waitFor({ state: "attached", timeout: 5000 });
    assert.equal(
      await page.getByText("Active generation: none", { exact: true }).count(),
      0,
      "failed package inspection retained a stale active-generation claim",
    );
    await page.unroute("**/api/v1/rpc");
    await nav("Play");
    await nav("Addons & Mods");
    await page
      .getByText("Active generation: none", { exact: true })
      .waitFor({ state: "attached" });
    checks.push(
      "failed package inspection discards cached state and recovers on retry",
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
