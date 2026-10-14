//! The file dialog's logic, apart from drawing: where it is and has been,
//! what of a listing shows and in what order, the type chips, and the checks
//! on typed names. Everything here is plain data, so it is tested without a
//! frame.

use std::cmp::Ordering;

use super::{FileEntry, FileKind};

/// A place the dialog has been: a folder, or the Recent list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Location {
    pub path: Vec<String>,
    pub recent: bool,
}

/// Where the dialog is, and the back / forward history.
#[derive(Clone, Debug, Default)]
pub(super) struct Nav {
    pub here: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
}

impl Nav {
    pub fn new(path: Vec<String>) -> Self {
        Self {
            here: Location {
                path,
                recent: false,
            },
            ..Self::default()
        }
    }

    /// Go to `to`, remembering where the dialog was. Going where it already
    /// is changes nothing.
    pub fn go(&mut self, to: Location) -> bool {
        if to == self.here {
            return false;
        }
        let from = std::mem::replace(&mut self.here, to);
        self.back.push(from);
        self.forward.clear();
        true
    }

    pub fn go_path(&mut self, path: Vec<String>) -> bool {
        self.go(Location {
            path,
            recent: false,
        })
    }

    pub fn back(&mut self) -> bool {
        let Some(to) = self.back.pop() else {
            return false;
        };
        let from = std::mem::replace(&mut self.here, to);
        self.forward.insert(0, from);
        true
    }

    pub fn forward(&mut self) -> bool {
        if self.forward.is_empty() {
            return false;
        }
        let to = self.forward.remove(0);
        let from = std::mem::replace(&mut self.here, to);
        self.back.push(from);
        true
    }

    /// The enclosing folder. Not from Recent, nor from the root.
    pub fn up(&mut self) -> bool {
        if !self.can_go_up() {
            return false;
        }
        let mut path = self.here.path.clone();
        path.pop();
        self.go_path(path)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn can_go_up(&self) -> bool {
        !self.here.recent && !self.here.path.is_empty()
    }
}

/// The dialog's purpose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileDialogMode {
    /// Open a file.
    #[default]
    Open,
    /// Save a file: a name field, and Replace when the name exists.
    Save,
    /// Choose a folder. Files are listed dimmed, for context.
    Folder,
}

/// The listing column sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortBy {
    /// By name, naturally (`2` before `10`).
    #[default]
    Name,
    /// By last modified.
    Modified,
    /// By size.
    Size,
    /// By extension.
    Kind,
}

/// The sort: a column and its direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sort {
    pub by: SortBy,
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            by: SortBy::Name,
            ascending: true,
        }
    }
}

impl Sort {
    /// A click on `by`'s header: the same column flips, another starts
    /// ascending.
    pub fn toggle(&mut self, by: SortBy) {
        if self.by == by {
            self.ascending = !self.ascending;
        } else {
            *self = Self {
                by,
                ascending: true,
            };
        }
    }
}

/// The extension of `name`, lower-cased; empty without one. A leading dot
/// alone (`.profile`) is not an extension.
pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// `name` without its extension.
pub fn base_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Hidden: marked so, or a dot file.
pub fn is_hidden(entry: &FileEntry) -> bool {
    entry.hidden || entry.name.starts_with('.')
}

/// Names compared as people read them: case-insensitive, with runs of digits
/// compared as numbers (`file2` before `file10`).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (tx, ty) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// What the dialog shows of a listing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Shown {
    /// Indices into the listing, in display order.
    pub rows: Vec<usize>,
    /// Accepted types present (hidden files counted only when shown), most
    /// common first, with their counts.
    pub types: Vec<(String, usize)>,
    /// Files of types not accepted (left out of the list).
    pub other_files: usize,
    /// Hidden entries in the listing, shown or not.
    pub hidden: usize,
    /// Entries in the listing.
    pub total: usize,
}

