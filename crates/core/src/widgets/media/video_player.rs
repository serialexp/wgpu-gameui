//! Video player — a picture with a toolbar strip beneath it (Forge
//! `VideoPlayer`). The controls never hide: Forge doesn't auto-hide UI.
//!
//! The picture is fitted inside the frame on black. Before the first one
//! arrives the frame is a striped well showing the time. Clicking the frame
//! plays or pauses, and a double-click asks for full screen. The toolbar
//! holds the timeline, then a row of the [`PlayButton`], a mute key, a
//! volume scrubber, the time, the speed key and the full screen key. While
//! the player has the keyboard, Space and K play or pause, ←/→ go 5 s back
//! or on, M mutes and F asks for full screen.
//!
//! Playback is a [`MediaSource`]'s. Full screen is the app's: the player is
//! told whether it is [`fullscreen`](VideoPlayer::fullscreen), drawn without
//! its border then, and says when the user asked to go in or out
//! ([`VideoPlayerOutput::toggle_fullscreen`]).
//!
//! It takes the Tab ring as `id` (the frame), `id + 1` (the timeline),
//! `id + 2` (play), `id + 3` (mute), `id + 4` (volume), `id + 5` (speed) and
//! `id + 6` (full screen).
//!
//! # Example
//! ```ignore
//! let height = player.height_for(width, 16.0 / 9.0, &ctx.styles());
//! let output = VideoPlayer::new(PLAYER)
//!     .title("intro.mp4")
//!     .fullscreen(ui.fullscreen.is_fullscreen())
//!     .draw(&mut source, &mut state, rect, &mut ctx);
//! if output.toggle_fullscreen {
//!     ui.fullscreen.toggle();
//! }
//! ```

use std::time::Duration;

use crate::Weight;
use crate::chrome::Edge;
use crate::layout::Rect;
use crate::render::PhosphorIcon;
use crate::shadow::{BoxShadow, CornerRadii};
use crate::style::{Ink, StyleKey, StyleResolver, TextSize};
use crate::text::TextBlock;

use super::super::material::Tone;
use super::super::{DragCapture, DrawContext, DrawList, FocusId, IconKey, Pressable};
use super::{
    MediaSource, PlayButton, PlayState, SPEEDS, Scrubber, format_speed, format_time, next_speed,
    toggle,
};
use crate::keys::Key;

/// The toolbar's padding (`6px 10px`).
const PAD_Y: f32 = 6.0;
const PAD_X: f32 = 10.0;
/// The timeline's row: 12 px tall, with margins of `2px 2px 8px`.
const TIMELINE: f32 = 12.0;
const TIMELINE_TOP: f32 = 2.0;
const TIMELINE_SIDE: f32 = 2.0;
const TIMELINE_BELOW: f32 = 8.0;
/// The gap between the keys in the row.
const GAP: f32 = 6.0;
/// The volume scrubber's width.
const VOLUME: f32 = 64.0;
/// The volume's step under ←/→.
const VOLUME_STEP: f32 = 0.05;
/// The volume an unmute at zero comes back at.
const UNMUTED: f32 = 0.5;
/// The time readout's extra room on its left (`margin-left: 4px`).
const TIME_LEFT: f32 = 4.0;
/// The speed key: at least 34 px, 6 px either side of its text.
const SPEED_MIN: f32 = 34.0;
const SPEED_PAD: f32 = 6.0;
/// How far ←/→ go on the frame.
const SEEK_STEP: Duration = Duration::from_secs(5);
/// The toolbar's lit line (`inset 0 1px 0 rgba(255,255,255,0.06)`).
const TOOLBAR_LIGHT: [f32; 4] = [1.0, 1.0, 1.0, 0.06];
/// The placeholder's lines: 1 px every 9 px along their 135° axis, `9·√2` px
/// across.
const STRIPES: [f32; 4] = [1.0, 1.0, 1.0, 0.035];
const STRIPE_STEP: f32 = 9.0 * std::f32::consts::SQRT_2;
/// The cover over a picture that can't play.
const ERROR_COVER: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
/// The title's shade, from the top of the frame to nothing, and its padding
/// (`8px 10px 18px`).
const TITLE_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.65];
const TITLE_TOP: f32 = 8.0;
const TITLE_BELOW: f32 = 18.0;
/// The carved text shadow's alpha.
const CARVE_ALPHA: u8 = 153;
/// The player's border, when not full screen.
const BORDER: f32 = 1.0;
/// Its drop: `0 1px 3px rgba(0,0,0,0.6)`.
const DROP: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0; 4];

