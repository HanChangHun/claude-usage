// Uses an existing Playwright installation and Chromium; installs nothing.
// PLAYWRIGHT_MODULE points to playwright/index.mjs; CHROMIUM_PATH to the browser.
import { readFile, mkdir } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE
  ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const root = fileURLToPath(new URL('../app/src/', import.meta.url));
const output = process.env.HISTORY_UI_OUTPUT || path.join(os.tmpdir(), 'claude-usage-history-ui');
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const context = await browser.newContext({ viewport: { width: 920, height: 640 } });
  await context.route('**/*', async route => {
    const url = new URL(route.request().url());
    if (url.hostname !== 'history.test') return route.abort();
    const name = path.basename(url.pathname);
    const contentType = name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html';
    await route.fulfill({ contentType, body: await readFile(path.join(root, name)) });
  });
  await context.addInitScript(() => {
    localStorage.setItem('codex-usage-enabled', 'true');
    window.testHistoryMode = 'normal';
    window.invoked = [];
    window.__TAURI__ = {
      event: { listen: async () => () => {} }, app: { getVersion: async () => 'test' },
      window: { getCurrentWindow: () => ({ setAlwaysOnTop: async () => {}, minimize: async () => {}, close: async () => {} }) },
      updater: { check: async () => null },
      core: { invoke: async (command, args) => {
        window.invoked.push(command);
        if (command === 'read_usage_history') {
          if (window.testHistoryMode === 'error') throw new Error('Test read failure');
          const now = Date.now(), start = now - args.hours * 3600000;
          const count = window.testHistoryMode === 'empty' ? 0 : window.testHistoryMode === 'single' ? 1
            : window.testHistoryMode === 'long' ? 25920 : 100;
          return { start, end: now, warning: null, samples: Array.from({ length: count }, (_, i) => ({
            ts: now - (count - 1 - i) * 300000, provider: args.provider, segment: i < 55 ? 'a' : 'b',
            windows: [{ id: 'weekly', label: `${args.provider} weekly`, used_percent: Math.min(94, 20 + Math.floor(i / 6) * 4),
              resets_at: new Date(now + 86400000).toISOString() }],
          })) };
        }
        if (command === 'read_codex_usage') return { rateLimits: { primary: { usedPercent: 42, windowDurationMins: 10080, resetsAt: Date.now() / 1000 + 86400 } } };
        return false;
      } },
    };
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('http://history.test/history.html');
  await page.waitForFunction(() => document.getElementById('historyCount').textContent === '100');
  assert.equal(await page.locator('button[data-provider="codex"]').getAttribute('aria-pressed'), 'true');
  assert.equal(await page.locator('.chart-line').count(), 1);
  const segments = await page.locator('.chart-line').getAttribute('d');
  assert.equal((segments.match(/M/g) || []).length, 2);
  await page.locator('#historyScrubber').focus();
  await page.keyboard.press('Home');
  assert.match(await page.locator('#historyDetail').textContent(), /20% used/);
  await page.screenshot({ path: path.join(output, 'history-usage.png'), fullPage: true });
  await page.getByRole('button', { name: 'Change', exact: true }).click();
  assert.equal(await page.locator('.chart-bar').count(), 98);
  await page.screenshot({ path: path.join(output, 'history-change.png'), fullPage: true });
  await page.setViewportSize({ width: 540, height: 420 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await page.screenshot({ path: path.join(output, 'history-small.png'), fullPage: true });
  await page.locator('.history-notes').scrollIntoViewIfNeeded();
  assert.equal(await page.evaluate(() => document.querySelector('.history-notes').getBoundingClientRect().bottom <= innerHeight + 1), true);
  await page.emulateMedia({ forcedColors: 'active' });
  await page.screenshot({ path: path.join(output, 'history-high-contrast.png'), fullPage: true });
  await page.emulateMedia({ forcedColors: 'none' });
  await page.evaluate(() => { window.testHistoryMode = 'empty'; });
  await page.getByRole('button', { name: 'Claude', exact: true }).click();
  await page.waitForFunction(() => document.getElementById('historyEmpty').textContent.includes('No Claude readings'));
  assert.equal(await page.locator('#historyScrubber').isDisabled(), true);
  await page.evaluate(() => { window.testHistoryMode = 'single'; });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.waitForFunction(() => document.getElementById('historyCount').textContent === '1');
  assert.match(await page.locator('#historyEmpty').textContent(), /two readings/);
  await page.evaluate(() => { window.testHistoryMode = 'long'; });
  await page.locator('#historyRange').selectOption('2160');
  await page.waitForFunction(() => document.getElementById('historyCount').textContent.replace(/\D/g, '') === '25920');
  await page.getByRole('button', { name: 'Usage %', exact: true }).click();
  assert.equal(await page.locator('.chart-line').count(), 1);
  await page.evaluate(() => { window.testHistoryMode = 'error'; });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.waitForFunction(() => document.getElementById('historyWarning').textContent.includes('Test read failure'));

  await page.setViewportSize({ width: 300, height: 180 });
  await page.goto('http://history.test/index.html');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Usage history', exact: true }).click();
  assert.equal(await page.evaluate(() => window.invoked.includes('open_usage_history')), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await page.screenshot({ path: path.join(output, 'widget-small-settings.png'), fullPage: true });
  assert.deepEqual(errors, []);
  console.log(`PASS: chart, changes, gaps, keyboard, empty/single/error states, 90-day range, 540x420, high contrast, 300x180 Settings. Screenshots: ${output}`);
} finally {
  await browser.close();
}
