//! Faces the unit tests load that gameui doesn't bundle: Spectral, a serif
//! whose own figures are tabular, with `tnum` and `pnum` features (Plex has
//! neither), so tests can tell a role's face and figures apart from the
//! bundled Plex. Kept in
//! `tests/fonts/` (SIL Open Font License, `tests/fonts/spectral/OFL.txt`) and
//! not packaged.

use crate::text::{FontHandle, FontSystemHandle, load_font_bytes};

const SPECTRAL_REGULAR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fonts/spectral/Spectral-Regular.ttf"
));
const SPECTRAL_ITALIC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fonts/spectral/Spectral-Italic.ttf"
));

/// Load Spectral (regular and italic) into `fs` and return its family.
pub(crate) fn spectral(fs: &FontSystemHandle) -> FontHandle {
    let family = load_font_bytes(fs, SPECTRAL_REGULAR).expect("Spectral Regular loads");
    let italic = load_font_bytes(fs, SPECTRAL_ITALIC).expect("Spectral Italic loads");
    assert_eq!(family, italic, "both faces are one family");
    family
}