/// The inputs [`Shown`] is built from; the state rebuilds it when any change.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ShowFilter {
    pub mode: FileDialogMode,
    /// Accepted extensions, lower-case without dots; empty accepts all.
    pub accept: Vec<String>,
    /// Types toggled off by their chips.
    pub off: Vec<String>,
    pub show_hidden: bool,
    pub query: String,
    pub sort: Sort,
    pub recent: bool,
}

/// Normalise accepted extensions: no leading dot, lower case.
pub fn normalise_accept<S: AsRef<str>>(accept: &[S]) -> Vec<String> {
    accept
        .iter()
        .map(|ext| ext.as_ref().trim_start_matches('.').to_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect()
}

impl ShowFilter {
    fn accepts(&self, ext: &str) -> bool {
        self.accept.is_empty() || self.accept.iter().any(|a| a == ext)
    }

    /// Filter and sort `entries`.
    pub fn apply(&self, entries: &[FileEntry]) -> Shown {
        let mut shown = Shown {
            total: entries.len(),
            hidden: entries.iter().filter(|e| is_hidden(e)).count(),
            ..Shown::default()
        };
        let mut counts: Vec<(String, usize)> = Vec::new();
        let query = self.query.trim().to_lowercase();
        for (index, entry) in entries.iter().enumerate() {
            if !self.show_hidden && is_hidden(entry) {
                continue;
            }
            let folder = entry.kind == FileKind::Folder;
            let ext = ext_of(&entry.name);
            if !folder {
                if self.accepts(&ext) {
                    match counts.iter_mut().find(|(t, _)| *t == ext) {
                        Some((_, n)) => *n += 1,
                        None => counts.push((ext.clone(), 1)),
                    }
                } else {
                    shown.other_files += 1;
                }
            }
            let listed = folder
                || self.mode == FileDialogMode::Folder
                || (self.accepts(&ext) && !self.off.contains(&ext));
            if listed && (query.is_empty() || entry.name.to_lowercase().contains(&query)) {
                shown.rows.push(index);
            }
        }
        counts.sort_by(|(a, n), (b, m)| m.cmp(n).then_with(|| a.cmp(b)));
        shown.types = counts;
        if self.recent {
            // Recent is newest first, whatever the sort.
            shown
                .rows
                .sort_by(|&a, &b| entries[b].modified.cmp(&entries[a].modified));
        } else {
            shown
                .rows
                .sort_by(|&a, &b| self.compare(&entries[a], &entries[b]));
        }
        shown
    }

    /// Folders first, then the sort column, then the name. The direction
    /// applies to the column (and to the name when sorting by name).
    fn compare(&self, a: &FileEntry, b: &FileEntry) -> Ordering {
        let (fa, fb) = (a.kind == FileKind::Folder, b.kind == FileKind::Folder);
        if fa != fb {
            return if fa {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let column = match self.sort.by {
            SortBy::Name => Ordering::Equal,
            SortBy::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortBy::Modified => a.modified.unwrap_or(0).cmp(&b.modified.unwrap_or(0)),
            SortBy::Kind => ext_of(&a.name).cmp(&ext_of(&b.name)),
        };
        let flip = |order: Ordering| {
            if self.sort.ascending {
                order
            } else {
                order.reverse()
            }
        };
        if column != Ordering::Equal {
            return flip(column);
        }
        let name = natural_cmp(&a.name, &b.name);
        if self.sort.by == SortBy::Name {
            flip(name)
        } else {
            name
        }
    }
}

/// Why a typed name can't be used, if it can't.
pub fn bad_name(name: &str) -> Option<&'static str> {
    if name.contains(['/', '\\', ':']) {
        Some("Names can't contain / \\ or :")
    } else {
        None
    }
}

/// Why a new folder named `name` can't be made in a folder holding
/// `entries`, if it can't.
pub fn bad_folder_name(name: &str, entries: &[FileEntry]) -> Option<&'static str> {
    let wanted = name.trim().to_lowercase();
    if entries.iter().any(|e| e.name.to_lowercase() == wanted) {
        Some("Already exists")
    } else if name.contains(['/', '\\', ':']) {
        Some("Can't contain / \\ or :")
    } else {
        None
    }
}

/// The file name a save will write: trimmed, with `ext` appended when it is
/// missing. Empty when nothing is typed.
pub fn save_name(name: &str, ext: Option<&str>) -> String {
    let name = name.trim();
    if name.is_empty() {
        return String::new();
    }
    match ext.map(|e| e.trim_start_matches('.')) {
        Some(ext) if !ext.is_empty() && ext_of(name) != ext.to_lowercase() => {
            format!("{name}.{ext}")
        }
        _ => name.to_owned(),
    }
}

/// "12 B", "1.5 KB", "18 KB", "2.4 MB", "1.1 GB"; "—" when unknown.
pub fn format_size(bytes: Option<u64>) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    let Some(b) = bytes else {
        return "—".into();
    };
    if b < KB {
        format!("{b} B")
    } else if b < MB {
        let kb = b as f64 / KB as f64;
        if b < 10 * KB {
            format!("{kb:.1} KB")
        } else {
            format!("{kb:.0} KB")
        }
    } else if b < GB {
        format!("{:.1} MB", b as f64 / MB as f64)
    } else {
        format!("{:.1} GB", b as f64 / GB as f64)
    }
}

