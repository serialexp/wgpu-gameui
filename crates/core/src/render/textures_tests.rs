use super::*;
use crate::render::yuv::{YuvMatrix, YuvRange};

fn pixels(width: u32, height: u32) -> Vec<u8> {
    vec![255; (width * height * 4) as usize]
}

#[test]
fn a_sprite_gets_a_texture_of_its_own_unless_it_names_an_atlas() {
    let mut textures = Textures::new(8192);
    let icons = textures.create_atlas(1024);
    let photo = textures
        .insert(
            Some("photo"),
            3000,
            2000,
            pixels(3000, 2000),
            Placement::Own,
        )
        .unwrap();
    let icon = textures
        .insert(
            Some("icon"),
            16,
            16,
            pixels(16, 16),
            Placement::Atlas(icons),
        )
        .unwrap();
    let photo_at = textures.resolve(photo).unwrap();
    assert_eq!(photo_at.binding, Binding::Own(photo));
    assert_eq!(
        photo_at.texture,
        (3000, 2000),
        "the whole texture is the photo"
    );
    assert_eq!((photo_at.region.x, photo_at.region.y), (0, 0));
    let icon_at = textures.resolve(icon).unwrap();
    assert_eq!(icon_at.binding, Binding::Atlas(icons));
    assert_eq!(icon_at.texture, (1024, 1024));
    assert_eq!((icon_at.region.w, icon_at.region.h), (16, 16));
    assert_eq!(textures.id_for("icon"), Some(icon));
    assert_eq!(textures.name(photo), Some("photo"));
}

#[test]
fn a_sprite_too_large_for_its_texture_is_refused() {
    let mut textures = Textures::new(4096);
    assert_eq!(
        textures.insert(None, 5000, 10, pixels(5000, 10), Placement::Own),
        Err(SpriteError::TooLarge {
            width: 5000,
            height: 10,
            max: 4096
        })
    );
    // An atlas can't be larger than the GPU allows either, and keeps a
    // one-pixel edge around each sprite.
    let atlas = textures.create_atlas(1 << 20);
    assert_eq!(textures.atlas_size(atlas), Some((1024, 1024)));
    assert_eq!(
        textures.insert(None, 4095, 1, pixels(4095, 1), Placement::Atlas(atlas)),
        Err(SpriteError::TooLarge {
            width: 4095,
            height: 1,
            max: 4094
        })
    );
}

#[test]
fn a_full_atlas_says_so_and_loads_nothing() {
    let mut textures = Textures::new(4096);
    let atlas = textures.create_atlas(128);
    textures
        .insert(
            Some("a"),
            100,
            100,
            pixels(100, 100),
            Placement::Atlas(atlas),
        )
        .unwrap();
    assert_eq!(
        textures.insert(
            Some("b"),
            100,
            100,
            pixels(100, 100),
            Placement::Atlas(atlas)
        ),
        Err(SpriteError::AtlasFull {
            atlas,
            width: 100,
            height: 100
        })
    );
    assert_eq!(textures.id_for("b"), None);
}

#[test]
fn bad_pixels_and_unknown_atlases_are_refused() {
    let mut textures = Textures::new(4096);
    assert_eq!(
        textures.insert(None, 2, 2, &[0; 15], Placement::Own),
        Err(SpriteError::PixelCount {
            expected: 16,
            got: 15
        })
    );
    assert_eq!(
        textures.insert(None, 0, 2, &[], Placement::Own),
        Err(SpriteError::Empty)
    );
    // An atlas made by another renderer.
    let elsewhere = Textures::new(4096).create_atlas(64);
    assert_eq!(
        textures.insert(None, 1, 1, &[0; 4], Placement::Atlas(elsewhere)),
        Err(SpriteError::UnknownAtlas(elsewhere))
    );
}

#[test]
fn unloading_frees_the_sprite_and_its_name_and_reuses_the_id() {
    let mut textures = Textures::new(4096);
    let atlas = textures.create_atlas(1024);
    let own = textures
        .insert(Some("own"), 4, 4, pixels(4, 4), Placement::Own)
        .unwrap();
    let shared = textures
        .insert(Some("shared"), 4, 4, pixels(4, 4), Placement::Atlas(atlas))
        .unwrap();
    assert!(textures.remove(own));
    assert!(textures.remove(shared));
    assert!(!textures.remove(own), "already unloaded");
    assert_eq!(textures.resolve(own), None);
    assert_eq!(textures.id_for("own"), None);
    assert!(
        textures.atlases[0].atlas.is_empty(),
        "the atlas slot is free"
    );
    let again = textures
        .insert(None, 4, 4, pixels(4, 4), Placement::Own)
        .unwrap();
    assert!(again == own || again == shared, "ids are reused");
}