/// What a player keeps between frames: the drags on its scrubbers and how
/// far its loading spinner has turned.
#[derive(Debug, Default)]
pub struct VideoPlayerState {
    capture: DragCapture,
    spin: f32,
    /// Where the timeline's drag last scrubbed to, to seek exactly when it
    /// ends.
    scrubbed: Option<f32>,
}

impl VideoPlayerState {
    /// A player's state before its first frame.
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a [`VideoPlayer`] saw this frame, beyond what it told its source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoPlayerOutput {
    /// The user asked to go in or out of full screen.
    pub toggle_fullscreen: bool,
}

/// A video player. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct VideoPlayer<'a> {
    id: FocusId,
    title: Option<&'a str>,
    fullscreen: bool,
}

/// The drag ids of the two scrubbers, in the player's own capture.
const TIMELINE_DRAG: u64 = 1;
const VOLUME_DRAG: u64 = 2;

impl<'a> VideoPlayer<'a> {
    /// How many focus ids it takes, from its `id` (see the [module
    /// docs](self)).
    pub const FOCUS_IDS: FocusId = 7;

    /// A player taking the Tab ring from `id` (see the [module docs](self)).
    pub fn new(id: FocusId) -> Self {
        Self {
            id,
            title: None,
            fullscreen: false,
        }
    }

    /// A title over the top of the picture.
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Whether the app shows it full screen: no border, and the full screen
    /// key held in.
    pub fn fullscreen(mut self, fullscreen: bool) -> Self {
        self.fullscreen = fullscreen;
        self
    }

    fn border(&self) -> f32 {
        if self.fullscreen { 0.0 } else { BORDER }
    }

    /// The toolbar's height: its edge, padding, the timeline row and the row
    /// of keys.
    pub fn toolbar_height(s: &StyleResolver) -> f32 {
        let keys = PlayButton::new(PlayState::Paused).outer_size(s)[1];
        1.0 + PAD_Y + TIMELINE_TOP + TIMELINE + TIMELINE_BELOW + keys + PAD_Y
    }

    /// The height it takes at `width` for a picture `aspect` wide to 1 tall.
    pub fn height_for(&self, width: f32, aspect: f32, s: &StyleResolver) -> f32 {
        let inner = (width - self.border() * 2.0).max(0.0);
        let frame = if aspect > 0.0 { inner / aspect } else { 0.0 };
        (frame + Self::toolbar_height(s) + self.border() * 2.0).round()
    }

    /// The widest it can be in `height` for a picture `aspect` wide to 1
    /// tall, in whole pixels: the inverse of [`height_for`](Self::height_for).
    pub fn width_for(&self, height: f32, aspect: f32, s: &StyleResolver) -> f32 {
        if aspect <= 0.0 {
            return 0.0;
        }
        // `height_for` rounds to nearest, so it fits while the frame and
        // the rest come to less than half a pixel over a whole `height`.
        let border = self.border() * 2.0;
        let frame = (height.floor() + 0.5 - Self::toolbar_height(s) - border).max(0.0);
        let mut width = ((frame * aspect + border).ceil() - 1.0).max(0.0);
        // Float error, a pixel at most either way.
        if width > 0.0 && self.height_for(width, aspect, s) > height {
            width -= 1.0;
        } else if self.height_for(width + 1.0, aspect, s) <= height {
            width += 1.0;
        }
        width
    }

