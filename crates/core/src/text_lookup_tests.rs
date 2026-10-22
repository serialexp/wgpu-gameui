//! The caret lookups ([`caret_for_byte`], [`byte_at_point`],
//! [`byte_under_point`], [`byte_on_adjacent_line`], [`selection_rects`]) go
//! to the line they need rather than through every caret. Each must answer as
//! the scan over every caret did, kept here as the reference, on the layouts
//! that put carets out of byte order (right-to-left text) or lines out of the
//! ordinary (wraps, empty lines, a final newline).

use super::*;
use crate::DrawList;

/// The texts the lookups are checked on, each with the width it wraps at.
fn blocks() -> Vec<TextBlock> {
    let texts = [
        ("ab\ncd", WrapMode::None, 1000.0),
        (
            "abc שלום def\nעברית 123 text\n\nend",
            WrapMode::None,
            1000.0,
        ),
        ("שלום עולם\nhello עברית world\n", WrapMode::Word, 90.0),
        (
            "the quick brown fox jumps over the lazy dog and keeps running",
            WrapMode::Word,
            120.0,
        ),
        ("مرحبا بالعالم abc مرحبا", WrapMode::Word, 80.0),
        ("", WrapMode::None, 1000.0),
    ];
    texts
        .into_iter()
        .map(|(text, wrap, width)| {
            TextBlock::new(text, 0.0, 0.0)
                .with_size(16.0)
                .with_wrap(wrap)
                .with_max_width(width)
        })
        .collect()
}

fn reference_caret_for_byte(layout: &[CaretPos], byte: usize) -> Option<CaretPos> {
    layout
        .iter()
        .copied()
        .find(|p| p.byte >= byte)
        .or_else(|| layout.last().copied())
}

fn reference_byte_at_point(layout: &[CaretPos], x: f32, y: f32) -> usize {
    if layout.is_empty() {
        return 0;
    }
    let first = layout.first().unwrap();
    let last = layout.last().unwrap();
    let target_line = if y < first.line_top {
        first.line
    } else if y >= last.line_top + last.line_height {
        last.line
    } else {
        layout
            .iter()
            .find(|p| y >= p.line_top && y < p.line_top + p.line_height)
            .map(|p| p.line)
            .unwrap_or(last.line)
    };
    let mut best_byte = 0usize;
    let mut best_dx = f32::MAX;
    for p in layout.iter().filter(|p| p.line == target_line) {
        let dx = (p.x - x).abs();
        if dx < best_dx {
            best_dx = dx;
            best_byte = p.byte;
        }
    }
    best_byte
}

fn reference_byte_under_point(layout: &[CaretPos], x: f32, y: f32) -> usize {
    let caret = reference_byte_at_point(layout, x, y);
    let Some(line) = reference_caret_for_byte(layout, caret).map(|p| p.line) else {
        return 0;
    };
    layout
        .iter()
        .filter(|p| p.line == line && p.x <= x)
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .or_else(|| layout.iter().find(|p| p.line == line))
        .map_or(0, |p| p.byte)
}

fn reference_byte_on_adjacent_line(
    layout: &[CaretPos],
    byte: usize,
    dir: i32,
    desired_x: f32,
) -> usize {
    if layout.is_empty() {
        return byte;
    }
    let cur = reference_caret_for_byte(layout, byte).unwrap();
    let target = cur.line as i32 + dir;
    if target < layout[0].line as i32 || target > layout.last().unwrap().line as i32 {
        return byte;
    }
    let mut best_byte = byte;
    let mut best_dx = f32::MAX;
    for p in layout.iter().filter(|p| p.line == target as usize) {
        let dx = (p.x - desired_x).abs();
        if dx < best_dx {
            best_dx = dx;
            best_byte = p.byte;
        }
    }
    best_byte
}

