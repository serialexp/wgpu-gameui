//! Waffle chart — a grid of small cells, each standing for an equal share of
//! a whole, beside a legend (Forge's "context usage" popover: a 10×10 grid of
//! token categories with name, tokens and percent per legend row).
//!
//! The caller decides which category each cell belongs to (`cells`, row
//! major) once, when the numbers change; the widget only paints and reports
//! which category the pointer is over. While one is hovered, every other
//! category's cells fade to a quarter and its legend row lights up.

use crate::InputState;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleResolver, TextSize};

use super::DrawList;

/// Side of one cell.
pub const WAFFLE_CELL: f32 = 12.0;
/// Space between cells.
const CELL_GAP: f32 = 2.0;
/// Height of a legend row.
pub const WAFFLE_LEGEND_ROW: f32 = 19.0;
/// Space between the grid and the legend.
const LEGEND_GAP: f32 = 12.0;
/// Legend columns: swatch, name (the rest), tokens, percent; 6px apart,
/// inside 4px of padding.
const SWATCH_COL: f32 = 12.0;
const SWATCH: f32 = 8.0;
const TOKENS_COL: f32 = 42.0;
const PERCENT_COL: f32 = 38.0;
const COL_GAP: f32 = 6.0;
const ROW_PAD: f32 = 4.0;
/// A faded cell (another category is hovered).
const FADED: f32 = 0.25;
const ROW_HOVER: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// A filled cell's lit top line and drop shadow.
const CELL_HI: [f32; 4] = [1.0, 1.0, 1.0, 0.22];
const CELL_DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.4];
/// A hatched cell: dark ground, faint diagonal lines every 4px, sunken.
const HATCH_GROUND: [f32; 4] = [0.0, 0.0, 0.0, 0.4];
const HATCH_LINE: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
const HATCH_RECESS: [f32; 4] = [0.0, 0.0, 0.0, 0.6];

/// How a category's cells are painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaffleFill {
    /// A lit solid colour.
    Solid([f32; 4]),
    /// Sunken, with faint diagonal hatching: space not taken (free).
    Hatched,
}

/// One category of a [`Waffle`], and its legend row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaffleCategory<'a> {
    /// Its name in the legend.
    pub name: &'a str,
    /// Its amount, as shown ("12.4k").
    pub amount: &'a str,
    /// Its share, as shown ("6.2%").
    pub percent: &'a str,
    /// Its cells' paint.
    pub fill: WaffleFill,
    /// Dimmed italic name (a reserve or remainder rather than a real use).
    pub aside: bool,
}

/// What a [`Waffle`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaffleOutput {
    /// The category under the pointer (a cell or a legend row), if any.
    pub hovered: Option<usize>,
    /// The height it took.
    pub height: f32,
}

/// A grid of category cells beside its legend.
#[derive(Clone, Copy, Debug)]
pub struct Waffle<'a> {
    categories: &'a [WaffleCategory<'a>],
    cells: &'a [u8],
    columns: usize,
    hovered: Option<usize>,
}

