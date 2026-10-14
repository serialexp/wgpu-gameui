//! Status detail — what a status bar zone opens above itself (Forge
//! `StatusDetail` in a `StatusBar` drop-up).
//!
//! A sheet over a drop-up zone's key ([`StatusZone::drop_up`]), aligned to
//! the key's left edge in the left zones and its right edge in the right
//! group: a caption title with a meta value at its right, label/value rows
//! (each with an optional meter under it), and a spark of recent values
//! against a budget, the last two parted by a rule. Forge's stacked bar and
//! footer are not drawn yet. Whoever owns it keeps it open and closes it
//! (Esc, a click outside, its zone again); this places and paints it.
//!
//! [`StatusZone::drop_up`]: super::StatusZone::drop_up

use crate::chrome::SurfacePainter;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize, Tracking};
use crate::text::vcentered_line_y;

use super::DrawList;
use super::meter::{MeterFill, inline_meter};
use super::status_zones::{ZoneAnchor, ZoneSide};

/// Forge's default width, and the least it is drawn at.
pub const STATUS_DETAIL_WIDTH: f32 = 240.0;
const MIN_WIDTH: f32 = 200.0;
/// Inside the sheet's edges.
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 9.0;
/// Between sections (the title, the rows, the spark), and a rule's lines
/// between the two after the title.
const SECTION_GAP: f32 = 9.0;
const RULE: [f32; 4] = [0.039, 0.047, 0.051, 1.0];
const RULE_HI: [f32; 4] = [0.149, 0.169, 0.184, 1.0];
/// The spark: its well, the space inside its ends, between its bars and
/// over its caption line; how far the tallest of value and budget reaches
/// up it; its budget line's dashes.
const SPARK_H: f32 = 40.0;
const SPARK_PAD: f32 = 2.0;
const SPARK_BAR_GAP: f32 = 1.0;
const SPARK_CAPTION_GAP: f32 = 3.0;
const SPARK_HEADROOM: f32 = 1.1;
const SPARK_BAR_ALPHA: f32 = 0.8;
const DASH: f32 = 3.0;
const BUDGET_LINE: [f32; 4] = [1.0, 1.0, 1.0, 0.45];
/// The spark's well (`rgba(0,0,0,0.45)`, a shade inside).
const WELL: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const WELL_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// One line of text (10px mono on 13px).
const LINE: f32 = 13.0;
/// Between rows, between a row and its meter, and the meter's height.
const ROW_GAP: f32 = 5.0;
const METER_GAP: f32 = 3.0;
const METER_H: f32 = 4.0;
/// Least space between a row's label and its value, and between the title
/// and its meta.
const VALUE_GAP: f32 = 10.0;
const META_GAP: f32 = 8.0;
/// Between the sheet and its zone's key.
const ABOVE_KEY: f32 = 5.0;

/// The colour a row's value (and meter) takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailTone {
    /// Healthy (`--ok`).
    Ok,
    /// Wants attention (`--warn-*`).
    Warn,
    /// Broken (`--danger-*`).
    Error,
}

impl DetailTone {
    fn color(self, s: &StyleResolver) -> [f32; 4] {
        match self {
            Self::Ok => s.color(StyleKey::StatusOk),
            Self::Warn => s.color(StyleKey::WarnMeta),
            Self::Error => s.color(StyleKey::DangerText),
        }
    }
}

/// One label/value row of a [`StatusDetail`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailRow<'a> {
    /// At the left, ellipsized where the value needs the room.
    pub label: &'a str,
    /// At the right, whole.
    pub value: &'a str,
    /// The value's colour, and its meter's; the plain value ink without.
    pub tone: Option<DetailTone>,
    /// A secondary row: its label in the muted ink.
    pub dim: bool,
    /// A meter under the row, this full (0 to 1).
    pub meter: Option<f32>,
}

