//! Status dot — the small glowing light in front of a session or agent row
//! (Forge's 6 px dot with a 5 px glow): green while it runs, amber while it
//! waits on the user, accent when it has something unread, any hue the
//! caller picks for a state of its own, and nothing when idle. One that asks
//! the user something outright is louder: 8 px, ringed and glowing amber.
//!
//! # Example
//! ```ignore
//! status_dot(list, &style, (x + 3.0, row_mid), Status::Running);
//! status_dot(list, &style, (x + 3.0, row_mid), Status::Hue(300.0));
//! ```

use crate::color::oklch;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{StyleKey, StyleResolver};

use super::DrawList;

/// The dot's diameter.
pub const STATUS_DOT_SIZE: f32 = 6.0;
/// How far the glow blurs out.
const GLOW_BLUR: f32 = 5.0;
/// An [`Status::Asking`] dot's diameter, its ring's width, and how far its
/// glow blurs out (the Agent Desktop design's "waiting on you" dot).
const ASKING_SIZE: f32 = 8.0;
const ASKING_RING: f32 = 2.0;
const ASKING_BLUR: f32 = 7.0;

/// Lightness and chroma of a [`Status::Hue`] dot: as bright and as coloured
/// as the status dots, so it reads as one of them.
const HUE_LIGHTNESS: f32 = 0.78;
const HUE_CHROMA: f32 = 0.12;
/// How strongly a [`Status::Hue`] dot glows.
const HUE_GLOW_ALPHA: f32 = 0.5;

/// What a [`status_dot`] shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Status {
    /// Nothing happening: no dot at all (the space stays, so rows line up).
    #[default]
    Idle,
    /// Working right now (`--ok`, glowing).
    Running,
    /// Waiting on the user (`--warn-meta`, with a soft amber glow).
    Waiting,
    /// Asking the user something, and stopped until they answer: a bigger
    /// `--warn-meta` dot in an amber ring and a wider glow, to be found from
    /// across the screen.
    Asking,
    /// Has something the user hasn't seen (`--accent-dirty`, accent glow).
    Unread,
    /// Any hue, in degrees (OKLCH), glowing in its own colour: for a state
    /// the three above don't name (an agent working somewhere else). Same
    /// lightness and chroma whatever the hue, like
    /// [`BadgeTone::Hue`](super::BadgeTone::Hue).
    Hue(f32),
}

impl Status {
    /// The dot's fill and its glow under `s`, or `None` when idle.
    pub fn colors(self, s: &StyleResolver) -> Option<([f32; 4], [f32; 4])> {
        let with = |c: [f32; 4], a: f32| [c[0], c[1], c[2], a];
        match self {
            Status::Idle => None,
            Status::Running => {
                let ok = s.color(StyleKey::StatusOk);
                Some((ok, with(ok, 0.6)))
            }
            Status::Waiting | Status::Asking => Some((
                s.color(StyleKey::WarnMeta),
                with(s.color(StyleKey::Warning), 0.2),
            )),
            Status::Unread => Some((
                s.color(StyleKey::AccentDirty),
                with(s.color(StyleKey::Accent), 0.65),
            )),
            Status::Hue(hue) => {
                let fill = oklch(HUE_LIGHTNESS, HUE_CHROMA, hue, 1.0);
                Some((fill, with(fill, HUE_GLOW_ALPHA)))
            }
        }
    }

    /// The dot's diameter, without its ring or glow.
    pub fn size(self) -> f32 {
        match self {
            Status::Asking => ASKING_SIZE,
            _ => STATUS_DOT_SIZE,
        }
    }
}

/// Draw a status dot centred on `center`. An idle status draws nothing.
pub fn status_dot(list: &mut DrawList, s: &StyleResolver, center: (f32, f32), status: Status) {
    let Some((fill, glow)) = status.colors(s) else {
        return;
    };
    if status == Status::Asking {
        asking_dot(list, s, center, ASKING_SIZE, ASKING_BLUR);
        return;
    }
    let r = STATUS_DOT_SIZE * 0.5;
    let rect = Rect::new(center.0 - r, center.1 - r, STATUS_DOT_SIZE, STATUS_DOT_SIZE);
    list.box_shadow_outset(
        rect,
        CornerRadii::uniform(r),
        BoxShadow {
            blur: GLOW_BLUR,
            color: glow,
            ..BoxShadow::default()
        },
    );
    list.rounded_rect(rect, r, fill);
}

/// The ringed amber dot of something asking the user, `size` across with a
/// glow `blur` wide: `0 0 0 2px oklch(0.75 0.13 75 / 0.22), 0 0 <blur>px
/// var(--warn-soft)`. [`Status::Asking`] is the 8 px one; a group header's
/// count of what asks is a 6 px one.
pub(crate) fn asking_dot(
    list: &mut DrawList,
    s: &StyleResolver,
    center: (f32, f32),
    size: f32,
    blur: f32,
) {
    let (fill, glow) = Status::Asking.colors(s).expect("asking has colours");
    let r = size * 0.5;
    let rect = Rect::new(center.0 - r, center.1 - r, size, size);
    let radii = CornerRadii::uniform(r);
    list.box_shadow_outset(
        rect,
        radii,
        BoxShadow {
            blur,
            color: glow,
            ..BoxShadow::default()
        },
    );
    list.box_shadow_outset(
        rect,
        radii,
        BoxShadow {
            spread: ASKING_RING,
            color: oklch(0.75, 0.13, 75.0, 0.22),
            ..BoxShadow::default()
        },
    );
    list.rounded_rect(rect, r, fill);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn frame(status: Status) -> DrawList {
        let theme = Theme::default();
        let mut list = DrawList::new();
        status_dot(&mut list, &StyleResolver::new(&theme), (10.0, 10.0), status);
        list
    }

    #[test]
    fn idle_draws_nothing() {
        let list = frame(Status::Idle);
        let counts = list.prim_counts();
        assert_eq!(counts.shadow_instances, 0);
        assert_eq!(counts.chrome_instances, 0);
        assert_eq!(counts.vertices, 0);
    }

    #[test]
    fn each_live_state_glows_in_its_own_colour() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let fills: Vec<_> = [Status::Running, Status::Waiting, Status::Unread]
            .into_iter()
            .map(|st| {
                let list = frame(st);
                assert_eq!(list.shadow_instance_count(), 1, "{st:?} glows");
                st.colors(&s).unwrap().0
            })
            .collect();
        assert_ne!(fills[0], fills[1]);
        assert_ne!(fills[1], fills[2]);
        assert_eq!(fills[0], theme.status_ok);
    }

    #[test]
    fn asking_is_waiting_made_louder() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        assert_eq!(Status::Asking.colors(&s), Status::Waiting.colors(&s));
        assert!(Status::Asking.size() > Status::Waiting.size());
        let list = frame(Status::Asking);
        assert_eq!(list.shadow_instance_count(), 2, "a glow and a ring");
    }

    #[test]
    fn a_hue_dot_glows_in_the_hue_it_is_given() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let list = frame(Status::Hue(300.0));
        assert_eq!(list.shadow_instance_count(), 1, "it glows");
        let (fill, glow) = Status::Hue(300.0).colors(&s).unwrap();
        assert_eq!(fill, oklch(0.78, 0.12, 300.0, 1.0));
        assert_eq!(glow, [fill[0], fill[1], fill[2], 0.5]);
        let (other, _) = Status::Hue(145.0).colors(&s).unwrap();
        assert_ne!(fill, other, "another hue, another colour");
    }
}
