//! FileDialog frame-loop tests: a fake disk, real frames.

use std::collections::HashMap;

use super::*;
use crate::{DrawList, FocusState, NavInput, Theme};

const SCREEN: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 1000.0,
    height: 700.0,
};
const ID: FocusId = 100;

/// A disk in memory. Folders listed in `slow` answer Loading until
/// `arrive` is called.
#[derive(Default)]
struct Disk {
    folders: HashMap<Vec<String>, Vec<FileEntry>>,
    slow: Vec<Vec<String>>,
    asked: Vec<Vec<String>>,
    generation: u64,
}

impl Disk {
    fn with(mut self, path: &[&str], entries: Vec<FileEntry>) -> Self {
        self.folders.insert(segments(path), entries);
        self
    }

    fn arrive(&mut self, path: &[&str]) {
        let path = segments(path);
        self.slow.retain(|p| *p != path);
    }

    fn add(&mut self, path: &[&str], entry: FileEntry) {
        self.folders.entry(segments(path)).or_default().push(entry);
        self.generation += 1;
    }
}

impl FileSource for Disk {
    fn list(&mut self, path: &[String]) -> Listing<'_> {
        if !self.asked.iter().any(|p| p == path) {
            self.asked.push(path.to_vec());
        }
        if self.slow.iter().any(|p| p == path) {
            return Listing::Loading;
        }
        match self.folders.get(path) {
            Some(entries) => Listing::Entries {
                entries,
                generation: self.generation,
            },
            None => Listing::Failed("No such folder"),
        }
    }
}

fn segments(path: &[&str]) -> Vec<String> {
    path.iter().map(|s| (*s).to_owned()).collect()
}

fn disk() -> Disk {
    Disk::default()
        .with(
            &[],
            vec![
                FileEntry::folder("Projects"),
                FileEntry::folder(".config"),
                FileEntry::file("notes.txt"),
            ],
        )
        .with(
            &["Projects"],
            vec![
                FileEntry::folder("alpha"),
                FileEntry::folder("beta"),
                FileEntry::file("todo.md"),
            ],
        )
        .with(&["Projects", "alpha"], vec![FileEntry::file("main.rs")])
        .with(&["Projects", "beta"], vec![])
        .with(&[".config"], vec![])
}

/// A dialog's state, its disk and focus, and the text the last frame drew.
struct Rig {
    state: FileDialogState,
    focus: FocusState,
    disk: Disk,
    theme: Theme,
    /// Text drawn last frame.
    texts: Vec<(String, f32, f32)>,
    /// When last frame asked to be drawn again, in seconds.
    repaint: Option<f32>,
}

impl Rig {
    fn new(path: &[&str]) -> Self {
        let mut state = FileDialogState::new();
        state.open(segments(path));
        Self {
            state,
            focus: FocusState::new(),
            disk: disk(),
            theme: Theme::default(),
            texts: Vec::new(),
            repaint: None,
        }
    }

    /// One frame, drawn only while the dialog is open (as a caller does).
    fn frame_with(&mut self, dialog: &FileDialog, mut input: InputState) -> FileDialogOutput {
        if !self.state.is_open() {
            return FileDialogOutput::default();
        }
        let mut list = DrawList::new();
        self.state.modal.begin_frame(&mut input);
        self.focus.begin_frame(&input);
        let out = {
            let mut ctx = DrawContext::new(
                &mut list,
                &mut self.focus,
                &self.theme,
                &input,
                SCREEN.width,
                SCREEN.height,
            )
            .with_layer(0);
            dialog.draw(ID, SCREEN, &mut self.state, &mut self.disk, &mut ctx)
        };
        self.focus.end_frame(Some(0));
        self.repaint = list.next_repaint();
        self.texts = list
            .texts
            .iter()
            .map(|t| (t.content.clone(), t.x, t.y))
            .collect();
        out
    }

    fn frame(&mut self, dialog: &FileDialog, input: InputState) -> Option<FileDialogEvent> {
        self.frame_with(dialog, input).event
    }

    fn idle(&mut self, dialog: &FileDialog) -> Option<FileDialogEvent> {
        self.frame(dialog, idle())
    }

    /// Where text `label` was drawn last frame (its first glyph's middle).
    fn find(&self, label: &str) -> (f32, f32) {
        let (_, x, y) = self
            .texts
            .iter()
            .find(|(t, _, _)| t == label)
            .unwrap_or_else(|| panic!("no text {label:?} in {:?}", self.labels()));
        (x + 3.0, y + 5.0)
    }

