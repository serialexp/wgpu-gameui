//! FileDialog — open a file, save one, or choose a folder (Forge
//! `FileDialog`).
//!
//! An in-app dialog, so the theme, keys and behaviour are the same on every
//! platform, and so it can show a disk that isn't this machine's. The caller
//! owns the filesystem through a [`FileSource`]; the dialog owns navigation.
//!
//! Places is one list: Recent (when the caller passes recent entries), the
//! caller's fixed places, a rule, then the user's ★ favourites. ☆ in the
//! toolbar pins the folder shown (not a fixed place); hovering a favourite
//! shows × to unpin it. Folders sort first; double-click or Enter goes in;
//! Backspace or Ctrl/Cmd+↑ goes up; Alt+← / Alt+→ walk the history; typing
//! jumps to the first name that starts with what was typed. A click on the
//! path well's empty space or `/` turns it into a field for typing a path
//! ([`FileDialogState::edit_path`] does the same, for a host's Ctrl/Cmd+L); a
//! path that can't be listed keeps the field open with the error in it.
//!
//! The type strip shows only accepted types present in the folder: one is a
//! caption, two or three are latching [`FilterChip`]s, more show the top
//! three and a "+N" chip that expands the rest inline. Files of other types
//! are not listed, and the empty state says how many were skipped. In folder
//! mode files are listed dimmed, for context. Saving puts the name in a well
//! with its base selected, the extension as a suffix; a name that exists
//! turns the primary key into a danger "Replace". New folder adds a row in
//! rename mode; Enter asks the caller to create it. Escape backs out one
//! layer at a time: the path field, the new folder row, then the dialog.
//!
//! ```ignore
//! // Each frame, while open, in a modal layer:
//! let out = FileDialog::new(FileDialogMode::Folder)
//!     .title("Choose project")
//!     .root_label("~")
//!     .places(&places)
//!     .draw(DIALOG_ID, screen, &mut state, &mut disk, &mut ctx);
//! match out.event {
//!     Some(FileDialogEvent::Confirm { path, .. }) => open_project(path),
//!     Some(FileDialogEvent::CreateFolder { path, name }) => disk.mkdir(&path, &name),
//!     Some(FileDialogEvent::FavouritesChanged) => save(state.favourites()),
//!     _ => {}
//! }
//! ```

mod model;
mod paint;

use crate::chrome::{Edge, SurfacePainter};
use crate::color::{HUE_DANGER, HUE_WARN_INK, oklch};
use crate::layout::Rect;
use crate::shadow::CornerRadii;
use crate::style::{Ink, StyleKey, StyleResolver, TextSize, Tracking};
use crate::text::TextBlock;
use crate::{InputState, PhosphorIcon, SpriteId};

use super::list::{List, ListState};
use super::material::Tone;
use super::{
    Button, Checkbox, DrawContext, FILTER_CHIP_HEIGHT, FilterChip, FocusId, Icon, IconKey,
    ModalState, SearchField, TextInput,
};

pub use model::{Clock, FileDialogMode, SortBy, format_date, format_size};
use model::{
    Location, Nav, ShowFilter, Shown, Sort, TypeAhead, bad_folder_name, bad_name, base_of, ext_of,
    find_prefix, is_hidden, normalise_accept, parse_typed_path, save_name,
};
use paint::{entry_icon, folder_glyph, glyph_width, line_y, mono_right, skeleton_width};

/// Forge's default size.
const SIZE: [f32; 2] = [820.0, 520.0];
/// Room kept between the screen's edge and the dialog.
const BACKDROP_PAD: f32 = 12.0;
/// One listing or place row.
const ROW: f32 = 22.0;
/// The header strip.
const HEADER_H: f32 = 46.0;
const HEADER_PAD_LEFT: f32 = 14.0;
const HEADER_PAD_RIGHT: f32 = 10.0;
const HEADER_GAP: f32 = 6.0;
/// A toolbar key.
const KEY: f32 = 22.0;
/// A view toggle key, inside its well.
const VIEW_KEY: f32 = 20.0;
/// The path well's height.
const WELL_H: f32 = 24.0;
const SEARCH_W: f32 = 190.0;
const PLACES_W: f32 = 164.0;
const PREVIEW_W: f32 = 204.0;
/// The strip above the listing with the type chips.
const TYPE_STRIP_H: f32 = 30.0;
/// The listing's column header (list view).
const COLUMN_HEAD_H: f32 = 20.0;
/// The listing's columns after the name: modified, size, kind.
const MODIFIED_W: f32 = 112.0;
const SIZE_W: f32 = 62.0;
const KIND_W: f32 = 58.0;
/// Space either side of a column's text.
const CELL_PAD: f32 = 9.0;
/// A grid cell's width, gap included.
const GRID_CELL: f32 = 92.0;
const GRID_CELL_H: f32 = 94.0;
/// Between grid cells, and around the grid.
const GRID_GAP: f32 = 4.0;
const GRID_PAD: f32 = 8.0;
const FOOTER_PAD_X: f32 = 12.0;
const FOOTER_PAD_Y: f32 = 9.0;
const NAME_W: f32 = 300.0;
/// The places column's and preview's shade over the surface.
const SIDE_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.16];
/// The listing's shade.
const LIST_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.12];
/// A hovered crumb or place.
const HOVER_WASH: [f32; 4] = [1.0, 1.0, 1.0, 0.09];
/// The place shown, and a selection in a list without the keyboard.
const HELD_WASH: [f32; 4] = [1.0, 1.0, 1.0, 0.1];
/// The rule under the type strip.
const STRIP_RULE: [f32; 4] = [0.0, 0.0, 0.0, 0.35];
/// The view toggles' well.
const VIEW_WELL: [f32; 4] = [0.0, 0.0, 0.0, 0.4];
/// A dimmed (unpickable) row, and a hidden entry.
const DIM_ALPHA: f32 = 0.4;
const HIDDEN_ALPHA: f32 = 0.55;
/// The loading rows' bars.
const SKELETON: [f32; 4] = [1.0, 1.0, 1.0, 0.07];
/// The Places rule.
const PLACES_RULE: [[f32; 4]; 2] = [[0.0, 0.0, 0.0, 0.4], [1.0, 1.0, 1.0, 0.05]];

/// An entry's kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileKind {
    /// A file.
    #[default]
    File,
    /// A folder.
    Folder,
}

/// A thumb to draw for an entry instead of its glyph (see [`Thumb`]).
///
/// [`Thumb`]: super::Thumb
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryThumb {
    /// An image.
    pub sprite: Option<SpriteId>,
    /// A flat colour.
    pub color: Option<[f32; 4]>,
    /// A monogram from this name, in its hue.
    pub monogram: Option<String>,
}

/// One file or folder in a listing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileEntry {
    /// The name, extension included.
    pub name: String,
    /// File or folder.
    pub kind: FileKind,
    /// Bytes.
    pub size: Option<u64>,
    /// Unix seconds.
    pub modified: Option<i64>,
    /// Hidden; a leading dot implies it.
    pub hidden: bool,
    /// Drawn in the list, the grid and the preview instead of the glyph.
    pub thumb: Option<EntryThumb>,
    /// A folder's number of entries.
    pub count: Option<u64>,
    /// Extra preview rows, such as `("Size", "2048×2048")`.
    pub info: Vec<(String, String)>,
    /// Recent entries only: the folder it is in.
    pub dir: Option<Vec<String>>,
}

impl FileEntry {
    /// A file named `name`.
    pub fn file(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// A folder named `name`.
    pub fn folder(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: FileKind::Folder,
            ..Self::default()
        }
    }
}

/// A folder's listing, as far as the caller has it.
#[derive(Clone, Copy, Debug)]
pub enum Listing<'a> {
    /// Asked for; not here yet.
    Loading,
    /// It can't be listed: why.
    Failed(&'a str),
    /// The entries. `generation` changes whenever they do, so the dialog
    /// knows to filter and sort again.
    Entries {
        /// The folder's entries, in any order.
        entries: &'a [FileEntry],
        /// Changes whenever `entries` does.
        generation: u64,
    },
}

/// The filesystem a [`FileDialog`] shows. `path` is a list of folder names
/// under the dialog's root.
pub trait FileSource {
    /// The listing of `path`. Ask for it (once) and answer
    /// [`Listing::Loading`] until it arrives.
    fn list(&mut self, path: &[String]) -> Listing<'_>;
}

/// A fixed place: always in the Places list, never unpinned.
#[derive(Clone, Debug, PartialEq)]
pub struct FilePlace {
    /// The row's label.
    pub label: String,
    /// The folder it goes to, under the root.
    pub path: Vec<String>,
    /// Drawn instead of the folder glyph.
    pub icon: Option<PhosphorIcon>,
}

impl FilePlace {
    /// A place labelled `label`, going to `path`.
    pub fn new(label: impl Into<String>, path: Vec<String>) -> Self {
        Self {
            label: label.into(),
            path,
            icon: None,
        }
    }

    /// Draw `icon` instead of the folder glyph.
    #[must_use]
    pub fn icon(mut self, icon: PhosphorIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A folder the user pinned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FavouriteFolder {
    /// The folder, under the root.
    pub path: Vec<String>,
    /// Shown instead of the folder's name.
    pub label: Option<String>,
}

/// List or grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileView {
    /// Rows with columns.
    #[default]
    List,
    /// Icons in cells.
    Grid,
}

/// What happened in one frame of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogEvent {
    /// The primary key, Enter, or a double-click on a file when opening.
    /// `path` is the folder (the chosen folder in folder mode); `name` the
    /// file opened or to save. `replaces`: saving over an existing file. The
    /// dialog has closed.
    Confirm {
        /// The folder.
        path: Vec<String>,
        /// The file, outside folder mode.
        name: Option<String>,
        /// Saving over an existing file.
        replaces: bool,
    },
    /// Cancel, or Escape. The dialog has closed.
    Cancel,
    /// Make folder `name` in `path`, then give `path` a new listing.
    CreateFolder {
        /// The folder to make it in.
        path: Vec<String>,
        /// The new folder's name, already checked.
        name: String,
    },
    /// The favourites changed; read them from
    /// [`FileDialogState::favourites`] and keep them.
    FavouritesChanged,
}

/// What one [`FileDialog::draw`] did, and where its footer slot is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDialogOutput {
    /// What happened, if anything.
    pub event: Option<FileDialogEvent>,
    /// The footer slot reserved with [`FileDialog::footer_slot`], for the
    /// caller's own control. Draw it after the dialog, on the same layer.
    pub slot: Option<Rect>,
}

/// Focus ids under the dialog's base id.
#[derive(Clone, Copy)]
struct Ids {
    listing: FocusId,
    search: FocusId,
    path: FocusId,
    new_folder: FocusId,
    name: FocusId,
    hidden: FocusId,
    cancel: FocusId,
    primary: FocusId,
}

