//! Cursor state control — let widgets request a cursor shape per frame — and
//! the Forge design system's cursor artwork to show for it.
//!
//! The library is windowing-agnostic: it never touches winit/SDL directly.
//! Instead, widgets that want a non-default pointer (a text field wants an
//! I-beam, a button wants a hand, a drag handle wants a grab cursor) call
//! [`DrawContext::request_cursor`](crate::DrawContext::request_cursor) during
//! draw. Those requests accumulate into a caller-owned [`CursorState`]; after
//! the frame the application reads [`CursorState::resolve`] (or
//! [`take`](CursorState::take)) and shows the [`CursorIcon`] through its own
//! windowing API: [`CursorIcon::image`] draws the Forge artwork at the
//! window's scale for a custom cursor, and [`CursorIcon::fallback`] names the
//! native cursor to use where custom ones aren't available.
//!
//! ```no_run
//! # use wgpu_gameui::{CursorState, CursorIcon};
//! let mut cursor = CursorState::new();
//! // --- each frame ---
//! cursor.begin_frame();          // clear last frame's request
//! // ... draw the UI; widgets call ctx.request_cursor(..) ...
//! let icon = cursor.resolve();   // CursorIcon::Default if nothing asked
//! // Draw it once per (icon, scale) and keep the window's custom cursor:
//! let image = icon.image(2.0);   // 64×64 RGBA, hotspot scaled with it
//! # let _ = image;
//! ```
//!
//! The artwork is `assets/cursors/cursors.css`, Forge's `tokens/cursors.css`
//! vendored unchanged; `build.rs` turns it into the table below. Every cursor
//! is a 32-unit square ([`CURSOR_SIZE`]) in one carved style: a light body,
//! a hard dark edge and a soft drop shadow.

/// One cursor of the Forge set, as `build.rs` reads it from the stylesheet.
struct ForgeCursor {
    /// The Forge (and CSS) name, `nwse-resize`.
    name: &'static str,
    /// The hotspot in the 32-unit grid.
    hotspot: [u16; 2],
    /// The CSS keyword of the native cursor to show instead.
    fallback: &'static str,
    /// The SVG, 32 units square.
    svg: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/forge_cursors.rs"));

/// The side of every Forge cursor's square, in logical pixels.
pub const CURSOR_SIZE: u16 = 32;

/// A cursor shape a widget can request for the current frame: the Forge
/// design system's cursor set, in its order. The application shows the
/// resolved icon through its windowing API, with [`image`](Self::image)'s
/// artwork or the native [`fallback`](Self::fallback).
///
/// Forge groups them as pointers, precision, resize, and two game sets: tools
/// (what a click does to the world) and orders (RTS / colony commands). The
/// game tools share one pose, handle bottom-right and head top-left, with the
/// hotspot on the head. An invalid action is [`NotAllowed`](Self::NotAllowed),
/// never a tool tinted red.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorIcon {
    /// The normal arrow pointer.
    #[default]
    Default,
    /// Hand / pointing cursor — over clickable things (buttons, tabs, links).
    Pointer,
    /// Open hand — over something draggable that isn't being dragged.
    Grab,
    /// Closed hand — while actively dragging.
    Grabbing,
    /// Four-way arrows — moving something freely.
    Move,
    /// Arrow with a plus — a drop that copies.
    Copy,
    /// Arrow with a (still) spinner — working, but still taking input.
    Busy,
    /// Arrow with a no-entry sign — over a disabled or invalid target.
    NotAllowed,
    /// I-beam — over editable or selectable text.
    Text,
    /// Crosshair — precise picking.
    Crosshair,
    /// Eyedropper — sampling a colour.
    Eyedropper,
    /// Magnifier with a plus.
    ZoomIn,
    /// Magnifier with a minus.
    ZoomOut,
    /// Horizontal resize (↔) — an edge, or dragging a value sideways.
    EwResize,
    /// Vertical resize (↕) — an edge.
    NsResize,
    /// Diagonal resize (↘, north-west/south-east) — bottom-right corner grips.
    NwseResize,
    /// Diagonal resize (↗, north-east/south-west).
    NeswResize,
    /// A column divider: arrows apart from a split — vertical split bars.
    ColResize,
    /// A row divider — horizontal split bars.
    RowResize,
    /// Game tool: hammer — build.
    Build,
    /// Game tool: wrench — repair.
    Repair,
    /// Game tool: sickle — harvest crops.
    Harvest,
    /// Game tool: axe — chop trees.
    Chop,
    /// Game tool: pickaxe — mine rock and ore.
    Mine,
    /// Game order: a ground marker — move here.
    MoveTo,
    /// Game order: a flag — set a rally point.
    Rally,
    /// Game order: a sword — attack (over an enemy).
    Attack,
    /// Game order: a shield — guard.
    Guard,
    /// Game order: arrow with a worker — assign a worker to a building.
    Assign,
    /// Game order: a speech bubble — talk to an NPC.
    Talk,
    /// Game tool: a brush — paint zones, stockpiles or farm plots (drag to paint).
    Zone,
    /// Game order: open hand with an up arrow — haul an item.
    Pickup,
    /// Game tool: corner brackets — a valid placement.
    Place,
    /// Game order: a reticle — ranged or ability aim.
    Target,
    /// Game tool: arrow with a cross — demolish.
    Demolish,
}