impl<'a> Waffle<'a> {
    /// A waffle of `cells` (each an index into `categories`, row major) laid
    /// out in rows of `columns`.
    pub fn new(categories: &'a [WaffleCategory<'a>], cells: &'a [u8], columns: usize) -> Self {
        Self {
            categories,
            cells,
            columns: columns.max(1),
            hovered: None,
        }
    }

    /// The category last frame's [`WaffleOutput::hovered`] reported: the
    /// fading needs it before the cells are painted.
    pub fn hovered(mut self, hovered: Option<usize>) -> Self {
        self.hovered = hovered;
        self
    }

    fn grid_size(&self) -> (f32, f32) {
        let rows = self.cells.len().div_ceil(self.columns);
        let side = |n: usize| n as f32 * WAFFLE_CELL + n.saturating_sub(1) as f32 * CELL_GAP;
        (side(self.columns), side(rows))
    }

    /// The height it takes: the taller of the grid and the legend.
    pub fn height(&self) -> f32 {
        let legend = self.categories.len() as f32 * WAFFLE_LEGEND_ROW;
        self.grid_size().1.max(legend)
    }

    fn cell_rect(&self, x: f32, y: f32, i: usize) -> Rect {
        let (col, row) = (i % self.columns, i / self.columns);
        Rect::new(
            x + col as f32 * (WAFFLE_CELL + CELL_GAP),
            y + row as f32 * (WAFFLE_CELL + CELL_GAP),
            WAFFLE_CELL,
            WAFFLE_CELL,
        )
    }

    /// The category the pointer is over, given the waffle at `(x, y)` with a
    /// legend `width` wide.
    fn hit(&self, x: f32, y: f32, width: f32, input: &InputState) -> Option<usize> {
        if input.mouse_consumed {
            return None;
        }
        let (mx, my) = (input.mouse_x, input.mouse_y);
        let (grid_w, _) = self.grid_size();
        for i in 0..self.cells.len() {
            if self.cell_rect(x, y, i).contains(mx, my) {
                return Some(usize::from(self.cells[i]));
            }
        }
        let legend_x = x + grid_w + LEGEND_GAP;
        let legend = Rect::new(
            legend_x,
            y,
            (x + width - legend_x).max(0.0),
            self.categories.len() as f32 * WAFFLE_LEGEND_ROW,
        );
        legend
            .contains(mx, my)
            .then(|| ((my - y) / WAFFLE_LEGEND_ROW) as usize)
            .filter(|&row| row < self.categories.len())
    }

    /// Draw it with its top-left corner at `(x, y)`, `width` wide in all.
    pub fn draw(
        &self,
        x: f32,
        y: f32,
        width: f32,
        list: &mut DrawList,
        s: &StyleResolver,
        input: &InputState,
    ) -> WaffleOutput {
        let hit = self.hit(x, y, width, input);
        let lit = hit.or(self.hovered).filter(|&h| h < self.categories.len());
        for (i, &category) in self.cells.iter().enumerate() {
            let Some(c) = self.categories.get(usize::from(category)) else {
                continue;
            };
            let alpha = match lit {
                Some(h) if h != usize::from(category) => FADED,
                _ => 1.0,
            };
            paint_cell(list, self.cell_rect(x, y, i), c.fill, alpha);
        }

        let (grid_w, _) = self.grid_size();
        let legend_x = x + grid_w + LEGEND_GAP;
        let legend_w = (x + width - legend_x).max(0.0);
        for (row, c) in self.categories.iter().enumerate() {
            let r = Rect::new(
                legend_x,
                y + row as f32 * WAFFLE_LEGEND_ROW,
                legend_w,
                WAFFLE_LEGEND_ROW,
            );
            if lit == Some(row) {
                list.quad(r.x, r.y, r.width, r.height, ROW_HOVER);
            }
            let cy = r.y + r.height * 0.5;
            let swatch = Rect::new(r.x + ROW_PAD, (cy - SWATCH * 0.5).round(), SWATCH, SWATCH);
            paint_cell(list, swatch, c.fill, 1.0);
            let right = r.right() - ROW_PAD;
            let pct_x = right - PERCENT_COL;
            let tok_x = pct_x - COL_GAP - TOKENS_COL;
            let name_x = r.x + ROW_PAD + SWATCH_COL + COL_GAP;
            let row_size = s.text_size(TextSize::Row);
            let meta = s.text_size(TextSize::Meta);
            let mut name = s
                .sans_block(
                    c.name,
                    name_x,
                    crate::text::vcentered_line_y(r.y, r.height, row_size),
                    TextSize::Row,
                    if c.aside { Ink::Caption } else { Ink::Row },
                )
                .with_max_width((tok_x - COL_GAP - name_x).max(0.0))
                .with_ellipsis();
            if c.aside {
                name = name.italic();
            }
            list.text(name);
            let meta_y = crate::text::vcentered_line_y(r.y, r.height, meta);
            right_text(list, s, c.amount, tok_x + TOKENS_COL, meta_y, Ink::Row);
            right_text(list, s, c.percent, right, meta_y, Ink::Caption);
        }
        WaffleOutput {
            hovered: hit,
            height: self.height(),
        }
    }
}

fn right_text(list: &mut DrawList, s: &StyleResolver, text: &str, right: f32, y: f32, ink: Ink) {
    let mut block = s.mono_block(text, 0.0, y, TextSize::Meta, ink);
    let (w, _) = list.measure_block(&block);
    block.x = right - w;
    list.text(block);
}