impl Ids {
    fn new(base: FocusId) -> Self {
        Self {
            listing: base,
            search: base + 1,
            path: base + 2,
            new_folder: base + 3,
            name: base + 4,
            hidden: base + 5,
            cancel: base + 6,
            primary: base + 7,
        }
    }
}

/// The listing as last filtered and sorted, and what that was built from.
#[derive(Default)]
struct ShownCache {
    shown: Shown,
    /// The location, listing generation (`None` for Recent), mode and
    /// accepted types it was built for.
    location: Location,
    generation: Option<u64>,
    mode: FileDialogMode,
    accept: Vec<String>,
    /// Filter, sort or toggles changed since.
    dirty: bool,
    built: bool,
}

/// Caller-owned dialog state. Keep one per dialog; [`open`](Self::open) it
/// on a folder, then draw it every frame while [`is_open`](Self::is_open).
#[derive(Default)]
pub struct FileDialogState {
    /// The modal underneath. Call its [`begin_frame`](ModalState::begin_frame)
    /// before the focus owner's, so Escape reaches the dialog.
    pub modal: ModalState,
    nav: Nav,
    /// The selected entry's name.
    selected: Option<String>,
    view: FileView,
    sort: Sort,
    search: TextInput,
    /// Types toggled off by their chips.
    off: Vec<String>,
    more_types: bool,
    show_hidden: bool,
    name: TextInput,
    path_field: TextInput,
    editing_path: bool,
    path_error: Option<String>,
    /// A typed path waiting for its listing before the dialog goes there.
    pending_path: Option<Vec<String>>,
    new_folder: Option<TextInput>,
    /// The new folder field held focus last frame (leaving it commits).
    new_folder_focused: bool,
    favourites: Vec<FavouriteFolder>,
    list: ListState,
    type_ahead: TypeAhead,
    cache: ShownCache,
    /// The listing's last drawn geometry, for scrolling a type-ahead match
    /// into view before this frame's list is drawn.
    geometry: ListingGeometry,
    /// Move focus to this next frame.
    focus_next: Option<Target>,
}

/// Where the listing's rows were last drawn.
#[derive(Clone, Copy, Debug, Default)]
struct ListingGeometry {
    columns: usize,
    /// One row of items, gap included.
    pitch: f32,
    /// The visible height.
    height: f32,
}

impl ListingGeometry {
    /// Scroll `scroll` so item `index` shows.
    fn reveal(self, scroll: &mut super::ScrollState, index: usize) {
        if self.columns == 0 || self.height <= 0.0 {
            return;
        }
        let top = (index / self.columns) as f32 * self.pitch;
        scroll.scroll_range_into_view(1, top, top + self.pitch, self.height);
    }
}

/// A field focus moves to next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Listing,
    Path,
    NewFolder,
    Name,
}

impl FileDialogState {
    /// A closed dialog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the dialog on `path`. History, selection, filter and fields
    /// start fresh; the view, sort, hidden toggle and favourites carry over.
    pub fn open(&mut self, path: Vec<String>) {
        self.nav = Nav::new(path);
        self.reset_folder_state();
        self.name.value.clear();
        self.name.cursor_pos = 0;
        self.name.selection_start = None;
        self.modal.open();
        self.focus_next = Some(Target::Listing);
    }

    /// Open for saving on `path`, with `name` in the name field and its base
    /// (the part before the extension) selected.
    pub fn open_with_name(&mut self, path: Vec<String>, name: &str) {
        self.open(path);
        self.name.value.push_str(name);
        self.name.selection_start = Some(0);
        self.name.cursor_pos = base_of(name).len();
        self.focus_next = Some(Target::Name);
    }

    /// Whether the dialog is open.
    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    /// The folder shown.
    pub fn path(&self) -> &[String] {
        &self.nav.here.path
    }

    /// Whether Recent is shown rather than a folder.
    pub fn showing_recent(&self) -> bool {
        self.nav.here.recent
    }

    /// The selected entry's name.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Select the entry named `name` (after creating it, say).
    pub fn select(&mut self, name: &str) {
        self.selected = Some(name.to_owned());
    }

    /// List or grid.
    pub fn view(&self) -> FileView {
        self.view
    }

    /// Show the listing as `view`.
    pub fn set_view(&mut self, view: FileView) {
        self.view = view;
    }

    /// The user's pinned folders, in order.
    pub fn favourites(&self) -> &[FavouriteFolder] {
        &self.favourites
    }

    /// Replace the favourites (loaded from where the caller keeps them).
    pub fn set_favourites(&mut self, favourites: Vec<FavouriteFolder>) {
        self.favourites = favourites;
    }

    /// Turn the path well into a field (a host's Ctrl/Cmd+L).
    pub fn edit_path(&mut self) {
        self.editing_path = true;
        self.path_error = None;
        self.focus_next = Some(Target::Path);
    }

    /// Whether the path field is open.
    pub fn editing_path(&self) -> bool {
        self.editing_path
    }

    /// Whether the new folder row is open.
    pub fn creating_folder(&self) -> bool {
        self.new_folder.is_some()
    }

    /// Let `dt` seconds pass on a tick that doesn't draw the dialog, as
    /// [`UiState::tick_clocks`](crate::UiState::tick_clocks) does for gameui's
    /// own clocks. A host that skips idle frames must call it, or a pause in
    /// typing goes unseen and the next letter extends the old prefix; drawn
    /// frames count their `frame_dt` themselves.
    pub fn tick_clocks(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.type_ahead.tick(dt);
        }
    }

    /// Close without an event (the caller is done with it).
    pub fn close(&mut self, focus: &mut super::FocusState) {
        self.modal.close(focus);
    }

    /// What arriving somewhere resets.
    fn reset_folder_state(&mut self) {
        self.selected = None;
        self.search.value.clear();
        self.search.cursor_pos = 0;
        self.search.selection_start = None;
        self.off.clear();
        self.more_types = false;
        self.new_folder = None;
        self.editing_path = false;
        self.path_error = None;
        self.pending_path = None;
        self.list = ListState::new();
        self.type_ahead.clear();
        self.cache.dirty = true;
    }

    fn go(&mut self, to: Location) {
        if self.nav.go(to) {
            self.reset_folder_state();
        }
    }

    fn go_path(&mut self, path: Vec<String>) {
        self.go(Location {
            path,
            recent: false,
        });
    }

    fn back(&mut self) {
        if self.nav.back() {
            self.reset_folder_state();
        }
    }

    fn forward(&mut self) {
        if self.nav.forward() {
            self.reset_folder_state();
        }
    }

    fn up(&mut self) {
        if self.nav.up() {
            self.reset_folder_state();
        }
    }

    fn pinned(&self) -> bool {
        !self.nav.here.recent && self.favourites.iter().any(|f| f.path == self.nav.here.path)
    }

    fn toggle_pin(&mut self) {
        let here = &self.nav.here.path;
        if self.pinned() {
            self.favourites.retain(|f| &f.path != here);
        } else {
            self.favourites.push(FavouriteFolder {
                path: here.clone(),
                label: None,
            });
        }
    }

    fn unpin(&mut self, path: &[String]) {
        self.favourites.retain(|f| f.path != path);
    }

    fn start_new_folder(&mut self) {
        let mut field = TextInput::default();
        field.value.push_str("New folder");
        field.select_all();
        self.new_folder = Some(field);
        self.selected = None;
        self.focus_next = Some(Target::NewFolder);
    }
}

/// The per-frame dialog. See the [module docs](self).
#[derive(Clone, Copy)]
pub struct FileDialog<'a> {
    mode: FileDialogMode,
    title: Option<&'a str>,
    root_label: &'a str,
    places: &'a [FilePlace],
    recent: Option<&'a [FileEntry]>,
    accept: &'a [&'a str],
    save_ext: Option<&'a str>,
    size: [f32; 2],
    clock: Option<Clock>,
    footer_slot: f32,
}

impl<'a> FileDialog<'a> {
    /// A dialog in `mode`.
    pub fn new(mode: FileDialogMode) -> Self {
        Self {
            mode,
            title: None,
            root_label: "~",
            places: &[],
            recent: None,
            accept: &[],
            save_ext: None,
            size: SIZE,
            clock: None,
            footer_slot: 0.0,
        }
    }

    /// The title (default "Open", "Save as" or "Choose folder").
    #[must_use]
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// The first crumb, such as "~" or the project's name.
    #[must_use]
    pub fn root_label(mut self, label: &'a str) -> Self {
        self.root_label = label;
        self
    }

    /// The fixed places, at the top of Places.
    #[must_use]
    pub fn places(mut self, places: &'a [FilePlace]) -> Self {
        self.places = places;
        self
    }

    /// A Recent place listing these (each with its `dir`).
    #[must_use]
    pub fn recent(mut self, recent: &'a [FileEntry]) -> Self {
        self.recent = Some(recent);
        self
    }

    /// The extensions that can be opened, such as `["lvl", "prefab"]`;
    /// empty accepts every file.
    #[must_use]
    pub fn accept(mut self, accept: &'a [&'a str]) -> Self {
        self.accept = accept;
        self
    }

    /// Appended to a saved name that lacks it; shown as a suffix.
    #[must_use]
    pub fn save_ext(mut self, ext: &'a str) -> Self {
        self.save_ext = Some(ext);
        self
    }

    /// The dialog's size (default 820 × 520), shrunk to fit the screen.
    #[must_use]
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = [width, height];
        self
    }

    /// The clock dates are shown against; without it they show "—".
    #[must_use]
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Reserve `width` px at the footer's start for the caller's control
    /// (see [`FileDialogOutput::slot`]).
    #[must_use]
    pub fn footer_slot(mut self, width: f32) -> Self {
        self.footer_slot = width;
        self
    }

    fn title_text(&self) -> &'a str {
        self.title.unwrap_or(match self.mode {
            FileDialogMode::Open => "Open",
            FileDialogMode::Save => "Save as",
            FileDialogMode::Folder => "Choose folder",
        })
    }

    /// Where the surface goes in `bounds`.
    pub fn rect(&self, bounds: Rect) -> Rect {
        let area = bounds.inset(BACKDROP_PAD);
        let w = self.size[0].min(area.width).max(1.0);
        let h = self.size[1].min(area.height).max(1.0);
        Rect::new(
            (area.x + (area.width - w) * 0.5).round(),
            (area.y + (area.height - h) * 0.5).round(),
            w,
            h,
        )
    }

    /// Draw the dialog over `bounds` (the screen), with its focusables at
    /// `id`, `id + 1`, … `id + 7`. Draw it into a modal layer with a context
    /// [`with_layer`](super::DrawContext::with_layer).
    pub fn draw(
        &self,
        id: FocusId,
        bounds: Rect,
        state: &mut FileDialogState,
        source: &mut dyn FileSource,
        ctx: &mut DrawContext,
    ) -> FileDialogOutput {
        Frame::new(self, Ids::new(id), bounds, state, ctx).run(source)
    }
}

