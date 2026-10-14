# GPU chrome and shadow browser fixtures

This directory is Phase-0 reference-fixture infrastructure for
[`docs/design/gpu-chrome-and-shadows.md`](../../../docs/design/gpu-chrome-and-shadows.md).
It is deliberately standalone: it adds neither an npm package manifest nor a Rust test
or build dependency.

## Contents

- `reference.html` is a static, font-independent CSS fixture matrix. It contains blur
  lengths 0/2/6/8/18/26/40/44px, positive and negative offsets, both spread signs,
  inset and collapsed-hole cases, square/1px/12px/asymmetric radii, thin 2x26px
  sources, black/cyan/multi-shadow colors, and rotate/scale/skew/reflection affine
  transforms.
- `manifest.json` is the machine-readable case list, CSS-pixel crop coordinates,
  output naming convention, fixed viewport/DPR settings, and browser metadata policy.
- `capture.mjs` captures every case over transparent, black, and white backdrops.
  Its alpha capture removes the opaque source fill and border while retaining the CSS
  shadow; black and white outputs retain the ordinary source and provide the known
  composites required to recover/check shadow coverage independently of RGB.

The reviewed baseline captures are checked in under `captures/` with
`capture-metadata.json`. Regeneration deletes and replaces that directory, so a
reference refresh must review its image diff and browser-version change before
updating the baseline.

## Capture

Use a Playwright installation outside this repository (the script imports only the
`playwright` package and does not alter package manifests):

```sh
# In a disposable tools directory, once:
npm install playwright
npx playwright install chromium

# From that tools environment, make Playwright resolvable and run:
NODE_PATH="$PWD/node_modules" node /home/bart/Projects/serialexp/wgpu-gameui/fixtures/browser/gpu-chrome-shadows/capture.mjs
```

`capture.mjs` explicitly resolves the package through Node's module resolver, so the
shown `NODE_PATH` works even though the script itself resides outside the tools
directory.

The script deletes and recreates `captures/`, then writes:

```text
captures/<case-id>/alpha@1x.png
captures/<case-id>/black@1.5x.png
captures/<case-id>/white@2x.png
captures/capture-metadata.json
```

The viewport is fixed at **1440x1400 CSS px** and the three device scale factors are
**1, 1.5, and 2**. Each individual crop is exactly **288x280 CSS px**, so output PNG
pixel dimensions are that crop multiplied by its DPR. The metadata file records the
actual Chromium version and UTC capture timestamp; use it to decide whether a refresh
is an intentional browser-version update.

## Comparison policy

Compare `alpha` images first for blur/sigma and geometry behavior. Compare `black` and
`white` only as final sRGB browser composites; they are not expected to equal the
renderer's linear-RGBA output pixel-for-pixel. This realizes the design's `sigma =
blur / 2` starting-contract investigation without fitting sigma to gamma differences.