fn reference_selection_rects(glyphs: &[VisualGlyph], start: usize, end: usize) -> Vec<SelRect> {
    let mut out = Vec::new();
    if start >= end || glyphs.is_empty() {
        return out;
    }
    let max_line = glyphs.iter().map(|g| g.line).max().unwrap_or(0);
    for line in 0..=max_line {
        let mut spans: Vec<(f32, f32, f32, f32)> = glyphs
            .iter()
            .filter(|g| g.line == line && g.byte_start < end && g.byte_end > start)
            .map(|g| (g.x, g.x + g.w, g.line_top, g.line_height))
            .collect();
        if spans.is_empty() {
            continue;
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (y, h) = (spans[0].2, spans[0].3);
        let mut cur = (spans[0].0, spans[0].1);
        for &(s, e, _, _) in &spans[1..] {
            if s <= cur.1 + 0.5 {
                cur.1 = cur.1.max(e);
            } else {
                out.push(SelRect {
                    x: cur.0,
                    y,
                    w: cur.1 - cur.0,
                    h,
                });
                cur = (s, e);
            }
        }
        out.push(SelRect {
            x: cur.0,
            y,
            w: cur.1 - cur.0,
            h,
        });
    }
    out
}

/// Whether some line of `layout` keeps its carets out of byte order, so the
/// texts do test what a binary search over bytes would get wrong.
fn out_of_order(layout: &[CaretPos]) -> bool {
    layout
        .windows(2)
        .any(|w| w[0].line == w[1].line && w[1].byte < w[0].byte)
}

#[test]
fn the_texts_put_carets_out_of_byte_order_and_wrap() {
    let layouts: Vec<_> = blocks()
        .iter()
        .map(|block| DrawList::new().caret_layout(block))
        .collect();
    assert!(
        layouts.iter().any(|l| out_of_order(l)),
        "right-to-left carets"
    );
    assert!(
        layouts
            .iter()
            .any(|l| l.last().is_some_and(|p| p.line >= 3)),
        "wrapped lines"
    );
}

#[test]
fn caret_for_byte_answers_as_a_scan_of_every_caret() {
    for block in blocks() {
        let layout = DrawList::new().caret_layout(&block);
        for byte in 0..=block.content.len() + 2 {
            assert_eq!(
                (!layout.is_empty()).then(|| caret_for_byte(&layout, byte)),
                reference_caret_for_byte(&layout, byte),
                "{:?} at byte {byte}",
                block.content
            );
        }
    }
}

#[test]
fn hit_tests_answer_as_a_scan_of_every_caret() {
    for block in blocks() {
        let layout = DrawList::new().caret_layout(&block);
        let bottom = layout.last().map_or(0.0, |p| p.line_top + p.line_height);
        let mut y = -12.0;
        while y < bottom + 12.0 {
            let mut x = -12.0;
            while x < block.max_width.min(400.0) + 12.0 {
                let what = format!("{:?} at ({x}, {y})", block.content);
                assert_eq!(
                    byte_at_point(&layout, x, y),
                    reference_byte_at_point(&layout, x, y),
                    "byte_at_point, {what}"
                );
                assert_eq!(
                    byte_under_point(&layout, x, y),
                    reference_byte_under_point(&layout, x, y),
                    "byte_under_point, {what}"
                );
                x += 3.0;
            }
            y += 2.5;
        }
    }
}

#[test]
fn moving_a_line_up_or_down_answers_as_a_scan_of_every_caret() {
    for block in blocks() {
        let layout = DrawList::new().caret_layout(&block);
        for p in layout.iter() {
            for dir in [-1, 1] {
                for desired in [-5.0, 0.0, p.x, 37.0, 500.0] {
                    assert_eq!(
                        byte_on_adjacent_line(&layout, p.byte, dir, desired),
                        reference_byte_on_adjacent_line(&layout, p.byte, dir, desired),
                        "{:?} from byte {} by {dir} at x {desired}",
                        block.content,
                        p.byte
                    );
                }
            }
        }
    }
}

#[test]
fn selection_rects_answer_as_a_scan_of_every_line() {
    for block in blocks() {
        let glyphs = DrawList::new().visual_layout(&block);
        let len = block.content.len();
        for start in 0..=len {
            for end in start..=len + 1 {
                assert_eq!(
                    selection_rects(&glyphs, start, end),
                    reference_selection_rects(&glyphs, start, end),
                    "{:?} selecting {start}..{end}",
                    block.content
                );
            }
        }
    }
}