impl<'a> DetailRow<'a> {
    /// A row reading `label` at the left and `value` at the right.
    pub const fn new(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value,
            tone: None,
            dim: false,
            meter: None,
        }
    }

    /// Colour the value (and the meter) in `tone`.
    pub const fn tone(mut self, tone: DetailTone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// A secondary row: the label in the muted ink.
    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// A meter under the row, `fraction` full.
    pub const fn meter(mut self, fraction: f32) -> Self {
        self.meter = Some(fraction);
        self
    }

    fn height(&self) -> f32 {
        match self.meter {
            Some(_) => LINE + METER_GAP + METER_H,
            None => LINE,
        }
    }
}

/// A [`StatusDetail`]'s history: a bar per value, from the oldest at the
/// left, scaled to the tallest of them and the budget; those over the
/// budget in the warning colour, and the budget a dashed line across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailSpark<'a> {
    /// Oldest first.
    pub values: &'a [f32],
    /// In the values' unit.
    pub budget: Option<f32>,
    /// Under it at the left ("120 frames").
    pub caption: &'a str,
    /// Under it at the right ("┄ budget 16.7 ms").
    pub budget_caption: Option<&'a str>,
}

impl DetailSpark<'_> {
    fn height() -> f32 {
        SPARK_H + SPARK_CAPTION_GAP + LINE
    }
}

/// A status zone's drop-up detail (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusDetail<'a> {
    title: Option<&'a str>,
    meta: Option<&'a str>,
    rows: &'a [DetailRow<'a>],
    spark: Option<DetailSpark<'a>>,
    width: f32,
}

