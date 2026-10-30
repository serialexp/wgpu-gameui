//! The `CodeEditor` widget: a [`Document`] drawn as Forge's `CodeEditor`
//! draws one, in a sunken well with a line-number gutter down its left, the
//! caret line's band, the selection, and Tree-sitter colours, over a status
//! strip with the caret's line and column.
//!
//! Only the lines in view are measured, coloured and drawn. The gutter
//! stays put while long lines scroll sideways under it.
//!
//! Problems the host reports ([`CodeEditor::set_diagnostics`]) are drawn
//! as Forge draws them: a wavy underline in the severity's ink, a wash
//! across the worst one's line, and a dot in its gutter. The message of the
//! one under the pointer is drawn above everything by
//! [`CodeEditor::draw_layers`], and the status strip counts them.
//!
//! Typing a word, or Ctrl-Space, opens a completion list under it: the
//! host's items ([`CodeEditor::set_completions`]), the language's keywords
//! and the document's own words. It is drawn by `draw_layers` too.

use std::ops::Range;

use wgpu_gameui::color::{hex, oklch};
use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    ClipboardGet, ClipboardSet, CursorIcon, DrawContext, DrawList, FocusId, FontHandle, Icon, Ink,
    InputState, Key, LayerStack, PhosphorIcon, ScrollState, ScrollView, Stroke, StyleKey,
    StyleResolver, SyntaxTheme, TextAlign, TextBlock, TextSize, TextStyleRange, Underline,
    WrapMode,
};

use crate::columns::{Piece, byte_at_column, column_of, pieces};
use crate::commands::{self, Motion, Rules};
use crate::completion::{self, Completing, Completion};
use crate::diagnostics::{Diagnostic, Diagnostics, Severity};
use crate::document::{Document, Selection};
use crate::highlight::{Span, Syntax};
use crate::language::Language;
use crate::marks;
use crate::popup::{self, Popup};

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
/// The wavy underline: its stroke, and how far its middle is above the
/// line's bottom.
const WAVE_WIDTH: f32 = 1.0;
const WAVE_RISE: f32 = 2.5;
/// The gutter's problem dot: its centre from the gutter's left, its
/// radius, and its glow's.
const DOT_X: f32 = 7.5;
const DOT_RADIUS: f32 = 2.5;
const GLOW_RADIUS: f32 = 4.5;
const GLOW_ALPHA: f32 = 0.3;
/// The status strip's problem counts: the glyph, and the gap after it.
const COUNT_ICON: f32 = 12.0;
const COUNT_GAP: f32 = 4.0;

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
    /// The inks of an error, a warning and a note: their underline, dot and
    /// line number (Forge `--danger-text`, `--warn-meta`, `--syn-function`).
    pub error: [f32; 4],
    pub warning: [f32; 4],
    pub info: [f32; 4],
    /// The wash across a line with an error or a warning (`--log-error-row`,
    /// `--log-warn-row`). A note's line has none.
    pub error_row: [f32; 4],
    pub warning_row: [f32; 4],
}

impl CodeTheme {
    /// The ink of `severity`.
    pub fn ink(&self, severity: Severity) -> [f32; 4] {
        match severity {
            Severity::Error => self.error,
            Severity::Warning => self.warning,
            Severity::Info => self.info,
        }
    }

    /// The wash across a line whose worst problem is `severity`, if any.
    pub fn row(&self, severity: Severity) -> Option<[f32; 4]> {
        match severity {
            Severity::Error => Some(self.error_row),
            Severity::Warning => Some(self.warning_row),
            Severity::Info => None,
        }
    }
}

impl Default for CodeTheme {
    fn default() -> Self {
        // Forge's accent hue.
        let accent = 200.0;
        let (danger, warn) = (25.0, 75.0);
        let syntax = SyntaxTheme::default();
        Self {
            syntax,
            background: hex(0x080a0c),
            number: hex(0x4f575e),
            number_current: hex(0xbdc5cc),
            line_current: [1.0, 1.0, 1.0, 0.035],
            caret: oklch(0.8, 0.1, accent, 1.0),
            selection: oklch(0.74, 0.11, accent, 0.3),
            error: oklch(0.72, 0.15, danger, 1.0),
            warning: oklch(0.82, 0.13, warn, 1.0),
            info: syntax.function,
            error_row: oklch(0.62, 0.18, danger, 0.09),
            warning_row: oklch(0.78, 0.14, warn, 0.055),
        }
    }
}