#[test]
fn reusing_a_name_unloads_the_sprite_that_had_it() {
    let mut textures = Textures::new(4096);
    textures
        .insert(Some("logo"), 4, 4, pixels(4, 4), Placement::Own)
        .unwrap();
    let second = textures
        .insert(Some("logo"), 8, 8, pixels(8, 8), Placement::Own)
        .unwrap();
    assert_eq!(textures.id_for("logo"), Some(second));
    assert_eq!(textures.resolve(second).unwrap().texture, (8, 8));
    assert_eq!(
        textures.sprites.iter().flatten().count(),
        1,
        "the first sprite is gone"
    );
    // A load that fails keeps the sprite that has the name.
    assert!(
        textures
            .insert(Some("logo"), 9000, 1, pixels(9000, 1), Placement::Own)
            .is_err()
    );
    assert_eq!(textures.id_for("logo"), Some(second));
}

#[test]
fn only_a_small_sprite_in_a_texture_of_its_own_counts_as_small() {
    let mut textures = Textures::new(4096);
    let atlas = textures.create_atlas(1024);
    let mut load = |width, height, placement| {
        let id = textures
            .insert(None, width, height, pixels(width, height), placement)
            .unwrap();
        textures.resolve(id).unwrap().small_own()
    };
    assert!(load(SMALL_SPRITE_EDGE, SMALL_SPRITE_EDGE, Placement::Own));
    assert!(!load(SMALL_SPRITE_EDGE + 1, 8, Placement::Own));
    assert!(!load(16, 16, Placement::Atlas(atlas)));
}

#[test]
fn draws_batch_while_they_share_a_texture_and_keep_their_order() {
    let atlas = Binding::Atlas(AtlasId(0));
    let photo = Binding::Own(7);
    assert_eq!(
        batches(&[atlas, atlas, photo, atlas, photo, photo]).collect::<Vec<_>>(),
        [(atlas, 0..2), (photo, 2..3), (atlas, 3..4), (photo, 4..6)]
    );
    assert_eq!(batches(&[]).count(), 0);
}

/// A `width`×`height` picture's planes, all brightness `y`, colour neutral.
fn planes(width: u32, height: u32, y: u8) -> YuvPlanes<'static> {
    let (cw, ch) = chroma_size(width, height);
    YuvPlanes::packed(
        width,
        vec![y; (width * height) as usize],
        vec![128; (cw * ch) as usize],
        vec![128; (cw * ch) as usize],
    )
}

const HD: YuvColour = YuvColour {
    matrix: YuvMatrix::Bt709,
    range: YuvRange::Limited,
};

/// The pixels waiting for `id`'s next upload.
fn waiting(textures: &Textures, id: SpriteId) -> Option<&[u8]> {
    match &textures.sprite(id)?.home {
        Home::Own { pixels, .. } => pixels.as_deref(),
        _ => None,
    }
}

#[test]
fn updating_a_sprite_keeps_its_id_and_uploads_only_the_latest_pixels() {
    let mut textures = Textures::new(4096);
    let frame = textures
        .insert(Some("frame"), 2, 2, pixels(2, 2), Placement::Own)
        .unwrap();
    textures.update_rgba(frame, vec![7; 16]).unwrap();
    textures.update_rgba(frame, &[9; 16]).unwrap();
    assert_eq!(textures.pending, [frame], "one upload waits, not three");
    assert_eq!(waiting(&textures, frame), Some(&[9; 16][..]));
    assert_eq!(textures.id_for("frame"), Some(frame));
    assert_eq!(
        textures.resolve(frame).unwrap().binding,
        Binding::Own(frame)
    );
}

