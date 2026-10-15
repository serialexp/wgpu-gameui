//! The committed test videos (`tests/fixtures`, made by its `make.sh`).

use crate::MediaReader;
use std::collections::HashSet;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The bytes of fixture `name`.
pub(crate) fn bytes(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Fixture `name` as something to play.
pub(crate) fn reader(name: &str) -> Box<dyn MediaReader> {
    Box::new(Cursor::new(bytes(name)))
}

/// Fixture `name` behind a [`Gate`], which can hold the worker at chosen
/// samples.
pub(crate) fn gated(name: &str) -> (Box<dyn MediaReader>, Gate) {
    let gate = Gate::default();
    let reader = GatedReader {
        file: Cursor::new(bytes(name)),
        gate: gate.0.clone(),
    };
    (Box::new(reader), gate)
}

#[derive(Default)]
struct Held {
    /// Reads starting at these offsets wait.
    offsets: HashSet<u64>,
    /// A read is waiting.
    waiting: bool,
}

/// Holds a [`gated`] reader's reads of chosen samples until released, so a
/// test can stop the worker exactly where it wants. Dropping it releases
/// them (so a player dropped after it doesn't wait on a held worker).
#[derive(Default)]
pub(crate) struct Gate(Arc<(Mutex<Held>, Condvar)>);

impl Gate {
    /// Hold every read of the samples at `offsets`.
    pub fn hold(&self, offsets: impl IntoIterator<Item = u64>) {
        self.0.0.lock().expect("gate").offsets.extend(offsets);
    }

    /// Let every read through.
    pub fn release(&self) {
        self.0.0.lock().expect("gate").offsets.clear();
        self.0.1.notify_all();
    }

    /// Wait until a read is held (with a limit that only says it hung).
    pub fn wait_until_holding(&self) {
        let (held, changed) = &*self.0;
        let (_held, timeout) = changed
            .wait_timeout_while(
                held.lock().expect("gate"),
                Duration::from_secs(30),
                |held| !held.waiting,
            )
            .expect("gate");
        assert!(!timeout.timed_out(), "the worker never reached the gate");
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

struct GatedReader {
    file: Cursor<Vec<u8>>,
    gate: Arc<(Mutex<Held>, Condvar)>,
}

impl Read for GatedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (held, changed) = &*self.gate;
        let mut held = held.lock().expect("gate");
        while held.offsets.contains(&self.file.position()) {
            held.waiting = true;
            changed.notify_all();
            held = changed.wait(held).expect("gate");
        }
        held.waiting = false;
        drop(held);
        self.file.read(buf)
    }
}

impl Seek for GatedReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.file.seek(to)
    }
}

/// One picture of ffmpeg's decode: when it's shown (in the stream's time
/// base) and the MD5 of its three planes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FfmpegFrame {
    pub pts: i64,
    pub md5: String,
}

/// ffmpeg's decode of fixture `name` (`name.framemd5`), in the order it shows
/// the pictures, and its time base as `(numerator, denominator)`.
pub(crate) fn ffmpeg_frames(name: &str) -> (Vec<FfmpegFrame>, (i64, i64)) {
    let text = String::from_utf8(bytes(&format!("{name}.framemd5"))).expect("UTF-8");
    let mut time_base = (1, 1);
    let mut frames = Vec::new();
    for line in text.lines() {
        if let Some(base) = line.strip_prefix("#tb 0: ") {
            let (numerator, denominator) = base.split_once('/').expect("a/b");
            time_base = (
                numerator.trim().parse().expect("number"),
                denominator.trim().parse().expect("number"),
            );
        } else if !line.starts_with('#') {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            frames.push(FfmpegFrame {
                pts: fields[2].parse().expect("pts"),
                md5: fields[5].to_owned(),
            });
        }
    }
    (frames, time_base)
}

/// The MD5 of a picture's three planes, as ffmpeg's `framemd5` takes it.
pub(crate) fn planes_md5(frame: &crate::VideoFrame) -> String {
    use md5::{Digest, Md5};
    let mut hash = Md5::new();
    hash.update(&frame.planes.y);
    hash.update(&frame.planes.u);
    hash.update(&frame.planes.v);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