    /// The narrowest it can be with its keys side by side and room for a
    /// time under ten minutes between them. `list` measures the speed key's
    /// widest label.
    pub fn min_width(&self, s: &StyleResolver, list: &mut DrawList) -> f32 {
        let key = |icon| IconKey::new(icon, IconKey::TOOLBAR).outer_size(s)[0];
        let play = PlayButton::new(PlayState::Paused).outer_size(s)[0];
        let speed = SPEEDS
            .iter()
            .map(|&speed| speed_width(s, list, &format_speed(speed)))
            .fold(0.0, f32::max);
        let time = s.mono_width(list, "0:00 / 0:00", TextSize::Dense);
        (self.border() * 2.0
            + PAD_X * 2.0
            + play
            + GAP
            + key(PhosphorIcon::SpeakerHigh)
            + GAP
            + VOLUME
            + GAP
            + TIME_LEFT
            + time
            + GAP
            + speed
            + GAP
            + key(PhosphorIcon::CornersOut))
        .ceil()
    }

    /// Draw it in `rect`, telling `source` what the user did.
    pub fn draw(
        &self,
        source: &mut dyn MediaSource,
        state: &mut VideoPlayerState,
        rect: Rect,
        ctx: &mut DrawContext,
    ) -> VideoPlayerOutput {
        let mut output = VideoPlayerOutput::default();
        ctx.push_debug_scope_rect("VideoPlayer", rect);
        let s = ctx.styles();
        if !self.fullscreen {
            ctx.draw_list.box_shadow_outset(
                rect,
                CornerRadii::uniform(s.scalar(StyleKey::BorderRadius)),
                BoxShadow {
                    offset: [0.0, 1.0],
                    blur: 3.0,
                    color: DROP,
                    ..BoxShadow::default()
                },
            );
        }
        let inner = rect.inset(self.border());
        let toolbar_h = Self::toolbar_height(&s).min(inner.height);
        let frame = Rect::new(inner.x, inner.y, inner.width, inner.height - toolbar_h);
        let toolbar = Rect::new(inner.x, frame.bottom(), inner.width, toolbar_h);

        if source.state() == PlayState::Loading {
            state.spin += ctx.input.frame_dt;
        }

        // The frame takes clicks and, while the player has the keyboard, keys.
        let input = ctx.input;
        ctx.register_focus(self.id);
        let over_frame = !input.mouse_consumed && frame.contains(input.mouse_x, input.mouse_y);
        if over_frame && input.mouse_clicked {
            ctx.focus.request(self.id);
            toggle(source);
            if input.mouse_double_clicked {
                output.toggle_fullscreen = true;
            }
        }
        self.keys(source, &mut output, ctx);

        self.paint_frame(source, frame, ctx);
        self.paint_toolbar(source, state, toolbar, &mut output, ctx);

        if !self.fullscreen {
            let s = ctx.styles();
            ctx.draw_list.chrome_rect(
                rect,
                s.scalar(StyleKey::BorderRadius),
                BORDER,
                [0.0; 4],
                s.color(StyleKey::EdgeHard),
            );
        }
        if ctx.focus.is_focused(self.id) {
            ctx.draw_focus_ring(rect);
        }
        if let Some(next) = source.next_change() {
            ctx.draw_list.repaint_after(next.as_secs_f32());
        }
        ctx.pop_debug_scope();
        output
    }