/// One frame of drawing: the dialog, its state and the context, and what
/// the frame decided.
struct Frame<'f, 'a, 'c> {
    dialog: &'f FileDialog<'a>,
    ids: Ids,
    bounds: Rect,
    state: &'f mut FileDialogState,
    ctx: &'f mut DrawContext<'c>,
    event: Option<FileDialogEvent>,
    /// The listing could not be read.
    error: Option<String>,
    loading: bool,
}

/// What the primary key does now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Primary {
    Open,
    OpenFolder,
    Save,
    Replace,
    Choose,
}

impl Primary {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::OpenFolder => "Open folder",
            Self::Save => "Save",
            Self::Replace => "Replace",
            Self::Choose => "Choose",
        }
    }
}

impl<'f, 'a, 'c> Frame<'f, 'a, 'c> {
    fn new(
        dialog: &'f FileDialog<'a>,
        ids: Ids,
        bounds: Rect,
        state: &'f mut FileDialogState,
        ctx: &'f mut DrawContext<'c>,
    ) -> Self {
        Self {
            dialog,
            ids,
            bounds,
            state,
            ctx,
            event: None,
            error: None,
            loading: false,
        }
    }

    fn run(mut self, source: &mut dyn FileSource) -> FileDialogOutput {
        let target = match self.dialog.mode {
            FileDialogMode::Save => self.ids.name,
            _ => self.ids.listing,
        };
        let cancelled = self.state.modal.present(self.ctx.focus, Some(target));
        self.state.type_ahead.tick(self.ctx.input.frame_dt);
        self.resolve_pending_path(source);
        if let Some(next) = self.state.focus_next.take() {
            let id = match next {
                Target::Listing => self.ids.listing,
                Target::Path => self.ids.path,
                Target::NewFolder => self.ids.new_folder,
                Target::Name => self.ids.name,
            };
            self.ctx.focus.focus(id);
        }

        let recent = self.state.nav.here.recent;
        let listing = if recent {
            Listing::Entries {
                entries: self.dialog.recent.unwrap_or(&[]),
                generation: 0,
            }
        } else {
            source.list(&self.state.nav.here.path)
        };
        let entries: &[FileEntry] = match listing {
            Listing::Loading => {
                self.loading = true;
                &[]
            }
            Listing::Failed(why) => {
                self.error = Some(if why.is_empty() {
                    "Can't read this folder".into()
                } else {
                    why.to_owned()
                });
                &[]
            }
            Listing::Entries { entries, .. } => entries,
        };
        let generation = match listing {
            Listing::Entries { generation, .. } if !recent => Some(generation),
            _ => None,
        };
        self.refresh_shown(entries, generation);

        if cancelled {
            self.escape();
        }

        let rect = self.dialog.rect(self.bounds);
        let slot = self.paint(rect, entries);
        // "Today" and "Yesterday" move on at local midnight.
        if let Some(clock) = self.dialog.clock {
            self.ctx
                .draw_list
                .repaint_after(clock.until_next_day() as f32);
        }
        if matches!(
            self.event,
            Some(FileDialogEvent::Confirm { .. } | FileDialogEvent::Cancel)
        ) {
            self.state.modal.close(self.ctx.focus);
        }
        FileDialogOutput {
            event: self.event,
            slot,
        }
    }

    fn emit(&mut self, event: FileDialogEvent) {
        if self.event.is_none() {
            self.event = Some(event);
        }
    }

    /// A typed path whose listing has arrived: go there, or keep the field
    /// open with why not.
    fn resolve_pending_path(&mut self, source: &mut dyn FileSource) {
        let Some(path) = self.state.pending_path.take() else {
            return;
        };
        match source.list(&path) {
            Listing::Loading => self.state.pending_path = Some(path),
            Listing::Failed(why) => {
                self.state.path_error = Some(if why.is_empty() {
                    "No such folder".into()
                } else {
                    why.to_owned()
                });
                self.state.focus_next = Some(Target::Path);
            }
            Listing::Entries { .. } => {
                self.state.editing_path = false;
                self.state.go_path(path);
                self.state.focus_next = Some(Target::Listing);
            }
        }
    }

    fn refresh_shown(&mut self, entries: &[FileEntry], generation: Option<u64>) {
        let cache = &mut self.state.cache;
        let here = &self.state.nav.here;
        let accept_same = cache.accept.len() == self.dialog.accept.len()
            && cache
                .accept
                .iter()
                .zip(self.dialog.accept)
                .all(|(a, b)| a == b.trim_start_matches('.').to_lowercase().as_str());
        let stale = !cache.built
            || cache.dirty
            || cache.location != *here
            || cache.generation != generation
            || cache.mode != self.dialog.mode
            || !accept_same;
        if !stale {
            return;
        }
        let filter = ShowFilter {
            mode: self.dialog.mode,
            accept: normalise_accept(self.dialog.accept),
            off: self.state.off.clone(),
            show_hidden: self.state.show_hidden,
            query: self.state.search.value.clone(),
            sort: self.state.sort,
            recent: here.recent,
        };
        cache.shown = filter.apply(entries);
        cache.location = here.clone();
        cache.generation = generation;
        cache.mode = self.dialog.mode;
        cache.accept = filter.accept;
        cache.dirty = false;
        cache.built = true;
    }

    fn folder_mode(&self) -> bool {
        self.dialog.mode == FileDialogMode::Folder
    }

    /// Whether `entry` can be picked (folder mode lists files for context
    /// only).
    fn pickable(&self, entry: &FileEntry) -> bool {
        !(self.folder_mode() && entry.kind == FileKind::File)
    }

