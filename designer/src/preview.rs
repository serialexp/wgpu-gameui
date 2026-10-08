//! The proof page: a spread of widgets drawn in the preset being edited, so
//! every seed and override shows the moment it moves.

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    Badge, BadgeTone, Banner, Button, Checkbox, DragCapture, DrawContext, Dropdown, DropdownId,
    DropdownState, FieldLabel, Ink, NumberInput, Panel, ProgressBar, RadioGroup, ScrollState,
    SearchField, Slider, StyleKey, StyleResolver, Tabs, TextInput, TextSize, Toggle, Tone,
    Tracking,
};

use crate::fields::{self, LABEL_GAP, SLIDER_HEIGHT};

/// Space around the page and between cards.
const PAD: f32 = 20.0;
/// Space between rows inside a card.
const GAP: f32 = 8.0;
/// A card column is at least this wide; narrower and the page is one column.
const MIN_COLUMN: f32 = 340.0;
/// A toggle, checkbox or radio row.
const CHOICE_H: f32 = 20.0;
/// The progress bar's height.
const PROGRESS_H: f32 = 10.0;
/// A key in the Keys card.
const KEY_W: f32 = 88.0;
/// The Tabs card's tab strip and body.
const TABS_H: f32 = 30.0;
const TAB_BODY_H: f32 = 56.0;

const DROPDOWN_ITEMS: &[&str] = &["Linear", "Ease in", "Ease out", "Spring"];
const RADIO_ITEMS: &[&str] = &["Low", "Medium", "High"];
const TAB_ITEMS: &[&str] = &["Scene", "Assets", "Settings"];
const TAB_BODIES: &[&str] = &[
    "Twelve objects, three lights and a camera.",
    "Textures, meshes and sounds the scene uses.",
    "Grid, snapping and units.",
];
const BADGES: &[(BadgeTone, &str)] = &[
    (BadgeTone::Draft, "Draft"),
    (BadgeTone::Baked, "Baked"),
    (BadgeTone::Stale, "Stale"),
    (BadgeTone::Error, "Error"),
    (BadgeTone::Live, "Live"),
];

/// Focus, drag and dropdown ids: one block, distinct from the other panels'.
const ID_KEYS: u64 = 0x9E71_0000;
const ID_TOGGLE: u64 = 0x9E71_0010;
const ID_RADIO: u64 = 0x9E71_0011;
const ID_TEXT: u64 = 0x9E71_0012;
const ID_SEARCH: u64 = 0x9E71_0013;
const ID_DROPDOWN: DropdownId = 0x9E71_0014;
const ID_NUMBER: u64 = 0x9E71_0015;
const ID_SLIDER: u64 = 0x9E71_0016;

/// The page's cards, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Card {
    Keys,
    Choices,
    Fields,
    Feedback,
    Tabs,
    Type,
    Ink,
}

impl Card {
    const ALL: [Card; 7] = [
        Card::Keys,
        Card::Choices,
        Card::Fields,
        Card::Feedback,
        Card::Tabs,
        Card::Type,
        Card::Ink,
    ];

    fn title(self) -> &'static str {
        match self {
            Card::Keys => "Keys",
            Card::Choices => "Choices",
            Card::Fields => "Fields",
            Card::Feedback => "Feedback",
            Card::Tabs => "Tabs",
            Card::Type => "Type scale",
            Card::Ink => "Ink ladder",
        }
    }
}

/// Caller-owned state of the proof page's widgets.
pub struct PreviewState {
    scroll: ScrollState,
    toggle: bool,
    checked: bool,
    radio: usize,
    text: TextInput,
    search: TextInput,
    dropdown: usize,
    number: TextInput,
    number_value: f64,
    slider: f32,
    tab: usize,
}