    fn shows(&self, label: &str) -> bool {
        self.texts.iter().any(|(t, _, _)| t == label)
    }

    fn labels(&self) -> Vec<&str> {
        self.texts.iter().map(|(t, _, _)| t.as_str()).collect()
    }

    fn click(&mut self, dialog: &FileDialog, label: &str) -> Option<FileDialogEvent> {
        let (x, y) = self.find(label);
        self.click_at(dialog, x, y)
    }

    fn click_at(&mut self, dialog: &FileDialog, x: f32, y: f32) -> Option<FileDialogEvent> {
        let input = InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: 1,
            ..InputState::default()
        };
        let event = self.frame(dialog, input);
        let release = InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_released: true,
            ..InputState::default()
        };
        event.or(self.frame(dialog, release))
    }

    fn double_click(&mut self, dialog: &FileDialog, label: &str) -> Option<FileDialogEvent> {
        let (x, y) = self.find(label);
        let first = self.click_at(dialog, x, y);
        let input = InputState {
            mouse_x: x,
            mouse_y: y,
            mouse_down: true,
            mouse_clicked: true,
            mouse_click_count: 2,
            mouse_double_clicked: true,
            ..InputState::default()
        };
        first.or(self.frame(dialog, input))
    }

    fn key(&mut self, dialog: &FileDialog, input: InputState) -> Option<FileDialogEvent> {
        self.frame(dialog, input)
    }

    fn typed(&mut self, dialog: &FileDialog, text: &str) -> Option<FileDialogEvent> {
        self.key(
            dialog,
            InputState {
                text_input: text.to_owned(),
                ..idle()
            },
        )
    }

    fn here(&self) -> Vec<&str> {
        self.state.path().iter().map(String::as_str).collect()
    }
}

fn idle() -> InputState {
    InputState {
        mouse_x: -100.0,
        mouse_y: -100.0,
        ..InputState::default()
    }
}

fn nav(f: impl FnOnce(&mut NavInput)) -> InputState {
    let mut input = idle();
    f(&mut input.nav);
    input
}

fn enter() -> InputState {
    InputState {
        enter_pressed: true,
        ..nav(|n| n.confirm = true)
    }
}

fn escape() -> InputState {
    nav(|n| n.cancel = true)
}

#[test]
fn opening_focuses_the_listing_and_lists_the_folder() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.idle(&dialog);
    assert_eq!(rig.focus.focused(), Some(ID));
    assert!(rig.shows("alpha") && rig.shows("beta") && rig.shows("todo.md"));
    assert_eq!(rig.disk.asked, vec![segments(&["Projects"])]);
}

#[test]
fn folder_mode_chooses_the_folder_shown_or_the_one_selected() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    assert!(rig.shows("Choose"));
    rig.click(&dialog, "alpha");
    assert_eq!(rig.state.selected(), Some("alpha"));
    let event = rig.click(&dialog, "Choose");
    assert_eq!(
        event,
        Some(FileDialogEvent::Confirm {
            path: segments(&["Projects", "alpha"]),
            name: None,
            replaces: false,
        })
    );
    assert!(!rig.state.is_open(), "confirming closes it");
}

#[test]
fn folder_mode_files_are_shown_but_not_picked() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.click(&dialog, "todo.md");
    assert_eq!(rig.state.selected(), None, "a file can't be chosen");
    let event = rig.click(&dialog, "Choose");
    assert_eq!(
        event,
        Some(FileDialogEvent::Confirm {
            path: segments(&["Projects"]),
            name: None,
            replaces: false,
        })
    );
}

#[test]
fn double_click_goes_in_and_history_walks_back_and_forward() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.double_click(&dialog, "alpha");
    rig.idle(&dialog);
    assert_eq!(rig.here(), ["Projects", "alpha"]);
    assert!(rig.shows("main.rs"));

    let alt = |f: fn(&mut NavInput)| InputState {
        alt_down: true,
        ..nav(f)
    };
    rig.key(&dialog, alt(|n| n.left = true));
    assert_eq!(rig.here(), ["Projects"]);
    rig.key(&dialog, alt(|n| n.right = true));
    assert_eq!(rig.here(), ["Projects", "alpha"]);
    rig.key(
        &dialog,
        InputState {
            backspace_pressed: true,
            ..idle()
        },
    );
    assert_eq!(rig.here(), ["Projects"], "Backspace goes up");
    rig.key(
        &dialog,
        InputState {
            ctrl_pressed: true,
            ..nav(|n| n.up = true)
        },
    );
    assert!(rig.here().is_empty(), "Ctrl+Up goes up");
}