    /// The selected entry, among those shown.
    fn selected<'e>(&self, entries: &'e [FileEntry]) -> Option<(usize, &'e FileEntry)> {
        let name = self.state.selected.as_deref()?;
        self.state
            .cache
            .shown
            .rows
            .iter()
            .enumerate()
            .map(|(row, &i)| (row, &entries[i]))
            .find(|(_, e)| e.name == name && self.pickable(e))
    }

    fn current_name(&self) -> &str {
        let here = &self.state.nav.here;
        if here.recent {
            "Recent"
        } else {
            here.path
                .last()
                .map_or(self.dialog.root_label, String::as_str)
        }
    }

    fn save_name(&self) -> String {
        save_name(&self.state.name.value, self.dialog.save_ext)
    }

    fn name_error(&self) -> Option<&'static str> {
        (self.dialog.mode == FileDialogMode::Save)
            .then(|| bad_name(&self.state.name.value))
            .flatten()
    }

    fn will_replace(&self, entries: &[FileEntry]) -> bool {
        if self.dialog.mode != FileDialogMode::Save || self.name_error().is_some() {
            return false;
        }
        let name = self.save_name();
        entries
            .iter()
            .any(|e| e.kind == FileKind::File && e.name == name)
    }

    fn primary(&self, entries: &[FileEntry]) -> Primary {
        match self.dialog.mode {
            FileDialogMode::Save if self.will_replace(entries) => Primary::Replace,
            FileDialogMode::Save => Primary::Save,
            FileDialogMode::Folder => Primary::Choose,
            FileDialogMode::Open => match self.selected(entries) {
                Some((_, e)) if e.kind == FileKind::Folder => Primary::OpenFolder,
                _ => Primary::Open,
            },
        }
    }

    fn primary_enabled(&self, entries: &[FileEntry]) -> bool {
        let recent = self.state.nav.here.recent;
        let selected = self.selected(entries);
        match self.dialog.mode {
            FileDialogMode::Open => selected.is_some(),
            FileDialogMode::Save => {
                !self.save_name().is_empty() && self.name_error().is_none() && !recent
            }
            FileDialogMode::Folder => !(recent && selected.is_none()) && self.error.is_none(),
        }
    }

    /// The folder folder mode would choose: the selected folder, or the one
    /// shown.
    fn folder_target(&self, entries: &[FileEntry]) -> Vec<String> {
        match self.selected(entries) {
            Some((_, e)) if e.kind == FileKind::Folder => self.entry_path(e),
            _ => self.state.nav.here.path.clone(),
        }
    }

    /// The path of folder `entry` (from Recent, where it says it is).
    fn entry_path(&self, entry: &FileEntry) -> Vec<String> {
        let mut path = match (&entry.dir, self.state.nav.here.recent) {
            (Some(dir), true) => dir.clone(),
            _ => self.state.nav.here.path.clone(),
        };
        path.push(entry.name.clone());
        path
    }

    /// Go into folder `entry`, or open file `entry`.
    fn enter(&mut self, entry: &FileEntry) {
        match entry.kind {
            FileKind::Folder => {
                let path = self.entry_path(entry);
                self.state.go_path(path);
            }
            FileKind::File if self.dialog.mode == FileDialogMode::Open => {
                let path = match (&entry.dir, self.state.nav.here.recent) {
                    (Some(dir), true) => dir.clone(),
                    _ => self.state.nav.here.path.clone(),
                };
                self.emit(FileDialogEvent::Confirm {
                    path,
                    name: Some(entry.name.clone()),
                    replaces: false,
                });
            }
            FileKind::File => {}
        }
    }

    /// The primary key.
    fn confirm(&mut self, entries: &[FileEntry], from_listing: bool) {
        if !self.primary_enabled(entries) {
            return;
        }
        let selected = self.selected(entries).map(|(_, e)| e.clone());
        match self.dialog.mode {
            FileDialogMode::Open => {
                if let Some(entry) = selected {
                    self.enter(&entry);
                }
            }
            FileDialogMode::Folder => {
                let path = self.folder_target(entries);
                self.emit(FileDialogEvent::Confirm {
                    path,
                    name: None,
                    replaces: false,
                });
            }
            FileDialogMode::Save => {
                if from_listing && let Some(entry) = selected.filter(|e| e.kind == FileKind::Folder)
                {
                    self.enter(&entry);
                    return;
                }
                let replaces = self.will_replace(entries);
                self.emit(FileDialogEvent::Confirm {
                    path: self.state.nav.here.path.clone(),
                    name: Some(self.save_name()),
                    replaces,
                });
            }
        }
    }

    /// Escape backs out one layer.
    fn escape(&mut self) {
        if self.state.editing_path {
            self.state.editing_path = false;
            self.state.path_error = None;
            self.state.pending_path = None;
            self.state.focus_next = Some(Target::Listing);
        } else if self.state.new_folder.take().is_some() {
            self.state.focus_next = Some(Target::Listing);
        } else {
            self.emit(FileDialogEvent::Cancel);
        }
    }

    // ---- drawing ------------------------------------------------------

    /// Draw everything; returns the footer slot.
    fn paint(&mut self, rect: Rect, entries: &[FileEntry]) -> Option<Rect> {
        let s = self.ctx.styles();
        let chrome = s.sheet();
        self.ctx.push_debug_scope_rect("FileDialog", self.bounds);
        let list = &mut *self.ctx.draw_list;
        list.push_clip_viewport(self.bounds);
        list.quad(
            self.bounds.x,
            self.bounds.y,
            self.bounds.width,
            self.bounds.height,
            chrome.backdrop,
        );
        let border = chrome.surface.border_widths.top;
        let inner = rect.inset(border);
        let radius = chrome.surface.corner_radii.bottom_left;
        SurfacePainter::new(
            list,
            rect,
            inner,
            CornerRadii::uniform((radius - border).max(0.0)),
            chrome.surface,
            &chrome.shadows,
            &[],
        )
        .paint_pre_content();

        let footer_h = s.scalar(StyleKey::ButtonHeight) + FOOTER_PAD_Y * 2.0 + 1.0;
        let header = Rect::new(inner.x, inner.y, inner.width, HEADER_H);
        let footer = Rect::new(inner.x, inner.bottom() - footer_h, inner.width, footer_h);
        let body = Rect::new(
            inner.x,
            header.bottom(),
            inner.width,
            (footer.y - header.bottom()).max(0.0),
        );
        let places = Rect::new(body.x, body.y, PLACES_W.min(body.width), body.height);
        let preview_w = PREVIEW_W.min((body.width - places.width).max(0.0));
        let preview = Rect::new(body.right() - preview_w, body.y, preview_w, body.height);
        let middle = Rect::new(
            places.right(),
            body.y,
            (preview.x - places.right()).max(0.0),
            body.height,
        );

        self.draw_header(header);
        self.draw_places(places);
        self.draw_middle(middle, entries);
        self.draw_preview(preview, entries);
        let slot = self.draw_footer(footer, radius - border, entries);

        SurfacePainter::new(
            self.ctx.draw_list,
            rect,
            inner,
            CornerRadii::default(),
            chrome.surface,
            &[],
            &[],
        )
        .paint_post_content();
        self.ctx.draw_list.pop_clip();
        self.ctx.pop_debug_scope();
        // A click on the surface's own chrome keeps whatever has focus.
        if self.hit(rect).1 {
            self.ctx.focus.claim_click();
        }
        slot
    }

    /// A pointer test on `rect` against this layer's input: (hovered,
    /// clicked).
    fn hit(&self, rect: Rect) -> (bool, bool) {
        let input = self.ctx.input;
        let hovered = !input.mouse_consumed && rect.contains(input.mouse_x, input.mouse_y);
        (hovered, hovered && input.mouse_clicked)
    }

    fn key(&mut self, icon: PhosphorIcon, rect: Rect, enabled: bool, held: bool) -> bool {
        IconKey::new(icon, rect.height)
            .tone(Tone::Ghost)
            .enabled(enabled)
            .held(held)
            .draw(rect, self.ctx)
            .clicked
    }

    fn draw_header(&mut self, header: Rect) {
        let s = self.ctx.styles();
        let rule = s.sheet().footer_rule;
        self.ctx
            .draw_list
            .edge_line(header, Edge::Bottom, rule.thickness, rule.color);
        let cy = header.y + header.height * 0.5;
        let mut x = header.x + HEADER_PAD_LEFT;
        let title_size = s.scalar(StyleKey::FontSizeTitle);
        let title = TextBlock::new(
            self.dialog.title_text(),
            x,
            crate::text::vcentered_line_y(header.y, header.height, title_size),
        )
        .with_size(title_size)
        .with_color_f32(s.ink(Ink::Value))
        .with_font_opt(s.theme().font.clone())
        .bold();
        let (title_w, _) = self.ctx.draw_list.measure_block(&title);
        self.ctx.draw_list.text(title);
        x += title_w + 8.0 + HEADER_GAP;

        let key_rect = |x: f32| Rect::new(x, (cy - KEY * 0.5).round(), KEY, KEY);
        let recent = self.state.nav.here.recent;
        if self.key(
            PhosphorIcon::CaretLeft,
            key_rect(x),
            self.state.nav.can_go_back(),
            false,
        ) {
            self.state.back();
        }
        x += KEY + HEADER_GAP;
        if self.key(
            PhosphorIcon::CaretRight,
            key_rect(x),
            self.state.nav.can_go_forward(),
            false,
        ) {
            self.state.forward();
        }
        x += KEY + HEADER_GAP;
        if self.key(
            PhosphorIcon::ArrowUp,
            key_rect(x),
            self.state.nav.can_go_up(),
            false,
        ) {
            self.state.up();
        }
        x += KEY + HEADER_GAP;

        // From the right: new folder, the view toggles, the filter, the pin.
        let mut right = header.right() - HEADER_PAD_RIGHT;
        right -= KEY;
        let can_create = !recent && !self.loading && self.error.is_none();
        if self.key(PhosphorIcon::FolderPlus, key_rect(right), can_create, false) {
            self.state.start_new_folder();
        }
        right -= HEADER_GAP;
        let toggles_w = VIEW_KEY * 2.0 + 2.0 + 4.0;
        right -= toggles_w;
        self.draw_view_toggles(Rect::new(
            right,
            cy - (VIEW_KEY + 4.0) * 0.5,
            toggles_w,
            VIEW_KEY + 4.0,
        ));
        right -= HEADER_GAP;
        right -= SEARCH_W;
        let search = Rect::new(
            right,
            (cy - SearchField::HEIGHT * 0.5).round(),
            SEARCH_W,
            SearchField::HEIGHT,
        );
        self.state.search.placeholder.clear();
        self.state.search.placeholder.push_str("Filter ");
        let current = self.current_name().to_owned();
        self.state.search.placeholder.push_str(&current);
        if SearchField::new().draw(&mut self.state.search, self.ids.search, search, self.ctx) {
            self.state.cache.dirty = true;
        }
        right -= HEADER_GAP + KEY;
        let fixed_here = !recent
            && self
                .dialog
                .places
                .iter()
                .any(|p| p.path == self.state.nav.here.path);
        let pinned = self.state.pinned();
        if self.key(
            PhosphorIcon::Star,
            key_rect(right),
            !recent && !fixed_here,
            pinned,
        ) {
            self.state.toggle_pin();
            self.emit(FileDialogEvent::FavouritesChanged);
        }
        right -= HEADER_GAP;

        let well = Rect::new(x, (cy - WELL_H * 0.5).round(), (right - x).max(0.0), WELL_H);
        self.draw_path_well(well);
    }

    fn draw_view_toggles(&mut self, well: Rect) {
        let list = &mut *self.ctx.draw_list;
        list.rounded_rect(well, 3.0, VIEW_WELL);
        list.rounded_rect_outline(well, 3.0, 1.0, [0.0, 0.0, 0.0, 0.6]);
        let key = |i: f32| {
            Rect::new(
                well.x + 2.0 + i * (VIEW_KEY + 2.0),
                well.y + 2.0,
                VIEW_KEY,
                VIEW_KEY,
            )
        };
        for (i, view, icon) in [
            (0.0, FileView::List, PhosphorIcon::List),
            (1.0, FileView::Grid, PhosphorIcon::SquaresFour),
        ] {
            let clicked = IconKey::new(icon, VIEW_KEY)
                .tone(Tone::Ghost)
                .hollow(true)
                .held(self.state.view == view)
                .draw(key(i), self.ctx)
                .clicked;
            if clicked {
                self.state.view = view;
            }
        }
    }

    fn draw_path_well(&mut self, well: Rect) {
        let s = self.ctx.styles();
        if self.state.editing_path {
            let field = &mut self.state.path_field;
            if !self.ctx.focus.is_focused(self.ids.path) && self.state.path_error.is_none() {
                // Opening: fill it with where the dialog is.
                if field.value.is_empty() || self.state.focus_next.is_none() {
                    field.value.clear();
                    field.value.push_str(self.dialog.root_label);
                    for segment in &self.state.nav.here.path {
                        field.value.push('/');
                        field.value.push_str(segment);
                    }
                    if self.state.nav.here.recent {
                        field.value.push('/');
                    }
                    field.select_all();
                }
            }
            field.x = well.x;
            field.y = well.y;
            field.width = well.width;
            field.height = well.height;
            field.invalid = self.state.path_error.is_some();
            let before = field.value.len();
            let was_focused = self.ctx.focus.is_focused(self.ids.path);
            field.draw(self.ids.path, self.ctx);
            if self.state.path_field.value.len() != before {
                self.state.path_error = None;
            }
            if let Some(error) = &self.state.path_error {
                let w = s.mono_width(self.ctx.draw_list, error, TextSize::Meta);
                self.ctx.draw_list.text(
                    s.mono_block(
                        error.clone(),
                        well.right() - 8.0 - w,
                        line_y(&s, well.y, well.height, TextSize::Meta),
                        TextSize::Meta,
                        Ink::Caption,
                    )
                    .with_color_f32(oklch(0.7, 0.15, HUE_DANGER, 1.0)),
                );
            }
            let focused = self.ctx.focus.is_focused(self.ids.path);
            if focused && self.ctx.input.enter_pressed {
                let path = parse_typed_path(&self.state.path_field.value, self.dialog.root_label);
                self.state.pending_path = Some(path);
            } else if was_focused && !focused && self.state.path_error.is_none() {
                // Left without an error: back to the crumbs.
                self.state.editing_path = false;
            }
            return;
        }

        // The crumbs.
        crate::widgets::material::draw_well(self.ctx.draw_list, &s, well, false, false);
        let recent = self.state.nav.here.recent;
        let size = s.text_size(TextSize::Row);
        let sep_w = s.sans_width(self.ctx.draw_list, "›", TextSize::Meta);
        let mut crumbs: Vec<(String, Option<Vec<String>>, bool)> = Vec::new();
        if recent {
            crumbs.push(("Recent".into(), None, false));
        } else {
            crumbs.push((self.dialog.root_label.to_owned(), Some(Vec::new()), true));
            let path = &self.state.nav.here.path;
            for i in 0..path.len() {
                crumbs.push((path[i].clone(), Some(path[..=i].to_vec()), false));
            }
        }
        // Each crumb as drawn (the last is bold), measured as drawn.
        let last = crumbs.len() - 1;
        let blocks: Vec<TextBlock> = crumbs
            .iter()
            .enumerate()
            .map(|(i, (label, _, mono))| {
                let (ink, font) = (
                    if i == last {
                        s.ink(Ink::Value)
                    } else {
                        s.ink(Ink::Glyph)
                    },
                    if *mono {
                        s.theme().mono_font.clone()
                    } else {
                        s.theme().font.clone()
                    },
                );
                let block = TextBlock::new(
                    label.clone(),
                    0.0,
                    crate::text::vcentered_line_y(well.y, well.height, size),
                )
                .with_size(size)
                .with_color_f32(ink)
                .with_font_opt(font);
                if i == last { block.bold() } else { block }
            })
            .collect();
        let widths: Vec<f32> = blocks
            .iter()
            .map(|b| self.ctx.draw_list.measure_block(b).0.ceil() + 10.0)
            .collect();
        // Earlier crumbs give up width first, down to 14 px each.
        let avail = well.width - 8.0 - sep_w * last as f32 - 2.0;
        let mut fitted = widths;
        let mut over = fitted.iter().sum::<f32>() - avail;
        for w in fitted.iter_mut().take(last) {
            if over <= 0.0 {
                break;
            }
            let give = (*w - 14.0).max(0.0).min(over);
            *w -= give;
            over -= give;
        }
        let mut x = well.x + 4.0;
        let mut on_crumb = false;
        for (i, (((_, path, _), mut block), w)) in
            crumbs.into_iter().zip(blocks).zip(fitted).enumerate()
        {
            let rect = Rect::new(x, well.y + 3.0, w, well.height - 6.0);
            let (hovered, clicked) = self.hit(rect);
            on_crumb |= hovered;
            if i != last && hovered {
                self.ctx.draw_list.rounded_rect(rect, 3.0, HOVER_WASH);
                self.ctx.request_cursor(crate::CursorIcon::Pointer);
            }
            block.x = x + 5.0;
            let block = block.with_max_width((w - 10.0).max(1.0)).with_ellipsis();
            self.ctx.draw_list.text(block);
            if clicked
                && i != last
                && let Some(path) = path
            {
                self.state.go_path(path);
                self.ctx.focus.request(self.ids.listing);
            }
            x += w;
            if i != last {
                self.ctx.draw_list.text(s.sans_block(
                    "›",
                    x,
                    line_y(&s, well.y, well.height, TextSize::Meta),
                    TextSize::Meta,
                    Ink::DisabledGlyph,
                ));
                x += sep_w;
            }
        }
        let (in_well, clicked) = self.hit(well);
        if in_well && !on_crumb {
            self.ctx.request_cursor(crate::CursorIcon::Text);
        }
        if clicked && !on_crumb {
            self.state.edit_path();
        }
    }

    fn draw_places(&mut self, rect: Rect) {
        let s = self.ctx.styles();
        let rule = s.sheet().footer_rule;
        let list = &mut *self.ctx.draw_list;
        list.quad(rect.x, rect.y, rect.width, rect.height, SIDE_SHADE);
        list.edge_line(rect, Edge::Right, rule.thickness, rule.color);
        list.push_clip(rect);
        list.text(s.caption_block(
            "Places",
            rect.x + 12.0,
            rect.y + 8.0,
            Tracking::Caption,
            Ink::Caption,
        ));
        let mut y = rect.y + 8.0 + s.text_size(TextSize::Caption) * 1.5 + 4.0;
        let here = self.state.nav.here.clone();

        if self.dialog.recent.is_some() {
            let on = here.recent;
            if self
                .place_row(
                    rect,
                    y,
                    "Recent",
                    PlaceGlyph::Icon(PhosphorIcon::ClockCounterClockwise),
                    on,
                    false,
                )
                .0
                && !on
            {
                let path = here.path.clone();
                self.state.go(Location { path, recent: true });
            }
            y += ROW;
        }
        for place in self.dialog.places {
            let on = !here.recent && place.path == here.path;
            let glyph = place.icon.map_or(PlaceGlyph::Folder, PlaceGlyph::Icon);
            if self.place_row(rect, y, &place.label, glyph, on, false).0 {
                self.state.go_path(place.path.clone());
            }
            y += ROW;
        }
        let favourites: Vec<FavouriteFolder> = self
            .state
            .favourites
            .iter()
            .filter(|f| !self.dialog.places.iter().any(|p| p.path == f.path))
            .cloned()
            .collect();
        if favourites.is_empty() {
            let note = s
                .mono_block(
                    "☆ in the toolbar pins a folder here",
                    rect.x + 12.0,
                    y + 6.0,
                    TextSize::Meta,
                    Ink::Disabled,
                )
                .with_max_width(rect.width - 24.0);
            self.ctx.draw_list.text(note);
        } else {
            let [dark, light] = PLACES_RULE;
            let list = &mut *self.ctx.draw_list;
            list.quad(rect.x + 12.0, y + 5.0, rect.width - 24.0, 1.0, dark);
            list.quad(rect.x + 12.0, y + 6.0, rect.width - 24.0, 1.0, light);
            y += 11.0;
            for favourite in favourites {
                let label = favourite.label.clone().unwrap_or_else(|| {
                    favourite
                        .path
                        .last()
                        .cloned()
                        .unwrap_or_else(|| self.dialog.root_label.to_owned())
                });
                let on = !here.recent && favourite.path == here.path;
                let (clicked, unpin) = self.place_row(rect, y, &label, PlaceGlyph::Star, on, true);
                if unpin {
                    self.state.unpin(&favourite.path);
                    self.emit(FileDialogEvent::FavouritesChanged);
                } else if clicked {
                    self.state.go_path(favourite.path.clone());
                }
                y += ROW;
            }
        }
        self.ctx.draw_list.pop_clip();
    }

    /// One Places row; returns (clicked, × clicked).
    fn place_row(
        &mut self,
        column: Rect,
        y: f32,
        label: &str,
        glyph: PlaceGlyph,
        on: bool,
        favourite: bool,
    ) -> (bool, bool) {
        let s = self.ctx.styles();
        let row = Rect::new(column.x + 5.0, y, column.width - 10.0, ROW);
        let (hovered, clicked) = self.hit(row);
        let list = &mut *self.ctx.draw_list;
        if on {
            list.rounded_rect(row, 3.0, HELD_WASH);
            list.quad(
                row.x + 3.0,
                row.y,
                row.width - 6.0,
                1.0,
                [1.0, 1.0, 1.0, 0.08],
            );
            list.quad(
                row.x + 3.0,
                row.bottom() - 1.0,
                row.width - 6.0,
                1.0,
                [0.0, 0.0, 0.0, 0.3],
            );
        } else if hovered {
            list.rounded_rect(row, 3.0, s.color(StyleKey::RowHover));
        }
        let glyph_x = row.x + 7.0;
        let glyph_box = Rect::new(glyph_x, row.y + (ROW - 12.0) * 0.5, 15.0, 12.0);
        match glyph {
            PlaceGlyph::Folder => folder_glyph(list, &s, glyph_x + 1.0, row.y + 5.0, 12.0, false),
            PlaceGlyph::Icon(icon) => Icon::new(icon)
                .tint(if on {
                    s.color(StyleKey::AccentGlyph)
                } else {
                    s.ink(Ink::Muted)
                })
                .draw(glyph_box, list),
            PlaceGlyph::Star => Icon::new(PhosphorIcon::Star)
                .tint(if on {
                    s.color(StyleKey::WarnMeta)
                } else {
                    oklch(0.78, 0.1, HUE_WARN_INK, 1.0)
                })
                .draw(glyph_box, list),
        }
        let text_x = glyph_x + 15.0 + 7.0;
        let unpin_w = if favourite && hovered { 18.0 } else { 0.0 };
        let mut block = s
            .sans_block(
                label,
                text_x,
                line_y(&s, row.y, ROW, TextSize::Menu),
                TextSize::Menu,
                if on { Ink::Max } else { Ink::Title },
            )
            .with_max_width((row.right() - 3.0 - unpin_w - text_x).max(1.0))
            .with_ellipsis();
        if on {
            block = block.bold();
        }
        list.text(block);
        let mut unpin = false;
        if favourite && hovered {
            let key = Rect::new(row.right() - 3.0 - 16.0, row.y + 3.0, 16.0, 16.0);
            unpin = IconKey::new(PhosphorIcon::X, 16.0)
                .tone(Tone::Ghost)
                .hollow(true)
                .travel(1.0)
                .draw(key, self.ctx)
                .clicked;
        }
        if hovered {
            self.ctx.request_cursor(crate::CursorIcon::Pointer);
        }
        if clicked && !unpin {
            self.ctx.focus.request(self.ids.listing);
        }
        (clicked && !unpin, unpin)
    }

    fn draw_middle(&mut self, rect: Rect, entries: &[FileEntry]) {
        let strip = Rect::new(rect.x, rect.y, rect.width, TYPE_STRIP_H);
        self.draw_type_strip(strip);
        let area = Rect::new(
            rect.x,
            strip.bottom(),
            rect.width,
            (rect.height - strip.height).max(0.0),
        );
        self.ctx
            .draw_list
            .quad(area.x, area.y, area.width, area.height, LIST_SHADE);
        self.draw_listing(area, entries);
    }

    fn draw_type_strip(&mut self, strip: Rect) {
        let s = self.ctx.styles();
        self.ctx
            .draw_list
            .edge_line(strip, Edge::Bottom, 1.0, STRIP_RULE);
        // The Hidden toggle, at the right.
        let hidden = self.state.cache.shown.hidden;
        let label = if hidden > 0 {
            format!("Hidden · {hidden}")
        } else {
            "Hidden".to_owned()
        };
        let check = Checkbox::new().focusable(self.ids.hidden);
        let (check_w, check_h) = check.intrinsic_size(&label, self.ctx.draw_list, &s);
        let check_w = check_w.ceil();
        let check_rect = Rect::new(
            strip.right() - 6.0 - check_w,
            (strip.y + (strip.height - check_h) * 0.5).round(),
            check_w,
            check_h,
        );
        if check.draw(self.state.show_hidden, &label, check_rect, self.ctx) {
            self.state.show_hidden = !self.state.show_hidden;
            self.state.cache.dirty = true;
        }

        let left = strip.x + 10.0;
        let limit = check_rect.x - 8.0;
        let caption_y = line_y(&s, strip.y, strip.height, TextSize::Meta);
        let caption = |text: String| {
            s.mono_block(text, left, caption_y, TextSize::Meta, Ink::Dim)
                .with_max_width((limit - left).max(1.0))
                .with_ellipsis()
        };
        let types = self.state.cache.shown.types.clone();
        if self.folder_mode() {
            self.ctx
                .draw_list
                .text(caption("Folders only · files shown for context".into()));
            return;
        }
        match types.len() {
            0 => {
                if !self.loading && self.error.is_none() {
                    let accept = normalise_accept(self.dialog.accept);
                    let text = if accept.is_empty() {
                        "No files here".to_owned()
                    } else {
                        format!("No .{} files here", accept.join(" / ."))
                    };
                    self.ctx.draw_list.text(caption(text));
                }
            }
            1 => {
                let (ext, count) = &types[0];
                let lead = "Showing ";
                let lead_w = s.mono_width(self.ctx.draw_list, lead, TextSize::Meta);
                let ext_text = format!(".{ext}");
                let ext_w = s.mono_width(self.ctx.draw_list, &ext_text, TextSize::Meta);
                let list = &mut *self.ctx.draw_list;
                list.text(caption(lead.into()));
                list.text(s.mono_block(
                    ext_text,
                    left + lead_w,
                    caption_y,
                    TextSize::Meta,
                    Ink::Value,
                ));
                list.text(s.mono_block(
                    format!(" · {count}"),
                    left + lead_w + ext_w,
                    caption_y,
                    TextSize::Meta,
                    Ink::Dim,
                ));
            }
            n => {
                let shown = if n > 3 && !self.state.more_types {
                    3
                } else {
                    n
                };
                let mut x = left - 4.0;
                let chip_y = strip.y + (strip.height - FILTER_CHIP_HEIGHT) * 0.5;
                for (ext, count) in types.iter().take(shown) {
                    let label = format!(".{ext}");
                    let chip = FilterChip::new(&label)
                        .count(*count)
                        .on(!self.state.off.contains(ext));
                    let w = chip.width(self.ctx.draw_list, &s);
                    if chip.draw(Rect::new(x, chip_y, w, FILTER_CHIP_HEIGHT), self.ctx) {
                        if let Some(i) = self.state.off.iter().position(|t| t == ext) {
                            self.state.off.remove(i);
                        } else {
                            self.state.off.push(ext.clone());
                        }
                        self.state.cache.dirty = true;
                    }
                    x += w + 4.0;
                }
                if n > 3 {
                    let label = if self.state.more_types {
                        "fewer".to_owned()
                    } else {
                        format!("+{}", n - 3)
                    };
                    let chip = FilterChip::new(&label);
                    let w = chip.width(self.ctx.draw_list, &s);
                    if chip.draw(Rect::new(x, chip_y, w, FILTER_CHIP_HEIGHT), self.ctx) {
                        self.state.more_types = !self.state.more_types;
                    }
                }
            }
        }
    }

    fn draw_listing(&mut self, area: Rect, entries: &[FileEntry]) {
        let s = self.ctx.styles();
        let recent = self.state.nav.here.recent;
        let list_view = self.state.view == FileView::List;
        let mut rows_rect = area;
        if list_view {
            let head = Rect::new(area.x, area.y, area.width, COLUMN_HEAD_H);
            self.draw_column_head(head, recent);
            rows_rect.y += COLUMN_HEAD_H;
            rows_rect.height = (rows_rect.height - COLUMN_HEAD_H).max(0.0);
        }

        // Focus: a click in the rows takes it; Tab reaches it.
        self.ctx.register_focus(self.ids.listing);
        let (_, clicked_in) = self.hit(rows_rect);
        if clicked_in {
            self.ctx.focus.request(self.ids.listing);
        }
        let focused = self.ctx.focus.is_focused(self.ids.listing);

        let mut input: InputState = self.ctx.input.clone();
        if focused {
            self.listing_keys(&mut input, entries);
        }

        if self.state.new_folder.is_some() {
            let row = Rect::new(
                rows_rect.x,
                rows_rect.y,
                rows_rect.width,
                if list_view { ROW } else { GRID_CELL_H },
            );
            self.draw_new_folder_row(row, entries);
            rows_rect.y += row.height;
            rows_rect.height = (rows_rect.height - row.height).max(0.0);
        }

        if self.loading {
            let list = &mut *self.ctx.draw_list;
            for i in 0..7 {
                let y = rows_rect.y + i as f32 * ROW;
                list.rounded_rect(
                    Rect::new(rows_rect.x + 9.0, y + 5.0, 14.0, 12.0),
                    2.0,
                    SKELETON,
                );
                list.rounded_rect(
                    Rect::new(rows_rect.x + 31.0, y + 7.5, skeleton_width(i), 7.0),
                    2.0,
                    SKELETON,
                );
            }
            return;
        }

        let rows = self.state.cache.shown.rows.clone();
        let selected_row = self.selected(entries).map(|(row, _)| row);
        self.state.list.sync_selection(selected_row);
        let folder_mode = self.folder_mode();
        let dimmed = |row: usize| folder_mode && entries[rows[row]].kind == FileKind::File;
        let grid_rect = if list_view {
            rows_rect
        } else {
            rows_rect.inset(GRID_PAD)
        };
        let columns = if list_view {
            1
        } else {
            ((grid_rect.width + GRID_GAP) / GRID_CELL).floor().max(1.0) as usize
        };
        self.state.geometry = ListingGeometry {
            columns,
            pitch: if list_view {
                ROW
            } else {
                GRID_CELL_H + GRID_GAP
            },
            height: grid_rect.height,
        };
        let mut widget = List::new()
            .with_item_height(if list_view { ROW } else { GRID_CELL_H })
            .with_zebra(list_view)
            .focused(focused)
            .disabled(&dimmed)
            .overlay_scrollbar();
        if !list_view {
            widget = widget.columns(columns).with_gap(GRID_GAP, GRID_GAP);
        }
        let clock = self.dialog.clock;
        let root_label = self.dialog.root_label;
        let out = widget.draw(
            grid_rect,
            rows.len(),
            &mut self.state.list,
            self.ctx.draw_list,
            &s,
            &mut input,
            |list, cell, item| {
                let entry = &entries[rows[item.index]];
                let on = item.selected && item.focused;
                let alpha = if item.disabled {
                    DIM_ALPHA
                } else if is_hidden(entry) {
                    HIDDEN_ALPHA
                } else {
                    1.0
                };
                list.push_tint();
                list.multiply_tint([1.0, 1.0, 1.0, alpha]);
                if list_view {
                    list_row(
                        list,
                        &s,
                        cell,
                        entry,
                        item.selected,
                        on,
                        recent,
                        clock,
                        root_label,
                    );
                } else {
                    grid_cell(list, &s, cell, entry, item.selected, on);
                }
                list.pop_tint();
            },
        );

        if let Some(row) = self.state.list.single_selected()
            && let Some(&i) = rows.get(row)
            && self.state.selected.as_deref() != Some(entries[i].name.as_str())
        {
            self.choose(&entries[i]);
        }
        if let Some(row) = out.activated
            && let Some(&i) = rows.get(row)
        {
            let entry = entries[i].clone();
            match entry.kind {
                FileKind::Folder => self.enter(&entry),
                FileKind::File => match self.dialog.mode {
                    FileDialogMode::Open => self.enter(&entry),
                    FileDialogMode::Save => self.confirm(entries, false),
                    FileDialogMode::Folder => {}
                },
            }
        }

        self.draw_state_box(rows_rect, entries);
    }

    /// Select `entry`; saving, a file's name goes into the name field.
    fn choose(&mut self, entry: &FileEntry) {
        self.state.selected = Some(entry.name.clone());
        if self.dialog.mode == FileDialogMode::Save && entry.kind == FileKind::File {
            self.state.name.value.clear();
            self.state.name.value.push_str(&entry.name);
            self.state.name.cursor_pos = entry.name.len();
            self.state.name.selection_start = None;
        }
    }

    /// The listing's own keys while it has focus. Keys used here are taken
    /// out of `input`, so the list doesn't also act on them.
    fn listing_keys(&mut self, input: &mut InputState, entries: &[FileEntry]) {
        let ctrl = input.ctrl_pressed;
        if ctrl && input.nav.up {
            input.nav.up = false;
            self.state.up();
            return;
        }
        if input.alt_down && (input.key_left || input.nav.left) {
            input.nav.left = false;
            self.state.back();
            return;
        }
        if input.alt_down && (input.key_right || input.nav.right) {
            input.nav.right = false;
            self.state.forward();
            return;
        }
        if input.backspace_pressed {
            self.state.up();
            return;
        }
        let space = input.text_input == " ";
        if input.nav.confirm && !(space && self.state.type_ahead.typing()) {
            // Enter: into a folder, else the primary key.
            input.nav.confirm = false;
            match self.selected(entries).map(|(_, e)| e.clone()) {
                Some(entry) if entry.kind == FileKind::Folder => self.enter(&entry),
                _ => self.confirm(entries, true),
            }
            return;
        }
        input.nav.confirm = false;
        if ctrl || input.alt_down || input.text_input.is_empty() {
            return;
        }
        if input.text_input == "/" {
            self.state.edit_path();
            return;
        }
        let rows = self.state.cache.shown.rows.clone();
        let prefix = self.state.type_ahead.push(&input.text_input).to_owned();
        if let Some(row) = find_prefix(&rows, entries, &prefix) {
            let entry = &entries[rows[row]];
            if self.pickable(entry) {
                let entry = entry.clone();
                self.choose(&entry);
                self.state.list.sync_selection(Some(row));
                self.state.geometry.reveal(&mut self.state.list.scroll, row);
            }
        }
    }

    fn draw_column_head(&mut self, head: Rect, recent: bool) {
        let s = self.ctx.styles();
        let list = &mut *self.ctx.draw_list;
        list.vertical_gradient(head, [1.0, 1.0, 1.0, 0.07], [1.0, 1.0, 1.0, 0.02]);
        list.quad(head.x, head.y, head.width, 1.0, [1.0, 1.0, 1.0, 0.1]);
        let rule = s.sheet().footer_rule;
        list.edge_line(head, Edge::Bottom, rule.thickness, rule.color);
        let kind_label = if recent { "Folder" } else { "Kind" };
        let right = head.right();
        let columns = [
            (
                SortBy::Name,
                "Name",
                head.x,
                right - KIND_W - SIZE_W - MODIFIED_W,
                false,
            ),
            (
                SortBy::Modified,
                "Modified",
                right - KIND_W - SIZE_W - MODIFIED_W,
                MODIFIED_W,
                true,
            ),
            (SortBy::Size, "Size", right - KIND_W - SIZE_W, SIZE_W, true),
            (SortBy::Kind, kind_label, right - KIND_W, KIND_W, true),
        ];
        for (by, label, x, w, align_right) in columns {
            let cell = Rect::new(x, head.y, w.max(0.0), head.height);
            let active = !recent && self.state.sort.by == by;
            let mut text = label.to_uppercase();
            if active {
                text.push_str(if self.state.sort.ascending {
                    " ▾"
                } else {
                    " ▴"
                });
            }
            let size = s.text_size(TextSize::Caption);
            let mut block = s
                .mono_block(
                    text,
                    0.0,
                    crate::text::vcentered_line_y(cell.y, cell.height, size),
                    TextSize::Caption,
                    if active { Ink::Menu } else { Ink::Body2 },
                )
                .with_letter_spacing(size * 0.12)
                .with_shadow(0, 0, 0, 153, 0.0, -1.0, 0.0);
            let (tw, _) = self.ctx.draw_list.measure_block(&block);
            block.x = if align_right {
                cell.right() - CELL_PAD - tw
            } else {
                cell.x + CELL_PAD
            };
            self.ctx.draw_list.text(block);
            let (hovered, clicked) = self.hit(cell);
            if hovered && !recent {
                self.ctx.request_cursor(crate::CursorIcon::Pointer);
            }
            if clicked && !recent {
                self.state.sort.toggle(by);
                self.state.cache.dirty = true;
                self.ctx.focus.request(self.ids.listing);
            }
        }
    }

    fn draw_new_folder_row(&mut self, row: Rect, entries: &[FileEntry]) {
        let s = self.ctx.styles();
        self.ctx.draw_list.quad(
            row.x,
            row.y,
            row.width,
            row.height,
            s.color(StyleKey::RowHover),
        );
        let error = self
            .state
            .new_folder
            .as_ref()
            .and_then(|f| bad_folder_name(&f.value, entries));
        let list_view = self.state.view == FileView::List;
        let field_rect = if list_view {
            folder_glyph(
                self.ctx.draw_list,
                &s,
                row.x + 9.0,
                row.y + 4.0,
                14.0,
                false,
            );
            let x = row.x + 9.0 + glyph_width(14.0) + 7.0;
            let right = row.right() - KIND_W - SIZE_W - MODIFIED_W;
            Rect::new(x, row.y + 2.0, (right - x).max(40.0), 18.0)
        } else {
            folder_glyph(
                self.ctx.draw_list,
                &s,
                row.x + 8.0 + (GRID_CELL - 8.0 - glyph_width(40.0)) * 0.5,
                row.y + 12.0,
                40.0,
                false,
            );
            Rect::new(row.x + 8.0, row.y + 64.0, GRID_CELL - 8.0, 18.0)
        };
        if list_view {
            let note = error.unwrap_or("↵ create · esc cancel");
            let color = if error.is_some() {
                oklch(0.7, 0.15, HUE_DANGER, 1.0)
            } else {
                s.ink(Ink::Dim)
            };
            let right = row.right() - CELL_PAD;
            let room = KIND_W + SIZE_W + MODIFIED_W - CELL_PAD * 2.0;
            mono_right(
                self.ctx.draw_list,
                &s,
                note,
                right,
                row.y,
                row.height,
                room,
                color,
            );
        }
        let Some(field) = self.state.new_folder.as_mut() else {
            return;
        };
        field.x = field_rect.x;
        field.y = field_rect.y;
        field.width = field_rect.width;
        field.height = field_rect.height;
        field.invalid = error.is_some();
        field.draw(self.ids.new_folder, self.ctx);
        let focused = self.ctx.focus.is_focused(self.ids.new_folder);
        let commit = (focused && self.ctx.input.enter_pressed)
            || (self.state.new_folder_focused && !focused);
        self.state.new_folder_focused = focused;
        if !commit {
            return;
        }
        let name = self
            .state
            .new_folder
            .as_ref()
            .map(|f| f.value.trim().to_owned())
            .unwrap_or_default();
        let error = bad_folder_name(&name, entries);
        if name.is_empty() || error.is_some() {
            if !focused {
                // Left with nothing usable: drop the row.
                self.state.new_folder = None;
            }
            return;
        }
        self.state.new_folder = None;
        self.state.new_folder_focused = false;
        self.state.selected = Some(name.clone());
        self.state.focus_next = Some(Target::Listing);
        self.emit(FileDialogEvent::CreateFolder {
            path: self.state.nav.here.path.clone(),
            name,
        });
    }

    /// The loading, error, empty and nothing-matches states over the rows.
    fn draw_state_box(&mut self, rect: Rect, entries: &[FileEntry]) {
        let s = self.ctx.styles();
        let recent = self.state.nav.here.recent;
        let shown = &self.state.cache.shown;
        let creating = self.state.new_folder.is_some();
        let query = self.state.search.value.trim().to_owned();
        let current = self.current_name().to_owned();
        let (title, hint, action): (String, String, Option<StateAction>) =
            if let Some(error) = &self.error {
                (format!("Can't open {current}"), error.clone(), None)
            } else if entries.is_empty() && !creating {
                let title = if recent {
                    "No recent files"
                } else {
                    "Empty folder"
                };
                let hint = if self.folder_mode() || recent {
                    String::new()
                } else {
                    format!("Nothing in {current}")
                };
                (title.into(), hint, None)
            } else if shown.rows.is_empty() && !creating {
                let title = if query.is_empty() {
                    "Nothing to show"
                } else {
                    "No matches"
                };
                let accept = normalise_accept(self.dialog.accept);
                let toggled_off = !self.state.off.is_empty() && !shown.types.is_empty();
                let hint = if !query.is_empty() {
                    format!("for “{query}” in {current}")
                } else if toggled_off {
                    "All file types are toggled off".into()
                } else if shown.other_files > 0 {
                    format!(
                        "{} other file{} · not .{}",
                        shown.other_files,
                        if shown.other_files > 1 { "s" } else { "" },
                        accept.join(" / .")
                    )
                } else if shown.hidden > 0 {
                    format!("{} hidden", shown.hidden)
                } else {
                    String::new()
                };
                let action = if toggled_off {
                    Some(StateAction::ShowAllTypes)
                } else if query.is_empty() && shown.hidden > 0 && !self.state.show_hidden {
                    Some(StateAction::ShowHidden)
                } else {
                    None
                };
                (title.into(), hint, action)
            } else {
                return;
            };

        let caption = s.caption_block(&title, 0.0, 0.0, Tracking::Caption, Ink::Caption);
        let caption = if self.error.is_some() {
            caption.with_color_f32(oklch(0.7, 0.15, HUE_DANGER, 1.0))
        } else {
            caption
        };
        let hint_block = (!hint.is_empty()).then(|| {
            s.mono_block(hint, 0.0, 0.0, TextSize::Meta, Ink::Dim)
                .with_max_width((rect.width - 48.0).max(1.0))
                .with_align(crate::TextAlign::Center)
        });
        let list = &mut *self.ctx.draw_list;
        let (cw, ch) = list.measure_block(&caption);
        let (hw, hh) = hint_block
            .as_ref()
            .map_or((0.0, 0.0), |b| list.measure_block(b));
        let button_h = s.scalar(StyleKey::ButtonHeight);
        let action_label = action.map(StateAction::label);
        let total = ch
            + if hint_block.is_some() { 6.0 + hh } else { 0.0 }
            + if action.is_some() {
                6.0 + button_h
            } else {
                0.0
            };
        let mut y = rect.y + ((rect.height - total) * 0.5).max(0.0);
        let mut caption = caption;
        caption.x = rect.x + (rect.width - cw) * 0.5;
        caption.y = y;
        list.text(caption);
        y += ch + 6.0;
        if let Some(mut block) = hint_block {
            block.x = rect.x + (rect.width - hw.min(rect.width - 48.0)) * 0.5;
            block.y = y;
            list.text(block);
            y += hh + 6.0;
        }
        if let (Some(action), Some(label)) = (action, action_label) {
            let key = Button::new(label).tone(Tone::Ghost);
            let (w, _) = key.intrinsic_size(self.ctx.draw_list, &s);
            let w = w.ceil();
            if key.draw(
                Rect::new(rect.x + (rect.width - w) * 0.5, y, w, button_h),
                self.ctx,
            ) {
                match action {
                    StateAction::ShowAllTypes => self.state.off.clear(),
                    StateAction::ShowHidden => self.state.show_hidden = true,
                }
                self.state.cache.dirty = true;
            }
        }
    }

    fn draw_preview(&mut self, rect: Rect, entries: &[FileEntry]) {
        if rect.width <= 0.0 {
            return;
        }
        let s = self.ctx.styles();
        let rule = s.sheet().footer_rule;
        let selected = self.selected(entries).map(|(_, e)| e.clone());
        let name = selected
            .as_ref()
            .map_or_else(|| self.current_name().to_owned(), |e| e.name.clone());
        let list = &mut *self.ctx.draw_list;
        list.quad(rect.x, rect.y, rect.width, rect.height, SIDE_SHADE);
        list.edge_line(rect, Edge::Left, rule.thickness, rule.color);
        list.push_clip(rect);
        let inner = rect.inset(12.0);
        let well = Rect::new(inner.x, inner.y, inner.width, 132.0);
        list.rounded_rect(well, 3.0, s.color(StyleKey::WellDeep));
        list.rounded_rect_outline(well, 3.0, 1.0, s.color(StyleKey::EdgeHard));
        list.vertical_gradient(
            Rect::new(well.x + 1.0, well.y + 1.0, well.width - 2.0, 6.0),
            [0.0, 0.0, 0.0, 0.35],
            [0.0, 0.0, 0.0, 0.0],
        );
        let icon_size = match &selected {
            Some(e) if e.thumb.is_some() => 112.0,
            Some(_) => 56.0,
            None => 48.0,
        };
        let icon_w = if selected.as_ref().is_some_and(|e| e.thumb.is_some()) {
            icon_size
        } else {
            glyph_width(icon_size)
        };
        let ix = well.x + (well.width - icon_w) * 0.5;
        let iy = well.y + (well.height - icon_size) * 0.5;
        match &selected {
            Some(e) => entry_icon(list, &s, e, ix, iy, icon_size, false),
            None => folder_glyph(list, &s, ix, iy, icon_size, false),
        }
        let mut y = well.bottom() + 10.0;
        let title = TextBlock::new(name, inner.x, y)
            .with_size(12.0)
            .with_color_f32(s.ink(Ink::Max))
            .with_font_opt(s.theme().font.clone())
            .with_max_width(inner.width)
            .bold();
        let (_, th) = list.measure_block(&title);
        list.text(title);
        y += th + 10.0;

        let shown = &self.state.cache.shown;
        let mut rows: Vec<(String, String)> = Vec::new();
        match &selected {
            None => {
                let items = if shown.total != shown.rows.len() {
                    format!("{} of {}", shown.rows.len(), shown.total)
                } else {
                    shown.rows.len().to_string()
                };
                rows.push(("Items".into(), items));
                if shown.hidden > 0 {
                    rows.push(("Hidden".into(), shown.hidden.to_string()));
                }
            }
            Some(e) => {
                let kind = match e.kind {
                    FileKind::Folder => "Folder".to_owned(),
                    FileKind::File => {
                        let ext = ext_of(&e.name);
                        format!(
                            "{} file",
                            if ext.is_empty() {
                                "file".to_owned()
                            } else {
                                ext.to_uppercase()
                            }
                        )
                    }
                };
                rows.push(("Kind".into(), kind));
                match e.kind {
                    FileKind::Folder => {
                        if let Some(count) = e.count {
                            rows.push(("Items".into(), count.to_string()));
                        }
                    }
                    FileKind::File => rows.push(("Size".into(), format_size(e.size))),
                }
                rows.push((
                    "Modified".into(),
                    format_date(e.modified, self.dialog.clock),
                ));
                if let (true, Some(dir)) = (self.state.nav.here.recent, &e.dir) {
                    let mut where_ = self.dialog.root_label.to_owned();
                    for segment in dir {
                        where_.push('/');
                        where_.push_str(segment);
                    }
                    rows.push(("Where".into(), where_));
                }
                rows.extend(e.info.iter().cloned());
            }
        }
        for (key, value) in rows {
            list.text(s.caption_block(&key, inner.x, y + 1.0, Tracking::Caption, Ink::Label));
            let value = s
                .mono_block(value, inner.x + 68.0, y, TextSize::Meta, Ink::Value)
                .with_max_width((inner.width - 68.0).max(1.0));
            let (_, vh) = list.measure_block(&value);
            list.text(value);
            y += vh.max(12.0) + 5.0;
        }
        if selected.is_none() {
            let size = s.text_size(TextSize::Meta);
            list.text(s.mono_block(
                "Select an item to preview",
                inner.x,
                inner.bottom() - size * 1.4,
                TextSize::Meta,
                Ink::Disabled,
            ));
        }
        list.pop_clip();
    }

    fn draw_footer(&mut self, footer: Rect, radius: f32, entries: &[FileEntry]) -> Option<Rect> {
        let s = self.ctx.styles();
        let chrome = s.sheet();
        let list = &mut *self.ctx.draw_list;
        list.paint_quad_background(
            footer,
            chrome.footer,
            CornerRadii::new(0.0, 0.0, radius.max(0.0), radius.max(0.0)),
        );
        let rule = chrome.footer_rule;
        list.edge_line(footer, Edge::Top, rule.thickness, rule.color);
        let key_h = s.scalar(StyleKey::ButtonHeight);
        let key_y = footer.y + rule.thickness + FOOTER_PAD_Y;
        let mut x = footer.x + FOOTER_PAD_X;

        // The keys, from the right.
        let primary = self.primary(entries);
        let enabled = self.primary_enabled(entries);
        let primary_key = Button::new(primary.label())
            .tone(if primary == Primary::Replace {
                Tone::Danger
            } else {
                Tone::Accent
            })
            .enabled(enabled)
            .focusable(self.ids.primary);
        let cancel_key = Button::new("Cancel").focusable(self.ids.cancel);
        let pw = primary_key.intrinsic_size(self.ctx.draw_list, &s).0.ceil();
        let cw = cancel_key.intrinsic_size(self.ctx.draw_list, &s).0.ceil();
        let primary_x = footer.right() - FOOTER_PAD_X - pw;
        let cancel_x = primary_x - 8.0 - cw;

        let slot = (self.dialog.footer_slot > 0.0).then(|| {
            let slot = Rect::new(x, key_y, self.dialog.footer_slot, key_h);
            x += self.dialog.footer_slot + 12.0;
            slot
        });
        let limit = (cancel_x - 8.0).max(x);
        let text_y = line_y(&s, key_y, key_h, TextSize::Meta);
        match self.dialog.mode {
            FileDialogMode::Save => self.draw_name_field(x, key_y, key_h, limit, entries),
            FileDialogMode::Folder => {
                let lead = "Choose ";
                let lead_w = s.mono_width(self.ctx.draw_list, lead, TextSize::Meta);
                let mut target = self.dialog.root_label.to_owned();
                for segment in self.folder_target(entries) {
                    target.push('/');
                    target.push_str(&segment);
                }
                let list = &mut *self.ctx.draw_list;
                list.text(s.mono_block(lead, x, text_y, TextSize::Meta, Ink::Dim));
                list.text(
                    s.mono_block(target, x + lead_w, text_y, TextSize::Meta, Ink::Value)
                        .with_max_width((limit - x - lead_w).max(1.0))
                        .with_ellipsis(),
                );
            }
            FileDialogMode::Open => {
                let text = match self.selected(entries) {
                    Some((_, e)) if e.kind == FileKind::File => {
                        s.mono_block(e.name.clone(), x, text_y, TextSize::Meta, Ink::Value)
                    }
                    _ => {
                        let shown = &self.state.cache.shown;
                        let n = shown.rows.len();
                        let mut text = format!("{n} item{}", if n == 1 { "" } else { "s" });
                        if shown.hidden > 0 && !self.state.show_hidden {
                            text.push_str(&format!(" · {} hidden", shown.hidden));
                        }
                        s.mono_block(text, x, text_y, TextSize::Meta, Ink::Dim)
                    }
                };
                self.ctx
                    .draw_list
                    .text(text.with_max_width((limit - x).max(1.0)).with_ellipsis());
            }
        }

        if cancel_key.draw(Rect::new(cancel_x, key_y, cw, key_h), self.ctx) {
            self.emit(FileDialogEvent::Cancel);
        }
        if primary_key.draw(Rect::new(primary_x, key_y, pw, key_h), self.ctx) {
            self.confirm(entries, false);
        }
        slot
    }

    fn draw_name_field(
        &mut self,
        x: f32,
        key_y: f32,
        key_h: f32,
        limit: f32,
        entries: &[FileEntry],
    ) {
        let s = self.ctx.styles();
        let label = s.caption_block(
            "Name",
            x,
            line_y(&s, key_y, key_h, TextSize::Caption),
            Tracking::Caption,
            Ink::Label,
        );
        let (lw, _) = self.ctx.draw_list.measure_block(&label);
        self.ctx.draw_list.text(label);
        let field_x = x + lw + 8.0;
        let field_w = NAME_W.min((limit - field_x).max(40.0));
        let height = s.scalar(StyleKey::InputHeight);
        let ext = self
            .dialog
            .save_ext
            .map(|e| e.trim_start_matches('.').to_lowercase());
        let suffix = ext
            .as_ref()
            .filter(|ext| ext_of(&self.state.name.value) != **ext)
            .map(|ext| format!(".{ext}"));
        let error = self.name_error();
        let field = &mut self.state.name;
        field.x = field_x;
        field.y = key_y + (key_h - height) * 0.5;
        field.width = field_w;
        field.height = height;
        field.invalid = error.is_some();
        field.insets = suffix.as_ref().map(|_| [7.0, 44.0]);
        field.draw(self.ids.name, self.ctx);
        if let Some(suffix) = suffix {
            let w = s.mono_width(self.ctx.draw_list, &suffix, TextSize::Meta);
            self.ctx.draw_list.text(s.mono_block(
                suffix,
                field_x + field_w - 8.0 - w,
                line_y(&s, key_y, key_h, TextSize::Meta),
                TextSize::Meta,
                Ink::Dim,
            ));
        }
        let entered = self.ctx.focus.is_focused(self.ids.name) && self.ctx.input.enter_pressed;
        let replace = self.will_replace(entries);
        let note_x = field_x + field_w + 8.0;
        let (note, color) = if let Some(error) = error {
            (error.to_owned(), oklch(0.7, 0.15, HUE_DANGER, 1.0))
        } else if replace {
            (
                "Replaces the existing file".to_owned(),
                s.color(StyleKey::WarnMeta),
            )
        } else if self.state.nav.here.recent {
            ("Pick a folder to save into".to_owned(), s.ink(Ink::Dim))
        } else {
            (format!("in {}", self.current_name()), s.ink(Ink::Dim))
        };
        if limit > note_x {
            self.ctx.draw_list.text(
                s.mono_block(
                    note,
                    note_x,
                    line_y(&s, key_y, key_h, TextSize::Meta),
                    TextSize::Meta,
                    Ink::Dim,
                )
                .with_color_f32(color)
                .with_max_width(limit - note_x)
                .with_ellipsis(),
            );
        }
        if entered {
            self.confirm(entries, false);
        }
    }
}

