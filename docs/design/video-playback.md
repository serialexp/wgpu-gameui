# Video playback

**Status:** Accepted
**Last updated:** 2026-10-16

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

Phase 0, the workspace (2026-10-14, gameui `8cd9515`, agent-ui `1137178`):

- Today's crate moved into `crates/core` (still published as `wgpu-gameui`), with its tests,
  examples and assets; the repo root is a Cargo workspace sharing version, edition, licence and
  repository through `[workspace.package]`
- agent-ui's `[patch]` entry points at `~/Projects/wgpu-gameui/crates/core`
- An empty `crates/media` (`wgpu-gameui-media`, `publish = false` until it plays something)

Phase 1, core (2026-10-14, the commit after `8cd9515`). Checked with the format check, strict
clippy, every test in the workspace, and the ignored GPU tests in `tests/yuv_sprites.rs` and
`tests/icon_instancing.rs`; what's left over is in TODO.md, "Video phase 1 follow-ups".

- [x] Sprites whose pixels can be replaced in place: same `SpriteId`, same texture, new pixels
  written with `queue.write_texture` (no new texture or bind group per frame)
- [x] Planar YUV 4:2:0 sprites: three single-channel (`R8Unorm`) planes, converted to RGB in a
  shader, drawn through `DrawList::image` like any other sprite (tint, crop, clip)
- [x] Colour handling for those sprites: BT.709 and BT.601 matrices, limited and full range
- [x] A headless GPU test: known YUV planes in, the expected RGB pixels out
- [x] Full screen on request: a widget asks to take the whole window, gameui asks the app to
  make the window full screen and draws that widget over everything, and Esc (or the window
  leaving full screen) ends it
- [x] Phosphor icons for the media widgets: pause, replay, speaker (high, low, none, muted), and
  full screen in and out

Phase 2, the media crate (2026-10-15, decoding and timing, no GPU, no audio device). The public
face is `Player` (`crates/media/src/player.rs`): `open` / `from_reader` / `with_ahead`, `play`,
`pause`, `seek(to, SeekMode, now)`, `set_speed`, and `update(now)`, which moves the picture on
and returns when the next one is due. Every time comes in as an argument, so the tests drive it
with made-up moments.

- [x] MP4 reading with `re_mp4`: the H.264 video track and the AAC audio track, their timing,
  and which video samples are keyframes (`container.rs`). `re_mp4` ignores edit lists and starts
  each track at zero, so we apply the first edit ourselves (its media time and any empty edit
  before it): the AAC encoder's priming block lands before zero, as ffmpeg puts it. A file cut
  short fails at open, because `re_mp4` refuses an `mdat` longer than the file
- [x] H.264 decoding with `rust_h264` on a worker thread (`video.rs`, `worker.rs`): one sample
  in, a flush, one picture out; display order from the sample table (`Reorder`); a bounded queue
  of decoded frames (`Ahead`, eight pictures and a second of sound by default)
- [x] The colour settings `rust_h264` skips, read from the stream's SPS ourselves (`sps.rs`),
  which also refuses interlaced, 10-bit and non-4:2:0 streams before decoding
- [x] AAC decoding with `symphonia-codec-aac` on the same worker (`audio.rs`); AAC-LC, mono or
  stereo
- [x] The clock passed in as an argument (`clock.rs`); following the audio device's position is
  phase 3
- [x] Choosing the frame to show for a clock time, and dropping frames that are late
  (`schedule.rs`, `advance`)
- [x] Seeking: back to the keyframe before the target, decode forward, show the target frame;
  also `SeekMode::NearestKeyframe`, which lands on the nearer keyframe for scrubbing
- [x] Playback state as the Forge components use it: paused, playing, loading (waiting for
  frames, with the clock held), ended, error
- [x] Clear errors for what isn't supported (another codec by name, interlaced or 10-bit H.264,
  AAC other than LC, a damaged file), and a decoder panic caught on the worker
  (`MediaError::DecoderPanicked`) instead of taking down the app
- [x] Test videos, made once with `ffmpeg` and committed (`crates/media/tests/fixtures/make.sh`,
  each a second or two), with ffmpeg's frame MD5s to compare decoded pictures against

Measured on 2026-10-15 (release, `cargo test -p wgpu-gameui-media --release -- --ignored`, on a
24-core machine at load 4): 1080p30 High at 8 Mbit/s decodes at 58.5 pictures a second (17 ms
each), 51% of one core to keep up at 30 fps; AAC at about 2,000× real time; the whole player, as
fast as it can go, at 1.9× real time with no pictures dropped. Under load 15–21 the same runs gave
48 pictures a second and 1.3–1.7× real time. Memory: eight 1080p pictures ahead is about 25 MB.