/// What a frame of the editor did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeEditorOutput {
    /// The text changed.
    pub changed: bool,
    /// Ctrl-Space asked for completions: a host with a language server asks
    /// it now, and hands what it says to
    /// [`set_completions`](CodeEditor::set_completions).
    pub completion_asked: bool,
}

/// What the keys of a frame did besides editing, for focus and the host.
#[derive(Clone, Copy, Debug, Default)]
struct Taken {
    /// Tab was typed, not used to move focus.
    tab: bool,
    /// Escape closed the completion list, so it mustn't blur the editor too.
    escape: bool,
    /// Ctrl-Space asked for completions.
    asked: bool,
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

/// The problem under the pointer at the last draw, and the screen rect of
/// what it is shown for: its underline, or its line's gutter.
#[derive(Clone, Debug, PartialEq)]
struct Hover {
    diagnostic: Diagnostic,
    anchor: Rect,
}

/// What was drawn where, for tests.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct Drawn {
    pub(crate) caret: Option<Rect>,
    pub(crate) selection: Vec<Rect>,
    pub(crate) band: Option<Rect>,
    /// Each underline: the line, its span across, and its severity.
    pub(crate) waves: Vec<(usize, Range<f32>, Severity)>,
    /// Each line washed for a problem, and the severity.
    pub(crate) washes: Vec<(usize, Severity)>,
    /// Each line with a dot in its gutter.
    pub(crate) dots: Vec<usize>,
    /// Where the status strip's error and warning counts were.
    pub(crate) counts: Vec<(Severity, Rect)>,
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
    diagnostics: Diagnostics,
    hover: Option<Hover>,
    /// The host's completions, offered before keywords and words.
    completions: Vec<Completion>,
    pub(crate) completing: Option<Completing>,
    /// Where the open list was drawn, for the clicks on it.
    pub(crate) popup: Option<Popup>,
    /// Where on the screen the word being completed starts, at its line.
    word: Option<Rect>,
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
            diagnostics: Diagnostics::default(),
            hover: None,
            completions: Vec::new(),
            completing: None,
            popup: None,
            word: None,
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
        if read_only {
            self.completing = None;
        }
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

