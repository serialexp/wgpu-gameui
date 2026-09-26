//! Status dot — the small glowing light in front of a session or agent row
//! (Forge's 6 px dot with a 5 px glow): green while it runs, amber while it
//! waits on the user, accent when it has something unread, and nothing when
//! idle.
//!
//! # Example
//! ```ignore
//! status_dot(list, &style, (x + 3.0, row_mid), Status::Running);
//! ```

use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{StyleKey, StyleResolver};

use super::DrawList;

/// The dot's diameter.
pub const STATUS_DOT_SIZE: f32 = 6.0;
/// How far the glow blurs out.
const GLOW_BLUR: f32 = 5.0;

/// What a [`status_dot`] shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Status {
    /// Nothing happening: no dot at all (the space stays, so rows line up).
    #[default]
    Idle,
    /// Working right now (`--ok`, glowing).
    Running,
    /// Waiting on the user (`--warn-meta`, with a soft amber glow).
    Waiting,
    /// Has something the user hasn't seen (`--accent-dirty`, accent glow).
    Unread,
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
            Status::Waiting => Some((
                s.color(StyleKey::WarnMeta),
                with(s.color(StyleKey::Warning), 0.2),
            )),
            Status::Unread => Some((
                s.color(StyleKey::AccentDirty),
                with(s.color(StyleKey::Accent), 0.65),
            )),
        }
    }
}

/// Draw a status dot centred on `center`. An idle status draws nothing.
pub fn status_dot(list: &mut DrawList, s: &StyleResolver, center: (f32, f32), status: Status) {
    let Some((fill, glow)) = status.colors(s) else {
        return;
    };
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
}
