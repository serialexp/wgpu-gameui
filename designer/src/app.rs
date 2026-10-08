//! The designer: a menu bar, the seed editor on the left, the token
//! inspector on the right and the live preview between them.
//!
//! The designer's own chrome always draws in Forge; only the preview (and
//! the swatches) draw in the preset being edited, so a preset that makes
//! text unreadable can't make the designer unusable.

use std::path::{Path, PathBuf};

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{
    AppShell, ConfirmChoice, ConfirmDialog, ConfirmDialogState, CursorIcon, CursorState,
    DockPanelState, DockTab, DragCapture, DrawContext, DropdownState, FocusState, InputState,
    InteractionScene, LayerStack, Menu, MenuBar, MenuBarId, MenuBarState, MenuDrawEnv, MenuItem,
    MenuItemId, PromptDialog, PromptDialogState, PromptOutcome, StatusCell, StyleKey,
    StyleResolver, Theme, ThemePreset, ThemeSeeds, draw_status_bar,
};

use crate::preview::PreviewState;
use crate::seeds::SeedsPanel;
use crate::tokens::TokensPanel;

/// Something to do to the document: from a shortcut or the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Quit,
}

/// What the host needs back from a frame.
pub struct FrameOut {
    /// The window's clear colour.
    pub clear: [f32; 4],
    /// The pointer the UI asked for.
    pub cursor: CursorIcon,
    /// The window title.
    pub title: String,
    /// The designer is done; close the window.
    pub quit: bool,
    /// Seconds until the UI needs another frame without input.
    pub next_frame_in: Option<f32>,
}

const MENU_BAR: MenuBarId = 0xDE51_0000;
const ITEM_NEW: MenuItemId = 1;
const ITEM_OPEN: MenuItemId = 2;
const ITEM_SAVE: MenuItemId = 3;
const ITEM_SAVE_AS: MenuItemId = 4;
const ITEM_QUIT: MenuItemId = 5;
const ITEM_RESET_SEEDS: MenuItemId = 6;
const ITEM_CLEAR_OVERRIDES: MenuItemId = 7;

const FILE_ITEMS: &[MenuItem] = &[
    MenuItem::new("New").id(ITEM_NEW).shortcut("Ctrl+N"),
    MenuItem::new("Open…").id(ITEM_OPEN).shortcut("Ctrl+O"),
    MenuItem::new("Save").id(ITEM_SAVE).shortcut("Ctrl+S"),
    MenuItem::new("Save as…")
        .id(ITEM_SAVE_AS)
        .shortcut("Ctrl+Shift+S"),
    MenuItem::separator(),
    MenuItem::new("Quit").id(ITEM_QUIT).shortcut("Ctrl+Q"),
];
const PRESET_ITEMS: &[MenuItem] = &[
    MenuItem::new("Reset seeds to Forge").id(ITEM_RESET_SEEDS),
    MenuItem::new("Clear overrides").id(ITEM_CLEAR_OVERRIDES),
];
const MENUS: &[Menu] = &[
    Menu::new("File").with_items(FILE_ITEMS),
    Menu::new("Preset").with_items(PRESET_ITEMS),
];

const SEEDS_TAB: &[DockTab] = &[DockTab { label: "Seeds" }];
const TOKENS_TAB: &[DockTab] = &[DockTab { label: "Tokens" }];

/// The dialogs' focus ids (each takes a few after its own).
const ID_PROMPT: u64 = 0xD1A1_0000;
const ID_CONFIRM: u64 = 0xD1A1_0010;

/// Where new presets are saved by default.
const PRESET_DIR: &str = "presets";

/// What the path prompt is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathFor {
    Open,
    SaveAs,
}

pub struct Designer {
    /// The preset being edited.
    preset: ThemePreset,
    /// The preset as last saved or loaded: unsaved changes are the difference.
    saved: ThemePreset,
    path: Option<PathBuf>,
    /// The preset the preview theme was resolved from, and that theme.
    resolved: ThemePreset,
    preview_theme: Theme,
    /// The designer's own chrome: always Forge.
    ui_theme: Theme,

    menu: MenuBarState,
    scene: InteractionScene,
    focus: FocusState,
    dropdowns: DropdownState,
    capture: DragCapture,
    cursor: CursorState,
    left: DockPanelState,
    right: DockPanelState,
    seeds: SeedsPanel,
    tokens: TokensPanel,
    preview: PreviewState,

