import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { chromium } from 'playwright-core';
import axe from 'axe-core';

const args = process.argv.slice(2);
const argument = (name, fallback) => args.includes(name) ? args[args.indexOf(name) + 1] : fallback;
const url = argument('--url', 'http://127.0.0.1:58473/');
assert.equal(new URL(url).hostname, '127.0.0.1', 'Only the owned loopback preview is allowed');
const output = resolve(argument('--output', 'local/browser-acceptance'));
await mkdir(dirname(output), { recursive: true });
await mkdir(output, { recursive: false });
const errors = [];
const unexpectedRequests = [];
const checks = [];
const accessibility = [];
const browser = await chromium.launch({ headless: true, args: ['--mute-audio'] });
const context = await browser.newContext({ viewport: { width: 1100, height: 850 }, reducedMotion: 'reduce' });
const page = await context.newPage();
page.on('pageerror', error => errors.push(error.message));
page.on('request', request => {
  if (!request.url().startsWith(new URL(url).origin + '/') && !request.url().startsWith('data:')) unexpectedRequests.push(request.url());
  if (!['GET', 'HEAD'].includes(request.method())) unexpectedRequests.push(request.method());
});
const key = 'ffxi.player-interface-preview.v1';
const state = () => page.evaluate(key => JSON.parse(localStorage.getItem(key)), key);
const nav = title => page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button', { name: title, exact: true }).click();
const text = async (selector, expected) => assert.match(await page.locator(selector).innerText(), expected);
const audit = async name => {
  await page.evaluate(axe.source);
  const result = await page.evaluate(async () => await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21aa'] } }));
  const violations = result.violations.map(v => ({ id: v.id, impact: v.impact, description: v.description, nodes: v.nodes.map(n => ({ target: n.target, summary: n.failureSummary })) }));
  accessibility.push({ name, violations });
  return violations;
};

try {
  await page.goto(url);
  await page.getByRole('button', { name: 'Play FFXI', exact: true }).waitFor();
  await page.evaluate(async () => { await document.fonts.ready; await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
  assert.equal((await state()).selected, 'classic-plus');
  await page.screenshot({ path: resolve(output, 'home-desktop.png') });
  await audit('home-desktop'); checks.push('initial screen and original-size capture');

  await page.getByRole('switch', { name: 'Enable HD environments', exact: true }).click();
  assert(!(await state()).profiles[0].enabled.includes('environments'));
  await nav('Profiles');
  await page.getByRole('button', { name: /Clean & Classic/ }).click();
  assert.equal((await state()).selected, 'classic');
  await page.getByRole('button', { name: /Classic Plus/ }).click();
  await page.getByRole('button', { name: 'Copy profile', exact: true }).click();
  await page.getByLabel('Profile name', { exact: true }).fill('Evening setup');
  await page.getByRole('button', { name: 'Save profile', exact: true }).click();
  await page.getByText('Saved', { exact: true }).waitFor();
  let saved = await state(); let current = saved.profiles.find(p => p.id === saved.selected);
  assert.equal(current.name, 'Evening setup');
  assert(!current.enabled.includes('environments'));
  assert.deepEqual(saved.profiles.find(p => p.id === 'classic').enabled, ['faces']);
  await page.getByRole('button', { name: 'New profile', exact: true }).click();
  await page.getByRole('dialog').getByLabel('Profile name', { exact: true }).fill('Temporary setup');
  await page.getByRole('button', { name: 'Create profile', exact: true }).click();
  assert.equal((await state()).profiles.length, 5);
  await page.getByRole('button', { name: 'Remove profile', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Remove profile', exact: true }).click();
  assert.equal((await state()).profiles.length, 4);
  await page.getByRole('button', { name: /Evening setup/ }).click();
  await page.screenshot({ path: resolve(output, 'profiles-desktop.png'), fullPage: true });
  await audit('profiles'); checks.push('profile create, copy, rename, remove and neighbor isolation');

  await nav('Addons & Mods');
  await page.getByRole('switch', { name: 'Enable Warm interface', exact: true }).click();
  await page.locator('.conflict-section > summary').click();
  await text('.conflict-section', /Clearer menus takes priority/);
  await page.getByRole('button', { name: 'Move Warm interface up', exact: true }).click();
  await page.getByRole('button', { name: 'Move Warm interface up', exact: true }).click();
  await text('.conflict-section', /Warm interface takes priority/);
  await page.getByRole('searchbox', { name: 'Search addons and mods' }).fill('Clearer');
  assert.equal(await page.locator('.library-row').count(), 1);
  await page.getByRole('button', { name: 'Details', exact: true }).click();
  await page.getByLabel('Menu text size', { exact: true }).fill('140');
  await page.getByLabel('Menu text size', { exact: true }).press('Tab');
  await page.getByLabel('Icon appearance', { exact: true }).selectOption('Soft contrast');
  saved = await state(); current = saved.profiles.find(p => p.id === saved.selected);
  assert.equal(current.addonSettings.menus.fontSize, 140);
  assert.equal(current.addonSettings.menus.icons, 'Soft contrast');
  await page.getByLabel('Menu text size', { exact: true }).fill('999');
  await page.getByLabel('Menu text size', { exact: true }).press('Tab');
  await page.getByText('Choose a number from 100 to 160.', { exact: true }).waitFor();
  assert.equal((await state()).profiles.find(p => p.id === saved.selected).addonSettings.menus.fontSize, 140);
  await page.getByLabel('Menu text size', { exact: true }).fill('140');
  await page.getByLabel('Menu text size', { exact: true }).press('Tab');
  await audit('generated-addon-form');
  await page.screenshot({ path: resolve(output, 'generated-settings.png'), fullPage: true });
  await page.getByRole('button', { name: 'Done', exact: true }).click();
  await page.getByRole('searchbox').fill('no-such-pack');
  await page.getByRole('heading', { name: 'No matches' }).waitFor();
  await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
  assert.equal(await page.locator('.library-row').count(), 7);
  await page.getByRole('button', { name: 'Plugins', exact: true }).click();
  assert.equal(await page.getByRole('switch', { name: 'Enable Native plugin example', exact: true }).isDisabled(), true);
  await page.getByRole('button', { name: 'All', exact: true }).click();
  await page.getByLabel('Only enabled', { exact: true }).check();
  assert.equal(await page.locator('.library-row').count(), current.enabled.length);
  await page.getByLabel('Only enabled', { exact: true }).uncheck();
  await audit('library');
  await page.screenshot({ path: resolve(output, 'library-desktop.png'), fullPage: true });
  checks.push('sample toggles, priority conflicts, filters and validated generated settings');

  await nav('Settings');
  await page.getByLabel('Game resolution', { exact: true }).selectOption('2560 × 1440');
  await page.getByLabel('In-game interface size', { exact: true }).fill('1.7');
  await page.getByLabel('In-game interface size', { exact: true }).press('Tab');
  saved = await state(); current = saved.profiles.find(p => p.id === saved.selected);
  assert.equal(current.graphics.resolution, '2560 × 1440'); assert.equal(current.graphics.uiScale, 1.7);
  await page.screenshot({ path: resolve(output, 'settings-desktop.png'), fullPage: true });
  await audit('graphics'); checks.push('profile-specific graphics preferences, with no active-backend claim');

  await nav('Settings');
  await page.getByRole('switch', { name: 'Reduce transparency', exact: true }).click();
  assert.equal(await page.locator('html').getAttribute('data-opaque'), 'true');
  await page.getByLabel('Launcher text size', { exact: true }).fill('110');
  await page.getByLabel('Launcher text size', { exact: true }).press('Tab');
  assert.equal(await page.locator('html').evaluate(el => getComputedStyle(el).fontSize), '17.6px');
  await page.getByText('Advanced app settings', { exact: true }).click();
  await page.getByRole('switch', { name: 'Check for updates', exact: true }).click();
  await audit('settings');
  await page.reload(); await page.getByRole('button', { name: 'Play FFXI', exact: true }).waitFor();
  saved = await state(); current = saved.profiles.find(p => p.id === saved.selected);
  assert.equal(current.name, 'Evening setup'); assert.equal(current.addonSettings.menus.fontSize, 140);
  assert.equal(current.graphics.uiScale, 1.7); assert.equal(saved.preferences.textScale, 110); assert.equal(saved.preferences.checkUpdates, false);
  await page.getByRole('button', { name: 'Play FFXI', exact: true }).click();
  await page.getByRole('heading', { name: 'Launch preview' }).waitFor();
  await text('dialog', /Evening setup/); await text('dialog', /2560 × 1440/);
  await page.keyboard.press('Escape');
  assert.equal(await page.getByRole('dialog').count(), 0);
  assert.equal(await page.getByRole('button', { name: 'Play FFXI', exact: true }).evaluate(el => document.activeElement === el), true);
  checks.push('persistence, actual launcher appearance preferences, preview summary and escape focus return');

  await page.evaluate(() => localStorage.setItem('unrelated-preview-sentinel', 'preserved'));
  await nav('Settings');
  await page.getByText('Advanced app settings', { exact: true }).click();
  await page.getByRole('button', { name: 'Reset preview', exact: true }).click();
  await page.getByRole('button', { name: 'Restore sample setup', exact: true }).click();
  assert.equal((await state()).profiles.length, 3); assert.equal((await state()).selected, 'classic-plus');
  assert.equal(await page.evaluate(() => localStorage.getItem('unrelated-preview-sentinel')), 'preserved');
  await page.evaluate(key => localStorage.setItem(key, '{broken'), key); await page.reload();
  await page.getByRole('button', { name: 'Play FFXI', exact: true }).waitFor();
  assert.equal((await state()).selected, 'classic-plus');
  checks.push('scoped reset, unrelated key preservation and corrupt-state recovery');

  await page.getByRole('button', { name: 'About this preview', exact: true }).click();
  for (let i = 0; i < 8; i++) {
    await page.keyboard.press('Tab');
    assert.equal(await page.evaluate(() => Boolean(document.activeElement.closest('dialog'))), true);
  }
  await page.keyboard.press('Escape');
  assert.equal(await page.getByRole('button', { name: 'About this preview', exact: true }).evaluate(el => document.activeElement === el), true);
  assert.equal(await page.locator('audio, video, iframe').count(), 0);
  checks.push('native dialog keyboard containment, focus return and absence of media');

  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.screenshot({ path: resolve(output, 'home-mobile.png'), fullPage: true });
  await audit('home-mobile');
  await nav('Addons & Mods');
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await audit('library-mobile');
  await page.screenshot({ path: resolve(output, 'library-mobile.png'), fullPage: true });
  await nav('Settings');
  await page.getByLabel('Launcher text size', { exact: true }).fill('125');
  await page.getByLabel('Launcher text size', { exact: true }).press('Tab');
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await audit('settings-mobile-large-text');
  checks.push('390px layout, no horizontal overflow and mobile library');

  assert.deepEqual(errors, []); assert.deepEqual(unexpectedRequests, []);
  const violations = accessibility.flatMap(a => a.violations.map(v => ({ screen: a.name, ...v })));
  await writeFile(resolve(output, 'report.json'), JSON.stringify({ checks_passed: violations.length === 0, checks, accessibility, errors, unexpectedRequests, muted: true, game_started: false, engine: 'isolated muted Playwright Chromium', screenshots: ['home-desktop.png', 'profiles-desktop.png', 'generated-settings.png', 'library-desktop.png', 'home-mobile.png', 'library-mobile.png', 'settings-desktop.png'] }, null, 2));
  assert.equal(violations.length, 0, JSON.stringify(violations, null, 2));
  console.log(`PASS: ${checks.length} interaction groups, ${accessibility.length} accessibility scans, no external requests or page errors.`);
} catch (error) {
  await writeFile(resolve(output, 'failure.json'), JSON.stringify({ checks_passed: false, error: String(error), checks, accessibility, errors, unexpectedRequests }, null, 2));
  throw error;
} finally {
  await context.close(); await browser.close();
}
