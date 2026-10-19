//! The `CodeEditor` widget: a [`Document`] drawn as Forge's `CodeEditor`
//! draws one, in a sunken well with a line-number gutter down its left, the
//! caret line's band, the selection, and Tree-sitter colours, over a status
//! strip with the caret's line and column.
//!
//! Only the lines in view are measured, coloured and drawn. The gutter
//! stays put while long lines scroll sideways under it.

use std::ops::Range;

use wgpu_gameui::color::{hex, oklch};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    ClipboardGet, ClipboardSet, CursorIcon, DrawContext, DrawList, FocusId, FontHandle, Ink,
    InputState, Key, ScrollState, ScrollView, StyleKey, SyntaxTheme, TextAlign, TextBlock,
    TextSize, TextStyleRange, Underline, WrapMode,
};

use crate::columns::{Piece, byte_at_column, column_of, pieces};
use crate::commands::{self, Motion, Rules};
use crate::document::{Document, Selection};
use crate::highlight::{Span, Syntax};
use crate::language::Language;

/// Space above the first line and below the last.
const PAD_Y: f32 = 6.0;
/// Space after the longest line.
const PAD_RIGHT: f32 = 12.0;
/// The gutter: space before the numbers, their narrowest column, and the gap
/// after them (Forge's listing geometry: 12, 30, 10).
const GUTTER_PAD: f32 = 12.0;
const NUMBERS_MIN: f32 = 30.0;
const GUTTER_GAP: f32 = 10.0;
/// A line is this many times the text size tall: 18 for 12.
const LINE_RATIO: f32 = 1.5;
const CARET_WIDTH: f32 = 1.5;
const STATUS_HEIGHT: f32 = 20.0;
const STATUS_PAD: f32 = 9.0;
const STATUS_GAP: f32 = 12.0;
/// Columns drawn beyond each side of what can be seen.
const MARGIN: usize = 16;
/// How fast a drag held past an edge scrolls: this many times the distance
/// past it, a second.
const DRAG_SCROLL_RATE: f32 = 20.0;

/// The editor's own colours, beside the syntax inks: Forge's `--code-*`
/// tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CodeTheme {
    pub syntax: SyntaxTheme,
    /// The well, opaque so the gutter can scroll over code.
    pub background: [f32; 4],
    pub number: [f32; 4],
    /// The caret line's number.
    pub number_current: [f32; 4],
    /// The caret line's band.
    pub line_current: [f32; 4],
    pub caret: [f32; 4],
    pub selection: [f32; 4],
}

impl Default for CodeTheme {
    fn default() -> Self {
        // Forge's accent hue.
        let accent = 200.0;
        Self {
            syntax: SyntaxTheme::default(),
            background: hex(0x080a0c),
            number: hex(0x4f575e),
            number_current: hex(0xbdc5cc),
            line_current: [1.0, 1.0, 1.0, 0.035],
            caret: oklch(0.8, 0.1, accent, 1.0),
            selection: oklch(0.74, 0.11, accent, 0.3),
        }
    }
}

/// What a frame of the editor did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeEditorOutput {
    /// The text changed.
    pub changed: bool,
}

/// What a drag after a press grows by: characters, or the words or lines a
/// double or triple click selected.
#[derive(Clone, Debug)]
enum Drag {
    Characters(usize),
    Words(Range<usize>),
    Lines(Range<usize>),
}

/// The colours of the lines last drawn, kept while neither the text, the
/// lines in view nor the syntax inks change.
#[derive(Clone, Debug)]
struct Colours {
    version: u64,
    range: Range<usize>,
    theme: SyntaxTheme,
    spans: Vec<Span>,
}

/// Sizes for this frame, from the theme's text size and the mono font.
#[derive(Clone, Copy, Debug)]
struct Metrics {
    size: f32,
    line_height: f32,
    /// The mono font's advance: one column.
    advance: f32,
    gutter: f32,
    numbers: f32,
}

/// What was drawn where, for tests.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct Drawn {
    pub(crate) caret: Option<Rect>,
    pub(crate) selection: Vec<Rect>,
    pub(crate) band: Option<Rect>,
    pub(crate) lines: Range<usize>,
    pub(crate) advance: f32,
    pub(crate) line_height: f32,
    /// Where the first line's text starts, before scrolling.
    pub(crate) origin: [f32; 2],
}