#[test]
fn enter_goes_into_the_selected_folder() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.click(&dialog, "beta");
    assert_eq!(rig.key(&dialog, enter()), None);
    assert_eq!(rig.here(), ["Projects", "beta"]);
}

#[test]
fn crumbs_go_to_their_folder() {
    let dialog = FileDialog::new(FileDialogMode::Folder).root_label("~");
    let mut rig = Rig::new(&["Projects", "alpha"]);
    rig.idle(&dialog);
    rig.click(&dialog, "Projects");
    assert_eq!(rig.here(), ["Projects"]);
    rig.idle(&dialog);
    rig.click(&dialog, "~");
    assert!(rig.here().is_empty());
}

#[test]
fn places_go_to_their_folder_and_show_which_is_current() {
    let places = [
        FilePlace::new("Code", segments(&["Projects"])),
        FilePlace::new("Home", Vec::new()),
    ];
    let dialog = FileDialog::new(FileDialogMode::Folder).places(&places);
    let mut rig = Rig::new(&["Projects", "alpha"]);
    rig.idle(&dialog);
    rig.click(&dialog, "Home");
    assert!(rig.here().is_empty());
    rig.idle(&dialog);
    rig.click(&dialog, "Code");
    assert_eq!(rig.here(), ["Projects"]);
}

#[test]
fn recent_lists_entries_where_they_are_and_chooses_them_there() {
    let mut recent = FileEntry::folder("alpha");
    recent.dir = Some(segments(&["Projects"]));
    let recent = [recent];
    let dialog = FileDialog::new(FileDialogMode::Folder).recent(&recent);
    let mut rig = Rig::new(&[]);
    rig.idle(&dialog);
    rig.click(&dialog, "Recent");
    assert!(rig.state.showing_recent());
    rig.idle(&dialog);
    assert!(rig.shows("alpha"));
    assert!(rig.shows("FOLDER"), "the kind column says where each is");
    assert!(
        rig.labels().contains(&"Projects"),
        "alpha's folder is shown in its row"
    );
    rig.click(&dialog, "alpha");
    let event = rig.click(&dialog, "Choose");
    assert_eq!(
        event,
        Some(FileDialogEvent::Confirm {
            path: segments(&["Projects", "alpha"]),
            name: None,
            replaces: false,
        })
    );
}

#[test]
fn the_star_pins_and_unpins_the_folder_shown() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects", "alpha"]);
    rig.idle(&dialog);
    assert!(rig.shows("☆ in the toolbar pins a folder here"));
    let star = star_rect(&dialog);
    let event = rig.click_at(&dialog, star.x + 5.0, star.y + 5.0);
    assert_eq!(event, Some(FileDialogEvent::FavouritesChanged));
    assert_eq!(
        rig.state.favourites(),
        [FavouriteFolder {
            path: segments(&["Projects", "alpha"]),
            label: None,
        }]
    );
    rig.idle(&dialog);
    assert!(rig.shows("alpha"), "the favourite is listed in Places");
    let event = rig.click_at(&dialog, star.x + 5.0, star.y + 5.0);
    assert_eq!(event, Some(FileDialogEvent::FavouritesChanged));
    assert!(rig.state.favourites().is_empty());
}

/// The ☆ key: left of the filter field in the header.
fn star_rect(dialog: &FileDialog) -> Rect {
    let inner = dialog.rect(SCREEN).inset(1.0);
    let right = inner.right()
        - HEADER_PAD_RIGHT
        - KEY
        - HEADER_GAP
        - (VIEW_KEY * 2.0 + 6.0)
        - HEADER_GAP
        - SEARCH_W
        - HEADER_GAP
        - KEY;
    let cy = inner.y + HEADER_H * 0.5;
    Rect::new(right, (cy - KEY * 0.5).round(), KEY, KEY)
}