impl CursorIcon {
    /// Every cursor, in Forge's order.
    pub const ALL: [CursorIcon; 35] = [
        CursorIcon::Default,
        CursorIcon::Pointer,
        CursorIcon::Grab,
        CursorIcon::Grabbing,
        CursorIcon::Move,
        CursorIcon::Copy,
        CursorIcon::Busy,
        CursorIcon::NotAllowed,
        CursorIcon::Text,
        CursorIcon::Crosshair,
        CursorIcon::Eyedropper,
        CursorIcon::ZoomIn,
        CursorIcon::ZoomOut,
        CursorIcon::EwResize,
        CursorIcon::NsResize,
        CursorIcon::NwseResize,
        CursorIcon::NeswResize,
        CursorIcon::ColResize,
        CursorIcon::RowResize,
        CursorIcon::Build,
        CursorIcon::Repair,
        CursorIcon::Harvest,
        CursorIcon::Chop,
        CursorIcon::Mine,
        CursorIcon::MoveTo,
        CursorIcon::Rally,
        CursorIcon::Attack,
        CursorIcon::Guard,
        CursorIcon::Assign,
        CursorIcon::Talk,
        CursorIcon::Zone,
        CursorIcon::Pickup,
        CursorIcon::Place,
        CursorIcon::Target,
        CursorIcon::Demolish,
    ];

