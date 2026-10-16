//! Random layouts for the order and plan tests.

use super::{BatchKey, PixelBox};

/// A unit as the tests give it: its key and its boxes.
pub(crate) type Unit = (BatchKey, Vec<PixelBox>);

/// splitmix64: random layouts without a dependency.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
}

/// A box somewhere on a `size` target: mostly small, some wide, some tall,
/// a few covering most of it.
fn random_box(rng: &mut Rng, size: (u32, u32)) -> PixelBox {
    let (w, h) = match rng.below(10) {
        0 => (rng.below(size.0) + 1, rng.below(8) + 1),
        1 => (rng.below(8) + 1, rng.below(size.1) + 1),
        2 if rng.below(4) == 0 => (size.0, size.1),
        _ => (rng.below(48) + 1, rng.below(48) + 1),
    };
    let x0 = rng.below(size.0);
    let y0 = rng.below(size.1);
    PixelBox {
        x0,
        y0,
        x1: (x0 + w).min(size.0),
        y1: (y0 + h).min(size.1),
    }
}

/// A target of random size and up to 120 units on it in up to four keys, some
/// with several boxes, some of those empty.
pub(crate) fn random_units(rng: &mut Rng) -> ((u32, u32), Vec<Unit>) {
    let size = (rng.below(500) + 1, rng.below(500) + 1);
    let keys = rng.below(4) + 1;
    let count = rng.below(120) + 1;
    let units = (0..count)
        .map(|_| {
            let key = BatchKey(u64::from(rng.below(keys)));
            let boxes = (0..rng.below(3) + 1)
                .map(|_| {
                    if rng.below(6) == 0 {
                        PixelBox::EMPTY
                    } else {
                        random_box(rng, size)
                    }
                })
                .collect();
            (key, boxes)
        })
        .collect();
    (size, units)
}
