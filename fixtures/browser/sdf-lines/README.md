# SDF line browser fixtures

Chromium reference captures for [`docs/design/sdf-lines.md`](../../../docs/design/sdf-lines.md),
compared by `tests/stroke_browser_parity.rs`. Like `../gpu-chrome-shadows`, this adds no npm
package manifest and no Rust build dependency.

## Contents

- `cases.json` is the one list of cases, read by both the capture script and the Rust test:
  each case's points, width, cap, join, dashes, opacity, whether it is closed, and an
  optional SVG `matrix(a b c d e f)` transform. It covers the three caps, the three joins, a
  miter past its limit, dashes round a corner, zero-length round dashes (dots), translucent
  polylines (miter and round), axis-aligned 1 px and 2 px lines, thin diagonals, a
  translucent closed outline, a dense chart line (two points a pixel) and a transformed
  polyline.
- `capture.mjs` writes `reference.html` (each case as a black SVG `polyline` or `polygon`
  in its own 120 x 120 CSS px crop, on a transparent page) and captures every case at
  DPR 1, 1.5 and 2 into `captures/<case>/alpha@<dpr>x.png`. The alpha is the stroke's
  coverage times its opacity.
- `captures/capture-metadata.json` records the Chromium version and capture time.

## Capture

With a Playwright installation outside this repository (see `../gpu-chrome-shadows/README.md`):

```sh
NODE_PATH="$PWD/node_modules" node <repo>/fixtures/browser/sdf-lines/capture.mjs
```

The script replaces `reference.html` and `captures/`; review the image diff and the browser
version before committing a refresh. Adding or changing a case means re-capturing and
updating the case count in the test.

## Comparison policy

The test compares alpha: total coverage within 2%, the mean difference over painted pixels,
and the share of pixels more than a fifth apart. Where Chromium's own coverage isn't exact,
the test says so per case and checks total coverage against the exact area instead:

- thin diagonals (1 px, 1.5 px): Chromium's coverage changes with DPR (the 1 px one paints
  90, 105 and 94 px² at DPR 1, 1.5 and 2, for an area of 104);
- small round dots: Chromium's are 2-3% heavier than a disc.

The dense chart line is allowed 3% at DPR 1: where the line folds back on itself, segments
that aren't neighbours each paint the soft pixels at its edge.