/// What a Places row shows before its label.
#[derive(Clone, Copy)]
enum PlaceGlyph {
    Folder,
    Icon(PhosphorIcon),
    Star,
}

/// The key under an empty listing's message.
#[derive(Clone, Copy)]
enum StateAction {
    ShowAllTypes,
    ShowHidden,
}

impl StateAction {
    fn label(self) -> &'static str {
        match self {
            Self::ShowAllTypes => "Show all types",
            Self::ShowHidden => "Show hidden",
        }
    }
}

/// One list-view row's content (the list has painted its background).
#[allow(clippy::too_many_arguments)]
fn list_row(
    list: &mut super::DrawList,
    s: &StyleResolver,
    cell: Rect,
    entry: &FileEntry,
    selected: bool,
    on: bool,
    recent: bool,
    clock: Option<Clock>,
    root_label: &str,
) {
    let right = cell.right();
    let name_right = right - KIND_W - SIZE_W - MODIFIED_W;
    entry_icon(
        list,
        s,
        entry,
        cell.x + CELL_PAD,
        cell.y + (cell.height - 14.0) * 0.5,
        14.0,
        on,
    );
    let text_x = cell.x + CELL_PAD + glyph_width(14.0) + 7.0;
    let ink = if on {
        s.color(StyleKey::OnAccent)
    } else if selected {
        s.ink(Ink::Max)
    } else {
        s.ink(Ink::Title)
    };
    let mut name = s
        .sans_block(
            entry.name.clone(),
            text_x,
            line_y(s, cell.y, cell.height, TextSize::Menu),
            TextSize::Menu,
            Ink::Title,
        )
        .with_color_f32(ink)
        .with_max_width((name_right - CELL_PAD - text_x).max(1.0))
        .with_ellipsis();
    if !on {
        name = name.with_shadow(0, 0, 0, 153, 0.0, -1.0, 0.0);
    }
    if selected {
        name = name.bold();
    }
    list.text(name);
    let dim = if on {
        s.ink(Ink::OnAccentSecond)
    } else {
        s.ink(Ink::Caption)
    };
    let modified = format_date(entry.modified, clock);
    let size = match entry.kind {
        FileKind::Folder => entry
            .count
            .map_or_else(|| "—".to_owned(), |n| n.to_string()),
        FileKind::File => format_size(entry.size),
    };
    let kind = if recent {
        entry
            .dir
            .as_ref()
            .and_then(|d| d.last().cloned())
            .unwrap_or_else(|| root_label.to_owned())
    } else {
        match entry.kind {
            FileKind::Folder => "folder".to_owned(),
            FileKind::File => {
                let ext = ext_of(&entry.name);
                if ext.is_empty() {
                    "file".to_owned()
                } else {
                    ext
                }
            }
        }
    };
    let pad = CELL_PAD * 2.0;
    mono_right(
        list,
        s,
        &modified,
        right - KIND_W - SIZE_W - CELL_PAD,
        cell.y,
        cell.height,
        MODIFIED_W - pad,
        dim,
    );
    mono_right(
        list,
        s,
        &size,
        right - KIND_W - CELL_PAD,
        cell.y,
        cell.height,
        SIZE_W - pad,
        dim,
    );
    mono_right(
        list,
        s,
        &kind,
        right - CELL_PAD,
        cell.y,
        cell.height,
        KIND_W - pad,
        dim,
    );
}