/// A source code editor. See the [module docs](self).
pub struct CodeEditor {
    document: Document,
    syntax: Syntax,
    rules: Rules,
    pub theme: CodeTheme,
    read_only: bool,
    status: bool,
    frame: bool,
    scroll: ScrollState,
    /// The column Up and Down keep to, through a run of them.
    column: Option<usize>,
    drag: Option<Drag>,
    /// Bring the caret into view at the next draw.
    reveal: bool,
    /// Lines to scroll by at the next draw, for Page Up and Down.
    paged: isize,
    /// The widest line drawn since the text last changed, for how far it
    /// scrolls sideways: scrolling never narrows it, an edit may.
    widest: f32,
    /// The text's version `widest` was measured for.
    widest_version: u64,
    colours: Option<Colours>,
    clipboard_get: Option<ClipboardGet>,
    clipboard_set: Option<ClipboardSet>,
    #[cfg(test)]
    pub(crate) drawn: Drawn,
}

impl std::fmt::Debug for CodeEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeEditor")
            .field("language", &self.syntax.language())
            .field("lines", &self.document.line_count())
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl CodeEditor {
    /// An editor holding `text`, coloured as `language`.
    pub fn new(text: &str, language: Language) -> Self {
        let mut document = Document::new(text);
        let syntax = Syntax::new(language, &mut document);
        Self {
            document,
            syntax,
            rules: language.rules(4),
            theme: CodeTheme::default(),
            read_only: false,
            status: true,
            frame: true,
            scroll: ScrollState::default(),
            column: None,
            drag: None,
            reveal: false,
            paged: 0,
            widest: 0.0,
            widest_version: 0,
            colours: None,
            clipboard_get: None,
            clipboard_set: None,
            #[cfg(test)]
            drawn: Drawn::default(),
        }
    }

    /// Shown and selectable, but not editable.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// With or without the status strip (on by default): off when the host
    /// has a status bar of its own.
    pub fn with_status(mut self, status: bool) -> Self {
        self.status = status;
        self
    }

    /// In its rounded, bordered well (the default), or filling its rect
    /// edge to edge, for a pane that is all editor.
    pub fn with_frame(mut self, frame: bool) -> Self {
        self.frame = frame;
        self
    }

    /// Tab stops every `tab_size` columns, and indents that many spaces.
    pub fn with_tab_size(mut self, tab_size: usize) -> Self {
        self.rules.tab_size = tab_size.max(1);
        self
    }

    /// Whether brackets and quotes close themselves (on by default).
    pub fn with_auto_close(mut self, auto_close: bool) -> Self {
        self.rules.auto_close = auto_close;
        self
    }

    /// Where Copy and Cut put text, and where Paste takes it from.
    pub fn set_clipboard(&mut self, get: ClipboardGet, set: ClipboardSet) {
        self.clipboard_get = Some(get);
        self.clipboard_set = Some(set);
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn language(&self) -> Language {
        self.syntax.language()
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The text.
    pub fn text(&self) -> String {
        self.document.text()
    }

    /// Draw it in `rect`, taking the keys while `id` has focus.
    pub fn draw(&mut self, id: FocusId, rect: Rect, cx: &mut DrawContext) -> CodeEditorOutput {
        cx.push_debug_scope_rect("CodeEditor", rect);
        cx.register_focus(id);
        let version = self.document.version();
        let font = cx.theme.mono_font.clone();
        let metrics = self.metrics(cx.draw_list, cx.styles().scalar(StyleKey::FontSize), &font);
        let status = if self.status { STATUS_HEIGHT } else { 0.0 };
        let body = Rect::new(rect.x, rect.y, rect.width, (rect.height - status).max(0.0));

        if cx.input.mouse_clicked && cx.input.is_hovered(body.x, body.y, body.width, body.height) {
            cx.focus.request(id);
        }
        let focused = cx.focus.is_focused(id);
        if focused {
            let page = ((body.height - 2.0 * PAD_Y) / metrics.line_height)
                .floor()
                .max(2.0) as isize
                - 1;
            if self.keys(cx.input, page) {
                cx.focus.claim_tab();
            }
        }
        self.syntax.update(&mut self.document);

        self.draw_well(rect, focused, cx);
        self.draw_body(body, &metrics, &font, focused, cx);
        if self.status {
            let strip = Rect::new(rect.x, body.bottom(), rect.width, status);
            self.draw_status(strip, cx);
        }
        cx.pop_debug_scope();
        CodeEditorOutput {
            changed: self.document.version() != version,
        }
    }

    fn metrics(&self, list: &mut DrawList, size: f32, font: &Option<FontHandle>) -> Metrics {
        const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
        let advance = list
            .measure_text_with_font(ZEROS, size, None, font.as_ref())
            .0
            / ZEROS.len() as f32;
        let digits = self.document.line_count().to_string().len() as f32;
        let numbers = (digits * advance).ceil().max(NUMBERS_MIN);
        Metrics {
            size,
            line_height: (size * LINE_RATIO).round(),
            advance,
            gutter: GUTTER_PAD + numbers + GUTTER_GAP,
            numbers,
        }
    }

    /// Act on this frame's keys. True when it took Tab.
    fn keys(&mut self, input: &InputState, page: isize) -> bool {
        let version = self.document.version();
        let selection = self.document.selection();
        let editable = !self.read_only;
        let shift = input.shift_pressed;
        // Cmd on macOS: Cmd-arrows go to the line's and the file's ends,
        // and Option-arrows step by words. Elsewhere Ctrl steps by words.
        let mac = cfg!(target_os = "macos");
        let command = mac && input.ctrl_pressed;
        let word = if mac {
            input.alt_down
        } else {
            input.ctrl_pressed
        };
        let letter = |key: char| input.ctrl_pressed && input.keys.was_pressed(Key::Char(key));
        let mut took_tab = false;
        let mut vertical = false;

        if input.key_select_all {
            commands::select_all(&mut self.document);
        } else if input.key_copy {
            self.copy();
        } else if input.key_cut {
            // Only what the clipboard took is taken out.
            if self.copy() && editable {
                commands::cut(&mut self.document);
            }
        } else if input.key_paste {
            if editable && let Some(get) = &self.clipboard_get {
                let text = (get.borrow_mut())();
                commands::paste(&mut self.document, &text);
            }
        } else if letter('z') {
            if editable {
                if shift {
                    self.document.redo();
                } else {
                    self.document.undo();
                }
            }
        } else if letter('y') && !mac {
            if editable {
                self.document.redo();
            }
        } else if input.key_left || input.key_right {
            let motion = match (input.key_left, command, word) {
                (true, true, _) => Motion::Home,
                (false, true, _) => Motion::End,
                (true, false, true) => Motion::WordLeft,
                (false, false, true) => Motion::WordRight,
                (true, false, false) => Motion::Left,
                (false, false, false) => Motion::Right,
            };
            commands::move_caret(&mut self.document, motion, shift);
        } else if input.key_up || input.key_down {
            if command {
                let motion = if input.key_up {
                    Motion::DocumentStart
                } else {
                    Motion::DocumentEnd
                };
                commands::move_caret(&mut self.document, motion, shift);
            } else {
                let lines = if input.key_up { -1 } else { 1 };
                self.move_lines(lines, shift);
                vertical = true;
            }
        } else if input.keys.was_pressed(Key::PageUp) || input.keys.was_pressed(Key::PageDown) {
            let lines = if input.keys.was_pressed(Key::PageUp) {
                -page
            } else {
                page
            };
            self.move_lines(lines, shift);
            self.paged = lines;
            vertical = true;
        } else if input.key_home || input.key_end {
            let motion = match (input.key_home, input.ctrl_pressed && !mac) {
                (true, true) => Motion::DocumentStart,
                (false, true) => Motion::DocumentEnd,
                (true, false) => Motion::Home,
                (false, false) => Motion::End,
            };
            commands::move_caret(&mut self.document, motion, shift);
        } else if input.key_tab && editable {
            if shift {
                commands::outdent(&mut self.document, &self.rules);
            } else {
                commands::indent(&mut self.document, &self.rules);
            }
            took_tab = true;
        } else if input.enter_pressed && editable {
            commands::newline(&mut self.document, &self.rules);
        } else if input.backspace_pressed && editable {
            if command {
                commands::delete_to(&mut self.document, Motion::LineStart);
            } else if word {
                commands::delete_to(&mut self.document, Motion::WordLeft);
            } else {
                commands::backspace(&mut self.document, &self.rules);
            }
        } else if input.key_delete && editable {
            if command {
                commands::delete_to(&mut self.document, Motion::End);
            } else if word {
                commands::delete_to(&mut self.document, Motion::WordRight);
            } else {
                commands::delete_forward(&mut self.document);
            }
        } else if !input.text_input.is_empty() && !input.ctrl_pressed && editable {
            for typed in input.text_input.chars() {
                let mut buffer = [0; 4];
                commands::type_text(
                    &mut self.document,
                    typed.encode_utf8(&mut buffer),
                    &self.rules,
                );
            }
        }

        if !vertical {
            // Any other key ends a run of Up and Down.
            if self.document.selection() != selection || self.document.version() != version {
                self.column = None;
            }
        }
        if self.document.selection() != selection || self.document.version() != version {
            self.reveal = true;
        }
        took_tab
    }

    fn move_lines(&mut self, lines: isize, extend: bool) {
        commands::move_lines(
            &mut self.document,
            lines,
            extend,
            &mut self.column,
            self.rules.tab_size,
        );
    }

    /// Put the selection on the clipboard; whether there was one to take it.
    fn copy(&mut self) -> bool {
        let (Some(text), Some(set)) = (commands::copied(&self.document), &self.clipboard_set)
        else {
            return false;
        };
        (set.borrow_mut())(text);
        true
    }

    /// The sunken well, its border the accent's while focused; without a
    /// frame, only its colour.
    fn draw_well(&self, rect: Rect, focused: bool, cx: &mut DrawContext) {
        if !self.frame {
            cx.draw_list.quad(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                self.theme.background,
            );
            return;
        }
        let s = cx.styles();
        let radius = s.scalar(StyleKey::BorderRadius);
        cx.draw_list
            .rounded_rect(rect, radius, self.theme.background);
        let border = if focused {
            s.color(StyleKey::InputFocusBorder)
        } else {
            s.color(StyleKey::InputBorder)
        };
        cx.draw_list.rounded_rect_outline(rect, radius, 1.0, border);
    }

    fn draw_body(
        &mut self,
        body: Rect,
        metrics: &Metrics,
        font: &Option<FontHandle>,
        focused: bool,
        cx: &mut DrawContext,
    ) {
        let line_count = self.document.line_count();
        let tab_size = self.rules.tab_size;
        if self.widest_version != self.document.version() {
            // The text changed: as wide as what is in view now.
            self.widest = 0.0;
            self.widest_version = self.document.version();
        }
        self.scroll.content_size = [
            metrics.gutter + self.widest + PAD_RIGHT,
            line_count as f32 * metrics.line_height + 2.0 * PAD_Y,
        ];
        let paged = std::mem::take(&mut self.paged);
        if paged != 0 {
            self.scroll.scroll_by(1, paged as f32 * metrics.line_height);
        }
        if std::mem::take(&mut self.reveal) {
            self.reveal_caret(body, metrics);
        }
        let view = ScrollView::new(body).overlay();
        let mut input = cx.input.clone();
        let begun = view.begin(&mut self.scroll, cx.draw_list, &mut input);
        let inner = begun.inner;
        let offset = self.scroll.offset;

        if inner.contains(input.mouse_x, input.mouse_y)
            && !input.mouse_consumed
            && input.mouse_x >= inner.x + metrics.gutter
        {
            cx.request_cursor(CursorIcon::Text);
        }
        self.pointer(&input, inner, metrics, cx.draw_list);

        // The lines in view.
        let top = offset[1] - PAD_Y;
        let first = ((top / metrics.line_height).floor().max(0.0) as usize).min(line_count - 1);
        // The last line starting above the view's bottom edge.
        let last = (((top + inner.height) / metrics.line_height).ceil().max(1.0) as usize - 1)
            .clamp(first, line_count - 1);
        let lines = first..last + 1;
        let spans = self.colours(lines.clone());
        let selection = self.document.selection();
        let caret_line = self.document.line_of(selection.head);
        let banded = focused && selection.is_empty();
        let text_x = inner.x + metrics.gutter;
        let column_x = |column: usize| text_x + column as f32 * metrics.advance;
        let line_top = |line: usize| inner.y + PAD_Y + line as f32 * metrics.line_height;
        // The columns in view, and a few either side.
        let first_column = (offset[0] / metrics.advance).floor().max(0.0) as usize;
        let visible = ((inner.width - metrics.gutter) / metrics.advance)
            .ceil()
            .max(0.0) as usize;
        let columns = first_column.saturating_sub(MARGIN)..first_column + visible + MARGIN;
        // Sticky to the left edge: the content is drawn shifted by the
        // scroll offset, so this is the view's left edge.
        let left = inner.x + offset[0];
        #[cfg(test)]
        {
            self.drawn = Drawn {
                lines: lines.clone(),
                advance: metrics.advance,
                line_height: metrics.line_height,
                origin: [text_x, line_top(0)],
                ..Drawn::default()
            };
        }

        if banded && lines.contains(&caret_line) {
            let band = Rect::new(left, line_top(caret_line), inner.width, metrics.line_height);
            cx.draw_list.quad(
                band.x,
                band.y,
                band.width,
                band.height,
                self.theme.line_current,
            );
            #[cfg(test)]
            {
                self.drawn.band = Some(band);
            }
        }

        let range = selection.range();
        let mut span_index = 0;
        for line in lines.clone() {
            let start = self.document.line_start(line);
            let end = self.document.line_end(line);
            let text = self.document.line(line);
            let y = line_top(line);

            // The selection on this line.
            if !range.is_empty() && range.start <= end && range.end >= start {
                let x0 = column_x(column_of(&text, range.start.max(start) - start, tab_size));
                let mut x1 = column_x(column_of(&text, range.end.min(end) - start, tab_size));
                if range.end > end {
                    // Past the line's end: a little of its line ending too.
                    x1 += metrics.advance * 0.5;
                }
                if x1 > x0 {
                    let rect = Rect::new(x0, y, x1 - x0, metrics.line_height);
                    cx.draw_list.quad(
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        self.theme.selection,
                    );
                    #[cfg(test)]
                    self.drawn.selection.push(rect);
                }
            }

            // The text, coloured.
            while span_index < spans.len() && spans[span_index].range.end <= start {
                span_index += 1;
            }
            let mut inks = Vec::new();
            let mut index = span_index;
            while index < spans.len() && spans[index].range.start < end {
                let span = &spans[index];
                let range = span.range.start.max(start) - start..span.range.end.min(end) - start;
                if !range.is_empty() {
                    inks.push((range, span.ink));
                }
                index += 1;
            }
            let (pieces, width) = pieces(&text, tab_size, columns.clone());
            for piece in pieces {
                let x = column_x(piece.column);
                self.draw_piece(piece, &inks, x, y, metrics, font, cx);
            }
            self.widest = self.widest.max(width as f32 * metrics.advance);
        }
        // As wide as the widest line seen, this frame's included.
        self.scroll.content_size[0] = metrics.gutter + self.widest + PAD_RIGHT;

        // The caret.
        if focused && lines.contains(&caret_line) {
            let start = self.document.line_start(caret_line);
            let text = self.document.line(caret_line);
            let x = column_x(column_of(&text, selection.head - start, tab_size));
            let y = line_top(caret_line);
            cx.draw_list
                .quad(x, y, CARET_WIDTH, metrics.line_height, self.theme.caret);
            if !self.read_only {
                cx.focus.request_ime(Rect::new(
                    x - offset[0],
                    y - offset[1],
                    CARET_WIDTH,
                    metrics.line_height,
                ));
            }
            #[cfg(test)]
            {
                self.drawn.caret = Some(Rect::new(x, y, CARET_WIDTH, metrics.line_height));
            }
        }

        // The gutter, over any code scrolled under it.
        let gutter_top = inner.y + offset[1];
        cx.draw_list.quad(
            left,
            gutter_top,
            metrics.gutter,
            inner.height,
            self.theme.background,
        );
        for line in lines {
            let y = line_top(line);
            let current = banded && line == caret_line;
            if current {
                cx.draw_list.quad(
                    left,
                    y,
                    metrics.gutter,
                    metrics.line_height,
                    self.theme.line_current,
                );
            }
            let ink = if current {
                self.theme.number_current
            } else {
                self.theme.number
            };
            cx.draw_list.text(
                TextBlock::new((line + 1).to_string(), left + GUTTER_PAD, y)
                    .with_size(metrics.size)
                    .with_line_height(metrics.line_height)
                    .with_font_opt(font.clone())
                    .with_color_f32(ink)
                    .with_max_width(metrics.numbers)
                    .with_align(TextAlign::Right),
            );
        }
        view.end(&mut self.scroll, cx.draw_list, &cx.styles(), &input, begun);
    }

    /// Draw `piece` at `x`, `y`, coloured by `inks`: ranges of its line.
    /// Where the piece is the line's own bytes, each ink covers its own part
    /// of it; a picture takes the ink of what it stands for.
    #[allow(clippy::too_many_arguments)]
    fn draw_piece(
        &self,
        piece: Piece,
        inks: &[(Range<usize>, [f32; 4])],
        x: f32,
        y: f32,
        metrics: &Metrics,
        font: &Option<FontHandle>,
        cx: &mut DrawContext,
    ) {
        let source = piece.source;
        let own = piece.text.len() == source.len();
        let ranges = inks
            .iter()
            .filter_map(|(range, ink)| {
                let start = range.start.max(source.start);
                let end = range.end.min(source.end);
                (start < end).then(|| TextStyleRange {
                    range: if own {
                        start - source.start..end - source.start
                    } else {
                        0..piece.text.len()
                    },
                    color: Some(*ink),
                    underline: Underline::None,
                })
            })
            .collect();
        cx.draw_list.text(
            TextBlock::new(piece.text, x, y)
                .with_size(metrics.size)
                .with_line_height(metrics.line_height)
                .with_font_opt(font.clone())
                .with_color_f32(self.theme.syntax.variable)
                .with_style_ranges(ranges)
                .with_wrap(WrapMode::None),
        );
    }

    /// The colours of `lines`, worked out again only when the text, the
    /// lines in view or the syntax inks changed.
    fn colours(&mut self, lines: Range<usize>) -> Vec<Span> {
        let range = self.document.line_start(lines.start)..self.document.line_end(lines.end - 1);
        let version = self.document.version();
        match &self.colours {
            Some(colours)
                if colours.version == version
                    && colours.range == range
                    && colours.theme == self.theme.syntax =>
            {
                colours.spans.clone()
            }
            _ => {
                let spans = self
                    .syntax
                    .spans(&self.document, range.clone(), &self.theme.syntax);
                self.colours = Some(Colours {
                    version,
                    range,
                    theme: self.theme.syntax,
                    spans: spans.clone(),
                });
                spans
            }
        }
    }

    /// Scroll so the caret can be seen.
    fn reveal_caret(&mut self, body: Rect, metrics: &Metrics) {
        let head = self.document.selection().head;
        let line = self.document.line_of(head);
        let top = PAD_Y + line as f32 * metrics.line_height;
        self.scroll.scroll_range_into_view(
            1,
            top - PAD_Y,
            top + metrics.line_height + PAD_Y,
            body.height,
        );
        let text = self.document.line(line);
        let start = self.document.line_start(line);
        let x = column_of(&text, head - start, self.rules.tab_size) as f32 * metrics.advance;
        self.widest = self.widest.max(x + CARET_WIDTH);
        self.scroll.content_size[0] = metrics.gutter + self.widest + PAD_RIGHT;
        self.scroll.scroll_range_into_view(
            0,
            x - metrics.advance,
            x + CARET_WIDTH + metrics.advance,
            body.width - metrics.gutter,
        );
    }

    /// Place the caret with a press, select words or lines with a double or
    /// triple one, and grow the selection while the button is held.
    fn pointer(&mut self, input: &InputState, inner: Rect, metrics: &Metrics, list: &mut DrawList) {
        let over = inner.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        if input.mouse_clicked && !over {
            // A press elsewhere ends any drag of ours.
            self.drag = None;
            return;
        }
        if input.mouse_clicked {
            let at = self.position_at(input.mouse_x, input.mouse_y, inner, metrics);
            let anchor = self.document.selection().anchor;
            let (selection, drag) = match input.mouse_click_count {
                2 => {
                    let word = commands::word_at(&self.document, at);
                    (
                        Selection {
                            anchor: word.start,
                            head: word.end,
                        },
                        Drag::Words(word),
                    )
                }
                count if count >= 3 => {
                    let line = commands::line_at(&self.document, at);
                    (
                        Selection {
                            anchor: line.start,
                            head: line.end,
                        },
                        Drag::Lines(line),
                    )
                }
                _ if input.shift_pressed => {
                    (Selection { anchor, head: at }, Drag::Characters(anchor))
                }
                _ => (Selection::caret(at), Drag::Characters(at)),
            };
            self.document.set_selection(selection);
            self.drag = Some(drag);
            self.column = None;
            // A line half out of view comes into it, at the next frame.
            self.reveal = true;
            list.repaint_after(0.0);
            return;
        }
        if !input.mouse_down {
            self.drag = None;
            return;
        }
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let at = self.position_at(input.mouse_x, input.mouse_y, inner, metrics);
        let grown = |unit: Range<usize>, around: Range<usize>| {
            if unit.start < around.start {
                Selection {
                    anchor: around.end,
                    head: unit.start,
                }
            } else {
                Selection {
                    anchor: around.start,
                    head: unit.end.max(around.end),
                }
            }
        };
        let selection = match drag {
            Drag::Characters(anchor) => Selection { anchor, head: at },
            Drag::Words(word) => grown(commands::word_at(&self.document, at), word),
            Drag::Lines(line) => grown(commands::line_at(&self.document, at), line),
        };
        self.document.set_selection(selection);
        // Held past an edge of the code: scroll towards it, by how far past
        // and for how long.
        let beyond = |at: f32, start: f32, end: f32| {
            if at < start {
                at - start
            } else if at > end {
                at - end
            } else {
                0.0
            }
        };
        let across = beyond(input.mouse_x, inner.x + metrics.gutter, inner.right());
        let down = beyond(input.mouse_y, inner.y, inner.bottom());
        if across != 0.0 || down != 0.0 {
            let dt = input.frame_dt.clamp(0.0, wgpu_gameui::MAX_DT);
            self.scroll.scroll_by(0, across * DRAG_SCROLL_RATE * dt);
            self.scroll.scroll_by(1, down * DRAG_SCROLL_RATE * dt);
            list.repaint_after(0.0);
        }
    }

    /// The caret position nearest the point `x`, `y` on the screen.
    fn position_at(&self, x: f32, y: f32, inner: Rect, metrics: &Metrics) -> usize {
        let offset = self.scroll.offset;
        let into = y - inner.y + offset[1] - PAD_Y;
        let line = ((into / metrics.line_height).floor().max(0.0) as usize)
            .min(self.document.line_count() - 1);
        let across = (x - inner.x + offset[0] - metrics.gutter).max(0.0);
        let text = self.document.line(line);
        self.document.line_start(line)
            + byte_at_column(&text, across / metrics.advance, self.rules.tab_size)
    }

    /// The strip under the code: Ln and Col, the indent, the language.
    fn draw_status(&self, strip: Rect, cx: &mut DrawContext) {
        let s = cx.styles();
        cx.draw_list.quad(
            strip.x,
            strip.y,
            strip.width,
            strip.height,
            s.color(StyleKey::Panel),
        );
        cx.draw_list.quad(
            strip.x,
            strip.y,
            strip.width,
            1.0,
            s.color(StyleKey::EdgeHard),
        );
        let selection = self.document.selection();
        let line = self.document.line_of(selection.head);
        let text = self.document.line(line);
        let column = column_of(
            &text,
            selection.head - self.document.line_start(line),
            self.rules.tab_size,
        );
        let mut position = format!("Ln {}, Col {}", line + 1, column + 1);
        if !selection.is_empty() {
            let selected = self.document.chars_in(selection.range());
            position.push_str(&format!(" ({selected} selected)"));
        }
        let parts = [
            (position, Ink::Muted),
            (format!("Spaces: {}", self.rules.tab_size), Ink::Muted),
            (self.language().label().to_owned(), Ink::Second),
        ];
        let mut right = strip.right() - STATUS_PAD;
        for (text, ink) in parts.into_iter().rev() {
            let block = s.mono_block(text, 0.0, 0.0, TextSize::Meta, ink);
            let (width, height) = cx.draw_list.measure_block(&block);
            right -= width;
            let y = strip.y + 1.0 + ((strip.height - 1.0 - height) / 2.0).round();
            cx.draw_list.text(TextBlock {
                x: right,
                y,
                ..block
            });
            right -= STATUS_GAP;
        }
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
