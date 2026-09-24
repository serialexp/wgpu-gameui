//! Headless offscreen render of all widgets → PNG for visual inspection.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test widget_gallery -- --ignored --nocapture
//! ```
//! Writes `test_output/widget_gallery.png`, one focused image per section under
//! `test_output/widget_gallery/`, and one image per labeled component preview
//! under `test_output/widget_gallery/components/`.
//!
//! Layout is driven by [`Flow`] — a left-to-right, wrapping grid of labeled
//! cells. Each preview reserves a cell (which draws its label) and gets back a
//! content `Rect` to draw into, so adding a widget is one `flow.cell(...)` call
//! plus the widget's own draw call — no hand-placed coordinates.

use wgpu_gameui::debug::DebugReport;
use wgpu_gameui::layout::{Flow as LayoutFlow, HStack, LayoutNode, MainAlign, Rect};
use wgpu_gameui::{
    Accelerator, AssetGrid, Backdrop, Banner, BlurParams, Breadcrumb, Button, Checkbox,
    ColorEncoding, ColorPicker, ColumnWidth, ContextMenu, ContextMenuState, Corner, DocTab,
    DragCapture, DragHandle, DrawContext, DrawList, Dropdown, DropdownState, Easing, FocusState,
    GradientStop, Group, HitZone, Hsva, ImageButton, ImageFit, InputState, InteractionScene, Key,
    LayerStack, List, ListItem, ListState, MeasureBuffer, MeasureConstraints, MeasuredChild, Menu,
    MenuBar, MenuBarState, MenuDrawEnv, MenuItem, NavInput, NumberInput, Pager, Popover,
    PopoverSide, ProgressBar, ProgressFill, RadioGroup, ScrollState, ScrollView, SelectionMode,
    Separator, Severity, Slider, Splitter, StatusCell, StyleKey, StyleOverlay, StyleResolver,
    Table, TableCell, TableColumn, Tabs, TextAlign, TextBlock, TextDirection, TextInput, TextSpan,
    Theme, Toast, ToastStack, Toggle, Tone, TooltipContent, TooltipLayer, TreeAction, TreeNode,
    TreeState, UiContext, UiRenderer, UiState, Underline, VectorField, VectorScrub, ease,
    lerp_color,
};
use wgpu_gameui::{
    EmptyState, STATUS_BAR_HEIGHT, badge, chip, dots, draw_combo_trigger, draw_curve_editor,
    draw_doc_tabs, draw_gradient_ramp, draw_status_bar, draw_tag_input, empty_state, keycap,
    place_popover, skeleton, spinner,
};
#[cfg(feature = "phosphor-icons")]
use wgpu_gameui::{Icon, PhosphorIcon};

/// Convenience: build a DrawContext for a single draw call in the gallery.
fn ctx<'a>(
    list: &'a mut DrawList,
    focus: &'a mut FocusState,
    theme: &'a Theme,
    input: &'a InputState,
) -> DrawContext<'a> {
    DrawContext::new(list, focus, theme, input, W as f32, 600.0)
}

const W: u32 = 800;
const LABEL_H: f32 = 16.0;
const LABEL_SIZE: f32 = 11.0;
/// Rough advance width per character at `LABEL_SIZE`, used so a long label
/// reserves enough horizontal room to not collide with the next cell.
const LABEL_CHAR_W: f32 = 6.0;

/// A wrapping grid of labeled preview cells.
///
/// `cell` reserves a `w`×`h` content box, draws its label just above it, and
/// returns the content `Rect`. Cells flow left-to-right and wrap when they run
/// past `max_x`. `section` breaks to a new row and draws a header.
struct Flow {
    x0: f32,
    y0: f32,
    max_x: f32,
    cur_x: f32,
    cur_y: f32,
    row_h: f32,
    col_gap: f32,
    row_gap: f32,
    current_section: Option<String>,
    sections: Vec<GallerySection>,
    components: Vec<GalleryComponent>,
}

impl Flow {
    fn new(x0: f32, y0: f32, max_x: f32) -> Self {
        Self {
            x0,
            y0,
            max_x,
            cur_x: x0,
            cur_y: y0,
            row_h: 0.0,
            col_gap: 22.0,
            row_gap: 16.0,
            current_section: None,
            sections: Vec::new(),
            components: Vec::new(),
        }
    }

    /// Break to a new row and draw a section header.
    fn section(&mut self, list: &mut DrawList, title: &'static str) -> f32 {
        if self.cur_x > self.x0 {
            self.cur_y += self.row_h;
        }
        // Extra gap above a header (except the very first one).
        if self.cur_y > self.y0 {
            self.cur_y += self.row_gap * 1.5;
        }
        self.cur_x = self.x0;
        self.row_h = 0.0;
        list.text(
            TextBlock::new(title, self.x0, self.cur_y)
                .with_size(15.0)
                .with_color(120, 180, 255),
        );
        let section_top = self.cur_y;
        if let Some(previous) = self.sections.last_mut() {
            previous.bottom = section_top - self.row_gap * 1.5;
        }
        let file_stem = section_file_stem(title);
        self.sections
            .push(GallerySection::new(file_stem.clone(), section_top));
        self.current_section = Some(file_stem);
        self.cur_y += 24.0;
        section_top
    }

    fn finish_sections(&mut self) {
        let bottom = self.bottom();
        if let Some(last) = self.sections.last_mut() {
            last.bottom = bottom;
        }
    }

    /// Reserve a labeled `w`×`h` content cell; returns the content rect.
    fn cell(&mut self, list: &mut DrawList, label: &str, w: f32, h: f32) -> Rect {
        // A cell is as wide as its content or its label, whichever is larger,
        // so labels never overlap the neighbouring cell.
        let cell_w = w.max(label.chars().count() as f32 * LABEL_CHAR_W);
        if self.cur_x + cell_w > self.max_x && self.cur_x > self.x0 {
            self.cur_x = self.x0;
            self.cur_y += self.row_h + self.row_gap;
            self.row_h = 0.0;
        }
        list.text(
            TextBlock::new(label, self.cur_x, self.cur_y)
                .with_size(LABEL_SIZE)
                .with_color(150, 160, 180),
        );
        let content = Rect::new(self.cur_x, self.cur_y + LABEL_H, w, h);
        if !label.is_empty() {
            let section = self
                .current_section
                .as_deref()
                .expect("gallery cells must belong to a section");
            self.components
                .push(GalleryComponent::new(section, label, content));
        }
        self.cur_x += cell_w + self.col_gap;
        self.row_h = self.row_h.max(LABEL_H + h);
        content
    }

    /// Reserve at least `content_h` of vertical space (measured from the last
    /// cell's content-rect top) for the current row. Use this for content that
    /// *paints taller than the `cell` rect it was handed* — an open `Dropdown`
    /// overlay or an auto-advancing `UiContext` verb stack — so the following
    /// row starts below it instead of underneath it.
    fn reserve(&mut self, content_h: f32) {
        self.row_h = self.row_h.max(LABEL_H + content_h);
        if let Some(component) = self.components.last_mut() {
            component.rect.height = component.rect.height.max(LABEL_H + content_h);
        }
    }

    /// The y just below all content drawn so far.
    fn bottom(&self) -> f32 {
        self.cur_y + self.row_h
    }
}

#[derive(Clone)]
struct GallerySection {
    file_stem: String,
    top: f32,
    bottom: f32,
}

struct GalleryComponent {
    section: String,
    file_stem: String,
    rect: Rect,
}

impl GalleryComponent {
    fn new(section: &str, label: &str, content: Rect) -> Self {
        Self {
            section: section.to_owned(),
            file_stem: section_file_stem(label),
            rect: Rect::new(
                content.x,
                content.y - LABEL_H,
                content.width,
                content.height + LABEL_H,
            ),
        }
    }
}

impl GallerySection {
    fn new(file_stem: String, top: f32) -> Self {
        Self {
            file_stem,
            top,
            bottom: top,
        }
    }
}

fn section_file_stem(title: &str) -> String {
    let mut stem = String::with_capacity(title.len());
    let mut needs_separator = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if needs_separator && !stem.is_empty() {
                stem.push('-');
            }
            stem.push(ch.to_ascii_lowercase());
            needs_separator = false;
        } else {
            needs_separator = true;
        }
    }
    stem
}

fn crop_with_margin(img: &image::RgbaImage, rect: Rect, margin: u32) -> image::RgbaImage {
    let left = (rect.x.floor().max(0.0) as u32).saturating_sub(margin);
    let top = (rect.y.floor().max(0.0) as u32).saturating_sub(margin);
    let right = (rect.right().ceil().max(0.0) as u32 + margin).min(img.width());
    let bottom = (rect.bottom().ceil().max(0.0) as u32 + margin).min(img.height());
    assert!(right > left && bottom > top, "gallery crop is empty");
    image::imageops::crop_imm(img, left, top, right - left, bottom - top).to_image()
}

fn save_gallery_images(
    img: &image::RgbaImage,
    sections: &[GallerySection],
    components: &[GalleryComponent],
) {
    let output_dir = "test_output/widget_gallery";
    let component_dir = format!("{output_dir}/components");
    std::fs::create_dir_all(&component_dir).expect("create gallery image directories");
    for entry in std::fs::read_dir(&component_dir).expect("read gallery component directory") {
        let entry = entry.expect("read gallery component entry");
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "png")
        {
            std::fs::remove_file(entry.path()).expect("remove stale gallery component PNG");
        }
    }

    for section in sections {
        assert!(
            section.bottom > section.top,
            "gallery section {} is empty",
            section.file_stem
        );
        let section_img = crop_with_margin(
            img,
            Rect::new(
                20.0,
                section.top,
                (img.width() - 40) as f32,
                section.bottom - section.top,
            ),
            10,
        );
        let path = format!("{output_dir}/{}.png", section.file_stem);
        section_img.save(&path).expect("save gallery section PNG");
        eprintln!(
            "wrote {path} ({}x{})",
            section_img.width(),
            section_img.height()
        );
    }

    let mut file_stem_counts = std::collections::BTreeMap::new();
    for component in components {
        let component_img = crop_with_margin(img, component.rect, 6);
        let base = format!("{}--{}", component.section, component.file_stem);
        let count = file_stem_counts.entry(base.clone()).or_insert(0usize);
        *count += 1;
        let file_stem = if *count == 1 {
            base
        } else {
            format!("{base}-{}", *count)
        };
        let path = format!("{component_dir}/{file_stem}.png");
        component_img
            .save(&path)
            .expect("save gallery component PNG");
    }
    eprintln!(
        "wrote {} focused component images under {component_dir}",
        components.len()
    );
}