    prompt: PromptDialogState,
    prompt_for: PathFor,
    confirm: ConfirmDialogState,
    /// What to do once the user agrees to discard unsaved changes.
    after_discard: Option<Command>,
    /// The last thing that happened, for the status bar.
    status: String,
}

impl Designer {
    /// A designer editing the preset at `path`, or a fresh one.
    pub fn new(path: Option<PathBuf>) -> Self {
        let template = Theme::default();
        let mut designer = Self {
            preset: ThemePreset::new("Untitled"),
            saved: ThemePreset::new("Untitled"),
            path: None,
            resolved: ThemePreset::new("Untitled"),
            preview_theme: Theme::default(),
            ui_theme: Theme::default(),
            menu: MenuBarState::new(),
            scene: InteractionScene::new(),
            focus: FocusState::new(),
            dropdowns: DropdownState::new(),
            capture: DragCapture::new(),
            cursor: CursorState::new(),
            left: DockPanelState::new(320.0).with_range(260.0, 480.0),
            right: DockPanelState::new(360.0).with_range(280.0, 560.0),
            seeds: SeedsPanel::new(&template),
            tokens: TokensPanel::new(&template),
            preview: PreviewState::new(),
            prompt: PromptDialogState::new(),
            prompt_for: PathFor::Open,
            confirm: ConfirmDialogState::new(),
            after_discard: None,
            status: "Seeds re-skin Forge; overrides replace single tokens.".to_string(),
        };
        if let Some(path) = path {
            designer.load(&path);
        }
        designer
    }

    /// The window title: the preset, its file, and a mark when unsaved.
    pub fn title(&self) -> String {
        let file = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or("unsaved".into(), |name| name.to_string_lossy());
        let dirty = if self.dirty() { " •" } else { "" };
        format!("{} ({file}){dirty} — gameui designer", self.preset.name)
    }

    /// Open `key` in the token inspector.
    pub fn select_token(&mut self, key: StyleKey) {
        self.tokens.select(key);
    }

    fn dirty(&self) -> bool {
        self.preset != self.saved
    }

    /// Replace the document with the preset at `path`, or report why not.
    fn load(&mut self, path: &Path) {
        let read = std::fs::read_to_string(path).map_err(|e| e.to_string());
        match read.and_then(|text| ThemePreset::from_toml(&text).map_err(|e| e.to_string())) {
            Ok(preset) => {
                self.saved = preset.clone();
                self.preset = preset;
                self.path = Some(path.to_path_buf());
                self.status = format!("Opened {}", path.display());
            }
            Err(error) => self.status = format!("Could not open {}: {error}", path.display()),
        }
    }

