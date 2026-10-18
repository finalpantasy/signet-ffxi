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
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
const nav = (label) =>
  page
    .getByRole("navigation")
    .getByRole("button", { name: label, exact: true })
    .click();
const snapshot = () =>
  page.evaluate(async () => (await fetch("/api/v1/snapshot")).json());
try {
  await page.goto(url);
  await page.getByText("Native workspace", { exact: true }).waitFor();
  if (mode === "damaged") {
    await page
      .getByRole("alert")
      .filter({ hasText: "unreadable profile version preserved" })
      .waitFor();
    await nav("Profiles");
    await page.locator(".advanced > summary").click();
    await page
      .getByRole("button", { name: "Restore Classic-0001", exact: true })
      .click();
    await page.getByText("Profile restored", { exact: true }).waitFor();
    assert.equal((await snapshot()).profiles[0].requires_restore, false);
    assert.equal((await snapshot()).profiles[0].id, "Classic-0003");
    await writeFile(
      resolve(output, "report.json"),
      JSON.stringify(
        {
          checks_passed: true,
          checks: ["damaged saved version has explicit browser restore path"],
          muted: true,
          game_started: false,
        },
        null,
        2,
      ),
    );
  } else {
    if (mode !== "transport") {
      const before = (await snapshot()).profiles[0].document.width;
      await nav("Profiles");
      await page.getByLabel("Width", { exact: true }).fill("800");
      await nav("Addons & Mods");
      const toggle = page.getByRole("switch", {
        name: "Enable example",
        exact: true,
      });
      const originallyEnabled = await toggle.getAttribute("aria-checked");
      await toggle.click();
      await page
        .getByText("Saved to native profile", { exact: true })
        .waitFor();
      assert.equal(
        (await snapshot()).profiles[0].document.width,
        before,
        "addon toggle committed an unfinished display edit",
      );
      await nav("Profiles");
      assert.equal(
        await page.getByLabel("Width", { exact: true }).inputValue(),
        "800",
        "addon save discarded the display draft",
      );
      await page
        .getByRole("button", { name: "Save profile", exact: true })
        .click();
      await page
        .getByText("Saved to native profile", { exact: true })
        .waitFor();
      assert.equal((await snapshot()).profiles[0].document.width, 800);
      if (originallyEnabled === "true") {
        await nav("Addons & Mods");
        await toggle.click();
        await page
          .getByText("Saved to native profile", { exact: true })
          .waitFor();
      }
    }
    await nav("Play");
    await page.locator(".advanced > summary").click();
    await page
      .getByRole("button", { name: "Test addons", exact: true })
      .click();
    await page
      .getByText("Loaded in offline runtime", { exact: true })
      .waitFor();
    // A lost response after a real successful stop is an uncertain browser outcome.
    // The native server remains real and authoritative; reconciliation must read it.
    await page.route("**/api/v1/rpc", async (route) => {
      if (route.request().postDataJSON()?.action === "stop_runtime") {
        await route.fetch();
        await route.abort("failed");
      } else await route.continue();
    });
    await page.getByRole("button", { name: "Stop test", exact: true }).click();
    await page
      .getByRole("button", { name: "Test addons", exact: true })
      .waitFor({ state: "visible" });
    await page.waitForFunction(() => {
      const b = [...document.querySelectorAll("button")].find(
        (b) => b.textContent.trim() === "Test addons",
      );
      return b && !b.disabled;
    });
    assert.equal(
      await page
        .getByText("Loaded in offline runtime", { exact: true })
        .count(),
      0,
      "transport error retained cached loaded state",
    );
    assert.equal((await snapshot()).session, null);
    await page.unroute("**/api/v1/rpc");
    await page
      .getByRole("button", { name: "Test addons", exact: true })
      .click();
    await page
      .getByText("Loaded in offline runtime", { exact: true })
      .waitFor();
    await page.getByRole("button", { name: "Stop test", exact: true }).click();
    await page.getByText("Offline test stopped", { exact: true }).waitFor();
    await page.route("**/api/v1/rpc", async (route) => {
      if (route.request().postDataJSON()?.action === "start_offline") {
        await route.fetch();
        await route.abort("failed");
      } else await route.continue();
    });
    await page
      .getByRole("button", { name: "Test addons", exact: true })
      .click();
    await page
      .getByText("Loaded in offline runtime", { exact: true })
      .waitFor();
    assert.equal(
      (await snapshot()).session.reply.status.loaded_sources.example.length,
      64,
    );
    await page.unroute("**/api/v1/rpc");
    // When neither mutation nor snapshot can be reached, hide cached loaded state.
    await page.route("**/api/v1/**", (route) => route.abort("failed"));
    await page
      .getByRole("button", { name: "Refresh runtime", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        ![...document.querySelectorAll("p")].some(
          (p) => p.textContent === "Loaded in offline runtime",
        ),
    );
    assert.equal(
      await page
        .getByRole("button", { name: "Test addons", exact: true })
        .isDisabled(),
      true,
    );
    await page.unroute("**/api/v1/**");
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await page
      .getByText("Loaded in offline runtime", { exact: true })
      .waitFor();
    await page.getByRole("button", { name: "Stop test", exact: true }).click();
    await page.getByText("Offline test stopped", { exact: true }).waitFor();
    await page
      .getByRole("button", { name: "Test addons", exact: true })
      .click();
    await page
      .getByText("Loaded in offline runtime", { exact: true })
      .waitFor();
    const stopReply = await page.evaluate(async () =>
      (
        await fetch("/api/v1/rpc", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-Signet-Request": "1",
          },
          body: JSON.stringify({ schema_version: 1, action: "stop_runtime" }),
        })
      ).json(),
    );
    assert.equal(stopReply.value.exit_code, 0);
    await page
      .getByText("Loaded in offline runtime", { exact: true })
      .waitFor({ state: "hidden", timeout: 10000 });
    assert.equal(
      await page
        .getByRole("button", { name: "Test addons", exact: true })
        .isDisabled(),
      false,
    );
    assert.deepEqual(errors, []);
    await writeFile(
      resolve(output, "report.json"),
      JSON.stringify(
        {
          checks_passed: true,
          checks: [
            "addon-only save preserves saved display and pending draft",
            "explicit display save persists",
            "lost stop response reconciles native session",
            "restart after transport recovery",
            "lost start response reconciles live session",
            "disconnect hides cached state and reconnect restores authoritative session",
            "background polling reconciles an externally stopped owned session",
          ],
          errors,
          muted: true,
          game_started: false,
        },
        null,
        2,
      ),
    );
  }
} finally {
  await browser.close();
}