    /// The keys it takes: its letters while any of it has the keyboard,
    /// Space and the arrows while its frame has.
    fn keys(
        &self,
        source: &mut dyn MediaSource,
        output: &mut VideoPlayerOutput,
        ctx: &DrawContext,
    ) {
        let input = ctx.input;
        let frame = ctx.focus.is_focused(self.id);
        let within = ctx
            .focus
            .focused()
            .is_some_and(|id| (self.id..self.id + Self::FOCUS_IDS).contains(&id));
        // The letters work from anywhere in the player; with the shortcut
        // key or Alt held, they are the app's.
        let letter = |key| {
            within
                && input.keys.was_pressed(Key::Char(key))
                && !input.ctrl_pressed
                && !input.alt_down
        };
        if (frame && input.key_space) || letter('K') {
            toggle(source);
        }
        // Space and the arrows are a focused key's or scrubber's own.
        if frame && (input.nav.right || input.nav.left) {
            let at = source.time();
            let to = if input.nav.right {
                at + SEEK_STEP
            } else {
                at.saturating_sub(SEEK_STEP)
            };
            let to = source.length().map_or(to, |length| to.min(length));
            if to != at && source.state() != PlayState::Error {
                source.seek(to);
            }
        }
        if letter('M') {
            toggle_mute(source);
        }
        if letter('F') {
            output.toggle_fullscreen = true;
        }
    }

    /// The picture (or the placeholder), the error cover and the title.
    fn paint_frame(&self, source: &dyn MediaSource, frame: Rect, ctx: &mut DrawContext) {
        let s = ctx.styles();
        let list = &mut *ctx.draw_list;
        list.push_clip(frame);
        match source.picture() {
            Some(picture) => {
                list.quad(frame.x, frame.y, frame.width, frame.height, BLACK);
                let fitted = contain(frame, picture.width as f32, picture.height as f32);
                list.image(picture.sprite, fitted, WHITE);
            }
            None => {
                list.quad(
                    frame.x,
                    frame.y,
                    frame.width,
                    frame.height,
                    s.color(StyleKey::WellDeep),
                );
                list.hatch(frame, STRIPE_STEP, STRIPES);
                // The error's words take the middle instead.
                if source.state() != PlayState::Error {
                    centred_line(
                        list,
                        s.mono_block(
                            format_time(source.time()),
                            0.0,
                            0.0,
                            TextSize::Row,
                            Ink::Dim,
                        ),
                        frame,
                        frame.y + frame.height * 0.5,
                    );
                }
            }
        }
        if source.state() == PlayState::Error {
            list.quad(frame.x, frame.y, frame.width, frame.height, ERROR_COVER);
            let middle = frame.y + frame.height * 0.5;
            let danger = s.color(StyleKey::DangerText);
            let line = s.text_size(TextSize::Menu) + 4.0;
            let headline = s
                .sans_block(
                    "Can’t play this video",
                    0.0,
                    0.0,
                    TextSize::Menu,
                    Ink::Value,
                )
                .with_weight(Weight::MEDIUM)
                .with_color_f32(danger);
            match source.error() {
                Some(why) => {
                    centred_line(list, headline, frame, middle - line * 0.5);
                    let why = s.sans_block(why, 0.0, 0.0, TextSize::Meta, Ink::Dim);
                    centred_line(list, why, frame, middle + line * 0.5);
                }
                None => centred_line(list, headline, frame, middle),
            }
        }
        if let Some(title) = self.title {
            let size = s.text_size(TextSize::Menu);
            let shade = Rect::new(
                frame.x,
                frame.y,
                frame.width,
                TITLE_TOP + size + TITLE_BELOW,
            );
            list.vertical_gradient(shade, TITLE_SHADE, [0.0; 4]);
            list.text(
                s.sans_block(
                    title,
                    frame.x + PAD_X,
                    frame.y + TITLE_TOP,
                    TextSize::Menu,
                    Ink::Title,
                )
                .with_weight(Weight::MEDIUM)
                .with_shadow(0, 0, 0, CARVE_ALPHA, 0.0, -1.0, 0.0)
                .with_max_width((frame.width - PAD_X * 2.0).max(0.0))
                .with_ellipsis(),
            );
        }
        list.pop_clip();
    }

