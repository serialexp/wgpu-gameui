#!/usr/bin/env node
/**
 * Captures every manifest case from reference.html with Playwright Chromium.
 * No project dependency is declared: run with an existing Playwright install,
 * e.g. `npx --yes playwright install chromium` followed by `node capture.mjs`.
 */
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path, { dirname } from 'node:path';

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const manifest = JSON.parse(await readFile(path.join(here, 'manifest.json'), 'utf8'));
const outputRoot = path.join(here, 'captures');

let chromium;
try {
  // resolve() searches NODE_PATH as well as the caller's ordinary node_modules.
  ({ chromium } = require(require.resolve('playwright')));
} catch (error) {
  console.error('Playwright is required but was not resolvable from this script.');
  console.error('Install it outside this repository, then run this script from that environment.');
  console.error(error.message);
  process.exitCode = 1;
  process.exit();
}

await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });
const browser = await chromium.launch({ headless: true });
const metadata = {
  schema_version: 1,
  captured_at_utc: new Date().toISOString(),
  browser: await browser.version(),
  engine: 'Playwright Chromium',
  source: 'reference.html',
  viewport_css_px: manifest.capture.viewport_css_px,
  device_scale_factors: manifest.capture.device_scale_factors,
  backdrops: manifest.capture.backdrops,
  command: process.argv.join(' ')
};

try {
  for (const dpr of manifest.capture.device_scale_factors) {
    const context = await browser.newContext({
      viewport: manifest.capture.viewport_css_px,
      deviceScaleFactor: dpr,
      colorScheme: 'light'
    });
    const page = await context.newPage();
    await page.goto(pathToFileURL(path.join(here, manifest.source)).href, { waitUntil: 'load' });
    await page.evaluate(() => document.fonts.ready);

    for (const backdrop of manifest.capture.backdrops) {
      await page.evaluate((name) => {
        document.body.className = name === 'alpha' ? 'capture-alpha' : `capture-${name}`;
      }, backdrop);
      for (const fixture of manifest.cases) {
        const locator = page.locator(`[data-case="${fixture.id}"]`);
        const destination = path.join(outputRoot, fixture.id, `${backdrop}@${dpr}x.png`);
        await mkdir(dirname(destination), { recursive: true });
        await locator.screenshot({ path: destination, omitBackground: backdrop === 'alpha' });
      }
    }
    await context.close();
  }
  await writeFile(path.join(outputRoot, 'capture-metadata.json'), `${JSON.stringify(metadata, null, 2)}\n`);
} finally {
  await browser.close();
}