    /// Show `diagnostics`, their ranges in the text as it is now, in place
    /// of any shown before. Until the next call they follow the edits made.
    pub fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.catch_up();
        self.diagnostics.set(&self.document, diagnostics);
        self.hover = None;
    }

    /// The problems shown, where they are now.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Offer `completions` before the language's keywords and the
    /// document's words, from the next list opened (and in one open now).
    pub fn set_completions(&mut self, completions: Vec<Completion>) {
        self.completions = completions;
        if let Some(open) = &self.completing {
            let (from, prefix) = (open.from, open.prefix.clone());
            self.completing = Completing::open(from, prefix, self.pool(from));
        }
    }

    /// Whether a completion list is open.
    pub fn is_completing(&self) -> bool {
        self.completing.is_some()
    }

    /// Draw, above everything, the open completion list and the message of
    /// the problem under the pointer at the last [`draw`](Self::draw): call
    /// it after the frame's base is drawn. `viewport` is the screen they are
    /// kept inside.
    pub fn draw_layers(&self, layers: &mut LayerStack, styles: &StyleResolver, viewport: Rect) {
        if let (Some(open), Some(popup)) = (&self.completing, &self.popup) {
            popup::paint(layers, styles, &self.theme.syntax, open, popup, viewport);
        }
        if let Some(hover) = &self.hover {
            let ink = self.theme.ink(hover.diagnostic.severity);
            marks::draw_message(
                layers,
                styles,
                &hover.diagnostic,
                ink,
                hover.anchor,
                viewport,
            );
        }
    }

    /// Everything completion offers for the word from `from`: the host's
    /// items, the keywords, and the document's other words (read through
    /// once, as a list opens).
    fn pool(&self, from: usize) -> Vec<Completion> {
        let words = completion::identifiers(&self.document, Some(from));
        completion::pool(&self.completions, self.language().keywords(), &words)
    }

    /// Open the completion list for the word before the caret, or match it
    /// again when one is open for that word. `asked`: Ctrl-Space, which opens
    /// one with no word typed too.
    fn complete(&mut self, asked: bool) {
        let selection = self.document.selection();
        let (from, prefix) = completion::prefix_at(&self.document, selection.head);
        if !selection.is_empty() || (prefix.is_empty() && !asked) {
            self.completing = None;
            return;
        }
        match &mut self.completing {
            Some(open) if open.from == from => {
                if !open.refine(from, prefix) {
                    self.completing = None;
                }
            }
            _ => self.completing = Completing::open(from, prefix, self.pool(from)),
        }
    }

    /// Put the open list's `index`th item in place of its word, and close it.
    fn accept(&mut self, index: usize) {
        let Some(open) = self.completing.take() else {
            return;
        };
        if self.read_only {
            return;
        }
        if let Some(item) = open.items.get(index) {
            completion::accept(&mut self.document, open.from, item);
            self.column = None;
            self.reveal = true;
        }
    }

    /// The pointer on the open list drawn last frame: resting on a row
    /// chooses it, a press accepts it. True when the press was the list's.
    /// `bounds` is the code's rect on the screen this frame: a list laid out
    /// inside another (the editor moved or resized) is not where it will be
    /// drawn, so the pointer is left to what is there now.
    fn popup_pointer(&mut self, input: &InputState, bounds: Rect) -> bool {
        let (Some(popup), Some(open)) = (&self.popup, &mut self.completing) else {
            return false;
        };
        if popup.bounds != bounds {
            return false;
        }
        let (x, y) = (input.mouse_x, input.mouse_y);
        let row = popup.row_at(x, y);
        if let Some(row) = row
            && !input.mouse_down
        {
            open.chosen = row;
        }
        if !input.mouse_clicked || !popup.rect.contains(x, y) {
            return false;
        }
        if let Some(row) = row {
            self.accept(row);
        }
        true
    }

    /// Tell the parser and the problems about the edits made since they
    /// were last told.
    fn catch_up(&mut self) {
        let edits = self.document.take_edits();
        self.syntax.follow(&self.document, &edits);
        self.diagnostics.follow(&edits);
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
        // The code's rect on the screen, through the scroll and any host
        // transform: the completion list is kept inside it.
        let bounds = {
            let transform = cx.draw_list.current_transform();
            let [x0, y0] = transform.transform_point([body.x, body.y]);
            let [x1, y1] = transform.transform_point([body.right(), body.bottom()]);
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        };
        // A press on the open list is the list's, not the code's under it.
        let listed = focused && self.popup_pointer(cx.input, bounds);
        let mut taken = Taken::default();
        if focused {
            let page = ((body.height - 2.0 * PAD_Y) / metrics.line_height)
                .floor()
                .max(2.0) as isize
                - 1;
            taken = self.keys(cx.input, page);
            if taken.tab {
                cx.focus.claim_tab();
            }
            if taken.escape {
                cx.focus.claim_escape();
            }
        } else {
            self.completing = None;
        }
        if self.status {
            // Before the body, so a jump to a problem is drawn this frame.
            let strip = Rect::new(rect.x, body.bottom(), rect.width, status);
            self.status_clicks(id, strip, cx);
        }
        self.catch_up();

        self.draw_well(rect, focused, cx);
        self.draw_body(body, &metrics, &font, focused, listed, cx);
        if self.status {
            let strip = Rect::new(rect.x, body.bottom(), rect.width, status);
            self.draw_status(strip, cx);
        }
        // The list, under its word and inside the code's rect, scrolled on
        // from where it was.
        // While its word is out of view (a scroll gliding to it) the list
        // stays open, unshown.
        let first = self.popup.as_ref().map_or(0, |popup| popup.first);
        self.popup = None;
        if let (Some(open), Some(word)) = (&self.completing, self.word) {
            let styles = cx.styles();
            self.popup = popup::layout(open, word, bounds, first, &styles, cx.draw_list);
            // An editor too short to show a row of it shows no list, so
            // none is left open to take the keys unseen.
            if self.popup.is_none() {
                self.completing = None;
            }
        }
        cx.pop_debug_scope();
        CodeEditorOutput {
            changed: self.document.version() != version,
            completion_asked: taken.asked,
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

    /// Act on this frame's keys.
    fn keys(&mut self, input: &InputState, page: isize) -> Taken {
        let version = self.document.version();
        let selection = self.document.selection();
        let editable = !self.read_only;
        let mut taken = Taken::default();
        if !editable {
            self.completing = None;
        }
        // An open list takes the keys that choose from it.
        if let Some(open) = &mut self.completing {
            if input.key_up || input.key_down {
                open.step(if input.key_up { -1 } else { 1 });
                return taken;
            }
            if input.enter_pressed || input.key_tab {
                let chosen = open.chosen;
                self.accept(chosen);
                taken.tab = input.key_tab;
                return taken;
            }
            if input.key_escape {
                self.completing = None;
                taken.escape = true;
                return taken;
            }
        }
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
        // Ctrl-Space: Control itself on macOS, where Command-Space is the
        // system's.
        let control = if mac {
            input.control_down
        } else {
            input.ctrl_pressed
        };
        let space = input.key_space || input.keys.was_pressed(Key::Space);
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
        } else if control && space {
            if editable {
                self.complete(true);
                taken.asked = true;
            }
        } else if input.key_tab && editable {
            if shift {
                commands::outdent(&mut self.document, &self.rules);
            } else {
                commands::indent(&mut self.document, &self.rules);
            }
            taken.tab = true;
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
        let moved = self.document.selection() != selection || self.document.version() != version;
        if moved {
            self.reveal = true;
        }
        // A word typed opens the list or narrows it, Backspace widens it;
        // anything else that moves the caret or changes the text closes it.
        let typed_word = !input.text_input.is_empty()
            && self.document.version() != version
            && input
                .text_input
                .chars()
                .last()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let backspaced = self.completing.is_some() && input.backspace_pressed && moved;
        if typed_word || backspaced {
            self.complete(false);
        } else if moved && !taken.asked {
            self.completing = None;
        }
        taken
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
        listed: bool,
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
        if listed {
            // The completion list took this press.
            input.mouse_clicked = false;
            input.mouse_click_count = 0;
        }
        let begun = view.begin(&mut self.scroll, cx.draw_list, &mut input);
        let inner = begun.inner;
        let offset = self.scroll.offset;

        if inner.contains(input.mouse_x, input.mouse_y)
            && !input.mouse_consumed
            && input.mouse_x >= inner.x + metrics.gutter
        {
            cx.request_cursor(CursorIcon::Text);
        }
        if self.pointer(&input, inner, metrics, cx.draw_list) {
            // A press in the code closes the list.
            self.completing = None;
        }

        // The lines in view.
        let top = offset[1] - PAD_Y;
        let first = ((top / metrics.line_height).floor().max(0.0) as usize).min(line_count - 1);
        // The last line starting above the view's bottom edge.
        let last = (((top + inner.height) / metrics.line_height).ceil().max(1.0) as usize - 1)
            .clamp(first, line_count - 1);
        let lines = first..last + 1;
        let spans = self.colours(lines.clone());
        // The worst problem on each line in view.
        let worst: Vec<Option<Severity>> = lines
            .clone()
            .map(|line| {
                let range = self.document.line_start(line)..self.document.line_end(line);
                self.diagnostics.worst_on(range)
            })
            .collect();
        let wash = |line: usize| worst[line - first].and_then(|severity| self.theme.row(severity));
        // The pointer, in the content's own place, while it rests over the
        // view: what shows a problem's message.
        let resting = inner.contains(input.mouse_x, input.mouse_y)
            && !input.mouse_consumed
            && !input.mouse_down;
        let pointer = resting.then(|| [input.mouse_x + offset[0], input.mouse_y + offset[1]]);
        let mut hover = None;
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

        // Washes across the lines with problems; the caret line's band gives
        // way to one.
        for line in lines.clone() {
            if let Some(colour) = wash(line) {
                let y = line_top(line);
                cx.draw_list
                    .quad(left, y, inner.width, metrics.line_height, colour);
                #[cfg(test)]
                if let Some(severity) = worst[line - first] {
                    self.drawn.washes.push((line, severity));
                }
            }
        }
        if banded && lines.contains(&caret_line) && wash(caret_line).is_none() {
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

        // The problems' underlines, only across the columns in view.
        let shown = left + metrics.gutter - metrics.advance..left + inner.width + metrics.advance;
        let transform = cx.draw_list.current_transform();
        let stroke = Stroke::new(WAVE_WIDTH);
        for line in lines.clone() {
            if worst[line - first].is_none() {
                continue;
            }
            let start = self.document.line_start(line);
            let end = self.document.line_end(line);
            let text = self.document.line(line);
            let y = line_top(line);
            for diagnostic in self.diagnostics.on(start..end) {
                let range = &diagnostic.range;
                let x0 = column_x(column_of(&text, range.start.max(start) - start, tab_size));
                let mut x1 = column_x(column_of(&text, range.end.min(end) - start, tab_size));
                if range.is_empty() {
                    // Drawn as the character after it.
                    x1 = x0 + metrics.advance;
                } else if x1 <= x0 {
                    // Only its line break is on this line.
                    continue;
                }
                let (from, to) = (x0.max(shown.start), x1.min(shown.end));
                let ink = self.theme.ink(diagnostic.severity);
                let points = marks::wave(from, to, text_x, y + metrics.line_height - WAVE_RISE);
                if points.len() > 1 {
                    cx.draw_list.stroke_polyline(&points, &stroke, ink);
                }
                #[cfg(test)]
                self.drawn.waves.push((line, x0..x1, diagnostic.severity));
                if let Some([px, py]) = pointer
                    && (x0..x1).contains(&px)
                    && (y..y + metrics.line_height).contains(&py)
                    && px >= left + metrics.gutter
                {
                    let [ax, ay] = transform.transform_point([x0, y]);
                    hover = Some(Hover {
                        diagnostic: diagnostic.clone(),
                        anchor: Rect::new(ax, ay, x1 - x0, metrics.line_height),
                    });
                }
            }
        }

        // The caret.
        if focused && lines.contains(&caret_line) {
            let start = self.document.line_start(caret_line);
            let text = self.document.line(caret_line);
            let x = column_x(column_of(&text, selection.head - start, tab_size));
            let y = line_top(caret_line);
            cx.draw_list
                .quad(x, y, CARET_WIDTH, metrics.line_height, self.theme.caret);
            if !self.read_only {
                // Where it is on the screen, through any transform the host
                // draws the editor under as well as the scroll.
                let [sx, sy] = cx.draw_list.current_transform().transform_point([x, y]);
                cx.focus
                    .request_ime(Rect::new(sx, sy, CARET_WIDTH, metrics.line_height));
            }
            #[cfg(test)]
            {
                self.drawn.caret = Some(Rect::new(x, y, CARET_WIDTH, metrics.line_height));
            }
        }

        // Where the word being completed starts, for its list.
        self.word = self.completing.as_ref().and_then(|open| {
            let line = self.document.line_of(open.from);
            if !lines.contains(&line) {
                return None;
            }
            let start = self.document.line_start(line);
            let text = self.document.line(line);
            let x = column_x(column_of(&text, open.from - start, tab_size));
            let [sx, sy] = transform.transform_point([x, line_top(line)]);
            Some(Rect::new(sx, sy, 1.0, metrics.line_height))
        });

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
            let marked = worst[line - first];
            if let Some(colour) = wash(line) {
                cx.draw_list
                    .quad(left, y, metrics.gutter, metrics.line_height, colour);
            } else if current {
                cx.draw_list.quad(
                    left,
                    y,
                    metrics.gutter,
                    metrics.line_height,
                    self.theme.line_current,
                );
            }
            if let Some(severity) = marked {
                let ink = self.theme.ink(severity);
                let centre = (left + DOT_X, y + metrics.line_height / 2.0);
                let [r, g, b, a] = ink;
                cx.draw_list
                    .circle(centre, GLOW_RADIUS, [r, g, b, a * GLOW_ALPHA]);
                cx.draw_list.circle(centre, DOT_RADIUS, ink);
                #[cfg(test)]
                self.drawn.dots.push(line);
                if let Some([px, py]) = pointer
                    && (left..left + metrics.gutter).contains(&px)
                    && (y..y + metrics.line_height).contains(&py)
                {
                    let range = self.document.line_start(line)..self.document.line_end(line);
                    if let Some(diagnostic) = self.diagnostics.on(range).min_by_key(|d| d.severity)
                    {
                        let [ax, ay] = transform.transform_point([left + DOT_X - DOT_RADIUS, y]);
                        hover = Some(Hover {
                            diagnostic: diagnostic.clone(),
                            anchor: Rect::new(ax, ay, metrics.gutter, metrics.line_height),
                        });
                    }
                }
            }
            let ink = match marked {
                Some(severity) => self.theme.ink(severity),
                None if current => self.theme.number_current,
                None => self.theme.number,
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
        self.hover = hover;
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
    /// True when it took a press.
    fn pointer(
        &mut self,
        input: &InputState,
        inner: Rect,
        metrics: &Metrics,
        list: &mut DrawList,
    ) -> bool {
        let over = inner.contains(input.mouse_x, input.mouse_y) && !input.mouse_consumed;
        if input.mouse_clicked && !over {
            // A press elsewhere ends any drag of ours.
            self.drag = None;
            return false;
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
            return true;
        }
        if !input.mouse_down {
            self.drag = None;
            return false;
        }
        let Some(drag) = self.drag.clone() else {
            return false;
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
        false
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

    /// Where the strip's error and warning counts go: each its glyph and
    /// number, from the strip's left.
    fn count_rects(
        &self,
        strip: Rect,
        styles: &StyleResolver,
        list: &mut DrawList,
    ) -> [(Severity, Rect, String); 2] {
        let mut x = strip.x + STATUS_PAD;
        [Severity::Error, Severity::Warning].map(|severity| {
            let count = self.diagnostics.count(severity).to_string();
            let width = COUNT_ICON + COUNT_GAP + styles.mono_width(list, &count, TextSize::Meta);
            let rect = Rect::new(x, strip.y + 1.0, width, strip.height - 1.0);
            x += width + STATUS_GAP;
            (severity, rect, count)
        })
    }

    /// A click on the strip's error or warning count: the caret goes to the
    /// next of them after it, wrapping round, and the editor takes focus.
    fn status_clicks(&mut self, id: FocusId, strip: Rect, cx: &mut DrawContext) {
        if !cx.input.mouse_clicked || cx.input.mouse_consumed {
            return;
        }
        let styles = cx.styles();
        let clicked = self
            .count_rects(strip, &styles, cx.draw_list)
            .into_iter()
            .find(|(_, rect, _)| rect.contains(cx.input.mouse_x, cx.input.mouse_y));
        let Some((severity, _, _)) = clicked else {
            return;
        };
        cx.focus.request(id);
        let head = self.document.selection().head;
        if let Some(next) = self.diagnostics.next_after(head, severity) {
            self.document
                .set_selection(Selection::caret(next.range.start));
            self.column = None;
            self.reveal = true;
            // Its word is no longer the one before the caret.
            self.completing = None;
        }
    }

    /// The strip under the code: the error and warning counts, then Ln and
    /// Col, the indent, the language.
    fn draw_status(&mut self, strip: Rect, cx: &mut DrawContext) {
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
        for (severity, rect, count) in self.count_rects(strip, &s, cx.draw_list) {
            let ink = if self.diagnostics.count(severity) > 0 {
                self.theme.ink(severity)
            } else {
                s.ink(Ink::Muted)
            };
            let glyph = match severity {
                Severity::Error => "x-circle",
                _ => "warning",
            };
            let icon_y = rect.y + ((rect.height - COUNT_ICON) / 2.0).round();
            if let Some(icon) = PhosphorIcon::from_name(glyph) {
                Icon::new(icon).tint(ink).draw(
                    Rect::new(rect.x, icon_y, COUNT_ICON, COUNT_ICON),
                    cx.draw_list,
                );
            }
            let block = s
                .mono_block(count, 0.0, 0.0, TextSize::Meta, Ink::Muted)
                .with_color_f32(ink);
            let (_, height) = cx.draw_list.measure_block(&block);
            cx.draw_list.text(TextBlock {
                x: rect.x + COUNT_ICON + COUNT_GAP,
                y: rect.y + ((rect.height - height) / 2.0).round(),
                ..block
            });
            #[cfg(test)]
            self.drawn.counts.push((severity, rect));
        }
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