    /// The artwork: the variants are declared in the stylesheet's order (a test
    /// checks the names line up).
    fn art(self) -> &'static ForgeCursor {
        &FORGE_CURSORS[self as usize]
    }

    /// The Forge name, which is also the CSS one where CSS has the shape
    /// (`"nwse-resize"`, `"zoom-in"`, `"move-to"`).
    pub fn name(self) -> &'static str {
        self.art().name
    }

    /// The CSS keyword of the native cursor closest to this one (`"progress"`
    /// for [`Busy`](Self::Busy), `"crosshair"` for
    /// [`Eyedropper`](Self::Eyedropper)), for where a custom cursor can't be
    /// shown. winit parses these names into its `CursorIcon`.
    pub fn fallback(self) -> &'static str {
        self.art().fallback
    }

    /// The hotspot in the [`CURSOR_SIZE`] grid: the point that sits on the
    /// pointer's position.
    pub fn hotspot(self) -> [u16; 2] {
        self.art().hotspot
    }

    /// The artwork as SVG, [`CURSOR_SIZE`] units square — to draw the cursor in
    /// the scene (a remote player's pointer, a tutorial) rather than as the
    /// pointer.
    pub fn svg(self) -> &'static str {
        self.art().svg
    }

    /// The artwork drawn for a screen with `scale` pixels per logical pixel
    /// (the window's scale factor): a square of `CURSOR_SIZE × scale` pixels,
    /// rounded, with the hotspot scaled into it. Draws from scratch on every
    /// call, so keep the result (or the windowing API's cursor made from it)
    /// per icon and scale.
    #[cfg(feature = "forge-cursors")]
    pub fn image(self, scale: f32) -> CursorImage {
        use resvg::{tiny_skia, usvg};

        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let side = (f32::from(CURSOR_SIZE) * scale).round().max(1.0);
        let k = side / f32::from(CURSOR_SIZE);
        let side = side as u16;
        let tree = usvg::Tree::from_str(self.svg(), &usvg::Options::default())
            .unwrap_or_else(|e| panic!("Forge cursor {:?} is not valid SVG: {e}", self.name()));
        let mut pixmap = tiny_skia::Pixmap::new(side.into(), side.into())
            .expect("a cursor is at least one pixel square");
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(k, k),
            &mut pixmap.as_mut(),
        );
        // tiny-skia keeps premultiplied alpha; windowing APIs take it straight.
        let rgba = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let hot = |v: u16| ((f32::from(v) * k).round() as u16).min(side - 1);
        let [x, y] = self.hotspot();
        CursorImage {
            rgba,
            width: side,
            height: side,
            hotspot: [hot(x), hot(y)],
            scale: k,
        }
    }

    /// Priority used to arbitrate competing requests within a frame: a request
    /// with a higher priority is never clobbered by a lower one, regardless of
    /// draw order. This keeps an active `Grabbing` (set while dragging) winning
    /// over a stray `Pointer`/`Default` from a widget drawn afterwards, even
    /// when the drag has pulled the pointer off the handle.
    fn priority(self) -> u8 {
        match self {
            CursorIcon::Default => 0,
            CursorIcon::Grab => 2,
            // An in-progress drag should beat hover cursors of equal-or-lower rank.
            CursorIcon::Grabbing => 3,
            _ => 1,
        }
    }
}

/// A cursor drawn for one screen scale ([`CursorIcon::image`]), for the
/// application to hand to its windowing API (winit's
/// `CustomCursor::from_rgba`, or on macOS the scale-aware constructor so the
/// image is shown at its size in points).
#[derive(Debug, Clone, PartialEq)]
#[cfg(feature = "forge-cursors")]
pub struct CursorImage {
    /// `width × height` pixels, row by row, RGBA with straight (not
    /// premultiplied) alpha.
    pub rgba: Vec<u8>,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// The hotspot in pixels.
    pub hotspot: [u16; 2],
    /// Pixels per logical pixel the image was drawn at: the scale asked for,
    /// corrected for the side's rounding to whole pixels.
    pub scale: f32,
}

/// Caller-owned, per-frame cursor request accumulator.
///
/// Widgets call [`request`](Self::request) (via
/// [`DrawContext::request_cursor`](crate::DrawContext::request_cursor)) while
/// drawing. After the frame the application reads [`resolve`](Self::resolve) and
/// applies the icon to its window. Clear it each frame with
/// [`begin_frame`](Self::begin_frame), or use [`take`](Self::take) to read and
/// clear in one call.
///
/// Conflicts are resolved by `CursorIcon::priority`: the highest-priority
/// request of the frame wins, and among equal priorities the **last** request
/// wins (the topmost widget draws last under back-to-front ordering). In
/// practice only the single topmost hovered widget requests at all, because
/// hover testing already honors [`InputState::mouse_consumed`](crate::InputState::mouse_consumed).
#[derive(Debug, Clone, Copy, Default)]
pub struct CursorState {
    requested: Option<CursorIcon>,
}

