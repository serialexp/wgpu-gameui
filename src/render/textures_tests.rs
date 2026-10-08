use super::*;

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