    /// Write the document to `path`, or report why not.
    fn save_to(&mut self, path: &Path) {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty())
            && let Err(error) = std::fs::create_dir_all(dir)
        {
            self.status = format!("Could not save {}: {error}", path.display());
            return;
        }
        match std::fs::write(path, self.preset.to_toml()) {
            Ok(()) => {
                self.saved = self.preset.clone();
                self.path = Some(path.to_path_buf());
                self.status = format!("Saved {}", path.display());
            }
            Err(error) => self.status = format!("Could not save {}: {error}", path.display()),
        }
    }

    /// Open the path prompt for `what`, filled with a sensible start.
    fn ask_path(&mut self, what: PathFor) {
        let start = match (&self.path, what) {
            (Some(path), _) => path.display().to_string(),
            (None, PathFor::Open) => format!("{PRESET_DIR}/"),
            (None, PathFor::SaveAs) => format!("{PRESET_DIR}/{}.toml", slug(&self.preset.name)),
        };
        self.prompt_for = what;
        self.prompt.open(&start);
    }

    /// Run `command`; returns whether the designer should quit.
    fn run(&mut self, command: Command) -> bool {
        if self.prompt.is_open() || self.confirm.is_open() {
            return false;
        }
        let discards = matches!(command, Command::New | Command::Open | Command::Quit);
        if discards && self.dirty() {
            self.after_discard = Some(command);
            self.confirm.open();
            return false;
        }
        self.run_discarding(command)
    }

    /// Run `command` without asking about unsaved changes.
    fn run_discarding(&mut self, command: Command) -> bool {
        match command {
            Command::New => {
                self.preset = ThemePreset::new("Untitled");
                self.saved = self.preset.clone();
                self.path = None;
                self.status = "New preset: Forge, unchanged.".to_string();
            }
            Command::Open => self.ask_path(PathFor::Open),
            Command::Save => match self.path.clone() {
                Some(path) => self.save_to(&path),
                None => self.ask_path(PathFor::SaveAs),
            },
            Command::SaveAs => self.ask_path(PathFor::SaveAs),
            Command::Quit => return true,
        }
        false
    }

    /// Re-resolve the preview theme if the preset moved since it was built.
    /// Returns whether it did.
    fn refresh_theme(&mut self) -> bool {
        if self.preset == self.resolved {
            return false;
        }
        self.preview_theme = self.preset.resolve();
        self.resolved.clone_from(&self.preset);
        true
    }

    /// Draw one frame into `layers` and act on `input` and `command`.
    pub fn frame(
        &mut self,
        layers: &mut LayerStack,
        input: &mut InputState,
        screen: Rect,
        command: Option<Command>,
    ) -> FrameOut {
        let mut quit = command.is_some_and(|command| self.run(command));
        self.refresh_theme();

        // Frame-top, in the order the widgets document: claims before focus.
        self.menu.begin_frame(input);
        self.prompt.modal.begin_frame(input);
        self.confirm.modal.begin_frame(input);
        self.dropdowns.begin_frame(input);
        self.scene.begin_frame(input);
        self.focus.begin_frame(input);
        self.cursor.begin_frame();

        let menu_slots = self.menu.push_open_layers(layers);
        let dropdown_layer = self.dropdowns.push_open_layer(layers);
        let dialog_layer = (self.prompt.is_open() || self.confirm.is_open()).then(|| {
            let index = layers.push_modal(screen);
            layers.pop_layer();
            index
        });
        let base = layers.input_for_base(input);
        let (w, h) = (screen.width, screen.height);

        let shell = AppShell::new().with_menu_bar().with_status_bar();
        let styles = StyleResolver::new(&self.ui_theme);
        let lay = shell.layout(
            screen,
            Some(&self.left),
            Some(&self.right),
            None,
            None,
            &styles,
        );

        // The bar alone resolves through the interaction scene (its open
        // chain needs it); everything else hit-tests the base input, which
        // the popup and modal layers have already masked.
        if let Some(bar) = lay.menu_bar {
            let mut ctx = DrawContext::new(
                layers.base_mut(),
                &mut self.focus,
                &self.ui_theme,
                &base,
                w,
                h,
            )
            .with_interactions(&mut self.scene)
            .with_cursor(&mut self.cursor);
            MenuBar::new(MENU_BAR, MENUS).draw(bar, &mut self.menu, &mut ctx);
        }
        {
            let mut ctx = DrawContext::new(
                layers.base_mut(),
                &mut self.focus,
                &self.ui_theme,
                &base,
                w,
                h,
            )
            .with_cursor(&mut self.cursor);
            let chrome = shell.draw_chrome(
                &lay,
                Some((SEEDS_TAB, &mut self.left)),
                Some((TOKENS_TAB, &mut self.right)),
                None,
                None,
                &mut self.capture,
                &mut ctx,
            );
            if let Some(dock) = chrome.left_dock {
                self.seeds.draw(
                    dock.body,
                    &mut self.preset,
                    &self.preview_theme,
                    &mut self.capture,
                    &mut ctx,
                );
            }
            if let Some(dock) = chrome.right_dock {
                self.tokens.draw(
                    dock.body,
                    &mut self.preset,
                    &self.preview_theme,
                    &mut self.capture,
                    &mut ctx,
                );
            }
            {
                let mut preview = DrawContext::new(
                    &mut *ctx.draw_list,
                    &mut *ctx.focus,
                    &self.preview_theme,
                    ctx.input,
                    w,
                    h,
                );
                preview.cursor = ctx.cursor.as_deref_mut();
                self.preview.draw(
                    lay.viewport,
                    &self.preset.name,
                    &mut self.dropdowns,
                    &mut self.capture,
                    &mut preview,
                );
            }
            if let Some(rect) = lay.status_bar {
                let file = self
                    .path
                    .as_ref()
                    .map_or("not saved yet".to_string(), |path| {
                        path.display().to_string()
                    });
                let overrides = match self.preset.overrides().len() {
                    1 => "1 override".to_string(),
                    n => format!("{n} overrides"),
                };
                let dirty = if self.preset != self.saved {
                    StatusCell::text("Unsaved").highlight()
                } else {
                    StatusCell::text("Saved")
                };
                let cells = [
                    StatusCell::text(&self.status),
                    StatusCell::text(&overrides),
                    StatusCell::text(&file),
                    dirty,
                ];
                draw_status_bar(rect, &cells, &mut ctx);
            }
        }

        // After the base: the open menu column, the preview's dropdown list
        // and the dialogs.
        let activated = {
            let mut env = MenuDrawEnv {
                theme: &self.ui_theme,
                style: None,
                input,
                focus: &mut self.focus,
                interactions: &mut self.scene,
                animations: None,
                cursor: Some(&mut self.cursor),
                screen_width: w,
                screen_height: h,
            };
            self.menu
                .draw_open_layers(layers, menu_slots, MENUS, &mut env)
                .map(|item| item.id)
        };
        let preview_styles = StyleResolver::new(&self.preview_theme);
        if let Some((id, index)) =
            self.dropdowns
                .draw_open_layer(layers, dropdown_layer, &preview_styles, input)
        {
            self.preview.dropdown_picked(id, index);
        }
        if let Some(index) = dialog_layer {
            quit |= self.draw_dialogs(layers, index, input, screen);
        }

        self.menu.end_frame(&mut self.focus);
        self.dropdowns.end_frame(&mut self.focus);
        self.scene.end_frame();
        self.focus.end_frame(dialog_layer);

        if let Some(item) = activated {
            quit |= match item {
                ITEM_NEW => self.run(Command::New),
                ITEM_OPEN => self.run(Command::Open),
                ITEM_SAVE => self.run(Command::Save),
                ITEM_SAVE_AS => self.run(Command::SaveAs),
                ITEM_QUIT => self.run(Command::Quit),
                ITEM_RESET_SEEDS => {
                    let name = std::mem::take(&mut self.preset.name);
                    self.preset.seeds = ThemeSeeds::default();
                    self.preset.name = name;
                    false
                }
                ITEM_CLEAR_OVERRIDES => {
                    let keys: Vec<_> = self.preset.overrides().iter().map(|(k, _)| *k).collect();
                    for key in keys {
                        self.preset.clear_override(key);
                    }
                    false
                }
                _ => false,
            };
        }

        // An edit this frame shows in the next one: ask for it now.
        let changed = self.refresh_theme();
        let next_frame_in = [
            changed.then_some(0.0),
            self.seeds.pending_deadline(),
            self.tokens.pending_deadline(),
            self.preview.pending_deadline(),
        ]
        .into_iter()
        .flatten()
        .reduce(f32::min);

        FrameOut {
            clear: self.ui_theme.background,
            cursor: self.cursor.resolve(),
            title: self.title(),
            quit,
            next_frame_in,
        }
    }

    /// The open dialog, on modal layer `index`; returns whether to quit.
    fn draw_dialogs(
        &mut self,
        layers: &mut LayerStack,
        index: usize,
        input: &InputState,
        screen: Rect,
    ) -> bool {
        let layer_input = layers.input_for_layer(index, input);
        let mut ctx = DrawContext::new(
            &mut layers.layers_mut()[index].list,
            &mut self.focus,
            &self.ui_theme,
            &layer_input,
            screen.width,
            screen.height,
        )
        .with_layer(index)
        .with_cursor(&mut self.cursor);

        if self.prompt.is_open() {
            let (title, ok) = match self.prompt_for {
                PathFor::Open => ("Open preset", "Open"),
                PathFor::SaveAs => ("Save preset as", "Save"),
            };
            let outcome = PromptDialog::new(title)
                .label("Path")
                .hint("A .toml preset file, relative to the working directory.")
                .ok_label(ok)
                .required(true)
                .width(440.0)
                .draw(ID_PROMPT, screen, &mut self.prompt, &mut ctx);
            if let Some(PromptOutcome::Submit(path)) = outcome {
                let path = PathBuf::from(path.trim());
                match self.prompt_for {
                    PathFor::Open => self.load(&path),
                    PathFor::SaveAs => self.save_to(&path),
                }
            }
            return false;
        }

        let outcome = ConfirmDialog::new("Discard unsaved changes?")
            .message("The preset has changes that are not saved yet.")
            .confirm_label("Discard")
            .destructive(true)
            .draw(ID_CONFIRM, screen, &mut self.confirm, &mut ctx);
        match outcome.map(|outcome| outcome.choice) {
            Some(ConfirmChoice::Confirm) => self
                .after_discard
                .take()
                .is_some_and(|command| self.run_discarding(command)),
            Some(_) => {
                self.after_discard = None;
                false
            }
            None => false,
        }
    }
}