A review of the crate (2026-10-15) found, and these were fixed with a test each: a panic on the
worker outside the decoder calls (or in the app's waker) left the player loading forever; the
sound bound stopped applying after the last picture; a stall was only noticed if the app happened
to update at the right moment; a damaged sample size allocated up to 4 GiB before the file length
was checked; a seek didn't clear an error from further on; a seek counted as landed on sound alone,
before its picture. Fixing those turned up three more: sound alone jumped to the end when its
queue ran dry, the pictures still queued once the video was all decoded could stall on sound, and
the last AAC block played its padding past the track's end. Tests hold the worker at chosen
samples with a gated reader (`test_fixtures::Gate`), so these orderings are reproduced exactly.

### Outstanding

Phase 2 leftover:

- [x] `rust_h264` 0.4.0 gets chroma wrong by ±1 at low chroma QP: `dequant_chroma_dc`
  (`src/residual.rs`) adds a rounding term when `qp_per < 5`, which the standard's eq. 8-330
  doesn't have, and the error carries through the GOP. In `h264_aac.mp4` 15 of 60 pictures
  differed from ffmpeg. Fixed on the fork `serialexp/rust_h264`, branch
  `fix/chroma-dc-dequant-rounding` (with byte-exact x264 test streams), which the root
  `Cargo.toml` patches in as a git dependency; all 60 pictures match
- [ ] Offer that fix upstream (`roticv/rust_h264`) as a PR, and drop the patch once a release
  carries it

Phase 3, sound:

- [x] Audio output through `cpal`: a callback fed from a lock-free buffer of samples
  (`device.rs`, `ring.rs`, the sound thread in `sound.rs`)
- [x] Resampling to the device's rate with `rubato`, and mixing the channels to what the device has
  (`convert.rs`, `mix.rs`)
- [x] Volume and mute (`Player::set_volume`, `set_muted`: a gain the callback ramps to)
- [x] Keeping sound and picture together: the clock is how far the device has played, so the
  picture follows the sound
- [x] Playback speed 0.5×, 1×, 1.5×, 2× (the Forge player's speed key), with the sound's pitch
  kept, through `wsola` (`soundtouch-rs` turned out LGPL-2.1; see question 5)
- [x] A player that has no sound device (none in the machine, or opened without sound) plays
  by the wall clock as in phase 2, and so do the tests, through a fake device they play by hand
  (`Options::sound`, `Player::sound_error`)

The `cpal` adaptor itself runs only in the ignored `plays_on_the_real_sound_device` test; every
other sound test uses the fake device.

Phase 4, widgets (the Forge `components/media` set):

Written 2026-10-16, not committed. The widgets are in core (`src/widgets/media/`) and read a
`MediaSource` trait: state, time, length, buffered, volume, mute, speed, error, the `Picture`
(a sprite and its size) and when something shown next changes, and they tell it play, pause,
seek, scrub (a quick keyframe seek while the timeline is dragged, then an exact seek where it is
let go), volume, mute and speed. The media crate's `PlayerSource` is that trait over a `Player`:
each frame the app calls `update(now, sink)`, which moves playback on and puts a new picture in
a YUV sprite through a `PictureSink` (`UiRenderer` is one; the tests use a fake), and
`release(sink)` frees the sprite.

- [x] `Scrubber`: a track with a dim "buffered" layer under the accent "played" fill, a knob
  that's always shown, the time under the pointer in a tip, seeking live while dragging, and
  ←/→ (5 s, Shift 30 s), Home, End when focused
- [x] `PlayButton`: a latch key, pressed in while playing, in five states (paused: play icon,
  playing: pause icon, loading: spinner, ended: replay icon, error: red "!"), and an optional
  progress strip along its bottom
- [x] `VideoPlayer`: the frame, fitted, on black, with the toolbar strip under it (scrubber,
  play, mute and volume, time / length, speed, full screen) always shown. Title over the top of
  the frame, "Can't play this video" over it on error (with the source's reason under it), a
  striped placeholder before the first frame. Click the frame to play or pause, double-click
  or F for full screen; Space and ←/→ 5 s while its frame has the keyboard, and K, M and F
  while any of its keys has it (not with the shortcut key or Alt held). `width_for` (the inverse
  of `height_for`) and `min_width` (its keys side by side) size it. Asks for its next frame at
  the time it's due. A widget can't reach `UiState`, so it doesn't request full screen itself: it
  is told whether it is full screen and returns `toggle_fullscreen`, which the app passes to
  its `FullscreenState`
- [x] `PlayerSource` and `PictureSink` in the media crate, as above
- [ ] `AudioPlayer`: the bar player (play key, title and time over the scrubber, mute and
  volume) and its 28 px compact row
- [ ] Audio-only files for `AudioPlayer`: M4A through `re_mp4`, and WAV, FLAC, Ogg Vorbis and MP3
  through Symphonia's readers
- [x] Widget gallery: a `Media` category, with the components under their Forge names, and
  a still source with a made-up YUV picture so the render is the same every time
- [ ] A windowed example that plays a file given on the command line

Later (deferred):

- [ ] Use in agent-ui: started 2026-10-16 (Bart: play videos in its Files tab, read from the
  server in ranges)
- [ ] Offer `rust_h264` a change that keeps the colour settings on the decoder, and drop our own
  SPS reading if it lands (deferred: Bart, 2026-10-14, "offer later")
- [ ] Check whether eight decoded frames ahead is enough, and what it costs in memory, once
  playback runs (deferred: measured in phase 2 and 3, adjusted if needed)
- [ ] Linux builds without ALSA's development package (deferred: cpal always builds its ALSA
  backend on Linux today. When it matters, offer cpal a change that makes ALSA optional so its
  pure-Rust PulseAudio backend can stand alone, or use a patched cpal meanwhile)
- [ ] AV1 through `rav1d`, behind the same decoder interface (deferred: H.264 is the format
  video most likely arrives in)
- [ ] Matroska / WebM files (deferred: MP4 first)
- [ ] Hardware decoding (VideoToolbox on macOS) behind the same interface (deferred: only if the
  software decoder turns out too slow in practice)

## Why this exists

Bart asked whether gameui can play video, preferably in pure Rust like the rest of it. Until
recently that wasn't possible for the format most video arrives in: H.264, the codec in almost
every MP4 from a phone, a browser or a screen recorder, had no usable Rust decoder. Two appeared
in 2026. This doc uses [`rust_h264`](https://docs.rs/rust_h264) (MIT or Apache-2.0), which
covers the Baseline, Main and High profiles in their common form (8-bit, 4:2:0, progressive),
including CABAC and B-frames.

gameui today has no way to show a picture that changes every frame, and no audio at all. The
Forge design system now has a media set (`components/media`: `VideoPlayer`, `AudioPlayer`,
`PlayButton`, `Scrubber`). This doc is the contract for building it: what goes into the
renderer, what goes into a separate media crate, how sound and picture stay together, and the
widgets on top.

## Goals

- Play H.264 + AAC in MP4, the common case, with sound, in pure Rust. The one non-Rust piece is
  the operating system's audio API, which `cpal` calls.
- No OS-specific code of our own: sound goes through `cpal` on every platform.
- 1080p at 30 frames a second on one core of a recent laptop, with room to spare.
  `rust_h264`'s own benchmark decodes 1080p at 112 frames a second on Apple Silicon.
- Sound and picture stay together, and keep together after a pause, a seek or a speed change.
- The app keeps drawing frames only when something changes: a playing video asks for its next
  frame at the time it is due, and a paused one asks for nothing.
- The widgets match the Forge media components' layout and behaviour, with Phosphor icons for
  their icons (as every gameui widget uses).
- Everything that decides something (which frame to show, when to ask for the next, where a seek
  lands) is tested without a GPU, an audio device or a real clock.

## Non-goals

- Other codecs in the first version (HEVC / H.265, VP8, VP9, AV1). The decoder sits behind an
  interface so they can be added.
- Interlaced, 10-bit, 4:2:2 or 4:4:4 H.264. `rust_h264` doesn't decode them; the player says so.
- Streaming over the network, adaptive bitrate, DRM, subtitles, more than one audio track to pick
  from.
- Recording or encoding video.
- Owning the window. gameui asks for full screen; the app makes the window full screen (it owns
  the window, gameui doesn't).

## How it fits together

### What's in gameui today

From reading the code (2026-10-14):

- **Sprites are written once.** `UiRenderer::load_image_rgba8` (`src/render/ui_renderer.rs:1020`)
  returns the existing sprite if the key is loaded already. Loading again under the same name
  makes a new sprite, a new texture and a new bind group (`src/render/textures.rs:306`, `:491`).
  At 30 frames a second that is a texture allocated and freed every frame, and a `SpriteId` that
  keeps changing.
- **One texture format.** Sprites are `Rgba8Unorm` only (`textures.rs:465`).
- **One way to draw a picture.** Sprites draw through the icon pipeline (`vs_icon` / `fs_icon`,
  `src/render/ui.wgsl:784`, `:808`), which samples one texture and multiplies by the tint. There is
  no hook for a widget to draw with its own shader.
- **No threads, no audio.** The library spawns no threads, and `README.md` calls it "render-only.
  No windowing, audio, or device I/O". `TODO.md` records a 2026-06 decision to leave UI sounds to
  the app for the same reason.
- **Frame requests exist.** `DrawList::repaint_after` (`src/widgets/draw_list.rs:809`) and
  `UiContext::request_repaint_after` (`src/ui_context.rs:1079`) ask for a frame at a time, and
  `UiFrameResult::next_deadline` (`src/frame_result.rs:28`) hands the earliest to the app.
- **The file dialog's source is a pattern to copy.** `FileSource` (`src/widgets/file_dialog/mod.rs:211`)
  is a trait the app implements; the dialog asks it for a listing each frame and shows "loading"
  until the answer is there, and the app does any threading behind it.
- **Few media icons.** The curated Phosphor set has Play but no Pause, replay, speaker or
  full-screen icons (`src/render/phosphor.rs:98`). The whole Phosphor regular font is vendored
  (`assets/fonts/phosphor/Phosphor-Regular.ttf`), so each one is a line in the `icons!` macro,
  whose tests check the codepoint against the font.
- **The cursor shows how a widget asks the app for something.** `CursorState`
  (`src/cursor.rs:87`) collects requests while widgets draw, and the app reads the winner after
  the frame and applies it to its window. Full screen follows the same shape.
- **No gallery category for media.** `Category` in `tests/widget_gallery.rs:285` has no `Media`.

### The split

Video needs threads and an audio device, which gameui has kept out of its core on purpose. So the
work splits in two:

The repo becomes a Cargo workspace with two crates: `crates/core` is today's library, moved, and
`crates/media` is new.

1. **gameui core** learns to show a picture that changes: sprites updated in place, YUV sprites,
   full screen on request, and the media widgets. The widgets don't decode anything. They ask a `MediaSource` (a trait,
   like `FileSource`) for the playback state, the time, the length and, for video, the frame to
   show now and the time of the next one. They hand it what the user did: play, pause, seek,
   volume, speed.
2. **The media crate** (`wgpu-gameui-media`) does everything with threads or devices: reading the file, decoding, the
   audio output and the clock. It implements `MediaSource`. An app that wants video depends on
   it; one that doesn't pulls in none of it.

Core keeps the package name `wgpu-gameui`, so nothing that depends on it changes its
`[dependencies]`. agent-ui's `[patch]` entry (its root `Cargo.toml`, which points at the local
checkout) has to point at `crates/core` from the same moment, or every agent-ui build breaks. The
move touches every file in the repo, so it happens when the gameui tree is clean, announced to
the other agents first.

### Core: full screen on request

Like the browser's `element.requestFullscreen()`: a widget asks to take over the screen, and
gameui does the rest.

- `UiContext::request_fullscreen(id)` (and `UiState::fullscreen.request(id)` for the raw
  widgets, a `FullscreenState`) records which widget wants the screen. The frame's `UiFrameResult` then carries a window request,
  "full screen on", which the app applies to its window (with winit, `set_fullscreen`), the way
  it applies the cursor.
- While a widget owns full screen (`owns_fullscreen(id)`, asked each frame it draws), it draws
  into a top layer covering the whole window, over everything else, without its border or
  rounding. The rest of the UI still draws underneath, so leaving full screen puts it back as it
  was. An owner not drawn in a frame loses the screen, so a closed player can't leave the window
  stuck in full screen.
- Esc reaching the owner (checked as it draws, so a dropdown, menu or modal open over it takes
  the key first), the widget's own key, or `exit_fullscreen` ends it, and the frame result
  carries "full screen off". The app reports the window's full-screen state when it changes
  with `UiState::fullscreen.set_window_fullscreen(on)`; leaving some other way (the OS's own
  key, say) clears the owner. A report contradicting a change still in progress (macOS animates
  into full screen) is ignored. A window that was full screen before the widget asked stays
  full screen when it leaves.
- The video player is the first user. Anything else (an image viewer, a chart) can use it too.

### Renderer: pictures that change

**Updating a sprite in place.** A new `UiRenderer::update_sprite_rgba8(sprite, pixels)` writes new
pixels into the sprite's existing texture with `queue.write_texture`, at the next flush, if the
size is the same. Anything drawn with that `SpriteId` shows the new pixels. This only applies to
sprites with their own texture (`Placement::Own`): an atlas re-uploads all of itself when
anything in it changes, so atlas sprites refuse.

**YUV sprites.** Decoders give a picture as three planes: brightness (Y) at full size, and two
colour planes (U and V) at half the width and half the height. Converting to RGB on the CPU at
1080p costs about as much as decoding again; on the GPU it is a few multiplications per pixel.

- `UiRenderer::create_yuv_sprite(name, width, height, planes, colour)` makes three `R8Unorm`
  textures in one bind group, and `update_yuv_sprite(sprite, planes)` writes new planes into
  them. `YuvPlanes` holds the three as `Cow`s with their row strides, so a decoder's owned,
  padded buffers move in and upload as they are.
- At an odd picture size the colour planes are rounded up, so the shader scales their UV to
  match the picture. Colour samples are treated as centred between the brightness samples;
  H.264's default siting (in line with the left column) differs by a quarter pixel sideways.
- A YUV sprite is drawn with `DrawList::image` like any other, so fitting, cropping, tinting and
  clipping work unchanged.
- The renderer gets one more pipeline, a copy of the icon pipeline whose fragment shader samples
  the three planes and converts. The batching code already splits draws by texture, so a YUV
  sprite is its own batch.

**Colour.** A frame's colours depend on two settings stored in the H.264 stream: which conversion
matrix (BT.709 for HD, BT.601 for older SD material) and whether the values use the full 0–255
range or the "limited" 16–235 range video normally uses. Getting them wrong gives washed-out or
over-saturated pictures. `rust_h264` reads past these settings without keeping them
(`src/sps.rs:310-315` in its source), so the media crate reads them from the stream's settings
block (the SPS) itself for now, and we offer `rust_h264` a change to expose them later. When a
stream doesn't say, the default is BT.709, limited range for HD sizes and BT.601 for SD sizes,
which is what other players do.

The shader's result is sRGB-encoded, which is what the rest of the UI blends in
(the `UiRenderer` docs on colour and target formats), so a video frame and the controls drawn over it mix correctly.

### Media crate: reading and decoding

**Reading the file.** [`re_mp4`](https://docs.rs/re_mp4) (Rerun's MP4 reader, MIT, pure Rust)
reads the file's index: for each track the codec, and for each sample (one compressed video frame,
or one block of audio) its byte position, size, decode time, display time and whether it is a
keyframe. Sample bytes are then read from the file by position as they're needed, so a long video
isn't loaded into memory whole. Rerun plays H.264 with it in production.

**Decoding video.** A worker thread runs `rust_h264`'s `OrderedDecoder`, which returns frames in
display order (H.264 with B-frames decodes them in a different order). Decoded frames go into a
bounded queue, eight frames by default (about 25 MB at 1080p). When the queue is full, the
worker waits. Each frame carries its display time from the MP4's sample table.

**Decoding audio.** The same worker decodes AAC with `symphonia-codec-aac` (Symphonia's AAC-LC
decoder, MPL-2.0, pure Rust), set up from the sample rate and channel count `re_mp4` reads from
the audio track. Decoded samples go into the audio buffer (next section). Using `re_mp4` for both
tracks means the file is read once, by one reader. Symphonia's own MP4 reader would also work for
the audio, but people fuzzing it filed several panics on malformed files in September and
October 2026.

**Audio-only files** for the `AudioPlayer` go through Symphonia's format readers for what they
cover well (WAV, FLAC, Ogg Vorbis, MP3), and through `re_mp4` for M4A, with the same output and
clock. There's no picture, so there's no frame queue.

**"Buffered".** The Forge scrubber draws a dim layer for how much is loaded. A local file is all
there, so the layer shows how far ahead the worker has decoded. It's a thin sliver in practice,
and it's what tells a seek far ahead from one into what's ready.

**Failures.** A file with another codec, an interlaced or 10-bit stream, or damaged data gives an
error, which the widgets show as the design does: "Can't play this video" over the frame, a red
title in the audio player, a red "!" on the play key. The worker runs each decode inside
`catch_unwind`, so a decoder panic on a bad file becomes an error, not a crashed app.
`rust_h264` is fuzzed, but it is young.

### Media crate: sound and the clock

**Output.** [`cpal`](https://docs.rs/cpal) opens the default output device on each platform
(CoreAudio on macOS, WASAPI on Windows, ALSA on Linux), so the media crate has no per-OS code.
cpal calls back on its own thread for samples. The callback reads from a lock-free ring buffer
that the worker fills; it never locks, allocates or waits. When the buffer runs dry it plays
silence and counts the gap. The device's sample rate is often different from the file's (48 kHz
against 44.1 kHz), so the worker resamples with `rubato` (pure Rust) before writing to the
buffer, and mixes the channels down (or up) to what the device has.

**The clock.** Sound is the clock: the player's current time is how many samples the device has
played since the last seek, times the speed, divided by the rate. The picture follows it, showing
the last frame whose display time has come. Following the sound instead of a timer is what keeps
the two together. A dropped audio sample is audible, while a frame shown 10 ms late isn't
visible.

- A video without sound uses a clock passed into the player as an argument. That clock is a
  start time plus elapsed time, read from `Instant` in the app and from a fake in tests.
- Pausing stops the device stream and freezes the clock. Resuming restarts both.

**Speed.** At 1.5× the sound has to play faster without rising in pitch. That needs a time
stretch: cut the sound into short overlapping pieces and lay them closer together (or further
apart for 0.5×), at points where the pieces line up. The video side only needs the clock to run
faster. [`wsola`](https://crates.io/crates/wsola) does the stretch (question 5).

**How phase 3 builds it** (planned and built 2026-10-16, changed after its review the same day).

- *Who fills the ring.* A sound thread per player, beside the decoding worker. The worker
  queues decoded blocks as in phase 2, and the player still lets each go once it has been
  heard. The sound thread reads the blocks from where the sound has got to, stretches them
  (skipped at 1×), resamples them to the device's rate (`rubato`'s `Fft`, skipped when the rates
  match), mixes the channels and writes the result into an `rtrb` ring of a second, kept a fifth
  of a second full (or twice the device's largest callback, if more).
- *Silence.* The worker says how far the sound is known: the time of the next sound sample it
  hasn't decoded, or "to the end" once the track is decoded. Up to there, wherever no block is
  queued (before the sound starts, a gap, after a sound track shorter than the video) the
  thread writes silence without waiting, so the device keeps counting time through it. Gaps
  under half a frame are none: block times are whole nanoseconds, so a block can start a
  nanosecond after the last ended.
- *Neither waits for the other.* While the sound known is less than 0.3 s ahead of the clock,
  the worker decodes pictures past `Ahead::frames`: the clock may be waiting for that sound,
  and in decode order it lies behind the pictures queued (B-frames, a high frame rate).
- *Emptying the ring without a lock.* The ring counts every frame written and read. To throw
  away what's in it (a seek, a pause, a new speed), the sound thread records the count written
  so far, and the callback skips reading up to that count before it plays anything. Nothing
  waits on the callback, which doesn't run while the stream is paused. What follows fades in
  over the first callback that plays it.
- *Segments.* Each time the player's clock starts (play, a seek landing, a stall ending, a new
  speed), the sound starts a segment: a media time, the speed, and the count written when it
  began. How far into the segment the device has played, at the speed, is the media time heard.
  When the clock stops, the stream pauses and the ring is emptied. The decoded blocks are still
  queued until heard, so the next segment converts again from where the sound was.
- *The picture follows the sound.* Each callback notes when its first frame is heard (cpal's
  playback time); the frame heard at a moment runs on from there at the rate, no further than
  the device has been handed. While a segment plays, each update sets the player's clock to the
  media time heard. Until the device has played any of it (opening, its output latency), the
  clock waits where the segment starts, for up to a second, then runs without it; once the
  device is ready, the segment starts again from where the clock has got to. Once the sound
  has all been heard, or without a working device, the clock runs on the wall.
- *What the worker counts as short* is measured from how far the sound thread has read, not
  from the clock: the thread works ahead of the clock by the device's latency (about 320 ms on
  a Bluetooth output) and what its ring holds. A callback of only silence leaves when the last
  of the sound is heard as it was. (Both found playing on a real device, 2026-10-16.)
- *Speed* is kept to 0.25–4×, what `wsola` stretches to.
- *Volume and mute* are a gain the callback applies, ramped over one callback so a change
  doesn't click.
- *No device.* A machine without one, a device that fails, an app that opens the player
  without sound, and the tests use the wall clock (`Player::sound_error` says why). The tests
  that check the sound path use a fake device, which reads from the ring only when a test says
  how many frames it played.

**Frame choice.** Each time the widget draws, it asks the source for the frame for the current
time. The source drops queued frames that are more than one frame late, so a slow machine skips
frames rather than drifting behind the sound, and counts the drops. The widget then asks for a
redraw at the next frame's display time with `request_repaint_after`. So playback draws at the
video's frame rate, not the display's.

**Seeking.**
1. Stop the decoder and empty the frame queue and the audio buffer.
2. Find the last keyframe at or before the target in the sample table, and decode forward from
   it. Frames before the target are decoded but not queued.
3. Show the target frame, and start the audio from the matching sample.

The Forge scrubber seeks live while it's dragged. During a drag, each seek stops at the keyframe
nearest the pointer, which is fast because nothing after it has to be decoded. When the drag
ends, the player seeks exactly.

### The widgets

These follow the Forge media components (`components/media` in the Forge project), and take
their colours from the theme's Forge tokens like the other widgets.

- **`Scrubber`** is its own widget, not the plain `Slider`.
  - The track is a dark well with the buffered layer, a faint hover layer, and the played fill
    in the accent gradient. The fill goes brighter while dragging.
  - The knob is always shown, and gets an accent ring while dragged.
  - A tip above the pointer shows the time there, or whatever `format` gives (the volume
    scrubber shows a percentage).
  - It seeks live while dragged and reports when the drag starts and ends.
  - Keys: ←/→ step (5 s for anything over 30 s; Shift steps six times as far), Home and End.
- **Icons** are Phosphor, as everywhere in gameui. Where Forge draws a glyph of its own, the
  widget uses the matching Phosphor icon instead: `play`, `pause`, `arrow-counter-clockwise`
  (replay), `speaker-high` / `speaker-low` / `speaker-none` / `speaker-x` (by volume, and muted),
  and `arrows-out` / `arrows-in` (full screen in and out).
- **`PlayButton`** is a latch key: held (pressed in, accent face) while playing or loading.
  - Its icon: play when paused, pause when playing, a spinner while loading, replay when ended,
    and a red "!" on error, where it can't be clicked.
  - An optional 2 px progress strip along its bottom, for previews in list rows.
- **`VideoPlayer`**:
  - The frame on black, fitted to the player's aspect (16:9 by default).
  - The toolbar strip underneath, always shown: the scrubber on its own line, then play, the
    speaker key and a 64 px volume scrubber, time / length, a gap, the speed key (cycles 0.5×,
    1×, 1.5×, 2×) and the full-screen key.
  - The title fades over the top of the frame, and errors show over the frame.
  - Before the first frame (or with no source), a striped placeholder shows the time.
  - Click the frame to play or pause. Double-click or F toggles full screen, through core's
    full-screen request. Space or K plays and pauses, ←/→ seek 5 s and M mutes, all while
    focused.
- **`AudioPlayer`** is the bar player: play key, title (with an optional "48 kHz · stereo" style
  note) and time over the scrubber, then mute and volume. Its compact form is one 28 px row.

State is caller-owned, like `ScrollState`. A `MediaState` holds the source, the YUV sprite, the
scrubber drag, hover and focus. Each widget returns what the user did (played, paused, sought,
changed volume or speed, asked for full screen), the way the other widgets report clicks.

### Testing

The tests count and compare. None of them measures time (the agent-ui rule, which this library
follows too).

- **Without a GPU or a device:**
  - frame choice for a given clock time, given a queue;
  - dropping late frames;
  - finding the keyframe for a seek target;
  - the clock with a fake audio position and a fake wall clock, at each speed;
  - the playback states;
  - the widgets' layout, icons, keys and drag, using `DrawList` like the other widget tests.
- **Decoding:** decode each committed test video and check frame count, sizes, display order
  and a few pixel values against ffmpeg's decode, made at the same time as the video and
  committed with it. ffmpeg is needed only to make the fixtures once, never to run the tests.
- **GPU, headless, like the gallery:** fixed YUV planes in, the expected RGB out, for each
  matrix and range.
- **Sound:**
  - the callback fed from a fake buffer, checking that what it wrote matches the samples in and
    that silence fills a gap;
  - the time stretch, checking a test tone keeps its pitch at each speed and the output has the
    expected length;
  - the real device only in an `#[ignore]`d test.

## Phasing

0. **Workspace:** today's crate moved to `crates/core`, an empty `crates/media`, and agent-ui's
   patch path updated in the same step. No behaviour changes; every test still passes.
1. **Core:** sprites updated in place, YUV sprites with colour handling, full screen on request,
   and the media icons. Useful on their own to anything that draws a changing picture.
2. **Media crate, decoding and timing:** file reading, both decoders, the worker, the clock with
   a passed-in time, frame choice, seeking, the playback states, errors, and the test videos.
   Playback works with no sound at this point.
3. **Sound:** `cpal` output, resampling, volume, speed, and the clock switched to the device's
   position.
4. **Widgets:** `Scrubber`, `PlayButton`, `VideoPlayer`, `AudioPlayer` with its audio-only
   formats, the gallery section and the windowed example.

Each phase ends with its tests and clippy passing, and is a commit of its own.

## Open questions for review

All answered by Bart on 2026-10-14; kept with their answers for the record.

1. **Where the media crate lives.**
   - *A second crate in the gameui repo* (`wgpu-gameui-media`), which turns the repo into a Cargo
     workspace. The two change together, and the gallery and the example can use the real thing.
     Recommended.
   - *A crate in its own repo.* Fully separate, but every change to the `MediaSource` trait
     touches two repos.
   - *A feature flag in gameui itself* (`media`, off by default). Simplest to build, but the core
     stops being render-only whenever the flag is on.
   > **2026-10-14, answered:** a new crate, after moving today's library into a core crate:
   > `crates/core` and `crates/media` in one workspace. Phase 0 does the move.
2. **The colour settings `rust_h264` skips.**
   - *Read the SPS ourselves*: about 60 lines in the media crate.
   - *Send `rust_h264` a change that keeps them on the decoder*: the right place for it, but it
     depends on the maintainer accepting and releasing it.
   - Recommended: both. Read them ourselves now, offer the change upstream, and drop ours if it
     lands.
   > **2026-10-14, answered:** read them ourselves now; offer the change upstream later (a
   > deferred bullet).
3. **How many decoded frames to keep ahead.** Eight 1080p frames are about 25 MB per playing
   video. Fewer saves memory but makes a hitch more likely on a busy machine. Recommended: eight,
   adjustable per player.
   > **2026-10-14, answered:** eight. We'll see in practice whether it's enough and how much it
   > uses (a deferred bullet).
4. **Linux builds.**
   > **2026-10-14, answered:** build on `cpal` now, and accept that a Linux build needs ALSA's
   > development package for the time being. A pure-Rust Linux build waits until it matters,
   > then goes through cpal (an upstream change making ALSA optional, or a patched cpal); see
   > the deferred bullet.
5. **How to keep the pitch at other speeds.** Pure-Rust crates exist for this (checked
   2026-10-14). Whichever we pick runs on the worker, before the ring buffer, never in the cpal
   callback: [a Rust video editor](https://github.com/AIEraDev/Clypra/pull/438) had to move its
   stretch off the callback because it glitched there.
   - [`soundtouch-rs`](https://github.com/apoint123/soundtouch-rs): a pure-Rust port of
     SoundTouch, the long-standing C++ library, with presets for speech and music. Recommended:
     the most proven method, and recordings of people talking are a likely use.
   - [`wsola`](https://crates.io/crates/wsola) (v0.1.0): the overlap-and-add method described
     under Speed, tiny (one dependency), with a streaming interface and the tempo changeable
     while playing. Simpler, but very new.
   - [`timestretch`](https://github.com/robmorgan/timestretch-rs): tuned for music with sharp
     beats. More than we need.
   - *Let the pitch change* (just resample). Trivial, but chipmunk voices at 2×.
   > **2026-10-14, answered:** `soundtouch-rs`.
   >
   > **2026-10-16, changed:** `soundtouch-rs` is LGPL-2.1 (like SoundTouch) and not on
   > crates.io, which doesn't fit gameui's MIT/Apache licence. Bart chose `wsola` (MIT, pure Rust,
   > v0.2.0 by then) instead.
6. **Should `AudioPlayer` play audio-only formats beyond M4A?** Symphonia makes WAV, FLAC, Ogg
   Vorbis and MP3 cheap to add, since the output and clock are shared. Recommended: yes, in
   phase 4 with the widget.
   > **2026-10-14, answered:** yes (a phase 4 bullet).

## References

- Forge design system, `components/media`: `VideoPlayer.jsx`, `AudioPlayer.jsx`,
  `PlayButton.jsx`, `Scrubber.jsx`, `useMedia.js` (the state model and time format), and their
  `.prompt.md` notes
- [rust_h264](https://docs.rs/rust_h264) and its source
  ([github.com/roticv/rust_h264](https://github.com/roticv/rust_h264)): `Decoder`,
  `OrderedDecoder`, `Frame` (`src/decoder.rs`), `parse_avcc` (`src/nal.rs`), `BENCHMARK.md`
- [re_mp4](https://docs.rs/re_mp4): `Mp4::read`, `Track`, `Sample` (`src/reader.rs`)
- [Symphonia](https://github.com/pdeljanov/Symphonia): `AacDecoder::try_new`
  (`symphonia-codec-aac/src/aac/mod.rs`)
- [cpal](https://docs.rs/cpal), [rubato](https://docs.rs/rubato)
- [moq-dev/moq PR #4903](https://github.com/moq-dev/moq/pull/4903): why cpal still needs
  libasound on Linux with only its PulseAudio backend in use
- In gameui: `src/render/ui_renderer.rs` (sprites, pipelines), `src/render/textures.rs`,
  `src/render/ui.wgsl` (`vs_icon` / `fs_icon`), `src/widgets/file_dialog/mod.rs` (`FileSource`),
  `src/widgets/slider.rs`, `src/widgets/image.rs`, `src/frame_result.rs`,
  `tests/widget_gallery.rs` (`Category`, `FORGE_COMPONENTS`)