#[test]
fn hidden_entries_wait_for_the_toggle() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&[]);
    rig.idle(&dialog);
    assert!(!rig.shows(".config"));
    assert!(rig.shows("Hidden · 1"));
    rig.click(&dialog, "Hidden · 1");
    rig.idle(&dialog);
    assert!(rig.shows(".config"));
}

#[test]
fn the_filter_narrows_the_listing() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.focus.focus(ID + 1);
    rig.typed(&dialog, "alp");
    rig.idle(&dialog);
    assert!(rig.shows("alpha"));
    assert!(!rig.shows("beta"));
}

#[test]
fn typing_in_the_listing_selects_by_prefix() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.idle(&dialog);
    rig.typed(&dialog, "b");
    assert_eq!(rig.state.selected(), Some("beta"));
    rig.typed(&dialog, "x");
    assert_eq!(rig.state.selected(), Some("beta"), "no match keeps it");
}

#[test]
fn a_pause_on_ticks_that_draw_nothing_starts_a_new_prefix() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.idle(&dialog);
    rig.typed(&dialog, "b");
    assert_eq!(rig.state.selected(), Some("beta"));
    // A host that skips idle frames: a second passes, nothing drawn.
    rig.state.tick_clocks(1.0);
    rig.typed(&dialog, "a");
    assert_eq!(rig.state.selected(), Some("alpha"), "not \"ba\"");
}

#[test]
fn dates_ask_for_a_frame_at_local_midnight() {
    let clock = Clock {
        now: 19_723 * 86_400 + 22 * 3_600,
        utc_offset: 0,
    };
    let dialog = FileDialog::new(FileDialogMode::Folder).clock(clock);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    assert_eq!(rig.repaint, Some(2.0 * 3_600.0));
}

#[test]
fn a_typed_path_waits_for_its_listing_and_reports_failure() {
    let dialog = FileDialog::new(FileDialogMode::Folder).root_label("~");
    let mut rig = Rig::new(&["Projects"]);
    rig.disk.slow.push(segments(&["Projects", "beta"]));
    rig.idle(&dialog);
    rig.idle(&dialog);
    rig.typed(&dialog, "/");
    assert!(rig.state.editing_path());
    rig.idle(&dialog);
    assert_eq!(rig.focus.focused(), Some(ID + 2));

    // Replace the text with a path that doesn't exist.
    rig.state.path_field.value = "~/nowhere".into();
    rig.key(&dialog, enter());
    rig.idle(&dialog);
    assert!(rig.state.editing_path(), "the field stays open");
    assert!(rig.shows("No such folder"));
    assert_eq!(rig.here(), ["Projects"]);

    rig.state.path_field.value = "~/Projects/beta".into();
    rig.key(&dialog, enter());
    rig.idle(&dialog);
    assert_eq!(rig.here(), ["Projects"], "still loading");
    rig.disk.arrive(&["Projects", "beta"]);
    rig.idle(&dialog);
    assert_eq!(rig.here(), ["Projects", "beta"]);
    assert!(!rig.state.editing_path());
}

#[test]
fn escape_backs_out_one_layer_at_a_time() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.state.edit_path();
    rig.idle(&dialog);
    assert_eq!(rig.key(&dialog, escape()), None);
    assert!(!rig.state.editing_path());
    assert!(rig.state.is_open());

    rig.state.start_new_folder();
    rig.idle(&dialog);
    assert_eq!(rig.key(&dialog, escape()), None);
    assert!(!rig.state.creating_folder());

    assert_eq!(rig.key(&dialog, escape()), Some(FileDialogEvent::Cancel));
    assert!(!rig.state.is_open());
}

#[test]
fn new_folder_asks_the_caller_and_selects_the_result() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.state.start_new_folder();
    rig.idle(&dialog);
    assert_eq!(rig.focus.focused(), Some(ID + 3));
    // A name that exists is refused.
    rig.state.new_folder.as_mut().unwrap().value = "alpha".into();
    assert_eq!(rig.key(&dialog, enter()), None);
    assert!(rig.state.creating_folder());

    rig.state.new_folder.as_mut().unwrap().value = "gamma".into();
    let event = rig.key(&dialog, enter());
    assert_eq!(
        event,
        Some(FileDialogEvent::CreateFolder {
            path: segments(&["Projects"]),
            name: "gamma".into(),
        })
    );
    assert!(!rig.state.creating_folder());
    rig.disk.add(&["Projects"], FileEntry::folder("gamma"));
    rig.idle(&dialog);
    assert!(rig.shows("gamma"), "a new generation is listed again");
    assert_eq!(rig.state.selected(), Some("gamma"));
}

