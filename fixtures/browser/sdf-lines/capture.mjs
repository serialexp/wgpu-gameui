#!/usr/bin/env node
/**
 * Captures every case in cases.json as an SVG stroke with Playwright Chromium:
 * black (at the case's opacity) on a transparent page, so each PNG's alpha is
 * the stroke's coverage. Writes reference.html (the page it captures) and
 * captures/<case>/alpha@<dpr>x.png, replacing captures/.
 * No project dependency is declared: run with an existing Playwright install
 * (see README.md).
 */
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path, { dirname } from 'node:path';

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const cases = JSON.parse(await readFile(path.join(here, 'cases.json'), 'utf8'));
const outputRoot = path.join(here, 'captures');
const CROP = 120;
const COLUMNS = 6;
const DPRS = [1, 1.5, 2];

let chromium;
try {
  ({ chromium } = require(require.resolve('playwright')));
} catch (error) {
  console.error('Playwright is required but was not resolvable from this script.');
  console.error(error.message);
  process.exit(1);
}

function shape(fixture) {
  const points = fixture.points.map(([x, y]) => `${x},${y}`).join(' ');
  const attrs = [
    `points="${points}"`,
    'fill="none"',
    `stroke="rgb(0 0 0 / ${fixture.opacity ?? 1})"`,
    `stroke-width="${fixture.width}"`,
    `stroke-linecap="${fixture.cap ?? 'butt'}"`,
    `stroke-linejoin="${fixture.join ?? 'miter'}"`,
    'stroke-miterlimit="4"',
  ];
  if (fixture.dash) attrs.push(`stroke-dasharray="${fixture.dash.join(' ')}"`);
  if (fixture.transform) attrs.push(`transform="matrix(${fixture.transform.join(' ')})"`);
  const tag = fixture.closed ? 'polygon' : 'polyline';
  return `<${tag} ${attrs.join(' ')}/>`;
}

const page = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="color-scheme" content="only light">
<title>SDF line browser references</title>
<style>
  /* No text or fonts: each case is one ${CROP}x${CROP} SVG crop. */
  html, body { margin: 0; background: transparent; }
  main { display: grid; grid-template-columns: repeat(${COLUMNS}, ${CROP}px); }
  svg { display: block; width: ${CROP}px; height: ${CROP}px; }
</style>
</head>
<body>
<main>
${cases.map((c) => `<svg data-case="${c.id}" viewBox="0 0 ${CROP} ${CROP}" xmlns="http://www.w3.org/2000/svg">${shape(c)}</svg>`).join('\n')}
</main>
</body>
</html>
`;
await writeFile(path.join(here, 'reference.html'), page);

await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });
const browser = await chromium.launch({ headless: true });
const metadata = {
  schema_version: 1,
  captured_at_utc: new Date().toISOString(),
  browser: await browser.version(),
  engine: 'Playwright Chromium',
  source: 'cases.json via reference.html',
  crop_css_px: CROP,
  device_scale_factors: DPRS,
};
try {
  for (const dpr of DPRS) {
    const context = await browser.newContext({
      viewport: { width: CROP * COLUMNS, height: CROP * Math.ceil(cases.length / COLUMNS) },
      deviceScaleFactor: dpr,
      colorScheme: 'light',
    });
    const tab = await context.newPage();
    await tab.goto(pathToFileURL(path.join(here, 'reference.html')).href, { waitUntil: 'load' });
    for (const fixture of cases) {
      const destination = path.join(outputRoot, fixture.id, `alpha@${dpr}x.png`);
      await mkdir(dirname(destination), { recursive: true });
      await tab.locator(`[data-case="${fixture.id}"]`).screenshot({ path: destination, omitBackground: true });
    }
    await context.close();
  }
  await writeFile(path.join(outputRoot, 'capture-metadata.json'), `${JSON.stringify(metadata, null, 2)}\n`);
} finally {
  await browser.close();
}