/// A file name for `name`: lower-case words joined by dashes.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for word in name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if !out.is_empty() {
            out.push('-');
        }
        out.extend(word.chars().flat_map(char::to_lowercase));
    }
    if out.is_empty() {
        out.push_str("preset");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 900.0,
    };

    /// Draw one frame with no input.
    fn frame(
        designer: &mut Designer,
        layers: &mut LayerStack,
        command: Option<Command>,
    ) -> FrameOut {
        layers.clear();
        let mut input = InputState::default();
        let out = designer.frame(layers, &mut input, SCREEN, command);
        input.end_frame();
        out
    }

    /// Scripted input, fed frame by frame the way the window host feeds it.
    struct Rig {
        designer: Designer,
        layers: LayerStack,
        input: InputState,
    }

    impl Rig {
        /// A designer drawn twice, so retained layout has settled.
        fn new() -> Self {
            let mut rig = Self {
                designer: Designer::new(None),
                layers: LayerStack::new(),
                input: InputState::default(),
            };
            rig.frame();
            rig.frame();
            rig
        }

        fn frame(&mut self) -> FrameOut {
            self.layers.clear();
            wgpu_gameui::map_keyboard(&mut self.input);
            let out = self
                .designer
                .frame(&mut self.layers, &mut self.input, SCREEN, None);
            self.input.end_frame();
            out
        }

        /// Every base-layer debug scope called `name` drawn last frame.
        fn scopes(&self, name: &str) -> Vec<Rect> {
            self.layers
                .base()
                .debug_scopes()
                .iter()
                .filter(|scope| scope.name == name)
                .filter_map(|scope| scope.declared)
                .collect()
        }

        /// Press at `(x, y)`, hold for a frame, and let go.
        fn click(&mut self, x: f32, y: f32) {
            self.drag((x, y), (x, y));
        }

        /// Press at `from`, move to `to` with the button held, let go.
        fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
            (self.input.mouse_x, self.input.mouse_y) = from;
            self.input.mouse_down = true;
            self.input.mouse_clicked = true;
            self.frame();
            (self.input.mouse_x, self.input.mouse_y) = to;
            self.frame();
            self.input.mouse_down = false;
            self.input.mouse_released = true;
            self.frame();
        }
    }

    /// The centre-left point a fraction `t` across `rect`.
    fn across(rect: Rect, t: f32) -> (f32, f32) {
        (rect.x + rect.width * t, rect.y + rect.height * 0.5)
    }

    #[test]
    fn dragging_a_seed_slider_moves_the_seed_and_the_preview() {
        let mut rig = Rig::new();
        let forge_padding = rig.designer.preview_theme.padding;
        // The first slider in the seed column is Density (0.6 to 1.6).
        let density = rig
            .scopes("Slider")
            .into_iter()
            .find(|rect| rect.right() <= rig.designer.left.size)
            .expect("the seed column has sliders");
        rig.drag(across(density, 0.5), across(density, 0.9));

        let seeds = &rig.designer.preset.seeds;
        assert!(
            (seeds.density - 1.5).abs() < 0.051,
            "density {}",
            seeds.density
        );
        rig.frame();
        assert!(rig.designer.preview_theme.padding > forge_padding);
        assert!(rig.designer.dirty());
    }

    #[test]
    fn a_hex_typed_into_a_token_overrides_it() {
        let mut rig = Rig::new();
        assert!(rig.scopes("accent").is_empty(), "scrolled out of view");
        let filter = rig.scopes("SearchField")[0];
        rig.click(filter.x + 40.0, filter.y + filter.height * 0.5);
        rig.input.text_input.push_str("accent");
        rig.frame();
        rig.frame();
        assert!(rig.scopes("background").is_empty(), "filtered away");
        let row = rig.scopes("accent")[0];
        rig.click(row.x + row.width * 0.5, row.y + row.height * 0.5);
        rig.frame();
        assert_eq!(rig.scopes("ColorPicker").len(), 1, "the editor opened");

        // The hex field is the lowest text field in the token panel.
        let hex = rig
            .scopes("TextInput")
            .into_iter()
            .filter(|rect| rect.x > SCREEN.width - rig.designer.right.size)
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .expect("the editor has a hex field");
        rig.click(hex.x + 20.0, hex.y + hex.height * 0.5);
        rig.input.key_select_all = true;
        rig.frame();
        rig.input.text_input.push_str("#ff0000");
        rig.frame();

        let red = wgpu_gameui::StyleValue::Color([1.0, 0.0, 0.0, 1.0]);
        assert_eq!(rig.designer.preset.override_of(StyleKey::Accent), Some(red));
        rig.frame();
        assert_eq!(rig.designer.preview_theme.accent, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(rig.designer.ui_theme.accent, Theme::default().accent);
    }

    /// A scratch directory of this test run's own, emptied first.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("gameui-designer-{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn unsaved_changes_are_confirmed_before_they_are_lost() {
        let mut designer = Designer::new(None);
        assert!(designer.run(Command::Quit), "nothing to lose: quit at once");

        designer.preset.seeds.density = 1.2;
        for command in [Command::New, Command::Open, Command::Quit] {
            assert!(!designer.run(command));
            assert!(designer.confirm.is_open(), "{command:?} asks first");
            assert_eq!(designer.after_discard, Some(command));
            designer.confirm = ConfirmDialogState::new();
        }
        assert_eq!(designer.preset.seeds.density, 1.2, "nothing was discarded");

        assert!(!designer.run_discarding(Command::New));
        assert_eq!(designer.preset, ThemePreset::new("Untitled"));
        assert!(!designer.dirty());
    }

    #[test]
    fn a_saved_preset_opens_as_it_was() {
        let path = scratch_dir("designer_round_trip").join("nested/neon.toml");
        let mut designer = Designer::new(None);
        designer.preset.name = "Neon".to_string();
        designer.preset.seeds.accent.hue = 320.0;
        designer.preset.set_override(
            StyleKey::Panel,
            wgpu_gameui::StyleValue::Color([0.1, 0.0, 0.2, 1.0]),
        );
        designer.save_to(&path);
        assert!(!designer.dirty(), "{}", designer.status);
        assert_eq!(designer.path.as_deref(), Some(path.as_path()));

        let opened = Designer::new(Some(path));
        assert_eq!(opened.preset, designer.preset);
        assert!(!opened.dirty());
    }

    #[test]
    fn a_missing_file_leaves_the_document_alone() {
        let path = scratch_dir("designer_missing").join("absent.toml");
        let designer = Designer::new(Some(path));
        assert_eq!(designer.preset, ThemePreset::new("Untitled"));
        assert_eq!(designer.path, None);
        assert!(
            designer.status.starts_with("Could not open"),
            "{}",
            designer.status
        );
    }

    #[test]
    fn the_preview_follows_the_preset() {
        let mut designer = Designer::new(None);
        let mut layers = LayerStack::new();
        frame(&mut designer, &mut layers, None);
        let forge_accent = designer.preview_theme.accent;

        designer.preset.seeds.accent.hue = 25.0;
        frame(&mut designer, &mut layers, None);
        assert_ne!(designer.preview_theme.accent, forge_accent);
        assert_eq!(
            designer.preview_theme.accent,
            designer.preset.resolve().accent
        );
        assert_eq!(
            designer.ui_theme.accent, forge_accent,
            "the chrome stays Forge"
        );
    }

    #[test]
    fn save_without_a_path_asks_for_one() {
        let mut designer = Designer::new(None);
        let mut layers = LayerStack::new();
        let out = frame(&mut designer, &mut layers, Some(Command::Save));
        assert!(!out.quit);
        assert!(designer.prompt.is_open());
        assert_eq!(designer.prompt_for, PathFor::SaveAs);
        assert_eq!(designer.prompt.value(), "presets/untitled.toml");
    }

    #[test]
    fn slugs_are_file_names() {
        assert_eq!(slug("Aldermoor Hearth"), "aldermoor-hearth");
        assert_eq!(slug("  Neon / Night!  "), "neon-night");
        assert_eq!(slug("…"), "preset");
    }
}