#[test]
fn only_a_sprite_of_its_own_at_its_own_size_can_be_updated() {
    let mut textures = Textures::new(4096);
    let atlas = textures.create_atlas(1024);
    let own = textures
        .insert(None, 2, 2, pixels(2, 2), Placement::Own)
        .unwrap();
    let shared = textures
        .insert(None, 2, 2, pixels(2, 2), Placement::Atlas(atlas))
        .unwrap();
    let video = textures
        .insert_yuv(None, 2, 2, planes(2, 2, 16), HD)
        .unwrap();
    assert_eq!(
        textures.update_rgba(shared, pixels(2, 2)),
        Err(SpriteError::InAtlas(shared))
    );
    assert_eq!(
        textures.update_rgba(own, pixels(4, 4)),
        Err(SpriteError::PixelCount {
            expected: 16,
            got: 64
        })
    );
    assert_eq!(
        textures.update_rgba(video, pixels(2, 2)),
        Err(SpriteError::WrongFormat(video))
    );
    assert_eq!(
        textures.update_yuv(own, planes(2, 2, 16)),
        Err(SpriteError::WrongFormat(own))
    );
    textures.remove(own);
    assert_eq!(
        textures.update_rgba(own, pixels(2, 2)),
        Err(SpriteError::UnknownSprite(own))
    );
    assert_eq!(
        textures.update_rgba(99, pixels(2, 2)),
        Err(SpriteError::UnknownSprite(99))
    );
}

#[test]
fn a_yuv_sprite_is_drawn_from_planes_of_its_own() {
    let mut textures = Textures::new(4096);
    let video = textures
        .insert_yuv(Some("video"), 5, 3, planes(5, 3, 16), HD)
        .unwrap();
    let at = textures.resolve(video).unwrap();
    assert_eq!(at.binding, Binding::Yuv(video));
    assert_eq!(at.texture, (5, 3));
    assert_eq!(
        (at.region.x, at.region.y, at.region.w, at.region.h),
        (0, 0, 5, 3)
    );
    assert!(
        !at.small_own(),
        "it has its own pipeline, so there is no atlas to put it in"
    );
    textures.update_yuv(video, planes(5, 3, 100)).unwrap();
    textures.update_yuv(video, planes(5, 3, 200)).unwrap();
    assert_eq!(textures.pending, [video], "one upload waits, not three");
    assert_eq!(textures.id_for("video"), Some(video));
    assert!(textures.remove(video));
    assert_eq!(textures.resolve(video), None);
    assert_eq!(textures.id_for("video"), None);
}

#[test]
fn yuv_planes_must_match_the_picture_size() {
    let mut textures = Textures::new(4096);
    assert_eq!(
        textures.insert_yuv(None, 0, 2, planes(1, 2, 16), HD),
        Err(SpriteError::Empty)
    );
    assert_eq!(
        textures.insert_yuv(None, 5000, 2, planes(5000, 2, 16), HD),
        Err(SpriteError::TooLarge {
            width: 5000,
            height: 2,
            max: 4096
        })
    );
    let mut short = planes(5, 3, 16);
    short.y.to_mut().pop();
    assert_eq!(
        textures.insert_yuv(None, 5, 3, short, HD),
        Err(SpriteError::PlaneSize {
            plane: YuvPlane::Y,
            expected: 15,
            got: 14
        })
    );
    let video = textures
        .insert_yuv(None, 5, 3, planes(5, 3, 16), HD)
        .unwrap();
    let mut long = planes(5, 3, 16);
    long.v.to_mut().push(128);
    assert_eq!(
        textures.update_yuv(video, long),
        Err(SpriteError::PlaneSize {
            plane: YuvPlane::V,
            expected: 6,
            got: 7
        })
    );
}

#[test]
fn yuv_rows_may_be_padded_as_decoders_give_them() {
    let mut textures = Textures::new(4096);
    // A 5×3 picture with 8-byte brightness rows and 4-byte colour rows
    // (colour planes are 3×2).
    let padded = |trailing: bool| {
        let cut = |len: usize, padding: usize| if trailing { len } else { len - padding };
        YuvPlanes {
            y: vec![16; cut(8 * 3, 3)].into(),
            u: vec![128; cut(4 * 2, 1)].into(),
            v: vec![128; cut(4 * 2, 1)].into(),
            y_stride: 8,
            chroma_stride: 4,
        }
    };
    let video = textures.insert_yuv(None, 5, 3, padded(true), HD).unwrap();
    textures.update_yuv(video, padded(false)).unwrap();

    let mut narrow = planes(5, 3, 16);
    narrow.chroma_stride = 2;
    assert_eq!(
        textures.update_yuv(video, narrow),
        Err(SpriteError::PlaneStride {
            plane: YuvPlane::U,
            stride: 2,
            width: 3
        })
    );
    let mut short = padded(false);
    short.y.to_mut().pop();
    assert_eq!(
        textures.update_yuv(video, short),
        Err(SpriteError::PlaneSize {
            plane: YuvPlane::Y,
            expected: 24,
            got: 20
        })
    );
}
