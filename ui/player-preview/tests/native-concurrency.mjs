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
page.on("pageerror", (e) => errors.push(e.message));
const nav = (name) =>
  page
    .getByRole("navigation")
    .getByRole("button", { name, exact: true })
    .click();
const save = () =>
  page.getByRole("button", { name: "Save profile", exact: true });
const width = () => page.getByLabel("Width", { exact: true });
const selected = () =>
  page.locator(".launch-toolbar select option:checked").innerText();
const snapshot = () =>
  page.evaluate(async () => (await fetch("/api/v1/snapshot")).json());
const profile = async (name) =>
  (await snapshot()).profiles.find((p) => p.document.name === name);
const fresh = async () => {
  await page.goto(url);
  await page.getByText("Native workspace", { exact: true }).waitFor();
  await page
    .locator(".launch-toolbar select")
    .selectOption({ label: "Classic" });
  await nav("Profiles");
};
// Delay only the response: the real native save has already committed and the
// server remains free to answer unrelated reads. Never substitute a fake reply.
const holdProfile = async (action = "save_profile") => {
  let held, release;
  const ready = new Promise((r) => (held = r)),
    gate = new Promise((r) => (release = r));
  const handler = async (route) => {
    if (route.request().postDataJSON()?.request?.action !== action)
      return route.continue();
    const response = await route.fetch();
    assert.equal(response.status(), 200);
    held();
    await gate;
    await route.fulfill({ response });
  };
  await page.route("**/api/v1/rpc", handler);
  return {
    ready,
    release,
    remove: () => page.unroute("**/api/v1/rpc", handler),
  };
};
const ensureEvening = async () => {
  await page.evaluate(async () => {
    const snapshot = await (await fetch("/api/v1/snapshot")).json();
    if (snapshot.profiles.some((p) => p.document.name === "Evening")) return;
    const response = await fetch("/api/v1/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Signet-Request": "1" },
      body: JSON.stringify({
        schema_version: 1,
        action: "profile",
        request: {
          schema_version: 1,
          action: "copy_profile",
          id: snapshot.profiles.find((p) => p.document.name === "Classic").id,
          name: "Evening",
        },
      }),
    });
    const reply = await response.json();
    if (!reply.ok) throw Error(reply.error);
  });
};
try {
  if (mode === "busy" || mode === "all") {
    await fresh();
    await width().fill("900");
    const held = await holdProfile();
    try {
      await save().click();
      await held.ready;
      assert.equal((await profile("Classic")).document.width, 900);
      await nav("Addons & Mods");
      await page
        .getByText("Active generation: none", { exact: true })
        .waitFor({ state: "attached" });
      await nav("Profiles");
      assert.equal(
        await save().isDisabled(),
        true,
        "completed package read unlocked a still-pending profile mutation",
      );
    } finally {
      held.release();
    }
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    await held.remove();
    checks.push("overlapping package read cannot unlock pending mutation");
  }
  if (mode === "draft" || mode === "all") {
    await fresh();
    await width().fill("1000");
    const held = await holdProfile();
    try {
      await save().click();
      await held.ready;
      await width().fill("1100");
    } finally {
      held.release();
    }
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    await held.remove();
    assert.equal((await profile("Classic")).document.width, 1000);
    assert.equal(
      await width().inputValue(),
      "1100",
      "save response erased edits typed after that save began",
    );
    await save().click();
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    assert.equal((await profile("Classic")).document.width, 1100);
    checks.push(
      "edits typed during save remain pending and can be explicitly saved",
    );
  }
  if (mode === "switch" || mode === "all") {
    await fresh();
    await ensureEvening();
    await fresh();
    const neighbor = (await profile("Evening")).document;
    await width().fill("1200");
    const held = await holdProfile();
    try {
      await save().click();
      await held.ready;
      await page
        .locator(".launch-toolbar select")
        .selectOption({ label: "Evening" });
    } finally {
      held.release();
    }
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    await held.remove();
    assert.equal(
      await selected(),
      "Evening",
      "late save response changed the user's newer profile selection",
    );
    assert.equal((await profile("Classic")).document.width, 1200);
    assert.deepEqual((await profile("Evening")).document, neighbor);
    checks.push(
      "late save response preserves newer profile selection and neighbor",
    );
  }
  if (mode === "copy" || mode === "all") {
    await fresh();
    await ensureEvening();
    await fresh();
    await page.getByLabel("Copy name", { exact: true }).fill("Aurora");
    const held = await holdProfile("copy_profile");
    try {
      await page
        .getByRole("button", { name: "Copy profile", exact: true })
        .click();
      await held.ready;
      await page.getByLabel("Copy name", { exact: true }).fill("LaterName");
      await page
        .locator(".launch-toolbar select")
        .selectOption({ label: "Evening" });
    } finally {
      held.release();
    }
    await page.getByText("Profile copied", { exact: true }).waitFor();
    await held.remove();
    assert.equal(
      await selected(),
      "Evening",
      "late copy response changed newer selection",
    );
    assert.equal(
      await page.getByLabel("Copy name", { exact: true }).inputValue(),
      "LaterName",
      "copy response erased newly typed copy name",
    );
    assert.ok(await profile("Aurora"));
    assert.equal(await profile("LaterName"), undefined);
    checks.push("copy completion preserves newer selection and next copy name");
  }
  if (mode === "restore" || mode === "all") {
    await fresh();
    await ensureEvening();
    await fresh();
    await width().fill("1400");
    await save().click();
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    await width().fill("1500");
    await page.locator(".advanced > summary").click();
    const held = await holdProfile("restore_profile");
    try {
      await page
        .getByRole("button", { name: "Restore Classic-0001", exact: true })
        .click();
      await held.ready;
      await width().fill("1600");
      await page
        .locator(".launch-toolbar select")
        .selectOption({ label: "Evening" });
    } finally {
      held.release();
    }
    await page.getByText("Profile restored", { exact: true }).waitFor();
    await held.remove();
    assert.equal(
      await selected(),
      "Evening",
      "late restore response changed newer selection",
    );
    assert.equal((await profile("Classic")).document.width, 1280);
    await page
      .locator(".launch-toolbar select")
      .selectOption({ label: "Classic" });
    assert.equal(
      await width().inputValue(),
      "1600",
      "restore completion erased edits typed after restore began",
    );
    await save().click();
    await page.getByText("Saved to native profile", { exact: true }).waitFor();
    assert.equal((await profile("Classic")).document.width, 1600);
    checks.push(
      "restore completion preserves newer selection and subsequent edits",
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