impl<'a> StatusDetail<'a> {
    /// A detail of `rows`, [`STATUS_DETAIL_WIDTH`] wide.
    pub const fn new(rows: &'a [DetailRow<'a>]) -> Self {
        Self {
            title: None,
            meta: None,
            rows,
            spark: None,
            width: STATUS_DETAIL_WIDTH,
        }
    }

    /// Its caption, in mono capitals.
    pub const fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// A value at the title's right ("42 ms", "5.1 / 16 GB").
    pub const fn meta(mut self, meta: &'a str) -> Self {
        self.meta = Some(meta);
        self
    }

    /// A history under the rows.
    pub const fn spark(mut self, spark: DetailSpark<'a>) -> Self {
        self.spark = Some(spark);
        self
    }

    /// Its width; never under 200px.
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    fn has_header(&self) -> bool {
        self.title.is_some() || self.meta.is_some()
    }

    fn rows_height(&self) -> f32 {
        self.rows.iter().map(DetailRow::height).sum::<f32>()
            + ROW_GAP * self.rows.len().saturating_sub(1) as f32
    }

    /// Its sections' heights, top down: the title line, the rows, the spark
    /// (those it has). Forge parts every two after the title with a rule.
    fn sections(&self) -> impl Iterator<Item = f32> + '_ {
        let header = self.has_header().then_some(LINE);
        let rows = (!self.rows.is_empty()).then(|| self.rows_height());
        let spark = self.spark.map(|_| DetailSpark::height());
        [header, rows, spark].into_iter().flatten()
    }

    /// The space before section `index`: none for the first, a gap after
    /// the title, a gap, a rule and a gap between the others.
    fn before(&self, index: usize) -> f32 {
        match index {
            0 => 0.0,
            1 if self.has_header() => SECTION_GAP,
            _ => SECTION_GAP * 2.0 + 1.0,
        }
    }

    /// Its width and height.
    pub fn size(&self) -> [f32; 2] {
        let height: f32 = self
            .sections()
            .enumerate()
            .map(|(index, height)| self.before(index) + height)
            .sum();
        [self.width.max(MIN_WIDTH), PAD_Y * 2.0 + height]
    }

    /// Where it goes over the key `anchor`, kept inside `bounds`.
    pub fn place(&self, anchor: ZoneAnchor, bounds: Rect) -> Rect {
        let [width, height] = self.size();
        let x = match anchor.side {
            ZoneSide::Left => anchor.rect.x,
            ZoneSide::Right => anchor.rect.right() - width,
        };
        let x = x.min(bounds.right() - width).max(bounds.x);
        let y = (anchor.rect.y - ABOVE_KEY - height).max(bounds.y);
        Rect::new(x, y, width, height)
    }

    /// Paint it in `rect` (see [`Self::place`]).
    pub fn draw(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) {
        list.push_debug_scope_rect("StatusDetail", rect);
        // Forge's drop-up is the menu sheet's surface: `--surface-sheet`,
        // a near-black edge and `--shadow-sheet`.
        let chrome = s.menu_sheet();
        let borders = chrome.surface.border_widths;
        let padding_box = Rect::new(
            rect.x + borders.left,
            rect.y + borders.top,
            (rect.width - borders.left - borders.right).max(0.0),
            (rect.height - borders.top - borders.bottom).max(0.0),
        );
        let mut painter = SurfacePainter::new(
            list,
            rect,
            padding_box,
            CornerRadii::default(),
            chrome.surface,
            &chrome.shadows,
            &chrome.lines,
        );
        painter.paint_pre_content();
        self.draw_content(rect, painter.draw_list(), s);
        painter.paint_post_content();
        list.pop_debug_scope();
    }

    fn draw_content(&self, rect: Rect, list: &mut DrawList, s: &StyleResolver) {
        let x = rect.x + PAD_X;
        let right = rect.right() - PAD_X;
        let mut y = rect.y + PAD_Y;
        let mut index = 0;
        // Move `y` to section `index`, drawing the rule before it if any.
        let mut next = |y: &mut f32, list: &mut DrawList| {
            let before = self.before(index);
            if before > SECTION_GAP {
                let rule_y = *y + SECTION_GAP;
                list.quad(rect.x + 1.0, rule_y, rect.width - 2.0, 1.0, RULE);
                list.quad(rect.x + 1.0, rule_y + 1.0, rect.width - 2.0, 1.0, RULE_HI);
            }
            *y += before;
            index += 1;
        };
        if self.has_header() {
            next(&mut y, list);
            let mut title_end = right;
            if let Some(meta) = self.meta {
                let width = s.mono_width(list, meta, TextSize::Meta);
                let ty = vcentered_line_y(y, LINE, s.text_size(TextSize::Meta));
                list.text(s.mono_block(meta, right - width, ty, TextSize::Meta, Ink::Second));
                title_end = right - width - META_GAP;
            }
            if let Some(title) = self.title {
                let ty = vcentered_line_y(y, LINE, s.text_size(TextSize::Caption));
                list.text(
                    s.caption_block(title, x, ty, Tracking::Caption, Ink::Caption)
                        .with_max_width((title_end - x).max(1.0))
                        .with_ellipsis(),
                );
            }
            y += LINE;
        }
        if !self.rows.is_empty() {
            next(&mut y, list);
            self.draw_rows(x, right, y, list, s);
            y += self.rows_height();
        }
        if let Some(spark) = self.spark {
            next(&mut y, list);
            draw_spark(&spark, x, right, y, list, s);
        }
    }

    fn draw_rows(&self, x: f32, right: f32, mut y: f32, list: &mut DrawList, s: &StyleResolver) {
        let ty_offset = vcentered_line_y(0.0, LINE, s.text_size(TextSize::Meta));
        for row in self.rows {
            let tone = row.tone.map(|tone| tone.color(s));
            let value_w = s.mono_width(list, row.value, TextSize::Meta);
            list.text(
                s.mono_block(
                    row.value,
                    right - value_w,
                    y + ty_offset,
                    TextSize::Meta,
                    Ink::Value,
                )
                .with_color_f32(tone.unwrap_or(s.ink(Ink::Value))),
            );
            let label_ink = if row.dim { Ink::Muted } else { Ink::Row };
            list.text(
                s.mono_block(row.label, x, y + ty_offset, TextSize::Meta, label_ink)
                    .with_max_width((right - value_w - VALUE_GAP - x).max(1.0))
                    .with_ellipsis(),
            );
            if let Some(fraction) = row.meter {
                let meter = Rect::new(x, y + LINE + METER_GAP, right - x, METER_H);
                let color = tone.unwrap_or(s.color(StyleKey::Accent));
                inline_meter(list, s, meter, MeterFill::colored(fraction, color));
            }
            y += row.height() + ROW_GAP;
        }
    }
}