fn solid_with_border(size: u32, fill: [u8; 4], border: [u8; 4], thickness: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let on_border =
                x < thickness || y < thickness || x >= size - thickness || y >= size - thickness;
            let c = if on_border { border } else { fill };
            let idx = ((y * size + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_widget_gallery() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter available");

    // The gallery is one tall image — taller than wgpu's portable default
    // texture limit (8192) — so ask for whatever this adapter supports.
    let max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("gallery device"),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d,
                ..Default::default()
            },
            ..Default::default()
        },
        None,
    ))
    .expect("request device");

    // A non-sRGB target takes the renderer's direct path, where every blend
    // happens in sRGB space exactly like the browser — so this PNG is directly
    // comparable with the DesignSync cards.
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    // Real PNG art from assets/ — decoded through the same `load_image_file`
    // path the game uses, so the gallery doubles as a smoke test for it.
    let load = |ui: &mut UiRenderer, name: &str| {
        ui.load_image_file(format!("assets/{name}.png"))
            .unwrap_or_else(|e| panic!("load assets/{name}.png: {e:?}"))
    };
    let duck = load(&mut ui, "rubberduck");
    let ball = load(&mut ui, "soccerball");
    let car = load(&mut ui, "toycar");
    let board = load(&mut ui, "skateboard");
    let snow = load(&mut ui, "snowflake");
    let suitcase = load(&mut ui, "suitcase");

    // Synthetic sprites for primitives that show off tinting / nine-slice.
    let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
    let frame_sprite = ui.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
    let nine_slice_id = ui.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);

    // (Slider needs no assets — it renders procedurally from the theme.)

    let mut layers = LayerStack::new();
    let theme = Theme::default();
    let mut input = InputState::default();
    let gallery_sections;
    let gallery_components;

    // Focus owner for the text inputs. Seed the first as focused so the rendered
    // PNG shows a caret; `begin_frame`/`end_frame` bracket the draws below.
    let mut focus = FocusState::new();
    focus.focus(0);
    focus.begin_frame(&input);

    // Dropdown owner, seeded OPEN so the PNG shows the floating option list.
    const DROPDOWN_ID: u64 = 100;
    const DROPDOWN_ITEMS: [&str; 4] = ["Red", "Green", "Blue", "Alpha"];
    let mut dropdowns = DropdownState::new();

    // Menubar owner. Seeded OPEN through the real input path (an Alt tap arms the
    // bar, Down opens the highlighted menu) so the PNG shows the strip with its
    // open menu highlighted and the dropped column: accelerators, a submenu
    // chevron, a check mark, a separator and a disabled row.
    const MENU_BAR_ID: u64 = 300;
    const RECENT_GROUPS: &[MenuItem<'static>] = &[
        MenuItem::new("Today")
            .with_children(&[MenuItem::new("spaceship.gui"), MenuItem::new("terrain.gui")]),
        MenuItem::new("Earlier").with_children(&[MenuItem::new("prototype.gui")]),
    ];
    const FILE_ITEMS: &[MenuItem<'static>] = &[
        MenuItem::new("New").accel(Accelerator::primary(Key::Char('N'))),
        MenuItem::new("Open…").shortcut("Ctrl+O"),
        MenuItem::new("Open Recent").with_children(RECENT_GROUPS),
        MenuItem::separator(),
        MenuItem::new("Auto-save").checked(true),
        MenuItem::new("Locked").enabled(false),
        MenuItem::new("Quit").accel(Accelerator::primary_shift(Key::Char('Q'))),
    ];
    const MENUS: &[Menu<'static>] = &[
        Menu::new("File").with_items(FILE_ITEMS),
        Menu::new("Edit").with_items(&[MenuItem::new("Undo"), MenuItem::new("Redo")]),
        Menu::new("View")
            .enabled(false)
            .with_items(&[MenuItem::new("Zoom")]),
    ];
    let menu_bar = MenuBar::new(MENU_BAR_ID, MENUS);
    let mut menu_state = MenuBarState::new();
    const CONTEXT_CREATE: &[MenuItem<'static>] = &[
        MenuItem::new("Mesh").with_children(&[MenuItem::new("Cube"), MenuItem::new("Sphere")]),
        MenuItem::new("Light"),
    ];
    const CONTEXT_ITEMS: &[MenuItem<'static>] = &[
        MenuItem::new("Frame Selection").shortcut("F"),
        MenuItem::new("Create").with_children(CONTEXT_CREATE),
        MenuItem::separator(),
        MenuItem::new("Copy").shortcut("Ctrl C"),
        MenuItem::new("Duplicate").shortcut("Ctrl D"),
        MenuItem::separator(),
        MenuItem::new("Delete").shortcut("Del"),
    ];
    let context_menu = ContextMenu::new(CONTEXT_ITEMS);
    let mut context_state = ContextMenuState::new();
    // The frame-2 input (and the one the chain is drawn with): no edges, so nothing
    // the gallery did to open the menu is replayed.
    let mut menu_input = InputState::default();

    // Reserved by the flow inside the scope below, used afterwards. The scope
    // runs unconditionally, so deferred init is sound (and avoids a dead store).
    let tooltip_rect;
    let content_bottom;
    // Reserved cell for the backdrop-blur demo; filled after layout (the blur is
    // a renderer pass, not a DrawList record, so it runs in the encoder below).
    let blur_rect;

    // =====================================================================
    // Build the base layer. The `list` borrow on `layers` is released at the
    // end of this scope so we can add tooltip layers afterwards.
    // =====================================================================
    {
        let list = layers.base_mut();

        list.text(
            TextBlock::new("wgpu-gameui Widget Gallery", 20.0, 16.0)
                .with_size(24.0)
                .with_color(255, 255, 255),
        );

        let mut flow = Flow::new(20.0, 56.0, (W as f32) - 20.0);

        // ---- Menubar ----------------------------------------------------
        // Chrome goes first, at the top: that is where a menubar lives, and its
        // open column then drops into the empty space the flow reserves for it.
        //
        // A chain's geometry is measured while the bar draws and promoted at the
        // next frame-top (a popup layer only blocks input if it is pushed before
        // the base layer resolves), so the bar is drawn twice: once into a scratch
        // list to stage the chain, then for real with that geometry promoted. The
        // open menu comes from the real input path — an Alt tap plus Down — so the
        // PNG exercises arming and opening rather than a seam.
        flow.section(list, "Menubar — 01-menu-bar.html states");

        let resting_rect = flow.cell(list, "Resting", 220.0, 26.0);
        let mut resting_state = MenuBarState::new();
        menu_bar.draw(
            resting_rect,
            &mut resting_state,
            &mut DrawContext::new(
                list,
                &mut focus,
                &theme,
                &InputState::default(),
                W as f32,
                600.0,
            ),
        );

        let armed_rect = flow.cell(list, "Title hover / armed", 220.0, 26.0);
        let mut armed_state = MenuBarState::new();
        let mut armed_input = InputState {
            alt_pressed: true,
            ..InputState::default()
        };
        armed_state.begin_frame(&mut armed_input);
        menu_bar.draw(
            armed_rect,
            &mut armed_state,
            &mut DrawContext::new(list, &mut focus, &theme, &armed_input, W as f32, 600.0),
        );

        let menu_rect = flow.cell(list, "Title open + menu sheet", 220.0, 26.0);
        {
            let mut scratch = DrawList::new();
            let mut opening = InputState {
                alt_pressed: true,
                nav: NavInput {
                    down: true,
                    ..Default::default()
                },
                ..InputState::default()
            };
            menu_state.begin_frame(&mut opening);
            menu_bar.draw(
                menu_rect,
                &mut menu_state,
                &mut DrawContext::new(&mut scratch, &mut focus, &theme, &opening, W as f32, 600.0),
            );
        }
        // Frame 2: the staged chain is promoted, and this frame re-measures it for
        // the next one.
        menu_state.begin_frame(&mut menu_input);
        menu_state.set_highlighted_item(MENUS, Some(2));
        menu_state.set_open_path(MENUS, &[2, 0]);
        // Re-measure after seeding the recursive path, then promote all three
        // columns before the real gallery draw.
        {
            let mut scratch = DrawList::new();
            menu_bar.draw(
                menu_rect,
                &mut menu_state,
                &mut DrawContext::new(
                    &mut scratch,
                    &mut focus,
                    &theme,
                    &menu_input,
                    W as f32,
                    600.0,
                ),
            );
        }
        menu_state.begin_frame(&mut menu_input);
        menu_bar.draw(
            menu_rect,
            &mut menu_state,
            &mut DrawContext::new(list, &mut focus, &theme, &menu_input, W as f32, 600.0),
        );
        // The column paints taller than the 26px strip, so the flow has to know how
        // far it drops or the next row would sit underneath it.
        if let Some((column, _, _)) = menu_state.debug_geometry() {
            flow.reserve(column.bottom() - menu_rect.y + 22.0);
        }

        // ---- Primitives -------------------------------------------------
        flow.section(list, "Primitives");

        let r = flow.cell(list, "Rounded rect", 120.0, 44.0);
        list.rounded_rect(r, 8.0, [0.25, 0.40, 0.65, 1.0]);

        let r = flow.cell(list, "Line", 90.0, 44.0);
        list.line(
            [r.x, r.y + r.height],
            [r.x + r.width, r.y],
            3.0,
            [0.95, 0.65, 0.25, 1.0],
        );

        let r = flow.cell(list, "Circle", 50.0, 50.0);
        list.circle(
            (r.x + r.width / 2.0, r.y + r.height / 2.0),
            r.width / 2.0,
            [0.30, 0.70, 0.40, 1.0],
        );

        let r = flow.cell(list, "Circle outline", 50.0, 50.0);
        list.circle_outline(
            (r.x + r.width / 2.0, r.y + r.height / 2.0),
            r.width / 2.0,
            3.0,
            [0.70, 0.30, 0.40, 1.0],
        );

        // Rect outline routes through the SDF chrome instance (transparent fill,
        // border-only band) when translate-only.
        let r = flow.cell(list, "Rect outline", 120.0, 44.0);
        list.rect_outline(r, 2.0, [0.55, 0.75, 0.95, 1.0]);

        // Rotated rounded rect: a non-translate transform falls back to the soup
        // tessellator, proving the rounded-rect primitive still draws correctly
        // off-axis (instanced fast path is translate-only).
        let r = flow.cell(list, "Rounded rect (rotated)", 120.0, 44.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.rounded_rect(
            Rect::new(-r.width / 2.0, -r.height / 2.0, r.width, r.height),
            8.0,
            [0.25, 0.40, 0.65, 1.0],
        );
        list.pop_transform();

        let r = flow.cell(list, "Nine-slice", 64.0, 44.0);
        list.nine_slice_id(nine_slice_id, r.x, r.y, r.width, r.height, [1.0; 4]);

        // Rotated nine-slice: unlike chrome, the instanced nine-slice bakes the
        // full affine into the instance (UV mapping is local-space), so rotation
        // is exact with no fallback — strictly more capable than the old soup.
        let r = flow.cell(list, "Nine-slice (rotated)", 64.0, 44.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.nine_slice_id(
            nine_slice_id,
            -r.width / 2.0,
            -r.height / 2.0,
            r.width,
            r.height,
            [1.0; 4],
        );
        list.pop_transform();

        let r = flow.cell(list, "Icon sprite", 40.0, 40.0);
        list.icon_sprite(ball, r.x, r.y, r.width, r.height, [1.0; 4]);

        let r = flow.cell(list, "Image", 40.0, 40.0);
        list.image(car, r, [1.0; 4]);

        let r = flow.cell(list, "Image (cropped)", 40.0, 40.0);
        list.image_cropped(snow, r, [0.0, 0.0, 0.5, 0.5], [1.0; 4]);

        let r = flow.cell(list, "Rect outline", 100.0, 32.0);
        list.rect_outline(r, 2.0, [0.70, 0.30, 0.40, 1.0]);

        // ---- Phosphor MSDF icons ---------------------------------------
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(list, "Icons (Phosphor MSDF)");

            // The full curated set at a single readable size.
            let set = [
                ("Plus", PhosphorIcon::Plus),
                ("Minus", PhosphorIcon::Minus),
                ("Check", PhosphorIcon::Check),
                ("X", PhosphorIcon::X),
                ("CaretUp", PhosphorIcon::CaretUp),
                ("CaretDown", PhosphorIcon::CaretDown),
                ("Eye", PhosphorIcon::Eye),
                ("EyeSlash", PhosphorIcon::EyeSlash),
                ("Trash", PhosphorIcon::Trash),
                ("Pencil", PhosphorIcon::PencilSimple),
                ("Gear", PhosphorIcon::Gear),
            ];
            for (label, icon) in set {
                let r = flow.cell(list, label, 32.0, 32.0);
                Icon::new(icon).draw(r, list);
            }

            // A few sizes of one icon to eyeball crispness across scales.
            for px in [16.0_f32, 24.0, 48.0] {
                let r = flow.cell(list, &format!("Gear {}px", px as u32), px, px);
                Icon::new(PhosphorIcon::Gear).draw(r, list);
            }

            // Tinted.
            let r = flow.cell(list, "Trash (red)", 32.0, 32.0);
            Icon::new(PhosphorIcon::Trash)
                .tint([0.90, 0.25, 0.25, 1.0])
                .draw(r, list);
        }

        // ---- Text -------------------------------------------------------
        flow.section(list, "Text");

        let r = flow.cell(list, "Plain", 190.0, 20.0);
        list.text(
            TextBlock::new("The quick brown fox", r.x, r.y)
                .with_size(16.0)
                .with_color(200, 210, 230),
        );

        let r = flow.cell(list, "Outline", 130.0, 24.0);
        list.text(
            TextBlock::new("Outlined", r.x, r.y)
                .with_size(20.0)
                .with_color(255, 255, 255)
                .with_outline(10, 12, 18, 255, 2.0),
        );

        let r = flow.cell(list, "Shadow", 120.0, 20.0);
        list.text(
            TextBlock::new("Shadowed", r.x, r.y)
                .with_size(16.0)
                .with_color(200, 220, 255)
                .with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0),
        );

        let r = flow.cell(list, "Glow", 120.0, 26.0);
        list.text(
            TextBlock::new("Glow", r.x, r.y)
                .with_size(24.0)
                .with_color(255, 255, 200)
                .with_glow(80, 200, 255, 255, 3.0),
        );

        let r = flow.cell(list, "Align right", 150.0, 20.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("Right", r.x, r.y + 2.0)
                .with_size(14.0)
                .with_color(180, 190, 210)
                .with_max_width(r.width)
                .with_align(TextAlign::Right),
        );

        let r = flow.cell(list, "Align center", 150.0, 20.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("Center", r.x, r.y + 2.0)
                .with_size(14.0)
                .with_color(180, 190, 210)
                .with_max_width(r.width)
                .with_align(TextAlign::Center),
        );

        // ---- RTL / bidi ------------------------------------------------
        // cosmic-text already runs the Unicode bidi algorithm, so Arabic/Hebrew
        // shape and lay out right-to-left automatically (via system fallback
        // faces — the glyphs only render where those faces are installed). The
        // public knobs added on top are: a forced base `TextDirection` (for
        // direction-neutral content that would otherwise auto-resolve LTR) and
        // direction-relative `TextAlign::{Start, End}`.
        flow.section(list, "RTL / bidi");

        let r = flow.cell(list, "Arabic (auto)", 200.0, 22.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("مرحبا بالعالم", r.x, r.y + 2.0)
                .with_size(16.0)
                .with_color(210, 220, 240)
                .with_max_width(r.width),
        );

        let r = flow.cell(list, "Hebrew (auto)", 200.0, 22.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("שלום עולם", r.x, r.y + 2.0)
                .with_size(16.0)
                .with_color(210, 220, 240)
                .with_max_width(r.width),
        );

        // Direction-neutral content (digits + punctuation) auto-resolves LTR;
        // forcing the base direction to RTL right-flushes and reorders it.
        let r = flow.cell(list, "Neutral: Auto", 200.0, 22.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("12:34 + 56", r.x, r.y + 2.0)
                .with_size(15.0)
                .with_color(200, 210, 230)
                .with_max_width(r.width),
        );

        let r = flow.cell(list, "Neutral: forced RTL", 200.0, 22.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("12:34 + 56", r.x, r.y + 2.0)
                .with_size(15.0)
                .with_color(200, 210, 230)
                .with_max_width(r.width)
                .with_direction(TextDirection::Rtl),
        );

        // Direction-relative alignment over RTL content: Start = reading start
        // (right edge for RTL), End = reading end (left); Left/Right stay
        // absolute regardless of direction.
        for (label, align) in [
            ("RTL Start", TextAlign::Start),
            ("RTL End", TextAlign::End),
            ("RTL Left", TextAlign::Left),
            ("RTL Right", TextAlign::Right),
        ] {
            let r = flow.cell(list, label, 120.0, 20.0);
            list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
            list.text(
                TextBlock::new("שלום", r.x, r.y + 2.0)
                    .with_size(14.0)
                    .with_color(180, 190, 210)
                    .with_max_width(r.width)
                    .with_align(align),
            );
        }

        // A focused LTR text input with the whole value selected — the LTR
        // counterpart to the RTL field below. Both show the single-line selection
        // band + caret centred on the box (not riding high at the line-box top).
        let r = flow.cell(list, "Input (selected)", 200.0, 28.0);
        {
            const SEL_ID: u64 = 204;
            let sel_input = InputState::default();
            let mut sel_focus = FocusState::new();
            sel_focus.focus(SEL_ID);
            sel_focus.begin_frame(&sel_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height).with_value("Hello, world");
            field.cursor_pos = field.value.len();
            field.selection_start = Some(0);
            field.draw(
                SEL_ID,
                &mut DrawContext::new(list, &mut sel_focus, &theme, &sel_input, W as f32, 600.0),
            );
        }

        // A focused, forced-RTL text input with an active selection — exercises
        // the bidi selection rectangles and edge-correct caret. Local
        // focus/input so it doesn't disturb the shared owner above.
        let r = flow.cell(list, "RTL input (selected)", 200.0, 28.0);
        {
            const RTL_ID: u64 = 203;
            let rtl_input = InputState::default();
            let mut rtl_focus = FocusState::new();
            rtl_focus.focus(RTL_ID);
            rtl_focus.begin_frame(&rtl_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height)
                .with_direction(TextDirection::Rtl)
                .with_value("שלום עולם");
            field.cursor_pos = field.value.len();
            field.selection_start = Some(0);
            field.draw(
                RTL_ID,
                &mut DrawContext::new(list, &mut rtl_focus, &theme, &rtl_input, W as f32, 600.0),
            );
        }

        // ---- Vertical (stacked) text -----------------------------------
        // `TextBlock::with_vertical` stacks each grapheme cluster on its own row,
        // top-to-bottom, centered within the column — the casual upright look for
        // Japanese game labels (not true CJK `vertical-rl`). Shown beside the same
        // phrase laid out horizontally for contrast.
        flow.section(list, "Vertical text");

        let r = flow.cell(list, "Stacked JP", 40.0, 160.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("こんにちは", r.x, r.y + 2.0)
                .with_size(22.0)
                .with_color(210, 220, 240)
                .with_max_width(r.width)
                .with_align(TextAlign::Center)
                .with_vertical(),
        );

        let r = flow.cell(list, "vs horizontal", 200.0, 28.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("こんにちは", r.x + 4.0, r.y + 4.0)
                .with_size(22.0)
                .with_color(210, 220, 240),
        );

        // Mixed full-width kana + Latin digits: each cluster stacks individually
        // (per-letter for the Latin run — the documented label-scope behaviour).
        let r = flow.cell(list, "Mixed JP+digits", 56.0, 200.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("レベル99", r.x, r.y + 2.0)
                .with_size(24.0)
                .with_color(255, 240, 200)
                .with_max_width(r.width)
                .with_align(TextAlign::Center)
                .with_vertical(),
        );

        // Tighter row pitch (line_height = font_size) reads better for full-width
        // kana than the default 1.25× airy spacing.
        let r = flow.cell(list, "Tight pitch", 40.0, 160.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        let mut tight = TextBlock::new("たてがき", r.x, r.y + 2.0)
            .with_size(24.0)
            .with_color(200, 230, 210)
            .with_max_width(r.width)
            .with_align(TextAlign::Center)
            .with_vertical();
        tight.line_height = 24.0;
        list.text(tight);

        flow.reserve(200.0);

        // ---- Vertical centering (debug) --------------------------------
        // Visualises the per-label optical centerer (`DrawList::vcentered_text_y`).
        // The band it centres is chosen from the *text*: a label with lowercase
        // letters centres on the x-height body; an all-caps/numeric label centres
        // on the taller cap height. For each sample we draw the container box, the
        // centred text, and guide lines:
        //   • GREEN  = the box's geometric centre.
        //   • RED (two lines) = the band the centerer actually chose (its top and
        //     the baseline) — its midpoint should sit on the green line.
        //   • dim GREY = the *other*, non-chosen band top (reference only).
        // Correct centring ⇒ the red band straddles the green line: for "Hxngy"
        // the lowercase body sits centred (caps overshoot up, descenders hang
        // below); for "HX100%" the cap/digit body sits centred.
        flow.section(list, "Vertical centering (debug)");
        let samples: [&str; 2] = ["Hxngy", "HX100%"];
        for sample in samples {
            for size in [15.0f32, 24.0] {
                let box_h = (size * 1.25 + 20.0).max(36.0);
                let has_lc = sample.chars().any(|c| c.is_lowercase());
                let label = format!(
                    "{sample} ({}) @ {size:.0}px",
                    if has_lc { "x" } else { "cap" }
                );
                let r = flow.cell(list, &label, 150.0, box_h);
                // Container.
                list.rect_outline(r, 1.0, [0.30, 0.34, 0.42, 1.0]);
                // Centre this exact sample, then recover the band it chose.
                let ty = list.vcentered_text_y(r.y, r.height, size, theme.font.as_ref(), sample);
                let m = list.font_vmetrics(theme.font.as_ref());
                let baseline = ty + m.baseline_ratio * size;
                let chosen = if has_lc { m.x_ratio } else { m.cap_ratio };
                let other = if has_lc { m.cap_ratio } else { m.x_ratio };
                let chosen_top = baseline - chosen * size;
                let other_top = baseline - other * size;
                // Green: box centre.
                list.quad(
                    r.x,
                    r.y + r.height / 2.0 - 0.5,
                    r.width,
                    1.0,
                    [0.25, 1.0, 0.45, 0.7],
                );
                // Grey (reference): the non-chosen band top.
                list.quad(r.x, other_top - 0.5, r.width, 1.0, [0.55, 0.58, 0.64, 0.6]);
                // Red: the chosen band (its top + the baseline) — the centring target.
                list.quad(r.x, chosen_top - 0.5, r.width, 1.0, [1.0, 0.25, 0.25, 0.95]);
                list.quad(r.x, baseline - 0.5, r.width, 1.0, [1.0, 0.25, 0.25, 0.95]);
                list.text(
                    TextBlock::new(sample, r.x + 6.0, ty)
                        .with_size(size)
                        .with_color(228, 232, 240),
                );
            }
        }
        // CJK centres on the ideographic ink centre: ideographs overhang the em
        // square and dip below the baseline, so neither the x- nor cap-band
        // applies. Here RED = the computed visual centre, which should land on the
        // GREEN box centre with the ideograph optically centred on it. (If no CJK
        // font is installed the glyphs render as tofu and this falls back to the
        // cap-band centre.)
        for sample in ["中字", "あ漢A"] {
            for size in [15.0f32, 24.0] {
                let box_h = (size * 1.25 + 20.0).max(36.0);
                let label = format!("{sample} (cjk) @ {size:.0}px");
                let r = flow.cell(list, &label, 150.0, box_h);
                list.rect_outline(r, 1.0, [0.30, 0.34, 0.42, 1.0]);
                let ty = list.vcentered_text_y(r.y, r.height, size, theme.font.as_ref(), sample);
                let m = list.font_vmetrics(theme.font.as_ref());
                let visual_center = ty + m.visual_center_ratio(sample) * size;
                // Green: box centre.
                list.quad(
                    r.x,
                    r.y + r.height / 2.0 - 0.5,
                    r.width,
                    1.0,
                    [0.25, 1.0, 0.45, 0.7],
                );
                // Red: computed ideographic visual centre (should coincide with green).
                list.quad(
                    r.x,
                    visual_center - 0.5,
                    r.width,
                    1.0,
                    [1.0, 0.25, 0.25, 0.95],
                );
                list.text(
                    TextBlock::new(sample, r.x + 6.0, ty)
                        .with_size(size)
                        .with_color(228, 232, 240),
                );
            }
        }
        // Numeric readout of the resolved per-font ratios.
        let font_name = theme
            .font
            .as_ref()
            .map(|f| f.family().to_string())
            .unwrap_or_else(|| "default sans (IBM Plex Sans)".to_string());
        let m = list.font_vmetrics(theme.font.as_ref());
        let r = flow.cell(list, "resolved ratios", 440.0, 36.0);
        list.text(
            TextBlock::new(
                format!(
                    "{font_name}: baseline {:.3} · x-height {:.3} · cap {:.3} · cjk-baseline {:.3} · cjk-centre {:.3}  (×font_size)",
                    m.baseline_ratio, m.x_ratio, m.cap_ratio, m.cjk_baseline_ratio, m.cjk_center_ratio
                ),
                r.x,
                r.y + 9.0,
            )
            .with_size(13.0)
            .with_color(200, 210, 230),
        );

        // ---- Fonts ------------------------------------------------------
        // The bundled IBM Plex Sans family (registered by `shared_font_system`)
        // resolves the default sans-serif and provides real bold/italic faces.
        flow.section(list, "Fonts");

        let r = flow.cell(list, "Regular", 130.0, 24.0);
        list.text(
            TextBlock::new("Regular", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235),
        );

        let r = flow.cell(list, "Bold", 130.0, 24.0);
        list.text(
            TextBlock::new("Bold", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .bold(),
        );

        let r = flow.cell(list, "Italic", 130.0, 24.0);
        list.text(
            TextBlock::new("Italic", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .italic(),
        );

        let r = flow.cell(list, "Bold Italic", 140.0, 24.0);
        list.text(
            TextBlock::new("Bold Italic", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .bold()
                .italic(),
        );

        // ---- Span-coloured text ----------------------------------------
        flow.section(list, "Span colour + underline");

        let r = flow.cell(list, "Colour spans", 200.0, 24.0);
        list.text(
            TextBlock::new("", r.x, r.y)
                .with_size(20.0)
                .with_color(255, 255, 255)
                .with_spans(vec![
                    TextSpan {
                        text: "Red".into(),
                        color: Some([1.0, 0.2, 0.2, 1.0]),
                        underline: Underline::None,
                    },
                    TextSpan {
                        text: " · ".into(),
                        color: Some([0.8, 0.8, 0.8, 1.0]),
                        underline: Underline::None,
                    },
                    TextSpan {
                        text: "Green".into(),
                        color: Some([0.2, 1.0, 0.4, 1.0]),
                        underline: Underline::None,
                    },
                    TextSpan {
                        text: " · ".into(),
                        color: Some([0.8, 0.8, 0.8, 1.0]),
                        underline: Underline::None,
                    },
                    TextSpan {
                        text: "Blue".into(),
                        color: Some([0.3, 0.6, 1.0, 1.0]),
                        underline: Underline::None,
                    },
                ]),
        );

        let r = flow.cell(list, "Underline", 200.0, 28.0);
        list.text(
            TextBlock::new("", r.x, r.y)
                .with_size(20.0)
                .with_color(220, 225, 235)
                .with_spans(vec![
                    TextSpan {
                        text: "normal ".into(),
                        color: None,
                        underline: Underline::None,
                    },
                    TextSpan {
                        text: "underlined".into(),
                        color: Some([1.0, 0.9, 0.3, 1.0]),
                        // Inherit → the underline tracks the glyph colour (the
                        // common case); use `Underline::Color` for a contrast rule.
                        underline: Underline::Inherit,
                    },
                    TextSpan {
                        text: " end".into(),
                        color: None,
                        underline: Underline::None,
                    },
                ]),
        );

        // ---- Interactive verbs (UiContext) ------------------------------
        // The crate-side stateful façade: each verb places + localizes the raw
        // widget and auto-advances a vertical cursor. Rendered at rest (the
        // static InputState isn't interacting), so this just eyeballs layout +
        // crispness of the verb stack.
        flow.section(list, "Interactive verbs (UiContext)");
        {
            let r = flow.cell(list, "Stacked verbs", 200.0, 168.0);
            let mut vstate = UiState::new();
            // Static render: dt = 0 freezes the animation clock so the verbs draw
            // their resolved (settled) colors, keeping the PNG deterministic.
            vstate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
            let mut buf = String::from("editable");
            // Scope the `ui.translate` to this block: `UiContext::translate`
            // mutates the shared list's transform-stack top in place and is not
            // restored on drop, so without a push/pop bracket the translate
            // leaks and shifts every later base-layer cell (the whole Widgets
            // section) off-position.
            list.push_transform();
            {
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                ui.text("text() label");
                ui.text_button("text_button()", Some(200.0), None);
                let _ = ui.slider(0, 0.6, 0.0, 1.0, Some(200.0));
                let _ = ui.checkbox("checkbox()", true);
                let _ = ui.text_input(1, &mut buf, "type…", Some(200.0));
            }
            // The verbs auto-advanced the transform cursor down from `r.y`; the
            // delta is the stack's true painted height. Reserve it so the cell's
            // 168px nominal height doesn't let the next section overlap (the
            // stack is taller than that). Read before `pop_transform` restores it.
            let stack_h = list.current_transform().ty - r.y;
            list.pop_transform();
            flow.reserve(stack_h);
            vstate.end_frame();
        }

        // enabled_scope(): an enabled block beside a disabled_scope block — same
        // widgets, the right one grayed and inert. Visual check for the dim tint.
        {
            let r = flow.cell(list, "enabled_scope() / disabled_scope()", 200.0, 96.0);
            let mut estate = UiState::new();
            estate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
            list.push_transform();
            {
                let mut ui = UiContext::interactive(list, &input, &mut estate, &theme);
                ui.translate(r.x, r.y);
                // Enabled block: a button + checkbox at full colour.
                ui.text_button("Enabled", Some(200.0), None);
                let _ = ui.checkbox("on", true);
                // Disabled block: same widgets, grayed + inert.
                ui.disabled_scope(|ui| {
                    ui.text_button("Disabled", Some(200.0), None);
                    let _ = ui.checkbox("off", true);
                });
            }
            list.pop_transform();
            estate.end_frame();
        }

        // ---- Non-interactive themed verbs --------------------------------
        {
            let r = flow.cell(list, "separator()", 200.0, 12.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                ui.separator();
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "progress_bar()", 200.0, 30.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                ui.progress_bar(0.65, Some(200.0));
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "banner()", 200.0, 50.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                ui.banner(Severity::Warning, "Banner message", Some(200.0));
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "group_begin()", 200.0, 80.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                // `group_begin` auto-advances the layout cursor past the group,
                // so wrap it in push/pop to discard that advance — `inner` is
                // local to the *pre*-group transform, and the label belongs
                // inside the group, not after it.
                ui.push();
                let inner = ui.group_begin("Group Title", Some(200.0), 80.0);
                ui.pop();
                ui.push();
                ui.translate(inner.x, inner.y);
                ui.text("Content");
                ui.pop();
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "panel()", 200.0, 50.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                // Same as the group above: discard `panel`'s auto-advance so the
                // label lands inside the panel rather than below it.
                ui.push();
                ui.panel(Some(200.0), 50.0);
                ui.pop();
                ui.push();
                ui.translate(12.0, 14.0);
                ui.text("Inside panel");
                ui.pop();
            }
            list.pop_transform();
        }

        // ---- More interactive verbs --------------------------------------
        {
            let r = flow.cell(list, "tabs()", 240.0, 32.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                vstate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
                {
                    let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                    ui.translate(r.x, r.y);
                    let _ = ui.tabs(&["Tab A", "Tab B", "Tab C"], 0);
                }
                vstate.end_frame();
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "image_button_key()", 40.0, 40.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                // Use the phosphor icon key present in the gallery renderer
                let _ = ui.image_button_key("eye", 32.0, 32.0);
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "color_picker()", 200.0, 180.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                vstate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
                {
                    let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                    ui.translate(r.x, r.y);
                    let mut hsva = Hsva {
                        h: 200.0,
                        s: 0.8,
                        v: 0.9,
                        a: 1.0,
                    };
                    let _ = ui.color_picker(100, &mut hsva, Some(200.0));
                }
                vstate.end_frame();
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "drag_handle()", 200.0, 24.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                vstate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
                {
                    let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                    ui.translate(r.x, r.y);
                    let _ = ui.drag_handle(200, Some(200.0), 24.0);
                }
                vstate.end_frame();
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "scroll_begin()/end()", 200.0, 100.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                vstate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
                // The content is taller than the viewport, so scrollbars appear.
                vstate.scroll.content_size = [200.0, 200.0];
                {
                    let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                    ui.translate(r.x, r.y);
                    let inner = ui.scroll_begin(Some(200.0), 100.0);
                    // `inner` is local to the caller's transform, so no
                    // compensation for the translate above is needed.
                    ui.push();
                    ui.translate(inner.x, inner.y);
                    ui.text("Row 1");
                    ui.text("Row 2");
                    ui.text("Row 3");
                    ui.text("Row 4");
                    ui.text("Row 5");
                    ui.pop();
                    ui.scroll_end();
                }
                vstate.end_frame();
            }
            list.pop_transform();
        }

        {
            let r = flow.cell(list, "dropdown()", 160.0, 30.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                vstate.begin_frame(&mut input, &theme, 0.0, &wgpu_gameui::ManualNav);
                {
                    let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                    ui.translate(r.x, r.y);
                    ui.dropdown(300, &["Alpha", "Beta", "Gamma"], 0, Some(160.0));
                }
                vstate.end_frame();
            }
            list.pop_transform();
        }

        // ---- Widgets ----------------------------------------------------
        flow.section(list, "Widgets");

        let r = flow.cell(list, "Button", 100.0, 32.0);
        Button::draw_at(
            "Button",
            r,
            true,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        let r = flow.cell(list, "Button (bare)", 100.0, 32.0);
        Button::new("Bare")
            .bare()
            .draw(r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Button (disabled)", 110.0, 32.0);
        Button::draw_at(
            "Disabled",
            r,
            false,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        // Keyboard-focused button: seed a local focus owner so the focus ring is
        // visible in the PNG without disturbing the shared focus state.
        let r = flow.cell(list, "Button (focused)", 110.0, 32.0);
        {
            const FOCUSED_BTN: u64 = 300;
            let btn_idle = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let mut btn_focus = FocusState::new();
            btn_focus.focus(FOCUSED_BTN);
            Button::new("Focused").focusable(FOCUSED_BTN).draw(
                r,
                &mut DrawContext::new(list, &mut btn_focus, &theme, &btn_idle, W as f32, 600.0),
            );
        }

        let cb = Checkbox::new();
        let r = flow.cell(list, "Checkbox", 120.0, 20.0);
        cb.draw(false, "Off", r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Checkbox (checked)", 120.0, 20.0);
        cb.draw(true, "On", r, &mut ctx(list, &mut focus, &theme, &input));

        // Ask the group how big it needs to be rather than guessing: a
        // hand-written 76.0 here used to clip the third option's row.
        let radio_opts = ["Low", "Medium", "High"];
        let vertical = RadioGroup::new(&radio_opts);
        let (rw, rh) = vertical.measure(list, &StyleResolver::new(&theme));
        let r = flow.cell(list, "Radio group", rw, rh);
        vertical.draw(1, r, &mut ctx(list, &mut focus, &theme, &input));

        let horizontal = RadioGroup::new(&radio_opts).horizontal();
        let (rw, rh) = horizontal.measure(list, &StyleResolver::new(&theme));
        let r = flow.cell(list, "Radio (horizontal)", rw, rh);
        horizontal.draw(0, r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Progress bar", 150.0, 20.0);
        ProgressBar::new(0.65).draw(r, list, &StyleResolver::new(&theme));

        // Stat banding (caller-owned policy): low/medium/high pick distinct colors.
        let r = flow.cell(list, "Progress (stat: low)", 150.0, 20.0);
        ProgressBar::new(0.15).draw(r, list, &StyleResolver::new(&theme));
        let r = flow.cell(list, "Progress (stat: med)", 150.0, 20.0);
        ProgressBar::new(0.40).draw(r, list, &StyleResolver::new(&theme));

        // Solid fill: neutral progress where "low" isn't bad.
        let r = flow.cell(list, "Progress (solid accent)", 150.0, 20.0);
        ProgressBar::new(0.30)
            .with_fill(ProgressFill::Solid(StyleKey::Accent))
            .draw(r, list, &StyleResolver::new(&theme));

        let r = flow.cell(list, "Slider", 160.0, 24.0);
        let mut capture = DragCapture::default();
        Slider::new(0.0, 100.0).draw(
            40.0,
            0,
            &mut capture,
            r,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        // Drag handle / window-mover: a labelled title bar and a bare grip
        // handle. Static (idle) here — the live delta comes from a DragTracker.
        let r = flow.cell(list, "Drag handle (title bar)", 200.0, 24.0);
        let mut dh_cap = DragCapture::default();
        DragHandle::new().with_label("Inspector").draw(
            10,
            &mut dh_cap,
            r,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        let r = flow.cell(list, "Drag handle (grip)", 64.0, 24.0);
        DragHandle::new().draw(
            11,
            &mut dh_cap,
            r,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        let r = flow.cell(list, "Tabs", 240.0, 30.0);
        Tabs::new(&["Tab A", "Tab B", "Tab C"]).draw(
            r,
            0,
            list,
            &StyleResolver::new(&theme),
            &input,
            None,
        );

        let r = flow.cell(list, "Text input", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_value("Hello, wgpu-gameui!")
            .draw(0, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Text input (empty)", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_placeholder("Placeholder...")
            .draw(1, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Password (masked)", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_value("hunter2")
            .password()
            .draw(201, &mut ctx(list, &mut focus, &theme, &input));

        // Text input mid-IME-composition: focused, with a non-empty preedit
        // spliced into the value at the caret and rendered underlined. Uses a
        // local focus+input so it doesn't disturb the shared focus owner above.
        let r = flow.cell(list, "Text input (composing)", 200.0, 28.0);
        {
            const COMPOSE_ID: u64 = 200;
            let compose_input = InputState {
                preedit: "nihongo".to_string(),
                ..Default::default()
            };
            let mut compose_focus = FocusState::new();
            compose_focus.focus(COMPOSE_ID);
            compose_focus.begin_frame(&compose_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height).with_value("ab cd");
            field.cursor_pos = 3; // caret after "ab ", before "cd"
            field.draw(
                COMPOSE_ID,
                &mut DrawContext::new(
                    list,
                    &mut compose_focus,
                    &theme,
                    &compose_input,
                    W as f32,
                    600.0,
                ),
            );
        }

        // Multi-line text area: focused, with two hard newlines and one line long
        // enough to wrap at the field width. Uses a local focus+input so it
        // doesn't disturb the shared focus owner above.
        let r = flow.cell(list, "Text area (multiline)", 200.0, 86.0);
        {
            const AREA_ID: u64 = 201;
            let area_input = InputState::default();
            let mut area_focus = FocusState::new();
            area_focus.focus(AREA_ID);
            area_focus.begin_frame(&area_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height)
                .with_multiline(true)
                .with_value("line one\nsecond line\nthis third line is long enough to wrap");
            field.cursor_pos = field.value.len();
            field.draw(
                AREA_ID,
                &mut DrawContext::new(list, &mut area_focus, &theme, &area_input, W as f32, 600.0),
            );
        }

        // Number input / spin box: a focused float field showing the +/- step
        // buttons in the right column and an editable value.
        let r = flow.cell(list, "Number input", 140.0, 28.0);
        {
            const NUM_ID: u64 = 202;
            let num_input = InputState::default();
            let mut num_focus = FocusState::new();
            num_focus.focus(NUM_ID);
            num_focus.begin_frame(&num_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height);
            NumberInput::new()
                .with_range(0.0, 100.0)
                .with_step(1.0)
                .with_decimals(1)
                .draw(
                    42.5,
                    NUM_ID,
                    &mut field,
                    r,
                    &mut DrawContext::new(
                        list,
                        &mut num_focus,
                        &theme,
                        &num_input,
                        W as f32,
                        600.0,
                    ),
                );
        }

        // Number input with a custom formatter: zero-padded HH:MM fields, the
        // shape a daily-summary time picker needs. Both fields are unfocused so
        // the formatter owns the displayed text ("07" / "30", not "7" / "30").
        let r = flow.cell(list, "Number input (zero-pad HH:MM)", 150.0, 28.0);
        {
            let num_input = InputState::default();
            let mut num_focus = FocusState::new();
            num_focus.begin_frame(&num_input);
            let hh = |v: f64| format!("{:02}", v.round() as i64);
            let colon_w = 8.0;
            let field_w = (r.width - colon_w) / 2.0;
            let hh_rect = Rect::new(r.x, r.y, field_w, r.height);
            let mm_rect = Rect::new(r.x + field_w + colon_w, r.y, field_w, r.height);
            let mut hh_field = TextInput::new(hh_rect.x, hh_rect.y, hh_rect.width, hh_rect.height);
            let mut mm_field = TextInput::new(mm_rect.x, mm_rect.y, mm_rect.width, mm_rect.height);
            let mut draw = |id, value, field, rect| {
                NumberInput::new()
                    .with_range(0.0, 59.0)
                    .with_step(1.0)
                    .with_formatter(hh)
                    .draw(
                        value,
                        id,
                        field,
                        rect,
                        &mut DrawContext::new(
                            list,
                            &mut num_focus,
                            &theme,
                            &num_input,
                            W as f32,
                            600.0,
                        ),
                    )
            };
            draw(203, 7.0, &mut hh_field, hh_rect);
            draw(204, 30.0, &mut mm_field, mm_rect);
            // The ":" separator between the two fields.
            list.text(TextBlock::new(":", r.x + field_w, r.y + 4.0).with_size(theme.font_size));
        }

        // Tree view (outliner): a seeded hierarchy — an expanded branch with
        // indented children (one selected), a collapsed branch, and a root leaf.
        // Each row carries a leading "visibility" icon plus trailing
        // rename/delete icons (their own hit targets), demonstrating the
        // scene/layer-outliner shape. Idle input, so nothing toggles.
        let r = flow.cell(list, "Tree view (outliner)", 200.0, 110.0);
        {
            const VIS: u32 = 1;
            const RENAME: u32 = 2;
            const DEL: u32 = 3;
            let mut tree = TreeState::new();
            tree.set_expanded(1, true); // "Materials" expanded
            tree.set_expanded(4, false); // "Foliage" collapsed
            tree.select(3); // "Metal" selected
            let idle = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let leading = [TreeAction::sprite(VIS, ball)];
            let trailing = [
                TreeAction::sprite(RENAME, board),
                TreeAction::sprite(DEL, suitcase),
            ];
            let rows: [(u64, &str, bool, usize); 5] = [
                (1, "Materials", false, 0),
                (2, "Wood", true, 1),
                (3, "Metal", true, 1),
                (4, "Foliage", false, 0),
                (5, "Stone", true, 0),
            ];
            for (i, (id, label, leaf, depth)) in rows.iter().enumerate() {
                let row = Rect::new(r.x, r.y + i as f32 * 21.0, r.width, 20.0);
                let mut tctx = DrawContext::new(list, &mut focus, &theme, &idle, W as f32, 600.0);
                TreeNode::new(label)
                    .with_leaf(*leaf)
                    .with_depth(*depth)
                    .with_leading(&leading)
                    .with_trailing(&trailing)
                    .draw(*id, row, &mut tree, &mut tctx);
            }
        }

        // Context menu state, shown over a viewport swatch. Its modal layer is
        // drawn after the base scope, matching the production integration path.
        let context_area = flow.cell(list, "Context menu (cursor anchored)", 700.0, 150.0);
        list.vertical_gradient(
            context_area,
            [0.03, 0.12, 0.16, 1.0],
            [0.08, 0.24, 0.28, 1.0],
        );
        context_state.open_at(context_area.x + 18.0, context_area.y + 14.0);
        assert!(context_state.set_open_path(&context_menu, &[1, 0]));
        flow.reserve(170.0);

        // Dropdown, seeded open: the floating list (drawn after the base scope)
        // renders above whatever cells sit below it.
        let r = flow.cell(list, "Dropdown (open)", 160.0, 28.0);
        dropdowns.open_for_test(DROPDOWN_ID, r, &DROPDOWN_ITEMS, 2);
        let dropdown = Dropdown::new(&DROPDOWN_ITEMS, 2);
        // The open list is an overlay (drawn later into a popup layer), so the
        // cell only nominally reserves the 28px button. Reserve its full open
        // footprint too, so the floating list doesn't paint over the rows below.
        let menu = dropdown.open_list_rect(r);
        flow.reserve(menu.y + menu.height - r.y);
        dropdown.draw(
            DROPDOWN_ID,
            r,
            &mut dropdowns,
            &mut DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0),
        );

        // The same 12 overflowing rows, drawn twice: at rest, and scrolled to a
        // target, so the clip and the moved scrubber are visible in a still
        // image. (The *easing* itself is motion and cannot show up in a PNG —
        // `scroll_view`'s unit tests cover the glide.)
        let scroll_rows = |list: &mut DrawList, vp: Rect| {
            for i in 0..12usize {
                let y = vp.y + i as f32 * 22.0;
                let bg = if i % 2 == 0 {
                    [0.16, 0.18, 0.24, 1.0]
                } else {
                    [0.10, 0.12, 0.18, 1.0]
                };
                // `vp` already excludes the scrollbar gutter, so fill it
                // edge-to-edge; the row only pads its own text.
                list.quad(vp.x, y + 2.0, vp.width, 18.0, bg);
                list.text(
                    TextBlock::new(format!("Item #{:02}", i), vp.x + 8.0, y + 3.0)
                        .with_size(12.0)
                        .with_color(180, 190, 210),
                );
            }
        };

        let r = flow.cell(list, "Scroll view", 180.0, 100.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        let mut scroll_state = ScrollState::default();
        scroll_state.content_size = [160.0, 300.0];
        ScrollView::new(r).vertical_only().draw(
            &mut scroll_state,
            list,
            &StyleResolver::new(&theme),
            &mut input,
            scroll_rows,
        );

        let r = flow.cell(list, "Scroll view (scrolled)", 180.0, 100.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        let mut scrolled_state = ScrollState::default();
        scrolled_state.content_size = [160.0, 300.0];
        // `snap_to`, not a bare `offset` write: the drawn offset eases toward its
        // target, so a one-frame render has to seed both to sit still.
        scrolled_state.snap_to(1, 66.0);
        ScrollView::new(r).vertical_only().draw(
            &mut scrolled_state,
            list,
            &StyleResolver::new(&theme),
            &mut input,
            scroll_rows,
        );

        let columns = &[
            TableColumn::new("Name", ColumnWidth::Fixed(100.0)),
            TableColumn::new("Score", ColumnWidth::Fixed(60.0)),
            TableColumn::new("Status", ColumnWidth::Flex(1.0)),
        ];
        let rows = vec![
            vec![
                TableCell::new("Alice"),
                TableCell::new("95"),
                TableCell::new("Pass"),
            ],
            vec![
                TableCell::new("Bob"),
                TableCell::new("72"),
                TableCell::new("Pass"),
            ],
            vec![
                TableCell::new("Charlie"),
                TableCell::new("48"),
                TableCell::new("Fail"),
            ],
        ];
        let r = flow.cell(list, "Table", 270.0, 88.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        Table::new(columns).draw(
            r,
            &rows,
            &mut ScrollState::default(),
            list,
            &StyleResolver::new(&theme),
            &mut input,
        );

        let r = flow.cell(list, "Image button", 40.0, 40.0);
        ImageButton::sprite(duck)
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &StyleResolver::new(&theme), &input);

        let r = flow.cell(list, "Image button (bare)", 40.0, 40.0);
        ImageButton::sprite(board)
            .bare()
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &StyleResolver::new(&theme), &input);

        let r = flow.cell(list, "Image button (disabled)", 40.0, 40.0);
        ImageButton::sprite(suitcase)
            .enabled(false)
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &StyleResolver::new(&theme), &input);

        // ---- Lists / Grids (virtualized) --------------------------------
        flow.section(list, "Lists / Grids");

        // (a) Vertical list: 12 rows, one selected + one hovered (seeded by
        // pointing the idle mouse at row 4 so the hover background shows).
        {
            let items: [&str; 12] = [
                "Sword", "Shield", "Potion", "Bow", "Arrow", "Helmet", "Gauntlet", "Boots", "Ring",
                "Amulet", "Scroll", "Torch",
            ];
            let r = flow.cell(list, "List (selectable)", 150.0, 150.0);
            list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
            let hover = InputState {
                mouse_x: r.x + 20.0,
                mouse_y: r.y + 22.0 * 4.0 + 8.0,
                ..InputState::default()
            };
            let mut state = ListState::new();
            state.select_one(1); // "Shield" selected
            let mut hover_in = hover;
            List::new()
                .with_item_height(22.0)
                .with_zebra(true)
                .selection(SelectionMode::Single)
                .draw(
                    r,
                    items.len(),
                    &mut state,
                    list,
                    &StyleResolver::new(&theme),
                    &mut hover_in,
                    |list, cell, it: ListItem| {
                        // Debug: outline the cell rect handed to the closure, so
                        // the item's content padding is visible.
                        list.rect_outline(cell, 1.0, [1.0, 0.25, 0.8, 0.9]);
                        let c = if it.selected {
                            (20, 24, 34)
                        } else {
                            (200, 210, 230)
                        };
                        list.text(
                            TextBlock::new(items[it.index], cell.x + 8.0, cell.y + 4.0)
                                .with_size(13.0)
                                .with_color(c.0, c.1, c.2),
                        );
                    },
                );
        }

        // (b) Grid: 4 columns of colored tiles with an index label.
        {
            let r = flow.cell(list, "Grid (4 cols)", 150.0, 150.0);
            list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
            let mut state = ListState::new();
            state.select_one(5);
            let mut idle_in = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            List::new()
                .with_item_height(32.0)
                .columns(4)
                .with_gap(6.0, 6.0)
                .selection(SelectionMode::Multi)
                .draw(
                    r,
                    24,
                    &mut state,
                    list,
                    &StyleResolver::new(&theme),
                    &mut idle_in,
                    |list, cell, it: ListItem| {
                        // Debug: outline the cell rect handed to the closure.
                        list.rect_outline(cell, 1.0, [1.0, 0.25, 0.8, 0.9]);
                        // Tile fill: a hue ramp so the grid reads as distinct cells.
                        let t = it.index as f32 / 24.0;
                        let fill = if it.selected {
                            [0.95, 0.85, 0.30, 1.0]
                        } else {
                            [0.20 + 0.5 * t, 0.30, 0.55 - 0.3 * t, 1.0]
                        };
                        list.rounded_rect(
                            Rect::new(
                                cell.x + 2.0,
                                cell.y + 2.0,
                                cell.width - 4.0,
                                cell.height - 4.0,
                            ),
                            3.0,
                            fill,
                        );
                        list.text(
                            TextBlock::new(format!("{}", it.index), cell.x + 6.0, cell.y + 9.0)
                                .with_size(12.0)
                                .with_color(240, 245, 255),
                        );
                    },
                );
        }

        // (c) Tall list in a short cell: shows the scrollbar + virtualization
        // (1000 items, scrolled partway down).
        {
            let r = flow.cell(list, "Virtualized (1000)", 150.0, 110.0);
            list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
            let mut state = ListState::new();
            state.scroll.offset[1] = 420.0; // scrolled partway
            let mut idle_in = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            List::new().with_item_height(20.0).draw(
                r,
                1000,
                &mut state,
                list,
                &StyleResolver::new(&theme),
                &mut idle_in,
                |list, cell, it: ListItem| {
                    // Debug: outline the cell rect handed to the closure. The
                    // cell already excludes the scrollbar gutter (ScrollView
                    // reserves it), so the item fills the cell edge-to-edge and
                    // only pads its *own* text.
                    list.rect_outline(cell, 1.0, [1.0, 0.25, 0.8, 0.9]);
                    let bg = if it.index.is_multiple_of(2) {
                        [0.13, 0.15, 0.20, 1.0]
                    } else {
                        [0.09, 0.11, 0.16, 1.0]
                    };
                    list.quad(cell.x, cell.y, cell.width, cell.height, bg);
                    list.text(
                        TextBlock::new(format!("Row #{:04}", it.index), cell.x + 8.0, cell.y + 3.0)
                            .with_size(12.0)
                            .with_color(180, 190, 210),
                    );
                },
            );
        }

        // ---- Instanced chrome (SDF rounded-rect) ------------------------
        // Every `Button` already routes its background+border through the
        // instanced `chrome_rect` path; this section makes the batching
        // explicit (a strip of same-shape buttons collapses to one base mesh +
        // N instances) and shows the rotated-transform fallback still renders.
        flow.section(list, "Instanced chrome");

        for i in 0..6 {
            let r = flow.cell(list, "", 70.0, 30.0);
            Button::new(format!("#{i}")).draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

        // Rotated chrome: `chrome_rect` can't express a rotation as a single
        // axis-aligned instance, so it falls back to immediate tessellation.
        let r = flow.cell(list, "Rotated (fallback)", 80.0, 40.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.chrome_rect(
            Rect::new(-r.width / 2.0, -r.height / 2.0, r.width, r.height),
            8.0,
            2.0,
            [0.30, 0.55, 0.35, 1.0],
            [0.80, 0.90, 0.80, 1.0],
        );
        list.pop_transform();

        // ---- Hit zone (draw-free sensor) --------------------------------
        // `HitZone` draws NOTHING — it only senses pointer interaction over a
        // rect (Teardown's UiMakeInteractive), for sensors over things the UI
        // didn't draw (3D viewports, world-projected regions). The gallery
        // can't show "nothing", so each cell paints its own outline + a caption
        // reporting the state `HitZone::test` returns for a synthetic pointer.
        flow.section(list, "Hit zone (sensor)");

        // Idle: pointer parked far away → not hovered.
        let r = flow.cell(list, "Idle (no pointer)", 150.0, 44.0);
        {
            let away = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let out = HitZone::new().test(r, &away);
            list.rounded_rect_outline(r, 4.0, 1.5, [0.40, 0.45, 0.55, 1.0]);
            list.text(
                TextBlock::new(
                    if out.hovered { "hovered" } else { "idle" },
                    r.x + 10.0,
                    r.y + 14.0,
                )
                .with_size(12.0)
                .with_color(150, 160, 180),
            );
        }

        // Hovered + clicked: synthetic pointer at the cell centre with a click.
        let r = flow.cell(list, "Hovered + click", 150.0, 44.0);
        {
            let over = InputState {
                mouse_x: r.x + r.width / 2.0,
                mouse_y: r.y + r.height / 2.0,
                mouse_down: true,
                mouse_clicked: true,
                ..InputState::default()
            };
            let out = HitZone::new().test(r, &over);
            // Highlight to reflect the sensed hover (the widget itself draws
            // none of this — the gallery does, from the returned state).
            let glow = if out.hovered {
                [0.20, 0.55, 0.95, 0.18]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            };
            list.rounded_rect(r, 4.0, glow);
            list.rounded_rect_outline(r, 4.0, 1.5, [0.35, 0.65, 1.0, 1.0]);
            let caption = if out.clicked {
                "hovered + clicked"
            } else if out.hovered {
                "hovered"
            } else {
                "idle"
            };
            list.text(
                TextBlock::new(caption, r.x + 10.0, r.y + 14.0)
                    .with_size(12.0)
                    .with_color(200, 215, 240),
            );
        }

        // --- Styling / overrides -------------------------------------------
        // Per-widget restyling with NO theme clone: a scoped `StyleOverlay`
        // layered over the theme via `DrawContext::with_style`, plus a custom
        // (mod-defined) key resolved by name.
        flow.section(list, "Styling / overrides");

        // Baseline button — straight theme colors.
        let r = flow.cell(list, "Button (theme)", 120.0, 32.0);
        Button::new("Normal").draw(r, &mut ctx(list, &mut focus, &theme, &input));

        // Same widget under an overlay — recolored fill/border/text only.
        let mut overlay = StyleOverlay::new();
        overlay
            .set_color(StyleKey::Button, [0.45, 0.12, 0.55, 1.0])
            .set_color(StyleKey::ButtonBorder, [0.85, 0.55, 0.95, 1.0])
            .set_color(StyleKey::Text, [1.0, 0.92, 1.0, 1.0]);
        let r = flow.cell(list, "Button (overlay)", 120.0, 32.0);
        {
            let mut octx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
                .with_style(&overlay);
            Button::new("Restyled").draw(r, &mut octx);
        }

        // Custom key: a mod-defined style resolved through the overlay (a custom
        // widget can carry its own style with zero core changes). Swatch + name.
        let mut custom = StyleOverlay::new();
        let glow_key = StyleKey::custom("mywidget.glow");
        custom.set_color(glow_key, [0.20, 0.85, 0.65, 1.0]);
        let r = flow.cell(list, "Custom key", 170.0, 32.0);
        let resolver = StyleResolver::with_overlay(&theme, &custom);
        let c = resolver.color_or(glow_key, [1.0, 0.0, 1.0, 1.0]);
        list.rounded_rect(Rect::new(r.x, r.y, 28.0, 28.0), 6.0, c);
        list.text(
            TextBlock::new("mywidget.glow", r.x + 36.0, r.y + 8.0)
                .with_size(12.0)
                .with_color(200, 210, 230),
        );

        // --- Layout primitives (weighted + flow) ---------------------------
        // The declarative layout engine (`wgpu_gameui::layout`), not widgets:
        // a weighted `HStack` Fill split and the wrapping `Flow` grid. Each
        // computed `Rect` is painted as a plain rounded rect so the split ratios
        // and the row-wrapping are eyeballable.
        flow.section(list, "Layout: weighted HStack + Flow grid");

        // Weighted HStack — remaining width split 2:1:1 across three Fill cells.
        {
            let r = flow.cell(list, "HStack weight 2:1:1", 300.0, 36.0);
            let split = HStack::new(6.0)
                .child_fill(0.0)
                .weight(2.0)
                .child_fill(0.0)
                .weight(1.0)
                .child_fill(0.0)
                .weight(1.0);
            let res = split.layout(r);
            let colors = [
                [0.30, 0.50, 0.90, 1.0],
                [0.30, 0.75, 0.55, 1.0],
                [0.85, 0.55, 0.30, 1.0],
            ];
            for (i, c) in colors.iter().enumerate() {
                list.rounded_rect(res.get(i + 1), 4.0, *c);
            }
        }

        // Flow grid — nine uniform 40px tiles wrapping within a fixed width.
        {
            let grid_w = 200.0;
            let mut grid = LayoutFlow::new(8.0);
            for _ in 0..9 {
                grid = grid.item(40.0, 40.0);
            }
            let grid_h = grid.measure_height(grid_w);
            let r = flow.cell(list, "Flow grid (wraps)", grid_w, grid_h);
            let res = grid.layout(r);
            for (i, rc) in res.children().enumerate() {
                let t = i as f32 / 8.0;
                list.rounded_rect(rc, 6.0, [0.25 + 0.5 * t, 0.45, 0.85 - 0.4 * t, 1.0]);
            }
        }

        // Main-axis justification (justify-content) — the same three fixed-size
        // cells distributed six ways across a fixed-width track, so the spacing
        // policies are eyeballable stacked vertically.
        flow.section(list, "Justify (main-axis distribution)");
        for (label, mode) in [
            ("Start", MainAlign::Start),
            ("Center", MainAlign::Center),
            ("End", MainAlign::End),
            ("SpaceBetween", MainAlign::SpaceBetween),
            ("SpaceAround", MainAlign::SpaceAround),
            ("SpaceEvenly", MainAlign::SpaceEvenly),
        ] {
            let track_w = 300.0;
            let r = flow.cell(list, label, track_w, 28.0);
            // Faint track backing so empty space reads as "the container".
            list.rounded_rect(r, 4.0, [0.16, 0.16, 0.20, 1.0]);
            let row = HStack::new(0.0)
                .justify(mode)
                .child(44.0, 24.0)
                .child(44.0, 24.0)
                .child(44.0, 24.0);
            let res = row.layout(r);
            for rc in res.children() {
                list.rounded_rect(rc, 4.0, [0.30, 0.55, 0.90, 1.0]);
            }
        }

        // --- Separators / dividers -----------------------------------------
        // Thin rules, centered in their cell. Defaults pull thickness from the
        // theme border width and color from the panel-border; the third row
        // overrides both. The vertical demo splits a cell into two columns.
        flow.section(list, "Separator / divider");
        {
            let style = StyleResolver::new(&theme);

            // Plain horizontal rule (theme defaults), centered in a tall cell.
            let r = flow.cell(list, "horizontal", 200.0, 20.0);
            Separator::horizontal().draw(r, list, &style);

            // Inset horizontal rule between two faux text lines.
            let r = flow.cell(list, "inset 16px", 200.0, 40.0);
            list.text(TextBlock::new("above", r.x, r.y).with_size(13.0));
            Separator::horizontal().with_inset(16.0).draw(
                Rect::new(r.x, r.y + 18.0, r.width, 4.0),
                list,
                &style,
            );
            list.text(TextBlock::new("below", r.x, r.y + 24.0).with_size(13.0));

            // Thick accent rule (overridden thickness + color).
            let r = flow.cell(list, "thick accent", 200.0, 20.0);
            Separator::horizontal()
                .with_thickness(4.0)
                .with_color(theme.accent)
                .draw(r, list, &style);

            // Vertical divider splitting a cell into two columns.
            let r = flow.cell(list, "vertical", 120.0, 48.0);
            list.text(TextBlock::new("L", r.x + 16.0, r.y + 16.0).with_size(13.0));
            Separator::vertical().with_inset(6.0).draw(
                Rect::new(r.x + r.width * 0.5 - 2.0, r.y, 4.0, r.height),
                list,
                &style,
            );
            list.text(TextBlock::new("R", r.x + r.width - 28.0, r.y + 16.0).with_size(13.0));
        }

        // --- Splitter ------------------------------------------------------
        flow.section(list, "Splitter — Forge dark chrome states");
        {
            // Vertical idle / hover / captured-drag states plus the rotated
            // horizontal variant. The pointer leaves the dragging strip to
            // exercise capture-persistent feedback in the static gallery.
            for (index, label) in ["Vertical idle", "Vertical hover", "Vertical dragging"]
                .iter()
                .enumerate()
            {
                let r = flow.cell(list, label, 72.0, 80.0);
                let strip = Rect::new(r.x + 33.0, r.y, 6.0, r.height);
                let mut splitter_input = InputState::default();
                let mut capture = DragCapture::new();
                if index == 1 {
                    splitter_input.mouse_x = strip.x + 3.0;
                    splitter_input.mouse_y = strip.y + 40.0;
                } else if index == 2 {
                    capture.try_begin(0x5A10 + index as u64);
                    splitter_input.mouse_x = strip.right() + 20.0;
                    splitter_input.mouse_y = strip.y + 40.0;
                    splitter_input.mouse_down = true;
                    splitter_input.is_dragging = true;
                }
                let mut sctx =
                    DrawContext::new(list, &mut focus, &theme, &splitter_input, W as f32, 600.0);
                Splitter::vertical(6.0).draw(0x5A10 + index as u64, &mut capture, strip, &mut sctx);
            }

            let r = flow.cell(list, "Horizontal idle", 80.0, 72.0);
            let strip = Rect::new(r.x, r.y + 33.0, r.width, 6.0);
            let mut capture = DragCapture::new();
            let splitter_input = InputState::default();
            let mut sctx =
                DrawContext::new(list, &mut focus, &theme, &splitter_input, W as f32, 600.0);
            Splitter::horizontal(6.0).draw(0x5A20, &mut capture, strip, &mut sctx);
        }

        // --- Color picker --------------------------------------------------
        // SV square (white→hue across, →black down) + vertical hue spectrum,
        // optionally an alpha bar (checkerboard under an opaque→transparent
        // fade). Cursors sit at the fixed sample colors below.
        flow.section(list, "Color picker");
        {
            let mut cap = DragCapture::new();

            // HSV only — a warm orange.
            let r = flow.cell(list, "HSV (no alpha)", 220.0, 120.0);
            {
                let mut c = ctx(list, &mut focus, &theme, &input);
                ColorPicker::new().draw(Hsva::opaque(28.0, 0.85, 0.95), 900, &mut cap, r, &mut c);
            }

            // HSVA — a half-transparent teal, showing the alpha bar.
            let r = flow.cell(list, "HSVA (alpha bar)", 248.0, 120.0);
            {
                let mut c = ctx(list, &mut focus, &theme, &input);
                ColorPicker::new().with_alpha(true).draw(
                    Hsva::new(175.0, 0.7, 0.8, 0.5),
                    901,
                    &mut cap,
                    r,
                    &mut c,
                );
            }
        }

        // --- Gradients -----------------------------------------------------
        // Linear (horizontal / vertical / arbitrary angle) and radial fills,
        // built straight on the DrawList's per-vertex color path.
        flow.section(list, "Gradients");
        {
            let h = 80.0;
            let r = flow.cell(list, "Horizontal", 150.0, h);
            list.horizontal_gradient(r, [0.95, 0.30, 0.35, 1.0], [0.20, 0.45, 0.95, 1.0]);

            let r = flow.cell(list, "Vertical", 150.0, h);
            list.vertical_gradient(r, [0.26, 0.72, 0.42, 1.0], [0.09, 0.10, 0.13, 1.0]);

            let r = flow.cell(list, "Linear 45°", 150.0, h);
            list.linear_gradient(
                r,
                [0.95, 0.70, 0.20, 1.0],
                [0.55, 0.20, 0.75, 1.0],
                std::f32::consts::FRAC_PI_4,
            );

            let r = flow.cell(list, "Radial", 150.0, h);
            list.radial_gradient(r, [1.0, 1.0, 1.0, 1.0], [0.09, 0.10, 0.13, 1.0], 64);
        }

        // --- Contextual measured layout ------------------------------------
        // Text and button are measured once under the active style/font, then a
        // reusable plain-data HStack aligns their first baselines. Each arranged
        // body is drawn once through `draw_in_rect`.
        flow.section(list, "Contextual measured layout");
        {
            let r = flow.cell(list, "Measured baseline row", 360.0, 54.0);
            let mut state = UiState::new();
            let mut layout = wgpu_gameui::layout::LayoutResult::default();
            let mut measured = MeasureBuffer::new();
            let mut ui = UiContext::interactive(list, &input, &mut state, &theme);
            let label = ui.measure(
                MeasureConstraints::UNBOUNDED,
                1.0,
                wgpu_gameui::WrapMode::None,
                |cx| cx.measure_text(cx.text_block("Player name")).measurement,
            );
            let save = ui.measure_text_button("Save", MeasureConstraints::UNBOUNDED, 1.0);
            measured.push(
                MeasuredChild::fit(label)
                    .align(wgpu_gameui::layout::CrossAlign::Baseline)
                    .id(1),
            );
            measured.push(
                MeasuredChild::fit(save)
                    .align(wgpu_gameui::layout::CrossAlign::Baseline)
                    .id(2),
            );
            measured
                .arrange_hstack_into(r, 12.0, 0.0, MainAlign::Start, &mut layout)
                .unwrap();
            ui.draw_in_rect_named(
                "MeasuredLabel",
                layout.get_by_id(1_u64).unwrap(),
                false,
                |ui| {
                    ui.text("Player name");
                },
            );
            ui.draw_in_rect_named(
                "MeasuredSave",
                layout.get_by_id(2_u64).unwrap(),
                false,
                |ui| {
                    ui.text_button("Save", Some(save.preferred[0]), Some(save.preferred[1]));
                },
            );
        }

        // --- Group / titled panel ------------------------------------------
        // A bordered container with a header strip; `draw` returns the inner
        // content rect, which we fill with a couple of child widgets.
        flow.section(list, "Group / titled panel");
        {
            let style = StyleResolver::new(&theme);
            let r = flow.cell(list, "Group", 240.0, 130.0);
            let content = Group::new("Inventory").draw(r, list, &style);
            // Place children inside the returned content rect.
            Separator::horizontal().draw(
                Rect::new(content.x, content.y + 24.0, content.width, 2.0),
                list,
                &style,
            );
            list.text(
                TextBlock::new("12 items · 3.4 kg", content.x, content.y)
                    .with_size(13.0)
                    .with_color(190, 200, 220),
            );
            list.text(
                TextBlock::new("Capacity: 60%", content.x, content.y + 34.0)
                    .with_size(13.0)
                    .with_color(150, 160, 180),
            );
        }

        // --- 4a design additions --------------------------------------------
        // The new widgets ported from the "4a" UI design folder. One section
        // per design-sheet grouping; every widget is drawn from its public API.
        let s = StyleResolver::new(&theme);

        flow.section(list, "4a: toggle · badge · keycap · chip");
        {
            let row = flow.cell(list, "", 360.0, 48.0);
            let mut x = row.x;
            // Toggle (off + on + labeled).
            let mut tctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let r = Rect::new(x, row.y + 2.0, 60.0, 18.0);
            Toggle::new().draw(false, r, &mut tctx);
            let r = Rect::new(x, row.y + 26.0, 90.0, 18.0);
            Toggle::new().label("shadows").draw(true, r, &mut tctx);
            x += 100.0;
            // Badges (status tones from the design's table).
            badge(
                list,
                &s,
                Rect::new(x, row.y + 2.0, 70.0, 15.0),
                "ok",
                s.color(StyleKey::Success),
            );
            badge(
                list,
                &s,
                Rect::new(x, row.y + 24.0, 90.0, 15.0),
                "over",
                s.color(StyleKey::Error),
            );
            x += 100.0;
            // Keycaps.
            let mut kx = x;
            for cap in ["⇧", "Ctrl", "F"] {
                kx = keycap(list, &s, Rect::new(kx, row.y + 6.0, 200.0, 22.0), cap, 18.0).right()
                    + 4.0;
            }
            x += 150.0;
            // Chips (filter row: first on, rest off).
            let mut cx = x;
            for (j, label) in ["info", "warn", "verbose"].iter().enumerate() {
                let out = chip(
                    list,
                    &s,
                    Rect::new(cx, row.y + 8.0, 70.0, 20.0),
                    label,
                    j == 0,
                    &input,
                );
                let _ = out;
                cx += 62.0;
            }
        }

        flow.section(list, "4a: breadcrumb · pager · status bar");
        {
            let r = flow.cell(list, "", 300.0, 20.0);
            let segs = ["World", "Region", "Forest"];
            Breadcrumb::new(&segs).draw(r, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });

            let r = flow.cell(list, "", 150.0, 20.0);
            Pager::new().draw(2, 12, r, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });

            let r = flow.cell(list, "", 300.0, 18.0);
            Pager::new().numeric().draw(0, 4, r, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });

            let r = flow.cell(list, "", 400.0, STATUS_BAR_HEIGHT);
            draw_status_bar(
                r,
                &[
                    StatusCell::text("Ready"),
                    StatusCell::text("118 fps").highlight(),
                    StatusCell::text(" tri 18,204"),
                    StatusCell::spacer(24.0),
                ],
                &mut DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0),
            );
        }

        flow.section(list, "4a: vector field · tag input · combo trigger");
        {
            let r_vec = flow.cell(list, "", 280.0, 46.0);
            let r_tags = flow.cell(list, "Tags", 220.0, 48.0);
            let r_combo = flow.cell(list, "Combo", 170.0, 24.0);
            let r_docs = flow.cell(list, "Doc tabs", 300.0, 24.0);

            let mut vctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let mut scrub: Option<VectorScrub> = None;
            let mut capture = DragCapture::new();
            let rows = [
                ("Position", [8.90f32, 12.90, 9.00]),
                ("Rotation", [0.00, 45.00, 0.00]),
            ];
            let _ = VectorField::new(&rows).draw(r_vec, &mut scrub, &mut capture, 900, &mut vctx);

            // Tag input with committed tags + a draft.
            let mut tctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let tags = vec!["occlusion".to_string(), "static".to_string()];
            let mut draft = String::new();
            let _ = draw_tag_input(r_tags, &tags, &mut draft, false, &mut tctx);

            // Combo trigger.
            let mut cctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let _ = draw_combo_trigger(r_combo, "Standard Lit", false, false, &mut cctx);

            // Document tabs.
            let docs = [
                DocTab {
                    label: "Level_01",
                    dirty: true,
                },
                DocTab {
                    label: "Arena",
                    dirty: false,
                },
                DocTab {
                    label: "Physics",
                    dirty: true,
                },
            ];
            let _ = draw_doc_tabs(r_docs, &docs, 0, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });
        }

        flow.section(list, "4a: asset grid · busy states · empty state");
        {
            let assets = ["Crate_A", "Barrel", "Lamp_Post", "Bridge_A"];
            let r = flow.cell(list, "", 380.0, 96.0);
            let _ = AssetGrid::new(&assets, "▣").draw(r, 0, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });

            // Busy states.
            let mut bctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let _ = &mut bctx;
            let r = flow.cell(list, "Skeleton", 120.0, 10.0);
            skeleton(list, &s, r, 0.3);
            let r = flow.cell(list, "Spinner + dots", 80.0, 24.0);
            spinner(list, &s, (r.x + 12.0, r.y + 12.0), 8.0, 0.4, 1.7);
            dots(list, &s, (r.x + 52.0, r.y + 12.0), 0.3);

            // Empty state with a CTA button under it.
            let r = flow.cell(list, "Empty state", 260.0, 110.0);
            let used = empty_state(
                list,
                &s,
                r,
                &EmptyState {
                    glyph: "◈",
                    title: "No entities",
                    body: "Create one to get started.",
                },
            );
            let cta = Rect::new(
                used.x + (used.width - 120.0) * 0.5,
                used.bottom() + 6.0,
                120.0,
                24.0,
            );
            let mut ectx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let _ = Button::new("Create Entity")
                .tone(Tone::Accent)
                .draw(cta, &mut ectx);
        }

        flow.section(list, "4a: gradient ramp · curve editor · popover");
        {
            let r_ramp = flow.cell(list, "Ramp (handles above)", 240.0, 42.0);
            let r_curve = flow.cell(list, "Curve", 170.0, 110.0);
            let r_pop = flow.cell(list, "Popover", 240.0, 96.0);

            // Gradient ramp with three stops.
            let mut gctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let stops = [
                GradientStop::new(0.0, [0.05, 0.1, 0.12, 1.0]),
                GradientStop::new(0.45, [0.24, 0.75, 0.78, 1.0]),
                GradientStop::new(1.0, [0.95, 0.97, 0.98, 1.0]),
            ];
            let mut drag: Option<usize> = None;
            let mut capture = DragCapture::new();
            let _ = draw_gradient_ramp(r_ramp, &stops, 1, &mut drag, &mut capture, 901, &mut gctx);

            // Curve editor.
            let mut dctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let keys = [[0.0f32, 0.0], [0.35, 0.65], [0.7, 0.8], [1.0, 1.0]];
            let mut cdrag: Option<usize> = None;
            let mut ccapture = DragCapture::new();
            let _ = draw_curve_editor(r_curve, &keys, 1, &mut cdrag, &mut ccapture, 902, &mut dctx);

            // Popover (drawn pointing up at an anchor stub).
            let anchor = Rect::new(r_pop.x + 90.0, r_pop.y + r_pop.height - 24.0, 60.0, 20.0);
            list.rounded_rect(anchor, 1.0, [0.16, 0.19, 0.22, 1.0]);
            list.text(
                TextBlock::new("Anchor", anchor.x + 8.0, anchor.y + 4.0)
                    .with_size(11.0)
                    .with_color(190, 200, 220),
            );
            let popover_lines = ["Enter a new name for the selected entity."];
            let popover_width = 212.0;
            let popover_height = wgpu_gameui::measure_sheet_height(
                popover_width,
                "Rename",
                &popover_lines,
                list,
                &s,
            );
            let body = place_popover(
                [anchor.x + anchor.width * 0.5, anchor.y],
                [popover_width, popover_height],
                Rect::new(0.0, 0.0, W as f32, 600.0),
                PopoverSide::Above,
            );
            Popover.draw(body, PopoverSide::Above, "Rename", &popover_lines, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });
        }

        // --- Banners & toasts ----------------------------------------------
        // Severity banners (info/success/warning/error) and a corner toast stack.
        // The toast stack normally anchors to the screen; here we translate it
        // into a reserved cell so it shows inline.
        flow.section(list, "Banners & toasts");
        {
            let style = StyleResolver::new(&theme);

            let banners: [Banner; 4] = [
                Banner::info("A new version is available."),
                Banner::success("Your settings were saved.").with_title("Saved"),
                Banner::warning("Low disk space (1.2 GB left)."),
                Banner::error("Connection lost. Retrying…").with_title("Error"),
            ];
            for banner in banners {
                // Size the cell to the banner's natural height so titled (two-line)
                // banners aren't clipped.
                let h = banner.measure_height(list, &style, 300.0);
                let r = flow.cell(list, "", 300.0, h);
                banner.draw(r, list, &style);
            }

            // Inline toast stack: a faint backdrop stands in for the screen, and
            // a transform maps the stack's (0,0) corner origin into the cell.
            let r = flow.cell(list, "Toast stack (top-right)", 320.0, 250.0);
            list.quad(r.x, r.y, r.width, r.height, [0.09, 0.10, 0.13, 1.0]);
            list.rect_outline(r, 1.0, [0.25, 0.28, 0.34, 1.0]);
            let mut stack = ToastStack::new()
                .with_corner(Corner::TopRight)
                .with_width(232.0)
                .with_margin(10.0);
            stack.push(Toast::new(Severity::Success, "Settings saved").with_title("Saved"));
            stack.push(Toast::new(Severity::Info, "New update available (v1.2)"));
            stack.push(Toast::new(Severity::Warning, "Low disk space"));
            list.push_transform();
            list.translate(r.x, r.y);
            stack.draw(r.width, r.height, list, &style);
            list.pop_transform();
        }

        // --- Toolbar ---------------------------------------------------------
        flow.section(list, "Toolbar — 03-toolbar.html key states");
        {
            use wgpu_gameui::render::PhosphorIcon;
            use wgpu_gameui::{DragCapture, Icon, Toolbar, ToolbarEdge, ToolbarItem, ToolbarState};

            let state_item = [ToolbarItem::tool(
                1,
                Icon::new(PhosphorIcon::ArrowClockwise),
                "Move",
                "W",
            )];
            let state_toolbar = Toolbar::new(&state_item);
            let state_width = 72.0;
            let state_height =
                state_toolbar.preferred_cross(theme.toolbar_button_size, theme.toolbar_padding);

            // Render the handoff's interaction states side by side. Each short
            // rail is deliberately wider than its natural content so the
            // trailing overflow control does not displace the key under test.
            for (index, label) in ["Idle", "Hover", "Pressed", "Latched"].iter().enumerate() {
                let r = flow.cell(list, label, state_width, state_height);
                let mut state = ToolbarState::new(ToolbarEdge::Top);
                if index == 3 {
                    state.active_tool = Some(1);
                }
                let mut state_input = InputState::default();
                if index == 1 || index == 2 {
                    state_input.mouse_x = r.x + 25.0;
                    state_input.mouse_y = r.y + state_height * 0.5;
                    state_input.mouse_down = index == 2;
                }
                let mut capture = DragCapture::new();
                let mut tctx =
                    DrawContext::new(list, &mut focus, &theme, &state_input, W as f32, 600.0);
                state_toolbar.draw(
                    r,
                    &mut state,
                    &mut capture,
                    0x7A00 + index as u64,
                    &mut tctx,
                );
            }
        }

        flow.section(list, "Toolbar — docked rails from 03-toolbar.html");
        {
            use wgpu_gameui::render::PhosphorIcon;
            use wgpu_gameui::{DragCapture, Icon, Toolbar, ToolbarEdge, ToolbarItem, ToolbarState};

            let items: Vec<ToolbarItem<'_>> = vec![
                ToolbarItem::tool(1, Icon::new(PhosphorIcon::Diamond), "Select", "Q"),
                ToolbarItem::tool(2, Icon::new(PhosphorIcon::ArrowClockwise), "Move", "W"),
                ToolbarItem::separator(),
                ToolbarItem::tool(3, Icon::new(PhosphorIcon::Cube), "Box", "B"),
                ToolbarItem::tool(4, Icon::new(PhosphorIcon::PaintBrush), "Paint", "P"),
                ToolbarItem::toggle(5, Icon::new(PhosphorIcon::Eraser), "Snap", "Shift+G"),
            ];
            let mut state = ToolbarState::new(ToolbarEdge::Left);
            state.active_tool = Some(2);
            state.active_toggles.push(5);
            let mut capture = DragCapture::new();
            let toolbar = Toolbar::new(&items);
            let btn_size = theme.toolbar_button_size;
            let pad = theme.toolbar_padding;
            let cross = toolbar.preferred_cross(btn_size, pad);
            let vertical_extent =
                toolbar.preferred_extent_for_edge(btn_size, pad, ToolbarEdge::Left);
            let horizontal_extent =
                toolbar.preferred_extent_for_edge(btn_size, pad, ToolbarEdge::Top);

            // Vertical toolbar.
            let r = flow.cell(
                list,
                "Docked left — Move latched, Snap held",
                cross,
                vertical_extent,
            );
            let mut tctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            toolbar.draw(r, &mut state, &mut capture, 0x7B01, &mut tctx);

            // Horizontal toolbar.
            let r2 = flow.cell(list, "Docked top — same tools", horizontal_extent, cross);
            state.edge = ToolbarEdge::Top;
            let mut tctx2 = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            toolbar.draw(r2, &mut state, &mut capture, 0x7B02, &mut tctx2);
        }

        // --- App shell -------------------------------------------------------
        flow.section(list, "App shell (mini layout)");
        {
            use wgpu_gameui::{AppShell, DockPanelState, ToolbarEdge, ToolbarState};

            let left = DockPanelState::new(60.0).with_range(40.0, 120.0);
            let right = DockPanelState::new(70.0).with_range(40.0, 120.0);
            let bottom = DockPanelState::new(40.0).with_range(30.0, 80.0);
            let toolbar = ToolbarState::new(ToolbarEdge::Left);

            let shell = AppShell::new()
                .with_menu_bar()
                .with_doc_tabs()
                .with_status_bar()
                .with_toolbar();

            let r = flow.cell(list, "Full shell layout", 360.0, 240.0);
            let s = StyleResolver::new(&theme);
            let layout = shell.layout(
                r,
                Some(&left),
                Some(&right),
                Some(&bottom),
                Some(&toolbar),
                &s,
            );

            // Draw zone outlines with labels to visualize the layout.
            let dim = [0.3, 0.35, 0.45, 1.0];
            let accent = theme.accent;
            let draw_zone = |list: &mut DrawList, rect: Rect, label: &str, color: [f32; 4]| {
                list.rounded_rect_outline(rect, 1.0, 1.0, color);
                if rect.width > 30.0 && rect.height > 12.0 {
                    list.text(
                        TextBlock::new(label, rect.x + 3.0, rect.y + 2.0)
                            .with_size(8.0)
                            .with_color_f32(color),
                    );
                }
            };

            if let Some(r) = layout.menu_bar {
                draw_zone(list, r, "menu", dim);
            }
            if let Some(r) = layout.doc_tabs {
                draw_zone(list, r, "tabs", dim);
            }
            if let Some(r) = layout.left_dock {
                draw_zone(list, r, "L dock", dim);
            }
            if let Some(r) = layout.left_splitter {
                list.quad(r.x, r.y, r.width, r.height, [0.5, 0.5, 0.5, 0.3]);
            }
            if let Some(r) = layout.right_dock {
                draw_zone(list, r, "R dock", dim);
            }
            if let Some(r) = layout.right_splitter {
                list.quad(r.x, r.y, r.width, r.height, [0.5, 0.5, 0.5, 0.3]);
            }
            if let Some(r) = layout.bottom_dock {
                draw_zone(list, r, "B dock", dim);
            }
            if let Some(r) = layout.bottom_splitter {
                list.quad(r.x, r.y, r.width, r.height, [0.5, 0.5, 0.5, 0.3]);
            }
            if let Some(r) = layout.toolbar {
                draw_zone(list, r, "toolbar", accent);
            }
            draw_zone(list, layout.viewport, "viewport", accent);
            if let Some(r) = layout.status_bar {
                draw_zone(list, r, "status", dim);
            }
        }

        // --- Dock panel -----------------------------------------------------
        flow.section(list, "DockPanel — 04-dock-panel.html tabs");
        {
            use wgpu_gameui::{DockPanel, DockPanelState, DockSide, DockTab as DPanelTab};

            // These are the handoff's sidebar-tab states, not generic Tabs or
            // editor DocTabs. A synthetic pointer keeps Hover visible in the
            // static gallery alongside Active and Idle.
            let tabs = vec![
                DPanelTab { label: "Active" },
                DPanelTab { label: "Hover" },
                DPanelTab { label: "Idle" },
            ];
            let mut state = DockPanelState::new(240.0);
            state.active_tab = 0;
            let r = flow.cell(list, "Active / Hover / Idle (closable)", 240.0, 130.0);
            let mut dock_input = InputState::default();
            dock_input.mouse_x = r.x + 120.0;
            dock_input.mouse_y = r.y + theme.dock_tab_height * 0.5;
            let mut dctx = DrawContext::new(list, &mut focus, &theme, &dock_input, W as f32, 600.0);
            let out = DockPanel::new(DockSide::Left, &tabs)
                .closable()
                .draw(r, &mut state, &mut dctx);

            // Draw placeholder content in the body rect.
            let body = out.body;
            let bd = dctx.styles().color(StyleKey::TextDim);
            dctx.draw_list.text(
                TextBlock::new("(body area)", body.x + 8.0, body.y + 8.0)
                    .with_size(10.0)
                    .with_color_f32(bd),
            );
        }

        flow.section(list, "Window — movable, closable, optional resize grip");
        {
            use wgpu_gameui::{DragCapture, Window, WindowState};

            // Each window sits inset in its cell so its elevation shadow reads.
            // A synthetic pointer on the first window's close key shows the
            // key's hover state; the second is fixed-size without a close key.
            let label = |dctx: &mut DrawContext, body: Rect, text: &str| {
                let bd = dctx.styles().color(StyleKey::TextDim);
                dctx.draw_list.text(
                    TextBlock::new(text, body.x + 8.0, body.y + 8.0)
                        .with_size(10.0)
                        .with_color_f32(bd),
                );
            };
            let r = flow.cell(list, "Resizable · close key hovered", 260.0, 170.0);
            let mut state = WindowState::new(Rect::new(r.x + 16.0, r.y + 8.0, 228.0, 140.0));
            let key = Window::close_rect(
                state.rect,
                theme.window_title_height,
                theme.chrome.window.surface.border_widths.top,
            );
            let mut window_input = InputState::default();
            window_input.mouse_x = key.x + key.width * 0.5;
            window_input.mouse_y = key.y + key.height * 0.5;
            let mut dctx =
                DrawContext::new(list, &mut focus, &theme, &window_input, W as f32, 600.0);
            let out = Window::new(9_100, "Generate recolor mask")
                .resizable(true)
                .draw(&mut state, &mut DragCapture::new(), &mut dctx);
            label(&mut dctx, out.body, "(body area)");

            let r = flow.cell(list, "Fixed size · not closable", 260.0, 170.0);
            let mut state = WindowState::new(Rect::new(r.x + 16.0, r.y + 8.0, 228.0, 140.0));
            let idle = InputState::default();
            let mut dctx = DrawContext::new(list, &mut focus, &theme, &idle, W as f32, 600.0);
            let out = Window::new(9_102, "Project").closable(false).draw(
                &mut state,
                &mut DragCapture::new(),
                &mut dctx,
            );
            label(&mut dctx, out.body, "(body area)");
        }

        // --- Backdrop blur (UiBlur) ----------------------------------------
        // Reserve a cell; the blur samples an app-provided "scene" texture into
        // this region (in the encoder below) and a crisp panel is drawn on top.
        flow.section(list, "Backdrop blur (UiBlur)");
        {
            let r = flow.cell(list, "Frosted-glass menu backdrop", 360.0, 150.0);
            flow.reserve(150.0);
            blur_rect = r;
        }

        // --- Hover animation (easing) --------------------------------------
        // The animation system eases a widget's color from its idle value toward
        // its hover value over `theme.animation_duration`. A static PNG has no
        // time axis, so we sample the *same* ease-out curve at five linear points
        // t ∈ {0, .25, .5, .75, 1} and paint a Button at each step.
        //
        // The endpoints here are exaggerated — idle slate (`button`) → bright
        // `accent` — *on purpose*: the real default hover delta (`button` →
        // `button_hover`) is only ~0.04/channel and reads as flat gray at this
        // size. With a high-contrast pair the ease-out shape is legible: the
        // steps bunch toward the bright end (fast start, slow finish). Drawn via
        // the public `ease`/`lerp_color` through a per-button `StyleOverlay`.
        flow.section(list, "Hover animation (ease-out curve)");
        for &t in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let eased = ease(Easing::EaseOut, t);
            let fill = lerp_color(theme.button, theme.accent, eased);
            let mut ramp = StyleOverlay::new();
            ramp.set_color(StyleKey::Button, fill);
            let label = format!("t={t:.2}");
            let r = flow.cell(list, &label, 90.0, 32.0);
            let mut rctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
                .with_style(&ramp);
            Button::new(&label).draw(r, &mut rctx);
        }

        // Tooltip target last: its popup floats down-and-right into the empty
        // headroom below, overlapping no other widget.
        let r = flow.cell(list, "Tooltip target", 120.0, 24.0);
        list.rounded_rect(r, 4.0, [0.25, 0.30, 0.40, 1.0]);
        list.text(
            TextBlock::new("Hover me", r.x + 8.0, r.y + 5.0)
                .with_size(12.0)
                .with_color(200, 210, 230),
        );
        tooltip_rect = r;

        // Leave headroom below the last row for the tooltip popup. Finalize the
        // section map before moving it out for per-section image generation.
        content_bottom = flow.bottom() + 70.0;
        flow.finish_sections();
        gallery_sections = flow.sections;
        gallery_components = flow.components;
    }

    // Size the target to the laid-out content first, so the tooltip layer
    // knows the real screen height (it flips the popup up/left near the edges).
    let h = (content_bottom.ceil() as u32).max(64);
    assert!(
        h <= max_texture_dimension_2d,
        "gallery is {h}px tall, over this adapter's {max_texture_dimension_2d}px texture limit"
    );

    // Cursor-anchored context menu (modal layer: outside clicks close without
    // reaching the base UI).
    {
        let styles = StyleResolver::new(&theme);
        let viewport = Rect::new(0.0, 0.0, W as f32, h as f32);
        let popup = context_state.push_open_layer(&mut layers, &context_menu, &styles, viewport);
        context_state.draw_open_layer(
            &mut layers,
            popup,
            &context_menu,
            &styles,
            &InputState::default(),
            &mut focus,
            viewport,
        );
    }

    // Floating dropdown list (Popup layer above the base content).
    {
        let popup = dropdowns.push_open_layer(&mut layers);
        dropdowns.draw_open_layer(
            &mut layers,
            popup,
            &StyleResolver::new(&theme),
            &InputState::default(),
        );
    }

    // Menubar chain: the viewport blocker plus the open column's own popup layer,
    // pushed above the base content exactly as a host would.
    {
        let slots = menu_state.push_open_layers(&mut layers);
        let mut interactions = InteractionScene::new();
        let mut menu_env = MenuDrawEnv {
            theme: &theme,
            style: None,
            input: &menu_input,
            focus: &mut focus,
            interactions: &mut interactions,
            animations: None,
            cursor: None,
            screen_width: W as f32,
            screen_height: h as f32,
        };
        menu_state.draw_open_layers(&mut layers, slots, MENUS, &mut menu_env);
        menu_state.end_frame(&mut focus);
    }

    // Tooltip layer, hovering the reserved target.
    {
        let mut tooltip = TooltipLayer::new();
        tooltip.register(tooltip_rect, TooltipContent::text("This is a tooltip!"));
        let tip_input = InputState {
            mouse_x: tooltip_rect.x + tooltip_rect.width / 2.0,
            mouse_y: tooltip_rect.y + tooltip_rect.height / 2.0,
            ..Default::default()
        };
        tooltip.tick(999.0, &tip_input);
        tooltip.draw_into_layers(
            &mut layers,
            &tip_input,
            &StyleResolver::new(&theme),
            W as f32,
            h as f32,
        );
    }

    // Dump the layout report alongside the PNG. The gallery is the largest real
    // corpus of this crate's own widgets, so it is also the best check on
    // whether the debug lints are useful or merely noisy — read the .txt when
    // you change them.
    {
        let screen = Rect::new(0.0, 0.0, W as f32, h as f32);
        let report = DebugReport::measured_layers(&mut layers, screen);
        std::fs::create_dir_all("test_output").unwrap();
        std::fs::write("test_output/widget_gallery.debug.txt", report.to_text())
            .expect("write debug report");
        std::fs::write("test_output/widget_gallery.debug.json", report.to_json())
            .expect("write debug json");
        eprintln!(
            "wrote test_output/widget_gallery.debug.txt — {} nodes, {} problems ({} inferred names)",
            report.nodes.len(),
            report.problems.len(),
            report.inferred_count()
        );
        let mut by_code: std::collections::BTreeMap<&str, usize> = Default::default();
        for p in report.problems() {
            *by_code.entry(p.code()).or_default() += 1;
        }
        for (code, n) in by_code {
            eprintln!("  {code}: {n}");
        }
    }

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gallery target"),
        size: wgpu::Extent3d {
            width: W,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // --- Backdrop-blur "scene" -------------------------------------------------
    // Stand in for the app's rendered game: a full-target texture with vivid
    // colored stripes + text inside the reserved blur cell. `blur_backdrop` then
    // samples this and writes a blurred copy into the cell, with a crisp panel on
    // top — exactly the pause-menu flow (render scene → blur → draw UI).
    let scene_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blur scene"),
        size: wgpu::Extent3d {
            width: W,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let scene_view = scene_tex.create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut scene_list = DrawList::new();
        // Vivid vertical stripes across the blur cell — soft blobs once blurred.
        let stripe_colors = [
            [0.92, 0.26, 0.30, 1.0],
            [0.96, 0.66, 0.18, 1.0],
            [0.30, 0.78, 0.40, 1.0],
            [0.22, 0.58, 0.95, 1.0],
            [0.60, 0.36, 0.90, 1.0],
            [0.95, 0.40, 0.70, 1.0],
        ];
        let n = stripe_colors.len() as f32;
        let sw = blur_rect.width / n;
        for (i, c) in stripe_colors.iter().enumerate() {
            scene_list.quad(
                blur_rect.x + i as f32 * sw,
                blur_rect.y,
                sw,
                blur_rect.height,
                *c,
            );
        }
        // Big bright text to show the blur softening edges.
        scene_list.text(
            TextBlock::new("GAME WORLD", blur_rect.x + 24.0, blur_rect.y + 52.0)
                .with_size(44.0)
                .with_color(255, 255, 255),
        );
        let mut scene_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene encoder"),
        });
        // This scene list is its own submission (submitted at `queue.submit` just
        // below), so it is its own frame for the renderer's arenas.
        ui.begin_frame();
        {
            scene_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.06,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        ui.render(
            &device,
            &queue,
            &mut scene_enc,
            &scene_view,
            (W, h),
            1.0,
            &scene_list,
        );
        queue.submit(Some(scene_enc.finish()));
    }

    // bytes_per_row must be 256-aligned for wgpu copy.
    let row_stride = W * 4;
    let bytes_per_row = (row_stride + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("encoder"),
    });
    // Everything from here to the `queue.submit` at the bottom — the widget stack,
    // the blurred backdrop and the PAUSED panel — is one submission, hence one
    // frame (see `UiRenderer::begin_frame`).
    ui.begin_frame();
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(ui.clear_color(theme.background)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    ui.render_layers(&device, &queue, &mut encoder, &view, (W, h), 1.0, &layers);

    // Blur the scene into the reserved cell (a darkening scrim tint), then draw a
    // crisp "PAUSED" panel on top — UI rendered after the blur sits sharp.
    ui.blur_backdrop(
        &device,
        &queue,
        &mut encoder,
        &view,
        &Backdrop {
            view: &scene_view,
            size: (W, h),
            encoding: ColorEncoding::Srgb,
        },
        blur_rect,
        (W, h),
        1.0,
        &BlurParams {
            radius: 9.0,
            downsample: 2,
            tint: [0.62, 0.64, 0.72, 1.0],
        },
    );
    {
        let mut panel_list = DrawList::new();
        let pw = 200.0;
        let ph = 84.0;
        let px = blur_rect.x + (blur_rect.width - pw) / 2.0;
        let py = blur_rect.y + (blur_rect.height - ph) / 2.0;
        let panel = Rect::new(px, py, pw, ph);
        // One combined SDF instance: fill + border share a single radius, so the
        // border can't drift from the fill (and it's one instance, not two).
        panel_list.chrome_rect(
            panel,
            8.0,
            1.0,
            [0.12, 0.13, 0.17, 0.92],
            [0.40, 0.44, 0.52, 1.0],
        );
        panel_list.text(
            TextBlock::new("PAUSED", px + 20.0, py + 16.0)
                .with_size(22.0)
                .with_color(240, 244, 255),
        );
        let btn = Rect::new(px + 20.0, py + 50.0, pw - 40.0, 22.0);
        panel_list.rounded_rect(btn, 4.0, [0.22, 0.50, 0.85, 1.0]);
        panel_list.text(
            TextBlock::new("Resume", btn.x + 12.0, btn.y + 4.0)
                .with_size(13.0)
                .with_color(255, 255, 255),
        );
        ui.render(
            &device,
            &queue,
            &mut encoder,
            &view,
            (W, h),
            1.0,
            &panel_list,
        );
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: h,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    // De-pad: the GPU buffer rows are 256-aligned (`bytes_per_row`), but a
    // tightly-packed RGBA image expects `row_stride` (W*4) per row. Copy each
    // row's real bytes, dropping the alignment padding — otherwise every row
    // drifts by the padding amount and the image shears diagonally.
    let row_stride = (W * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * h as usize);
    for row in 0..h as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }

    std::fs::create_dir_all("test_output").unwrap();
    let img = image::RgbaImage::from_raw(W, h, pixels).expect("image from raw");
    img.save("test_output/widget_gallery.png")
        .expect("save png");
    eprintln!("wrote test_output/widget_gallery.png ({W}x{h})");
    save_gallery_images(&img, &gallery_sections, &gallery_components);

    // Sanity: at least some pixels are not the theme clear color.
    let [cr, cg, cb, _] = wgpu_gameui::color::to_rgba8(theme.background);
    let clear = [cr, cg, cb];
    let drew = img.pixels().any(|p| {
        let d = (p.0[0] as i32 - clear[0] as i32).abs()
            + (p.0[1] as i32 - clear[1] as i32).abs()
            + (p.0[2] as i32 - clear[2] as i32).abs();
        d > 30
    });
    assert!(
        drew,
        "no widget pixels rendered — pipeline produced an empty frame"
    );
}