/// The local clock dates are shown against: now (Unix seconds) and the
/// local offset from UTC (seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    /// Now, in Unix seconds.
    pub now: i64,
    /// The local offset from UTC, in seconds.
    pub utc_offset: i64,
}

impl Clock {
    /// Seconds until the next local midnight, when "Today" becomes
    /// "Yesterday": always 1 to 86,400.
    pub fn until_next_day(&self) -> i64 {
        86_400 - (self.now + self.utc_offset).rem_euclid(86_400)
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The civil date (year, month 1-12, day) of `days` since 1970-01-01
/// (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// "Today 14:02", "Yesterday 09:10", "Oct 3", or "Oct 3, 2024" for another
/// year; "—" when unknown or without a clock.
pub fn format_date(modified: Option<i64>, clock: Option<Clock>) -> String {
    let (Some(t), Some(clock)) = (modified, clock) else {
        return "—".into();
    };
    let local = t + clock.utc_offset;
    let today = (clock.now + clock.utc_offset).div_euclid(86_400);
    let day = local.div_euclid(86_400);
    let minutes = local.rem_euclid(86_400) / 60;
    let hm = format!("{:02}:{:02}", minutes / 60, minutes % 60);
    if day == today {
        return format!("Today {hm}");
    }
    if day == today - 1 {
        return format!("Yesterday {hm}");
    }
    let (year, month, d) = civil(day);
    let (this_year, _, _) = civil(today);
    let month = MONTHS[month as usize - 1];
    if year == this_year {
        format!("{month} {d}")
    } else {
        format!("{month} {d}, {year}")
    }
}

/// Type-ahead: letters typed within a short pause build one prefix.
#[derive(Clone, Debug, Default)]
pub(super) struct TypeAhead {
    typed: String,
    /// Seconds since the last letter.
    idle: f32,
}

/// The pause after which typing starts a new prefix (the design's 700 ms).
const TYPE_AHEAD_PAUSE: f32 = 0.7;

impl TypeAhead {
    /// Let `dt` seconds pass, drawn or not.
    pub fn tick(&mut self, dt: f32) {
        self.idle += dt;
    }

    /// Add `text`; returns the prefix to look for.
    pub fn push(&mut self, text: &str) -> &str {
        if self.idle >= TYPE_AHEAD_PAUSE {
            self.typed.clear();
        }
        self.idle = 0.0;
        self.typed.push_str(&text.to_lowercase());
        &self.typed
    }

    pub fn clear(&mut self) {
        self.typed.clear();
    }

    /// Whether a prefix is being typed (a space then continues it).
    pub fn typing(&self) -> bool {
        !self.typed.is_empty() && self.idle < TYPE_AHEAD_PAUSE
    }
}

/// The first of `rows` whose name starts with `prefix`.
pub(super) fn find_prefix(rows: &[usize], entries: &[FileEntry], prefix: &str) -> Option<usize> {
    rows.iter()
        .position(|&i| entries[i].name.to_lowercase().starts_with(prefix))
}

/// Parse a typed path against the root's label: `root/a/b`, `/a/b` and `a/b`
/// all mean `["a", "b"]`.
pub fn parse_typed_path(text: &str, root_label: &str) -> Vec<String> {
    let mut segments: Vec<String> = text
        .trim()
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if segments.first().is_some_and(|s| s == root_label) {
        segments.remove(0);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> FileEntry {
        FileEntry::file(name)
    }

    fn folder(name: &str) -> FileEntry {
        FileEntry::folder(name)
    }

    fn names(shown: &Shown, entries: &[FileEntry]) -> Vec<String> {
        shown
            .rows
            .iter()
            .map(|&i| entries[i].name.clone())
            .collect()
    }

    #[test]
    fn history_goes_back_and_forward_and_a_new_place_drops_forward() {
        let mut nav = Nav::new(vec![]);
        assert!(nav.go_path(vec!["a".into()]));
        assert!(nav.go_path(vec!["a".into(), "b".into()]));
        assert!(!nav.go_path(vec!["a".into(), "b".into()]), "already there");
        assert!(nav.back());
        assert_eq!(nav.here.path, ["a"]);
        assert!(nav.can_go_forward());
        assert!(nav.forward());
        assert_eq!(nav.here.path, ["a", "b"]);
        nav.back();
        nav.go_path(vec!["c".into()]);
        assert!(!nav.can_go_forward());
        assert!(nav.back());
        assert_eq!(nav.here.path, ["a"]);
    }

    #[test]
    fn up_stops_at_the_root_and_in_recent() {
        let mut nav = Nav::new(vec!["a".into()]);
        assert!(nav.up());
        assert!(nav.here.path.is_empty());
        assert!(!nav.up());
        nav.go(Location {
            path: vec!["x".into()],
            recent: true,
        });
        assert!(!nav.can_go_up());
    }

    #[test]
    fn extensions_and_hidden() {
        assert_eq!(ext_of("map.LVL"), "lvl");
        assert_eq!(ext_of(".profile"), "");
        assert_eq!(ext_of("README"), "");
        assert_eq!(base_of("act1.lvl"), "act1");
        assert_eq!(base_of(".profile"), ".profile");
        assert!(is_hidden(&file(".git")));
        assert!(is_hidden(&FileEntry {
            hidden: true,
            ..file("thumbs.db")
        }));
    }

    #[test]
    fn names_sort_naturally() {
        let mut names = vec!["file10", "File2", "file1", "alpha", "file02b"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["alpha", "file1", "File2", "file02b", "file10"]);
    }

    #[test]
    fn folders_come_first_then_the_sort() {
        let entries = vec![
            file("b.lvl"),
            folder("zeta"),
            file("a.lvl"),
            folder("Alpha"),
        ];
        let mut filter = ShowFilter::default();
        assert_eq!(
            names(&filter.apply(&entries), &entries),
            ["Alpha", "zeta", "a.lvl", "b.lvl"]
        );
        filter.sort.toggle(SortBy::Name);
        assert_eq!(
            names(&filter.apply(&entries), &entries),
            ["zeta", "Alpha", "b.lvl", "a.lvl"],
            "descending still keeps folders first"
        );
    }

    #[test]
    fn sorting_by_size_and_modified_breaks_ties_by_name() {
        let entries = vec![
            FileEntry {
                size: Some(10),
                modified: Some(3),
                ..file("c")
            },
            FileEntry {
                size: Some(30),
                modified: Some(1),
                ..file("a")
            },
            FileEntry {
                size: Some(10),
                modified: Some(2),
                ..file("b")
            },
        ];
        let mut filter = ShowFilter::default();
        filter.sort.toggle(SortBy::Size);
        assert_eq!(names(&filter.apply(&entries), &entries), ["b", "c", "a"]);
        filter.sort.toggle(SortBy::Size);
        assert_eq!(names(&filter.apply(&entries), &entries), ["a", "b", "c"]);
        filter.sort.toggle(SortBy::Modified);
        assert_eq!(names(&filter.apply(&entries), &entries), ["a", "b", "c"]);
    }

    #[test]
    fn accepted_types_are_counted_and_others_left_out() {
        let entries = vec![
            file("a.lvl"),
            file("b.lvl"),
            file("c.prefab"),
            file("d.png"),
            file(".e.lvl"),
            folder("sub"),
        ];
        let filter = ShowFilter {
            accept: normalise_accept(&[".LVL", "prefab"]),
            ..ShowFilter::default()
        };
        let shown = filter.apply(&entries);
        assert_eq!(
            names(&shown, &entries),
            ["sub", "a.lvl", "b.lvl", "c.prefab"]
        );
        assert_eq!(
            shown.types,
            [("lvl".to_owned(), 2), ("prefab".to_owned(), 1)]
        );
        assert_eq!(shown.other_files, 1);
        assert_eq!(shown.hidden, 1);
        assert_eq!(shown.total, 6);
    }

    #[test]
    fn a_type_toggled_off_leaves_the_list_but_keeps_its_chip() {
        let entries = vec![file("a.lvl"), file("b.prefab")];
        let filter = ShowFilter {
            off: vec!["lvl".into()],
            ..ShowFilter::default()
        };
        let shown = filter.apply(&entries);
        assert_eq!(names(&shown, &entries), ["b.prefab"]);
        assert_eq!(shown.types.len(), 2);
    }

    #[test]
    fn folder_mode_lists_every_file_and_hidden_needs_the_toggle() {
        let entries = vec![file("a.lvl"), file("b.png"), folder(".git"), folder("src")];
        let mut filter = ShowFilter {
            mode: FileDialogMode::Folder,
            accept: vec!["lvl".into()],
            ..ShowFilter::default()
        };
        assert_eq!(
            names(&filter.apply(&entries), &entries),
            ["src", "a.lvl", "b.png"]
        );
        filter.show_hidden = true;
        assert_eq!(
            names(&filter.apply(&entries), &entries),
            [".git", "src", "a.lvl", "b.png"]
        );
    }

    #[test]
    fn the_query_matches_anywhere_in_the_name() {
        let entries = vec![file("forest.lvl"), file("desert.lvl"), folder("Rest")];
        let filter = ShowFilter {
            query: " REST ".into(),
            ..ShowFilter::default()
        };
        assert_eq!(
            names(&filter.apply(&entries), &entries),
            ["Rest", "forest.lvl"]
        );
    }

    #[test]
    fn recent_is_newest_first() {
        let entries = vec![
            FileEntry {
                modified: Some(1),
                ..file("old")
            },
            FileEntry {
                modified: Some(9),
                ..folder("new")
            },
        ];
        let filter = ShowFilter {
            recent: true,
            ..ShowFilter::default()
        };
        assert_eq!(names(&filter.apply(&entries), &entries), ["new", "old"]);
    }

    #[test]
    fn typed_names_are_checked() {
        assert!(bad_name("a/b").is_some());
        assert!(bad_name("a:b").is_some());
        assert!(bad_name("level 1.lvl").is_none());
        let entries = vec![folder("Assets")];
        assert_eq!(
            bad_folder_name(" assets ", &entries),
            Some("Already exists")
        );
        assert!(bad_folder_name("x\\y", &entries).is_some());
        assert!(bad_folder_name("New folder", &entries).is_none());
    }

    #[test]
    fn a_save_name_gets_its_extension() {
        assert_eq!(save_name(" act1 ", Some("lvl")), "act1.lvl");
        assert_eq!(save_name("act1.LVL", Some(".lvl")), "act1.LVL");
        assert_eq!(save_name("act1.txt", None), "act1.txt");
        assert_eq!(save_name("  ", Some("lvl")), "");
    }

    #[test]
    fn sizes_read_like_the_design() {
        assert_eq!(format_size(None), "—");
        assert_eq!(format_size(Some(12)), "12 B");
        assert_eq!(format_size(Some(1536)), "1.5 KB");
        assert_eq!(format_size(Some(18 * 1024)), "18 KB");
        assert_eq!(format_size(Some(2_516_582)), "2.4 MB");
        assert_eq!(format_size(Some(1_181_116_006)), "1.1 GB");
    }

    #[test]
    fn dates_read_like_the_design() {
        // 2026-10-08 14:02:00 UTC, seen from UTC+9 (23:02 local).
        let now = 1_791_468_120;
        let clock = Some(Clock {
            now,
            utc_offset: 9 * 3600,
        });
        assert_eq!(format_date(Some(now), clock), "Today 23:02");
        assert_eq!(format_date(Some(now - 86_400), clock), "Yesterday 23:02");
        assert_eq!(format_date(Some(now - 5 * 86_400), clock), "Oct 3");
        assert_eq!(format_date(Some(now - 400 * 86_400), clock), "Sep 3, 2025");
        // 23 hours back is 00:02 local, the same day; three minutes before
        // that is yesterday.
        assert_eq!(format_date(Some(now - 23 * 3600), clock), "Today 00:02");
        assert_eq!(
            format_date(Some(now - 23 * 3600 - 180), clock),
            "Yesterday 23:59"
        );
        assert_eq!(format_date(None, clock), "—");
        assert_eq!(format_date(Some(now), None), "—");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(19_782), (2024, 2, 29));
    }

    #[test]
    fn the_next_day_is_counted_in_local_time() {
        // 2024-01-01 00:00 UTC, which is 09:00 in UTC+9.
        let utc = Clock {
            now: 19_723 * 86_400,
            utc_offset: 0,
        };
        assert_eq!(utc.until_next_day(), 86_400, "midnight itself: a whole day");
        let tokyo = Clock {
            utc_offset: 9 * 3_600,
            ..utc
        };
        assert_eq!(tokyo.until_next_day(), 15 * 3_600);
        let new_york = Clock {
            now: utc.now - 1,
            utc_offset: -5 * 3_600,
        };
        assert_eq!(new_york.until_next_day(), 5 * 3_600 + 1);
    }

    #[test]
    fn type_ahead_builds_a_prefix_until_a_pause() {
        let mut ta = TypeAhead::default();
        ta.tick(1.0);
        assert_eq!(ta.push("F"), "f");
        ta.tick(0.2);
        assert_eq!(ta.push("o"), "fo");
        ta.tick(0.8);
        assert_eq!(ta.push("b"), "b");
        let entries = vec![file("alpha"), file("Beta"), file("bravo")];
        assert_eq!(find_prefix(&[0, 1, 2], &entries, "br"), Some(2));
        assert_eq!(find_prefix(&[0, 1, 2], &entries, "z"), None);
    }

    #[test]
    fn typed_paths_drop_the_root_label() {
        assert_eq!(
            parse_typed_path("~/Projects/app/", "~"),
            ["Projects", "app"]
        );
        assert_eq!(parse_typed_path("/Projects//app", "~"), ["Projects", "app"]);
        assert_eq!(parse_typed_path("~", "~"), Vec::<String>::new());
    }
}