/// One grid cell's content.
fn grid_cell(
    list: &mut super::DrawList,
    s: &StyleResolver,
    cell: Rect,
    entry: &FileEntry,
    selected: bool,
    on: bool,
) {
    let size = if entry.thumb.is_some() { 52.0 } else { 40.0 };
    let w = if entry.thumb.is_some() {
        size
    } else {
        glyph_width(size)
    };
    let icon_top = cell.y + 6.0 + (52.0 - size) * 0.5;
    entry_icon(
        list,
        s,
        entry,
        cell.x + (cell.width - w) * 0.5,
        icon_top,
        size,
        on,
    );
    let ink = if on {
        s.color(StyleKey::OnAccent)
    } else {
        s.ink(Ink::Row)
    };
    let text_top = cell.y + 6.0 + 52.0 + 5.0;
    let mut block = TextBlock::new(entry.name.clone(), cell.x + 4.0, text_top)
        .with_size(10.5)
        .with_line_height(13.0)
        .with_color_f32(ink)
        .with_font_opt(s.theme().font.clone())
        .with_max_width(cell.width - 8.0)
        .with_align(crate::TextAlign::Center)
        .with_clip(Rect::new(cell.x, text_top, cell.width, 26.0));
    if selected {
        block = block.bold();
    }
    list.text(block);
}

#[cfg(test)]
mod tests;