    fn paint_toolbar(
        &self,
        source: &mut dyn MediaSource,
        state: &mut VideoPlayerState,
        toolbar: Rect,
        output: &mut VideoPlayerOutput,
        ctx: &mut DrawContext,
    ) {
        let s = ctx.styles();
        {
            let list = &mut *ctx.draw_list;
            let [top, bottom] = s.toolbar().rail_colors;
            list.vertical_gradient(toolbar, top, bottom);
            list.quad(
                toolbar.x,
                toolbar.y + 1.0,
                toolbar.width,
                1.0,
                TOOLBAR_LIGHT,
            );
            list.edge_line(toolbar, Edge::Top, 1.0, s.color(StyleKey::EdgeHard));
        }
        let error = source.state() == PlayState::Error;
        let left = toolbar.x + PAD_X;
        let right = toolbar.right() - PAD_X;

        // The timeline.
        let length = source.length().unwrap_or_default().as_secs_f32();
        let timeline = Rect::new(
            left + TIMELINE_SIDE,
            toolbar.y + 1.0 + PAD_Y + TIMELINE_TOP,
            (right - left - TIMELINE_SIDE * 2.0).max(0.0),
            TIMELINE,
        );
        let mut scrubber = Scrubber::new(length)
            .enabled(!error && length > 0.0)
            .focusable(self.id + 1)
            .name("Timeline");
        if let Some(buffered) = source.buffered() {
            scrubber = scrubber.buffered(buffered.as_secs_f32());
        }
        let seek = scrubber.draw(
            source.time().as_secs_f32(),
            TIMELINE_DRAG,
            &mut state.capture,
            timeline,
            ctx,
        );
        let at = |seconds: f32| Duration::from_secs_f32(seconds.max(0.0));
        match seek.seek {
            Some(to) if seek.dragging => {
                source.scrub(at(to));
                state.scrubbed = Some(to);
            }
            Some(to) => source.seek(at(to)),
            None => {}
        }
        if seek.ended
            && let Some(to) = state.scrubbed.take()
        {
            source.seek(at(to));
        }

        // The row of keys, its faces' tops on one line.
        let row_y = timeline.bottom() + TIMELINE_BELOW;
        let play = PlayButton::new(source.state())
            .phase(state.spin)
            .focusable(self.id + 2);
        let [w, h] = play.outer_size(&s);
        let face_h = PlayButton::SIZE;
        if play.draw(Rect::new(left, row_y, w, h), ctx).clicked {
            toggle(source);
        }
        let mut x = left + w + GAP;

        let mute = IconKey::new(speaker(source), IconKey::TOOLBAR)
            .tone(Tone::Ghost)
            .focusable(self.id + 3);
        let [w, h] = mute.outer_size(&s);
        if mute.draw(Rect::new(x, row_y, w, h), ctx).clicked {
            toggle_mute(source);
        }
        x += w + GAP;

        let shown = if source.muted() { 0.0 } else { source.volume() };
        let percent = |volume: f32| format!("{}%", (volume * 100.0).round());
        let volume = Scrubber::new(1.0)
            .step(VOLUME_STEP)
            .format(&percent)
            .focusable(self.id + 4)
            .name("Volume");
        let set = volume.draw(
            shown,
            VOLUME_DRAG,
            &mut state.capture,
            Rect::new(x, row_y + (face_h - TIMELINE) * 0.5, VOLUME, TIMELINE),
            ctx,
        );
        if let Some(to) = set.seek {
            set_volume(source, to);
        }
        x += VOLUME + GAP + TIME_LEFT;

        // From the right: full screen, then speed.
        let full = IconKey::new(
            if self.fullscreen {
                PhosphorIcon::CornersIn
            } else {
                PhosphorIcon::CornersOut
            },
            IconKey::TOOLBAR,
        )
        .tone(if self.fullscreen {
            Tone::Latch
        } else {
            Tone::Ghost
        })
        .held(self.fullscreen)
        .focusable(self.id + Self::FOCUS_IDS - 1);
        let [w, h] = full.outer_size(&s);
        let full_x = right - w;
        if full.draw(Rect::new(full_x, row_y, w, h), ctx).clicked {
            output.toggle_fullscreen = true;
        }

        let speed_text = format_speed(source.speed());
        let speed_w = speed_width(&s, ctx.draw_list, &speed_text);
        let speed_key = Pressable::new()
            .tone(Tone::Ghost)
            .focusable(self.id + 5)
            .name("Speed");
        let travel = speed_key.travel_px(&s);
        let speed_rect = Rect::new(full_x - GAP - speed_w, row_y, speed_w, face_h + travel);
        let pressed = speed_key.draw(speed_rect, ctx, |key, ctx| {
            let s = ctx.styles();
            let ink = if key.down { Ink::Glyph } else { Ink::Cell };
            let block = s.mono_block(speed_text.as_str(), 0.0, 0.0, TextSize::Meta, ink);
            centred_line(
                ctx.draw_list,
                block,
                key.face,
                key.face.y + key.face.height * 0.5,
            );
        });
        if pressed.clicked {
            source.set_speed(next_speed(source.speed()));
        }

        // The time, in what room is left.
        let room = (speed_rect.x - GAP - x).max(0.0);
        let now = format_time(source.time());
        let of = format!(" / {}", format_time(source.length().unwrap_or_default()));
        let size = s.text_size(TextSize::Dense);
        let y = crate::text::vcentered_line_y(row_y, face_h, size);
        let list = &mut *ctx.draw_list;
        list.push_clip(Rect::new(x, row_y, room, face_h));
        let now_w = s.mono_width(list, &now, TextSize::Dense);
        list.text(s.mono_block(now, x, y, TextSize::Dense, Ink::Cell));
        list.text(s.mono_block(of, x + now_w, y, TextSize::Dense, Ink::Glyph));
        list.pop_clip();
    }
}