impl CursorState {
    /// A fresh accumulator with no request (resolves to [`CursorIcon::Default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Request `icon` for this frame. Kept only if its
    /// `priority` is ≥ the current request's, so a
    /// lower-priority request drawn later can't clobber a higher one.
    pub fn request(&mut self, icon: CursorIcon) {
        match self.requested {
            Some(current) if current.priority() > icon.priority() => {}
            _ => self.requested = Some(icon),
        }
    }

    /// The icon to show this frame — the winning request, or
    /// [`CursorIcon::Default`] if nothing was requested.
    pub fn resolve(&self) -> CursorIcon {
        self.requested.unwrap_or_default()
    }

    /// Whether any widget requested a cursor this frame.
    pub fn is_set(&self) -> bool {
        self.requested.is_some()
    }

    /// Clear the request for the next frame. Call once at the top of each frame
    /// before drawing the UI.
    pub fn begin_frame(&mut self) {
        self.requested = None;
    }

    /// Read the resolved icon and clear in one call (read-and-reset). Handy when
    /// you resolve at the end of the frame and want the accumulator empty for
    /// the next one.
    pub fn take(&mut self) -> CursorIcon {
        let icon = self.resolve();
        self.requested = None;
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_resolves_to_default() {
        let c = CursorState::new();
        assert_eq!(c.resolve(), CursorIcon::Default);
        assert!(!c.is_set());
    }

    #[test]
    fn single_request_wins() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Text);
        assert!(c.is_set());
        assert_eq!(c.resolve(), CursorIcon::Text);
    }

    #[test]
    fn equal_priority_last_wins() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Pointer);
        c.request(CursorIcon::Text); // same priority (1) → last wins
        assert_eq!(c.resolve(), CursorIcon::Text);
    }

    #[test]
    fn higher_priority_not_clobbered_by_lower() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Grabbing); // priority 3
        c.request(CursorIcon::Pointer); // priority 1 — ignored
        c.request(CursorIcon::Default); // priority 0 — ignored
        assert_eq!(c.resolve(), CursorIcon::Grabbing);
    }

    #[test]
    fn higher_priority_overrides_earlier_lower() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Pointer); // 1
        c.request(CursorIcon::Grab); // 2 → wins
        assert_eq!(c.resolve(), CursorIcon::Grab);
    }

    #[test]
    fn begin_frame_and_take_clear_state() {
        let mut c = CursorState::new();
        c.request(CursorIcon::Text);
        c.begin_frame();
        assert_eq!(c.resolve(), CursorIcon::Default);

        c.request(CursorIcon::Pointer);
        assert_eq!(c.take(), CursorIcon::Pointer);
        assert!(!c.is_set(), "take() clears the request");
        assert_eq!(c.resolve(), CursorIcon::Default);
    }

    #[test]
    fn default_icon_is_default() {
        assert_eq!(CursorIcon::default(), CursorIcon::Default);
    }

    /// The variants are the stylesheet's cursors, one each, in its order, so
    /// a re-vendored `cursors.css` with a new cursor fails here until it has a
    /// variant.
    #[test]
    fn every_forge_cursor_has_a_variant_in_order() {
        let names: Vec<_> = CursorIcon::ALL.iter().map(|c| c.name()).collect();
        let forge: Vec<_> = FORGE_CURSORS.iter().map(|c| c.name).collect();
        assert_eq!(names, forge);
        for (i, icon) in CursorIcon::ALL.iter().enumerate() {
            assert_eq!(*icon as usize, i, "{icon:?} is out of place in ALL");
        }
    }

    #[test]
    fn names_hotspots_and_fallbacks_come_from_the_stylesheet() {
        assert_eq!(CursorIcon::NwseResize.name(), "nwse-resize");
        assert_eq!(CursorIcon::MoveTo.name(), "move-to");
        assert_eq!(CursorIcon::Default.hotspot(), [4, 3]);
        assert_eq!(CursorIcon::Pointer.hotspot(), [13, 3]);
        assert_eq!(CursorIcon::Eyedropper.hotspot(), [6, 26]);
        assert_eq!(CursorIcon::Busy.fallback(), "progress");
        assert_eq!(CursorIcon::Eyedropper.fallback(), "crosshair");
        assert_eq!(CursorIcon::Demolish.fallback(), "not-allowed");
        assert_eq!(CursorIcon::Pickup.fallback(), "grab");
        for icon in CursorIcon::ALL {
            let [x, y] = icon.hotspot();
            assert!(
                x < CURSOR_SIZE && y < CURSOR_SIZE,
                "{icon:?} hotspot off the square"
            );
            assert!(icon.svg().starts_with("<svg"), "{icon:?}");
        }
    }

    #[test]
    fn resize_cursors_keep_their_priority() {
        let mut c = CursorState::new();
        c.request(CursorIcon::ColResize);
        c.request(CursorIcon::Default);
        assert_eq!(c.resolve(), CursorIcon::ColResize);
    }

    #[cfg(feature = "forge-cursors")]
    mod image {
        use super::super::*;

        /// The RGBA of pixel `(x, y)`.
        fn px(image: &CursorImage, x: u16, y: u16) -> [u8; 4] {
            let i = (usize::from(y) * usize::from(image.width) + usize::from(x)) * 4;
            image.rgba[i..i + 4].try_into().unwrap()
        }

        #[test]
        fn is_drawn_at_the_scale_with_the_hotspot_scaled() {
            let one = CursorIcon::Default.image(1.0);
            assert_eq!((one.width, one.height, one.hotspot), (32, 32, [4, 3]));
            assert_eq!(one.rgba.len(), 32 * 32 * 4);
            let two = CursorIcon::Default.image(2.0);
            assert_eq!((two.width, two.height, two.hotspot), (64, 64, [8, 6]));
            assert_eq!(two.scale, 2.0);
            // 32 × 1.25 = 40; the pointer's 13 × 1.25 = 16.25 rounds to 16.
            let odd = CursorIcon::Pointer.image(1.25);
            assert_eq!((odd.width, odd.hotspot), (40, [16, 4]));
            // Nonsense scales draw at 1.
            assert_eq!(CursorIcon::Text.image(0.0).width, 32);
            assert_eq!(CursorIcon::Text.image(f32::NAN).width, 32);
        }

        #[test]
        fn the_arrow_is_a_light_body_in_a_dark_edge_on_clear_ground() {
            let arrow = CursorIcon::Default.image(2.0);
            // Inside the arrow's body (the path runs 4..18 × 3..22 units).
            assert_eq!(px(&arrow, 12, 20), [0xf1, 0xf5, 0xf9, 0xff]);
            // Just inside its left edge, x = 4 units: the dark edge.
            let [r, g, b, a] = px(&arrow, 7, 30);
            assert!(
                r < 0x20 && g < 0x20 && b < 0x20 && a > 0xc0,
                "edge {r} {g} {b} {a}"
            );
            // Far from it: nothing.
            assert_eq!(px(&arrow, 60, 4)[3], 0);
        }

        #[test]
        fn every_cursor_draws_inside_its_square() {
            for icon in CursorIcon::ALL {
                let image = icon.image(1.0);
                let opaque = image.rgba.chunks_exact(4).filter(|p| p[3] == 0xff).count();
                assert!(opaque > 20, "{icon:?} drew {opaque} opaque pixels");
                // The hotspot sits on or right by the art, never on clear ground
                // far from it.
                let [hx, hy] = image.hotspot;
                let near = (hy.saturating_sub(2)..=(hy + 2).min(31)).any(|y| {
                    (hx.saturating_sub(2)..=(hx + 2).min(31)).any(|x| px(&image, x, y)[3] > 0)
                });
                assert!(near, "{icon:?} hotspot {hx},{hy} is off the art");
            }
        }
    }
}