/// A spark from `x` to `right`, its well's top at `y`.
fn draw_spark(
    spark: &DetailSpark,
    x: f32,
    right: f32,
    y: f32,
    list: &mut DrawList,
    s: &StyleResolver,
) {
    let well = Rect::new(x, y, right - x, SPARK_H);
    let radius = s.scalar(StyleKey::BorderRadius);
    list.chrome_rect(well, radius, 0.0, WELL, [0.0; 4]);
    list.box_shadow_inset(
        well,
        CornerRadii::uniform(radius),
        BoxShadow {
            offset: [0.0, 1.0],
            blur: 2.0,
            color: WELL_SHADE,
            inset: true,
            ..BoxShadow::default()
        },
    );
    let tallest = spark.values.iter().copied().fold(0.0_f32, f32::max);
    let peak = tallest.max(spark.budget.unwrap_or(0.0)) * SPARK_HEADROOM;
    let count = spark.values.len();
    if peak > 0.0 && count > 0 {
        let inner = well.width - SPARK_PAD * 2.0;
        let gaps = SPARK_BAR_GAP * (count - 1) as f32;
        let bar_w = ((inner - gaps) / count as f32).max(1.0);
        let accent = s.color(StyleKey::Accent);
        let warn = s.color(StyleKey::WarnMeta);
        let over = |value: f32| spark.budget.is_some_and(|budget| value > budget);
        for (i, &value) in spark.values.iter().enumerate() {
            let height = (value.max(0.0) / peak * SPARK_H).min(SPARK_H);
            let bar_x = well.x + SPARK_PAD + i as f32 * (bar_w + SPARK_BAR_GAP);
            if bar_x >= well.right() - SPARK_PAD {
                break;
            }
            let [r, g, b, _] = if over(value) { warn } else { accent };
            let alpha = if over(value) { 1.0 } else { SPARK_BAR_ALPHA };
            list.quad(
                bar_x,
                well.bottom() - height,
                bar_w,
                height,
                [r, g, b, alpha],
            );
        }
        if let Some(budget) = spark.budget {
            let line_y = (well.bottom() - budget / peak * SPARK_H).round();
            list.dashed_hline(well.x, line_y, well.width, DASH, DASH, BUDGET_LINE);
        }
    }
    let caption_top = well.bottom() + SPARK_CAPTION_GAP;
    let ty = vcentered_line_y(caption_top, LINE, s.text_size(TextSize::Caption));
    list.text(s.mono_block(spark.caption, x, ty, TextSize::Caption, Ink::Dim));
    if let Some(budget) = spark.budget_caption {
        let width = s.mono_width(list, budget, TextSize::Caption);
        list.text(s.mono_block(budget, right - width, ty, TextSize::Caption, Ink::Dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    const ROWS: [DetailRow<'static>; 3] = [
        DetailRow::new("server", "leviticus:8787"),
        DetailRow::new("disk", "71%").meter(0.71),
        DetailRow::new("build", "older")
            .tone(DetailTone::Warn)
            .dim(),
    ];

    #[test]
    fn its_height_is_the_title_and_each_row_with_its_meter() {
        let detail = StatusDetail::new(&ROWS).title("Connection").meta("42 ms");
        let rows = LINE + (LINE + METER_GAP + METER_H) + LINE + 2.0 * ROW_GAP;
        assert_eq!(
            detail.size(),
            [STATUS_DETAIL_WIDTH, PAD_Y * 2.0 + LINE + SECTION_GAP + rows]
        );
        assert_eq!(
            StatusDetail::new(&ROWS[..1]).width(120.0).size(),
            [MIN_WIDTH, PAD_Y * 2.0 + LINE]
        );
    }

    #[test]
    fn it_opens_above_its_key_on_the_key_s_side_and_stays_on_screen() {
        let detail = StatusDetail::new(&ROWS).title("Connection");
        let [width, height] = detail.size();
        let screen = Rect::new(0.0, 0.0, 800.0, 600.0);
        let key = Rect::new(500.0, 578.0, 90.0, 18.0);
        let right = detail.place(
            ZoneAnchor {
                rect: key,
                side: ZoneSide::Right,
            },
            screen,
        );
        assert_eq!(
            right,
            Rect::new(
                key.right() - width,
                key.y - ABOVE_KEY - height,
                width,
                height
            )
        );
        let left = detail.place(
            ZoneAnchor {
                rect: key,
                side: ZoneSide::Left,
            },
            screen,
        );
        assert_eq!(left.x, key.x);
        // A key near the right edge keeps a left-aligned sheet on screen.
        let edge = detail.place(
            ZoneAnchor {
                rect: Rect::new(700.0, 578.0, 60.0, 18.0),
                side: ZoneSide::Left,
            },
            screen,
        );
        assert_eq!(edge.right(), screen.right());
    }

    const SPARK: DetailSpark<'static> = DetailSpark {
        values: &[2.0, 3.0, 20.0, 2.5],
        budget: Some(16.7),
        caption: "4 frames",
        budget_caption: Some("┄ budget 16.7 ms"),
    };

    #[test]
    fn a_spark_goes_under_the_rows_past_a_rule() {
        let detail = StatusDetail::new(&ROWS).title("Frame time").spark(SPARK);
        let rows = LINE + (LINE + METER_GAP + METER_H) + LINE + 2.0 * ROW_GAP;
        let spark = SPARK_H + SPARK_CAPTION_GAP + LINE;
        assert_eq!(
            detail.size()[1],
            PAD_Y * 2.0 + LINE + SECTION_GAP + rows + SECTION_GAP * 2.0 + 1.0 + spark
        );
        // Without a title, the rule parts the rows and the spark all the same.
        assert_eq!(
            StatusDetail::new(&ROWS[..1]).spark(SPARK).size()[1],
            PAD_Y * 2.0 + LINE + SECTION_GAP * 2.0 + 1.0 + spark
        );
    }

    #[test]
    fn a_spark_bar_over_its_budget_is_in_the_warning_colour() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let detail = StatusDetail::new(&[]).spark(SPARK);
        let [width, height] = detail.size();
        let mut list = DrawList::new();
        detail.draw(Rect::new(0.0, 0.0, width, height), &mut list, &s);
        let [r, g, b, _] = s.color(StyleKey::WarnMeta);
        let warn = [r, g, b, 1.0];
        let well_bottom = PAD_Y + SPARK_H;
        // Bars stand on the well's floor; the well itself is wider than any.
        let bars: Vec<_> = list
            .chrome_instances()
            .filter(|c| {
                let [_, y, w, h] = c.rect;
                (y + h - well_bottom).abs() < 0.01 && h > 1.0 && w < 100.0
            })
            .map(|c| (c.rect, c.bg))
            .collect();
        assert_eq!(bars.len(), 4, "{bars:?}");
        assert_eq!(bars[2].1, warn, "20 ms is over 16.7");
        assert_ne!(bars[0].1, warn);
        // The tallest reaches the headroom under the well's top.
        assert!((bars[2].0[3] - SPARK_H / SPARK_HEADROOM).abs() < 0.01);
        let texts: Vec<_> = list.texts.iter().map(|t| t.content.as_str()).collect();
        assert!(texts.contains(&"4 frames") && texts.contains(&"┄ budget 16.7 ms"));
    }

    #[test]
    fn rows_draw_their_label_and_value_in_their_tone() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let detail = StatusDetail::new(&ROWS).title("Connection").meta("42 ms");
        let rect = Rect::new(0.0, 0.0, STATUS_DETAIL_WIDTH, detail.size()[1]);
        let mut list = DrawList::new();
        detail.draw(rect, &mut list, &s);
        let texts = list.texts.clone();
        let text = |content: &str| {
            texts
                .iter()
                .find(|block| block.content == content)
                .unwrap_or_else(|| panic!("{content} drawn"))
        };
        assert_eq!(text("CONNECTION").x, PAD_X);
        let older = text("older");
        assert_eq!(
            older.color,
            crate::color::text_color(DetailTone::Warn.color(&s))
        );
        let (width, _) = list.measure_block(older);
        assert!((older.x + width - (rect.right() - PAD_X)).abs() < 0.5);
        assert!(text("leviticus:8787").y < text("71%").y);
    }
}