/// The speed key's width with `label` on it.
fn speed_width(s: &StyleResolver, list: &mut DrawList, label: &str) -> f32 {
    (s.mono_width(list, label, TextSize::Meta) + SPEED_PAD * 2.0)
        .ceil()
        .max(SPEED_MIN)
}

/// Mute, or unmute, coming back at half volume from none.
fn toggle_mute(source: &mut dyn MediaSource) {
    if source.muted() {
        source.set_muted(false);
        if source.volume() <= 0.0 {
            source.set_volume(UNMUTED);
        }
    } else {
        source.set_muted(true);
    }
}

/// Set the volume, muted at none and unmuted at any other.
fn set_volume(source: &mut dyn MediaSource, volume: f32) {
    let volume = volume.clamp(0.0, 1.0);
    source.set_volume(volume);
    let muted = volume <= 0.0;
    if source.muted() != muted {
        source.set_muted(muted);
    }
}

/// The speaker for the volume as heard.
fn speaker(source: &dyn MediaSource) -> PhosphorIcon {
    let volume = source.volume();
    if source.muted() {
        PhosphorIcon::SpeakerX
    } else if volume <= 0.0 {
        PhosphorIcon::SpeakerNone
    } else if volume < 0.5 {
        PhosphorIcon::SpeakerLow
    } else {
        PhosphorIcon::SpeakerHigh
    }
}

/// The largest rect `width` by `height` in shape that fits in `frame`,
/// centred.
fn contain(frame: Rect, width: f32, height: f32) -> Rect {
    if width <= 0.0 || height <= 0.0 {
        return frame;
    }
    let scale = (frame.width / width).min(frame.height / height);
    let (w, h) = (width * scale, height * scale);
    Rect::new(
        frame.x + (frame.width - w) * 0.5,
        frame.y + (frame.height - h) * 0.5,
        w,
        h,
    )
}

/// Draw `block` as one line centred across `area` with its middle at `middle`.
fn centred_line(list: &mut DrawList, mut block: TextBlock, area: Rect, middle: f32) {
    let (w, _) = list.measure_block(&block);
    block.x = (area.x + (area.width - w) * 0.5).round();
    block.y = crate::text::vcentered_line_y(middle, 0.0, block.font_size);
    list.text(block);
}

#[cfg(test)]
#[path = "video_player_tests.rs"]
mod tests;