#[test]
fn loading_and_failure_show_their_states() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects", "beta"]);
    rig.disk.slow.push(segments(&["Projects", "beta"]));
    rig.idle(&dialog);
    assert!(!rig.shows("Empty folder"), "loading is not empty");
    rig.disk.arrive(&["Projects", "beta"]);
    rig.idle(&dialog);
    assert!(rig.shows("EMPTY FOLDER") || rig.shows("Empty folder"));

    let mut rig = Rig::new(&["gone"]);
    rig.idle(&dialog);
    assert!(rig.labels().iter().any(|t| t.contains("No such folder")));
}

#[test]
fn open_mode_opens_a_file_on_double_click() {
    let dialog = FileDialog::new(FileDialogMode::Open);
    let mut rig = Rig::new(&["Projects", "alpha"]);
    rig.idle(&dialog);
    let event = rig.double_click(&dialog, "main.rs");
    assert_eq!(
        event,
        Some(FileDialogEvent::Confirm {
            path: segments(&["Projects", "alpha"]),
            name: Some("main.rs".into()),
            replaces: false,
        })
    );
}

#[test]
fn open_mode_type_chips_filter_by_extension() {
    let dialog = FileDialog::new(FileDialogMode::Open);
    let mut rig = Rig::new(&[]);
    rig.disk.folders.insert(
        Vec::new(),
        vec![
            FileEntry::file("a.png"),
            FileEntry::file("b.png"),
            FileEntry::file("c.lvl"),
        ],
    );
    rig.idle(&dialog);
    assert!(rig.shows(".png") && rig.shows(".lvl"));
    rig.click(&dialog, ".png");
    rig.idle(&dialog);
    assert!(!rig.shows("a.png"));
    assert!(rig.shows("c.lvl"));
}

#[test]
fn save_mode_names_the_file_and_warns_before_replacing() {
    let dialog = FileDialog::new(FileDialogMode::Save).save_ext("rs");
    let mut rig = Rig::new(&["Projects", "alpha"]);
    rig.state
        .open_with_name(segments(&["Projects", "alpha"]), "lib.rs");
    rig.idle(&dialog);
    rig.idle(&dialog);
    assert_eq!(rig.focus.focused(), Some(ID + 4));
    assert!(rig.shows("Save"));

    rig.state.name.value = "main".into();
    rig.idle(&dialog);
    assert!(rig.shows("Replace"), "main.rs exists");
    let event = rig.click(&dialog, "Replace");
    assert_eq!(
        event,
        Some(FileDialogEvent::Confirm {
            path: segments(&["Projects", "alpha"]),
            name: Some("main.rs".into()),
            replaces: true,
        })
    );
}

#[test]
fn the_footer_slot_is_reserved_at_the_start() {
    let dialog = FileDialog::new(FileDialogMode::Folder).footer_slot(90.0);
    let mut rig = Rig::new(&["Projects"]);
    let out = rig.frame_with(&dialog, idle());
    let slot = out.slot.expect("a slot");
    let surface = dialog.rect(SCREEN);
    assert_eq!(slot.width, 90.0);
    assert!(slot.x > surface.x && slot.x < surface.x + 20.0);
    assert!(slot.bottom() < surface.bottom() && slot.y > surface.bottom() - 50.0);
    let (choose_x, _) = rig.find("Choose ");
    assert!(choose_x > slot.right(), "the summary starts after the slot");
}

#[test]
fn cancel_closes_and_says_so() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    assert_eq!(rig.click(&dialog, "Cancel"), Some(FileDialogEvent::Cancel));
    assert!(!rig.state.is_open());
}

#[test]
fn an_unchanged_listing_is_not_filtered_again() {
    let dialog = FileDialog::new(FileDialogMode::Folder);
    let mut rig = Rig::new(&["Projects"]);
    rig.idle(&dialog);
    rig.state.cache.shown.total = 999;
    rig.idle(&dialog);
    assert_eq!(rig.state.cache.shown.total, 999, "the cache was reused");
    rig.disk.add(&["Projects"], FileEntry::folder("delta"));
    rig.idle(&dialog);
    assert_eq!(rig.state.cache.shown.total, 4);
}