impl PreviewState {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
            toggle: true,
            checked: false,
            radio: 1,
            text: TextInput::new(0.0, 0.0, 0.0, 0.0).with_value("Aldermoor"),
            search: TextInput::new(0.0, 0.0, 0.0, 0.0).with_placeholder("Search"),
            dropdown: 2,
            number: TextInput::new(0.0, 0.0, 0.0, 0.0),
            number_value: 12.5,
            slider: 0.4,
            tab: 0,
        }
    }

    /// When the page next needs a frame on its own (a scroll glide).
    pub fn pending_deadline(&self) -> Option<f32> {
        self.scroll.pending_deadline()
    }

    /// An option picked in one of the page's dropdowns.
    pub fn dropdown_picked(&mut self, id: DropdownId, index: usize) {
        if id == ID_DROPDOWN {
            self.dropdown = index;
        }
    }

    /// Draw the page over `rect` in `ctx`'s theme, headed by the preset's
    /// `name`.
    pub fn draw(
        &mut self,
        rect: Rect,
        name: &str,
        dropdowns: &mut DropdownState,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) {
        let s = ctx.styles();
        ctx.draw_list.quad(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            s.color(StyleKey::Background),
        );
        ctx.draw_list.push_clip(rect);
        let mut scroll = std::mem::take(&mut self.scroll);
        fields::scrolled(rect, &mut scroll, ctx, |inner, _, ctx| {
            self.page(inner, name, dropdowns, capture, ctx)
        });
        self.scroll = scroll;
        ctx.draw_list.pop_clip();
    }

    /// The heading and the cards, in as many columns as fit; returns the
    /// page's height.
    fn page(
        &mut self,
        inner: Rect,
        name: &str,
        dropdowns: &mut DropdownState,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) -> f32 {
        let s = ctx.styles();
        let x = inner.x + PAD;
        let width = (inner.width - 2.0 * PAD).max(0.0);
        let mut y = inner.y + PAD;

        let caption = s.caption_block("Theme preview", x, y, Tracking::Section, Ink::Caption);
        y += ctx.draw_list.measure_block(&caption).1.ceil() + 4.0;
        ctx.draw_list.text(caption);
        let title = s
            .sans_block(name, x, y, TextSize::Menu, Ink::Title)
            .with_size((s.scalar(StyleKey::FontSizeTitle) * 1.5).round());
        y += ctx.draw_list.measure_block(&title).1.ceil() + PAD;
        ctx.draw_list.text(title);

        let columns = if width >= 2.0 * MIN_COLUMN + PAD {
            2
        } else {
            1
        };
        let column_w = ((width - PAD * (columns - 1) as f32) / columns as f32).floor();
        let mut bottoms = [y; 2];
        for card in Card::ALL {
            let column = if bottoms[1] < bottoms[0] && columns == 2 {
                1
            } else {
                0
            };
            let panel = Panel::new().title(card.title());
            let probe = Rect::new(0.0, 0.0, column_w, 1000.0);
            let chrome = 1000.0 - panel.content_rect(probe, ctx.draw_list, &s).height;
            let content_w = panel.content_rect(probe, ctx.draw_list, &s).width;
            let height = chrome + self.card_height(card, content_w, ctx);
            let rect = Rect::new(
                x + column as f32 * (column_w + PAD),
                bottoms[column],
                column_w,
                height,
            );
            let body = panel.draw(rect, ctx.draw_list, &s);
            self.card(card, body, dropdowns, capture, ctx);
            bottoms[column] = rect.bottom() + PAD;
        }
        bottoms[0].max(bottoms[1]) - inner.y
    }

    /// A card's content height at `width`.
    fn card_height(&self, card: Card, width: f32, ctx: &mut DrawContext) -> f32 {
        let s = ctx.styles();
        let button = s.scalar(StyleKey::ButtonHeight);
        let input = s.scalar(StyleKey::InputHeight);
        match card {
            Card::Keys => {
                let rows = KEY_COUNT.div_ceil(keys_per_row(width)) as f32;
                rows * button + (rows - 1.0) * GAP
            }
            Card::Choices => 3.0 * CHOICE_H + 2.0 * GAP,
            Card::Fields => {
                FieldLabel::height(ctx.draw_list, &s)
                    + LABEL_GAP
                    + input
                    + GAP
                    + SearchField::HEIGHT
                    + GAP
                    + input
                    + GAP
                    + input
                    + GAP
                    + SLIDER_HEIGHT
                    + GAP
                    + PROGRESS_H
            }
            Card::Feedback => {
                let banners: f32 = banners()
                    .iter()
                    .map(|banner| banner.measure_height(ctx.draw_list, &s, width) + GAP)
                    .sum();
                banners + Badge::new(BadgeTone::Live).height()
            }
            Card::Tabs => TABS_H + TAB_BODY_H,
            Card::Type => TextSize::ALL
                .iter()
                .map(|&step| type_row_height(step, &s))
                .sum::<f32>(),
            Card::Ink => Ink::ALL.len() as f32 * ink_row_height(&s),
        }
    }

    /// Draw a card's content into `body`.
    fn card(
        &mut self,
        card: Card,
        body: Rect,
        dropdowns: &mut DropdownState,
        capture: &mut DragCapture,
        ctx: &mut DrawContext,
    ) {
        let s = ctx.styles();
        let button = s.scalar(StyleKey::ButtonHeight);
        let input = s.scalar(StyleKey::InputHeight);
        let (x, mut y, width) = (body.x, body.y, body.width);
        match card {
            Card::Keys => {
                let keys: [Button; KEY_COUNT] = [
                    Button::new("Default"),
                    Button::new("Accent").tone(Tone::Accent),
                    Button::new("Danger").tone(Tone::Danger),
                    Button::new("Ghost").tone(Tone::Ghost),
                    Button::new("Disabled").enabled(false),
                    Button::new("Hollow").hollow(true),
                ];
                let per_row = keys_per_row(width);
                for (i, key) in keys.into_iter().enumerate() {
                    let (row, col) = (i / per_row, i % per_row);
                    let r = Rect::new(
                        x + col as f32 * (KEY_W + GAP),
                        y + row as f32 * (button + GAP),
                        KEY_W,
                        button,
                    );
                    key.focusable(ID_KEYS + i as u64).draw(r, ctx);
                }
            }
            Card::Choices => {
                let row = |y: f32| Rect::new(x, y, width, CHOICE_H);
                self.toggle = Toggle::new()
                    .label("Snap to grid")
                    .focusable(ID_TOGGLE)
                    .draw(self.toggle, row(y), ctx);
                y += CHOICE_H + GAP;
                if Checkbox::new().draw(self.checked, "Show guides", row(y), ctx) {
                    self.checked = !self.checked;
                }
                y += CHOICE_H + GAP;
                if let Some(picked) = RadioGroup::new(RADIO_ITEMS)
                    .horizontal()
                    .focusable(ID_RADIO)
                    .draw(self.radio, row(y), ctx)
                {
                    self.radio = picked;
                }
            }
            Card::Fields => {
                let label =
                    FieldLabel::new("Name")
                        .value("12 chars")
                        .draw(ctx.draw_list, &s, x, y, width);
                y = label.bottom() + LABEL_GAP;
                fields::draw_text(&mut self.text, ID_TEXT, Rect::new(x, y, width, input), ctx);
                y += input + GAP;
                let search = Rect::new(x, y, width, SearchField::HEIGHT);
                SearchField::new().draw(&mut self.search, ID_SEARCH, search, ctx);
                y += SearchField::HEIGHT + GAP;
                let half = ((width - GAP) * 0.5).floor();
                Dropdown::new(DROPDOWN_ITEMS, self.dropdown).draw(
                    ID_DROPDOWN,
                    Rect::new(x, y, half, input),
                    dropdowns,
                    ctx,
                );
                y += input + GAP;
                let out = NumberInput::new()
                    .with_range(0.0, 100.0)
                    .with_decimals(1)
                    .draw(
                        self.number_value,
                        ID_NUMBER,
                        &mut self.number,
                        Rect::new(x, y, half, input),
                        ctx,
                    );
                if out.changed {
                    self.number_value = out.value;
                }
                y += input + GAP;
                let slider = Slider::new(0.0, 1.0).focusable(ID_SLIDER).draw(
                    self.slider,
                    ID_SLIDER,
                    capture,
                    Rect::new(x, y, width, SLIDER_HEIGHT),
                    ctx,
                );
                self.slider = slider.value;
                y += SLIDER_HEIGHT + GAP;
                ProgressBar::new(self.slider).draw(
                    Rect::new(x, y, width, PROGRESS_H),
                    ctx.draw_list,
                    &s,
                );
            }
            Card::Feedback => {
                for banner in banners() {
                    let h = banner.measure_height(ctx.draw_list, &s, width);
                    banner.draw(Rect::new(x, y, width, h), ctx.draw_list, &s);
                    y += h + GAP;
                }
                let mut bx = x;
                for &(tone, text) in BADGES {
                    let r = Badge::new(tone).draw(ctx.draw_list, &s, bx, y, text);
                    bx = r.right() + GAP;
                }
            }
            Card::Tabs => {
                let out = Tabs::new(TAB_ITEMS).with_height(TABS_H).draw(
                    Rect::new(x, y, width, TABS_H + TAB_BODY_H),
                    self.tab,
                    ctx.draw_list,
                    &s,
                    ctx.input,
                    None,
                );
                if let Some(tab) = out.clicked {
                    self.tab = tab;
                }
                let text = s
                    .sans_block(
                        TAB_BODIES[self.tab],
                        x + GAP,
                        y + TABS_H + GAP,
                        TextSize::Row,
                        Ink::Row,
                    )
                    .with_max_width(width - 2.0 * GAP);
                ctx.draw_list.text(text);
            }
            Card::Type => {
                for step in TextSize::ALL {
                    let h = type_row_height(step, &s);
                    let label = s.mono_block(step.name(), x, y, TextSize::Caption, Ink::Dim);
                    ctx.draw_list.text(label);
                    let sample =
                        s.sans_block("Sphinx of black quartz", x + 130.0, y, step, Ink::Primary);
                    ctx.draw_list.text(sample);
                    y += h;
                }
            }
            Card::Ink => {
                let h = ink_row_height(&s);
                for role in Ink::ALL {
                    let block = s.sans_block(role.name(), x, y, TextSize::Row, role);
                    ctx.draw_list.text(block);
                    y += h;
                }
            }
        }
    }
}

/// How many keys the Keys card holds.
const KEY_COUNT: usize = 6;

/// How many keys fit across `width`: at least one.
fn keys_per_row(width: f32) -> usize {
    (((width + GAP) / (KEY_W + GAP)).floor() as usize).max(1)
}

/// The Feedback card's banners, one per severity.
fn banners() -> [Banner<'static>; 4] {
    [
        Banner::info("Autosave is on; changes are kept every minute."),
        Banner::success("Build finished in 4.2 s."),
        Banner::warning("Two textures exceed the memory budget."),
        Banner::error("The shader failed to compile."),
    ]
}

/// A Type card row: the step's own line height.
fn type_row_height(step: TextSize, s: &StyleResolver) -> f32 {
    (s.text_size(step) * 1.6).ceil()
}

/// An Ink card row.
fn ink_row_height(s: &StyleResolver) -> f32 {
    (s.text_size(TextSize::Row) * 1.6).ceil()
}