fn fade(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

fn paint_cell(list: &mut DrawList, r: Rect, fill: WaffleFill, alpha: f32) {
    match fill {
        WaffleFill::Solid(color) => {
            list.quad(r.x, r.bottom(), r.width, 1.0, fade(CELL_DROP, alpha));
            list.quad(r.x, r.y, r.width, r.height, fade(color, alpha));
            list.quad(r.x, r.y, r.width, 1.0, fade(CELL_HI, alpha));
        }
        WaffleFill::Hatched => {
            list.quad(r.x, r.y, r.width, r.height, fade(HATCH_GROUND, alpha));
            // 135° lines every 4px, clipped to the cell.
            list.push_clip(r);
            let line = fade(HATCH_LINE, alpha);
            let mut k = 0.0;
            while k < r.width + r.height {
                list.line([r.x + k, r.y], [r.x + k - r.height, r.bottom()], 1.0, line);
                k += 4.0;
            }
            list.pop_clip();
            list.box_shadow_inset(
                r,
                CornerRadii::uniform(0.0),
                BoxShadow {
                    offset: [0.0, 1.0],
                    blur: 2.0,
                    color: fade(HATCH_RECESS, alpha),
                    inset: true,
                    ..BoxShadow::default()
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn categories() -> [WaffleCategory<'static>; 3] {
        [
            WaffleCategory {
                name: "System prompt",
                amount: "3k",
                percent: "1.5%",
                fill: WaffleFill::Solid(RED),
                aside: false,
            },
            WaffleCategory {
                name: "Messages",
                amount: "80k",
                percent: "40.0%",
                fill: WaffleFill::Solid(BLUE),
                aside: false,
            },
            WaffleCategory {
                name: "Free space",
                amount: "117k",
                percent: "58.5%",
                fill: WaffleFill::Hatched,
                aside: true,
            },
        ]
    }

    fn solid_cells(list: &DrawList, color: [f32; 4]) -> usize {
        list.chrome_instances()
            .filter(|c| c.bg == color && c.rect[2] == WAFFLE_CELL && c.rect[3] == WAFFLE_CELL)
            .count()
    }

    #[test]
    fn cells_take_their_category_and_the_legend_lists_each() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let cats = categories();
        let cells = [0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2];
        let waffle = Waffle::new(&cats, &cells, 4);
        let mut list = DrawList::new();
        let away = InputState {
            mouse_x: 900.0,
            mouse_y: 900.0,
            ..Default::default()
        };
        let out = waffle.draw(0.0, 0.0, 300.0, &mut list, &s, &away);
        assert_eq!(solid_cells(&list, RED), 1);
        assert_eq!(solid_cells(&list, BLUE), 3);
        // Three rows of cells (40px) against three legend rows (57px).
        assert_eq!(out.height, 3.0 * WAFFLE_LEGEND_ROW);
        assert_eq!(out.hovered, None);
        for text in ["System prompt", "80k", "58.5%"] {
            assert!(list.texts.iter().any(|t| t.content == text), "{text}");
        }
        let free = list
            .texts
            .iter()
            .find(|t| t.content == "Free space")
            .unwrap();
        assert_eq!(free.style, cosmic_text::Style::Italic, "an aside is italic");
    }

    #[test]
    fn hovering_a_cell_or_a_row_fades_the_other_categories() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let cats = categories();
        let cells = [0, 1, 1, 1];
        let waffle = Waffle::new(&cats, &cells, 4);
        // Over the second cell (Messages).
        let over_cell = InputState {
            mouse_x: WAFFLE_CELL + CELL_GAP + 3.0,
            mouse_y: 3.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = waffle.draw(0.0, 0.0, 300.0, &mut list, &s, &over_cell);
        assert_eq!(out.hovered, Some(1));
        assert_eq!(solid_cells(&list, BLUE), 3, "Messages stays lit");
        assert_eq!(solid_cells(&list, fade(RED, FADED)), 1, "the rest fade");

        // Over the first legend row (System prompt).
        let (grid_w, _) = waffle.grid_size();
        let over_row = InputState {
            mouse_x: grid_w + LEGEND_GAP + 30.0,
            mouse_y: 5.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = waffle.draw(0.0, 0.0, 300.0, &mut list, &s, &over_row);
        assert_eq!(out.hovered, Some(0));
        assert_eq!(solid_cells(&list, fade(BLUE, FADED)), 3);

        // Last frame's hover fades the others before this frame's hit test.
        let away = InputState {
            mouse_x: 900.0,
            mouse_y: 900.0,
            ..Default::default()
        };
        let mut list = DrawList::new();
        let out = waffle
            .hovered(Some(1))
            .draw(0.0, 0.0, 300.0, &mut list, &s, &away);
        assert_eq!(out.hovered, None);
        assert_eq!(solid_cells(&list, fade(RED, FADED)), 1);
        assert_eq!(solid_cells(&list, BLUE), 3);
    }
}
