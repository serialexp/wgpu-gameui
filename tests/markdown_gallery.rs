//! A markdown document drawn at two widths, to look at.
//!
//! ```text
//! DISPLAY=:0 cargo test --features markdown --test markdown_gallery -- --ignored --nocapture
//! ```
//! Needs a GPU; writes `test_output/markdown_gallery.png`: the document at a
//! width its table fits, and beside it at one too narrow for the table's
//! words, scrolled part of the way.
#![cfg(all(feature = "markdown", feature = "headless"))]

use wgpu_gameui::{Markdown, MarkdownStyle, StyleResolver, Theme};

const SOURCE: &str = "\
## Calls across the boundary

Each figure is **per call**, measured with `perf stat` over a *warm* loop; see \
[the method](https://example.com/method) for how. ~~Cold runs~~ are left out.

| | Instructions per call | Cycles per call | Store→load stalls per call |
|---|--:|--:|--:|
| LuaJIT C API | 304 | 54 | 0 |
| Lua 5.4 C API | 449 | 75 | 3.2 |
| mahina C API | 567 | 119 | 7.3 |
| mlua over LuaJIT / Lua 5.4 | — | 24 / 26 ns | — |
| mahina Rust crate | 2,894 | 172 ns | — |
| mahina Rust crate, linking fixed (experiment) | 1,516 | 76 ns | 26.8 |

1. Build with `--release`.
2. Run the bench:
   - once cold,
   - then **five** times warm.
- [x] LuaJIT measured
- [ ] Lua 5.5 next

> The stalls come from the store forwarding
> a 16-byte value through two 8-byte loads.

```rust
fn call(state: &mut State) -> i32 {
    state.push(1);
}
```

---

That's all.";

#[test]
#[ignore = "needs a GPU adapter"]
fn render_markdown_gallery() {
    use wgpu_gameui::{HeadlessGpu, write_png};

    let Some(mut gpu) = HeadlessGpu::new() else {
        eprintln!("no GPU adapter — skipping");
        return;
    };
    let theme = Theme::default();
    let style = MarkdownStyle::forge(&StyleResolver::new(&theme));
    let doc = Markdown::parse(SOURCE);
    let mut list = gpu.draw_list();
    let (wide, narrow, margin) = (720.0, 300.0, 20.0);
    let left = doc.layout(&mut list, &style, wide);
    left.paint(&mut list, margin, margin, &[], None);
    let right = doc.layout(&mut list, &style, narrow);
    let scroll: Vec<f32> = right
        .tables()
        .iter()
        .map(|t| t.max_scroll() * 0.4)
        .collect();
    right.paint(&mut list, margin * 2.0 + wide, margin, &scroll, None);

    let height = left.height().max(right.height()) + margin * 2.0;
    let size = ((wide + narrow + margin * 3.0) as u32, height.ceil() as u32);
    let pixels = gpu.capture_on(&list, size, wgpu_gameui::Srgb::new([0.09, 0.1, 0.11, 1.0]));
    write_png("test_output/markdown_gallery.png", &pixels, size).expect("write png");
    eprintln!(
        "wrote test_output/markdown_gallery.png ({}x{})",
        size.0, size.1
    );
}
