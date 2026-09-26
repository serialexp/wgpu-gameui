//! Headless offscreen render of all widgets → PNGs for visual inspection.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test widget_gallery -- --ignored --nocapture
//! ```
//! Writes one focused image per section under `test_output/widget_gallery/`,
//! one image per labeled component preview under
//! `test_output/widget_gallery/components/`, and
//! `test_output/widget_gallery/index.html` to browse them all.
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
    GradientStop, Group, HitZone, Hsva, Image, ImageFit, InputState, InteractionScene, Key,
    LayerStack, List, ListItem, ListState, MeasureBuffer, MeasureConstraints, MeasuredChild, Menu,
    MenuBar, MenuBarState, MenuDrawEnv, MenuItem, NavInput, NumberInput, Pager, Popover,
    PopoverSide, PressState, Pressable, ProgressBar, ProgressFill, RadioGroup, ScrollState,
    ScrollView, SelectionMode, Separator, Severity, Slider, Splitter, StatusCell, StyleKey,
    StyleOverlay, StyleResolver, Table, TableCell, TableColumn, Tabs, TextAlign, TextBlock,
    TextDirection, TextInput, TextSpan, Theme, Toast, ToastStack, Toggle, TooltipContent,
    TooltipLayer, TreeAction, TreeNode, TreeState, UiContext, UiRenderer, UiState, Underline,
    VectorField, VectorScrub, ease, lerp_color,
};
use wgpu_gameui::{
    AlertDialog, AlertDialogState, ConfirmDialog, ConfirmDialogState, Modal, ModalState,
    PromptDialog, PromptDialogState, Sheet, SheetAction,
};
use wgpu_gameui::{
    BADGE_HEIGHT, Badge, BadgeTone, BarSegment, COUNT_BUBBLE_HEIGHT, CountBubble, DROP_ZONE_SIZE,
    DropZone, EmptyState, FieldLabel, Ink, MeterFill, Panel, Placeholder, SPAN_TABS_HEIGHT,
    STATUS_BAR_HEIGHT, STATUS_ICON_INLINE_SIZE, STATUS_ICON_SIZE, SpanTab, SpanTabs, Status,
    StatusIcon, StatusPart, StatusToggle, StatusZone, TextSize, WELL_CHIP_HEIGHT, Waffle,
    WaffleCategory, WaffleFill, WellChip, WellChipPart, ZonedStatusBar, chip, dots,
    draw_combo_trigger, draw_curve_editor, draw_doc_tabs, draw_gradient_ramp, draw_popover_frame,
    draw_status_bar, draw_tag_input, inline_meter, keycap, place_popover, skeleton, spinner,
    stacked_bar, toolbar_band,
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

/// A stage for a modal to cover, like the Forge dialogs card: a viewport-blue
/// ground, its name in the corner, and a card of content, so the backdrop's
/// dimming shows.
fn dialog_stage(list: &mut DrawList, s: &StyleResolver, r: Rect, name: &str) {
    list.paint_quad_background(
        r,
        wgpu_gameui::Background::LinearGradient {
            start: [0.17, 0.35, 0.4, 1.0],
            end: [0.05, 0.11, 0.14, 1.0],
            axis: wgpu_gameui::GradientAxis::Vertical,
        },
        wgpu_gameui::CornerRadii::uniform(2.0),
    );
    list.text(
        s.mono_block(
            name.to_uppercase(),
            r.x + 9.0,
            r.y + 7.0,
            TextSize::Caption,
            Ink::Glyph,
        )
        .with_color_f32([1.0, 1.0, 1.0, 0.45]),
    );
    let card = Rect::new(r.x + 12.0, r.y + 26.0, (r.width * 0.45).round(), 70.0);
    let body = Panel::new().padding(10.0).draw(card, list, s);
    Placeholder::text(3).draw(body, list, s);
}

/// Reserve a cell for a free-standing `w`×`h` sheet that takes in its drop
/// shadow, so the shadow doesn't fall on the neighbouring cells; returns the
/// sheet's rect inside the cell.
fn sheet_cell(
    flow: &mut Flow,
    list: &mut DrawList,
    s: &StyleResolver,
    label: &str,
    w: f32,
    h: f32,
) -> Rect {
    let sheet = Rect::new(0.0, 0.0, w, h);
    let ink = s
        .sheet()
        .shadows
        .iter()
        .fold(sheet, |area, shadow| area.union(shadow.ink_rect(sheet)));
    let cell = flow.cell(list, label, ink.width, ink.height);
    Rect::new(cell.x - ink.x, cell.y - ink.y, w, h)
}

const W: u32 = 800;
const LABEL_H: f32 = 16.0;
const LABEL_SIZE: f32 = 11.0;
/// Rough advance width per character at `LABEL_SIZE`, used so a long label
/// reserves enough horizontal room to not collide with the next cell.
const LABEL_CHAR_W: f32 = 6.0;

/// A category of the gallery catalogue. The component categories are the
/// Forge Design System's groups (its `components/<group>/` folders);
/// `Foundations` holds type, text and icons, and `Engine` holds rendering
/// and layout machinery Forge has no component for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Category {
    Foundations,
    Chrome,
    Data,
    Dialogs,
    Editors,
    Feedback,
    Forms,
    Inspector,
    Keys,
    Layout,
    Mobile,
    Palette,
    Settings,
    Sidebar,
    Windows,
    Engine,
}

impl Category {
    /// Every category, in index-page order.
    const ALL: [Category; 16] = [
        Category::Foundations,
        Category::Chrome,
        Category::Data,
        Category::Dialogs,
        Category::Editors,
        Category::Feedback,
        Category::Forms,
        Category::Inspector,
        Category::Keys,
        Category::Layout,
        Category::Mobile,
        Category::Palette,
        Category::Settings,
        Category::Sidebar,
        Category::Windows,
        Category::Engine,
    ];

    /// The category's name: its directory under
    /// `test_output/widget_gallery/`, and (for the component categories)
    /// Forge's folder name.
    fn name(self) -> &'static str {
        match self {
            Category::Foundations => "foundations",
            Category::Chrome => "chrome",
            Category::Data => "data",
            Category::Dialogs => "dialogs",
            Category::Editors => "editors",
            Category::Feedback => "feedback",
            Category::Forms => "forms",
            Category::Inspector => "inspector",
            Category::Keys => "keys",
            Category::Layout => "layout",
            Category::Mobile => "mobile",
            Category::Palette => "palette",
            Category::Settings => "settings",
            Category::Sidebar => "sidebar",
            Category::Windows => "windows",
            Category::Engine => "engine",
        }
    }
}

/// The Forge Design System's components, by category, as listed in its
/// `_ds_manifest.json` (Claude Design project "Forge Design System",
/// 2026-09-26). The index page lists the ones the gallery has no section for
/// as missing. Update it when Forge gains or renames a component.
const FORGE_COMPONENTS: &[(Category, &[&str])] = &[
    (
        Category::Chrome,
        &[
            "Console",
            "ContextMenu",
            "DockPanel",
            "MenuBar",
            "MenuSheet",
            "StatusBar",
        ],
    ),
    (
        Category::Data,
        &[
            "AssetGrid",
            "CountBubble",
            "DragList",
            "DropZone",
            "ListView",
            "Table",
            "Thumb",
            "Tree",
            "Waffle",
        ],
    ),
    (
        Category::Dialogs,
        &["AlertDialog", "ConfirmDialog", "PromptDialog"],
    ),
    (
        Category::Editors,
        &["ColorPicker", "CurveEditor", "GradientRamp"],
    ),
    (
        Category::Feedback,
        &[
            "Badge",
            "Banner",
            "BusyDots",
            "EmptyState",
            "Placeholder",
            "Skeleton",
            "Spinner",
            "StatusDot",
            "StatusIcon",
            "Toast",
            "Tooltip",
        ],
    ),
    (
        Category::Forms,
        &[
            "Checkbox",
            "ComboBox",
            "Dropdown",
            "FieldLabel",
            "NumberField",
            "ProgressBar",
            "Radio",
            "SearchField",
            "Slider",
            "Switch",
            "TagInput",
            "TextArea",
            "TextField",
            "VectorField",
        ],
    ),
    (
        Category::Inspector,
        &["FileField", "Inspector", "PropertyGroup", "PropertyRow"],
    ),
    (
        Category::Keys,
        &[
            "Button",
            "FilterChip",
            "IconKey",
            "Key",
            "Keycap",
            "Toolbar",
        ],
    ),
    (
        Category::Layout,
        &[
            "Breadcrumb",
            "DocumentTabs",
            "Group",
            "Modal",
            "Pager",
            "Panel",
            "Popover",
            "ScrollArea",
            "SegmentedTabs",
            "Separator",
            "Sheet",
            "SpanTabs",
            "Splitter",
        ],
    ),
    (Category::Mobile, &["AppBar", "TabBar"]),
    (Category::Palette, &["CommandPalette"]),
    (Category::Settings, &["SettingRow", "SettingsPanel"]),
    (Category::Sidebar, &["DockSection", "DockStack"]),
    (Category::Windows, &["FloatingWindow"]),
];

/// Forge's components in `category`, in Forge's order (empty for
/// `Foundations` and `Engine`).
fn forge_components(category: Category) -> &'static [&'static str] {
    FORGE_COMPONENTS
        .iter()
        .find(|(c, _)| *c == category)
        .map_or(&[], |(_, names)| *names)
}

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
    /// Index into `sections` of the section cells go to.
    current_section: Option<usize>,
    sections: Vec<GallerySection>,
    cells: Vec<GalleryCell>,
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
            cells: Vec::new(),
        }
    }

    /// Break to a new row and draw a section header for one catalogue entry:
    /// `component` in `category`, with an optional one-line `subtitle` (`""`
    /// for none). Use Forge's component name where Forge has the component,
    /// so the index page can tell which Forge components are missing. Each
    /// `(category, component)` gets exactly one section.
    fn section(
        &mut self,
        list: &mut DrawList,
        category: Category,
        component: &'static str,
        subtitle: &'static str,
    ) -> f32 {
        if self.cur_x > self.x0 {
            self.cur_y += self.row_h;
        }
        // Extra gap above a header (except the very first one).
        if self.cur_y > self.y0 {
            self.cur_y += self.row_gap * 1.5;
        }
        self.cur_x = self.x0;
        self.row_h = 0.0;
        let heading = TextBlock::new(component, self.x0, self.cur_y)
            .with_size(15.0)
            .with_color(120, 180, 255);
        let (heading_w, _) = list.measure_block(&heading);
        list.text(heading);
        let tag = if subtitle.is_empty() {
            category.name().to_owned()
        } else {
            format!("{} · {subtitle}", category.name())
        };
        list.text(
            TextBlock::new(tag, self.x0 + heading_w + 10.0, self.cur_y + 3.0)
                .with_size(LABEL_SIZE)
                .with_color(110, 120, 135),
        );
        let section_top = self.cur_y;
        if let Some(previous) = self.sections.last_mut() {
            previous.bottom = section_top - self.row_gap * 1.5;
        }
        self.current_section = Some(self.sections.len());
        self.sections.push(GallerySection {
            category,
            component,
            subtitle,
            top: section_top,
            bottom: section_top,
        });
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
        let section = self
            .current_section
            .expect("gallery cells must belong to a section");
        if !label.is_empty() {
            self.cells
                .push(GalleryCell::new(section, label, content, cell_w));
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
        if let Some(cell) = self.cells.last_mut() {
            cell.rect.height = cell.rect.height.max(LABEL_H + content_h);
        }
    }

    /// The y just below all content drawn so far.
    fn bottom(&self) -> f32 {
        self.cur_y + self.row_h
    }
}

/// One catalogue entry: a component's section of the canvas.
struct GallerySection {
    category: Category,
    component: &'static str,
    subtitle: &'static str,
    top: f32,
    bottom: f32,
}

/// One labeled preview cell inside a section.
struct GalleryCell {
    /// Index of the section the cell belongs to.
    section: usize,
    /// The cell label, as drawn.
    label: String,
    file_stem: String,
    rect: Rect,
}

impl GalleryCell {
    /// `cell_w` is the cell's full width: its content's or its label's,
    /// whichever is wider, so the crop never cuts the label off.
    fn new(section: usize, label: &str, content: Rect, cell_w: f32) -> Self {
        Self {
            section,
            label: label.to_owned(),
            file_stem: file_stem(label),
            rect: Rect::new(
                content.x,
                content.y - LABEL_H,
                cell_w,
                content.height + LABEL_H,
            ),
        }
    }
}

/// A lowercase, dash-separated file name for `title`.
fn file_stem(title: &str) -> String {
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

/// Most canvas rows rendered in one page. Well inside every adapter's texture
/// limit, and pages split between sections, so the gallery can grow without
/// bound as long as no single section is taller than this.
const PAGE_MAX: u32 = 4096;
/// Margin around a section image, in pixels.
const SECTION_MARGIN: u32 = 10;
/// Margin around a cell image, in pixels. Smaller than `SECTION_MARGIN`, so a
/// cell's image fits in its section's page.
const CELL_MARGIN: u32 = 6;

/// One rendered horizontal strip of the canvas, starting `top` rows down.
struct GalleryPage {
    top: u32,
    img: image::RgbaImage,
}

/// Everything `render_gallery_page` draws: the widget layers, plus the
/// backdrop-blur demo (a stand-in game scene blurred into `blur_rect`, with a
/// crisp panel on top).
struct GalleryScene<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    format: wgpu::TextureFormat,
    /// The theme background, as a clear value for this renderer's target.
    clear: wgpu::Color,
    layers: &'a LayerStack,
    backdrop: &'a DrawList,
    blur_rect: Rect,
    panel: &'a DrawList,
}

/// The canvas area a section's image shows, before its margin.
fn section_rect(section: &GallerySection) -> Rect {
    Rect::new(
        20.0,
        section.top,
        (W - 40) as f32,
        section.bottom - section.top,
    )
}

/// The canvas rows a section's image covers, margin included — the same rows
/// `crop_with_margin` cuts.
fn section_rows(section: &GallerySection) -> (u32, u32) {
    let rect = section_rect(section);
    let top = (rect.y.floor().max(0.0) as u32).saturating_sub(SECTION_MARGIN);
    let bottom = rect.bottom().ceil().max(0.0) as u32 + SECTION_MARGIN;
    (top, bottom)
}

/// Split the canvas into pages of at most `PAGE_MAX` rows, each holding whole
/// sections: `(top, bottom)` row ranges, in canvas order.
fn page_spans(sections: &[GallerySection]) -> Vec<(u32, u32)> {
    let mut spans = Vec::new();
    let mut page: Option<(u32, u32)> = None;
    for section in sections {
        let (top, bottom) = section_rows(section);
        assert!(
            bottom - top <= PAGE_MAX,
            "gallery section {}/{} is {}px tall, over the {PAGE_MAX}px page; split it",
            section.category.name(),
            section.component,
            bottom - top
        );
        page = Some(match page {
            Some((page_top, page_bottom)) if bottom - page_top <= PAGE_MAX => {
                (page_top, page_bottom.max(bottom))
            }
            Some(full) => {
                spans.push(full);
                (top, bottom)
            }
            None => (top, bottom),
        });
    }
    spans.extend(page);
    spans
}

/// A `W`-wide render target (or backdrop) texture, `height` rows tall.
fn page_texture(
    scene: &GalleryScene,
    label: &str,
    height: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    scene.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: W,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: scene.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
        view_formats: &[],
    })
}

/// Clear `view` to `color` (the renderer loads, rather than clears, its
/// target).
fn clear_view(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: wgpu::Color) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("gallery clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
}

/// Render canvas rows `top..bottom` through the renderer's view origin and
/// read them back.
fn render_gallery_page(
    ui: &mut UiRenderer,
    scene: &GalleryScene,
    top: u32,
    bottom: u32,
) -> GalleryPage {
    let (device, queue) = (scene.device, scene.queue);
    let h = bottom - top;
    let viewport = (W, h);
    ui.set_view_origin(0.0, top as f32);

    let target = page_texture(scene, "gallery page", h, wgpu::TextureUsages::COPY_SRC);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // The blur demo, when its cell is on this page: render the stand-in scene
    // first. It is its own submission, so its own frame for the renderer's
    // arenas.
    let blur = scene.blur_rect;
    let backdrop = (blur.y < bottom as f32 && blur.bottom() > top as f32).then(|| {
        let tex = page_texture(scene, "blur scene", h, wgpu::TextureUsages::TEXTURE_BINDING);
        let backdrop_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene encoder"),
        });
        ui.begin_frame();
        clear_view(
            &mut encoder,
            &backdrop_view,
            wgpu::Color {
                r: 0.05,
                g: 0.06,
                b: 0.10,
                a: 1.0,
            },
        );
        ui.render(
            device,
            queue,
            &mut encoder,
            &backdrop_view,
            viewport,
            1.0,
            scene.backdrop,
        );
        queue.submit(Some(encoder.finish()));
        backdrop_view
    });

    // bytes_per_row must be 256-aligned for wgpu copy.
    let row_stride = W * 4;
    let bytes_per_row = (row_stride + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gallery readback"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gallery encoder"),
    });
    // Everything from here to the `queue.submit` below — the widget stack, the
    // blurred backdrop and the PAUSED panel — is one submission, hence one
    // frame (see `UiRenderer::begin_frame`).
    ui.begin_frame();
    clear_view(&mut encoder, &view, scene.clear);
    ui.render_layers(
        device,
        queue,
        &mut encoder,
        &view,
        viewport,
        1.0,
        scene.layers,
    );
    if let Some(backdrop_view) = &backdrop {
        // Blur the scene into the reserved cell (a darkening scrim tint), then
        // draw the crisp panel on top.
        ui.blur_backdrop(
            device,
            queue,
            &mut encoder,
            &view,
            &Backdrop {
                view: backdrop_view,
                size: viewport,
                encoding: ColorEncoding::Srgb,
            },
            blur,
            viewport,
            1.0,
            &BlurParams {
                radius: 9.0,
                downsample: 2,
                tint: [0.62, 0.64, 0.72, 1.0],
            },
        );
        ui.render(
            device,
            queue,
            &mut encoder,
            &view,
            viewport,
            1.0,
            scene.panel,
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
    ui.set_view_origin(0.0, 0.0);

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    // De-pad: the GPU buffer rows are 256-aligned (`bytes_per_row`), but a
    // tightly-packed RGBA image expects `row_stride` (W*4) per row. Copy each
    // row's real bytes, dropping the alignment padding — otherwise every row
    // drifts by the padding amount and the image shears diagonally.
    let row_stride = row_stride as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * h as usize);
    for row in 0..h as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }
    let img = image::RgbaImage::from_raw(W, h, pixels).expect("image from raw");
    GalleryPage { top, img }
}

/// Cut canvas `rect`, grown by `margin`, out of the page that holds it.
fn crop_with_margin(pages: &[GalleryPage], rect: Rect, margin: u32) -> image::RgbaImage {
    let left = (rect.x.floor().max(0.0) as u32).saturating_sub(margin);
    let top = (rect.y.floor().max(0.0) as u32).saturating_sub(margin);
    let right = (rect.right().ceil().max(0.0) as u32 + margin).min(W);
    let bottom = rect.bottom().ceil().max(0.0) as u32 + margin;
    assert!(right > left && bottom > top, "gallery crop is empty");
    let page = pages
        .iter()
        .find(|page| page.top <= top && bottom <= page.top + page.img.height())
        .unwrap_or_else(|| panic!("gallery crop rows {top}..{bottom} span two pages"));
    image::imageops::crop_imm(&page.img, left, top - page.top, right - left, bottom - top)
        .to_image()
}

/// Cut the rendered pages into one image per section
/// (`<group>/<Component>.png`) and one per labeled cell
/// (`<group>/<Component>/<label>.png`), and write `index.html` to browse
/// them by group, with Forge's missing components listed.
fn save_gallery_images(pages: &[GalleryPage], sections: &[GallerySection], cells: &[GalleryCell]) {
    let output_dir = "test_output/widget_gallery";
    // Start from an empty directory so a renamed or removed section or cell
    // never leaves a stale PNG behind.
    match std::fs::remove_dir_all(output_dir) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("wipe {output_dir}: {err}"),
    }

    let mut seen = std::collections::HashSet::with_capacity(sections.len());
    let mut entries = Vec::with_capacity(sections.len());
    for section in sections {
        let (category, component) = (section.category.name(), section.component);
        assert!(
            section.bottom > section.top,
            "gallery section {category}/{component} is empty"
        );
        assert!(
            seen.insert((section.category, component)),
            "two gallery sections are both {category}/{component}; merge them"
        );
        assert!(
            !component.is_empty() && component.chars().all(|c| c.is_ascii_alphanumeric()),
            "gallery component {component:?} must be a PascalCase name"
        );
        std::fs::create_dir_all(format!("{output_dir}/{category}/{component}"))
            .expect("create gallery image directories");
        let section_img = crop_with_margin(pages, section_rect(section), SECTION_MARGIN);
        let file = format!("{category}/{component}.png");
        section_img
            .save(format!("{output_dir}/{file}"))
            .expect("save gallery section PNG");
        entries.push(IndexEntry {
            section,
            file,
            size: section_img.dimensions(),
            cells: Vec::new(),
        });
    }

    // Per section, a repeated label gets a `-2`, `-3`, … suffix.
    let mut stem_counts = std::collections::HashMap::new();
    for cell in cells {
        let entry = &mut entries[cell.section];
        let count = stem_counts
            .entry((cell.section, cell.file_stem.as_str()))
            .or_insert(0usize);
        *count += 1;
        let stem = if *count == 1 {
            cell.file_stem.clone()
        } else {
            format!("{}-{}", cell.file_stem, *count)
        };
        let file = format!(
            "{}/{}/{stem}.png",
            entry.section.category.name(),
            entry.section.component
        );
        let cell_img = crop_with_margin(pages, cell.rect, CELL_MARGIN);
        cell_img
            .save(format!("{output_dir}/{file}"))
            .expect("save gallery cell PNG");
        entry.cells.push(IndexCell {
            label: &cell.label,
            file,
            size: cell_img.dimensions(),
        });
    }

    let index = format!("{output_dir}/index.html");
    std::fs::write(&index, gallery_index_html(&entries)).expect("write gallery index");
    let covered = FORGE_COMPONENTS
        .iter()
        .flat_map(|(category, names)| names.iter().map(move |name| (*category, *name)))
        .filter(|key| seen.contains(key))
        .count();
    let forge_total: usize = FORGE_COMPONENTS.iter().map(|(_, names)| names.len()).sum();
    eprintln!(
        "wrote {} section and {} cell images ({covered} of {forge_total} Forge components), \
         browse them at {index}",
        sections.len(),
        cells.len()
    );
}

/// One section on the gallery index page, with its images.
struct IndexEntry<'a> {
    section: &'a GallerySection,
    /// Section image path, relative to the index page.
    file: String,
    /// Section image size in pixels.
    size: (u32, u32),
    cells: Vec<IndexCell<'a>>,
}

/// One cell image on the gallery index page.
struct IndexCell<'a> {
    label: &'a str,
    /// Path relative to the index page.
    file: String,
    /// Image size in pixels.
    size: (u32, u32),
}

/// Escape text for HTML element content and quoted attribute values.
fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// A self-contained page for browsing the gallery. On the left, a
/// filterable list of the groups and their components, with Forge's missing
/// ones greyed out; on the right, each component's section image with its
/// cell images (folded), at a chosen pixel zoom.
fn gallery_index_html(entries: &[IndexEntry]) -> String {
    use std::fmt::Write as _;

    let mut nav = String::new();
    let mut body = String::new();
    let mut missing_total = Vec::new();
    for category in Category::ALL {
        let g = category.name();
        let forge = forge_components(category);
        let mut in_group: Vec<&IndexEntry> = entries
            .iter()
            .filter(|e| e.section.category == category)
            .collect();
        // Forge's components in Forge's order, then gameui's own by name.
        in_group.sort_by_key(|e| {
            let forge_index = forge.iter().position(|name| *name == e.section.component);
            (forge_index.unwrap_or(usize::MAX), e.section.component)
        });
        let missing: Vec<&str> = forge
            .iter()
            .copied()
            .filter(|name| !in_group.iter().any(|e| e.section.component == *name))
            .collect();
        if in_group.is_empty() && missing.is_empty() {
            continue;
        }
        missing_total.extend(missing.iter().map(|name| (g, *name)));

        let count = if forge.is_empty() {
            in_group.len().to_string()
        } else {
            format!("{} / {}", forge.len() - missing.len(), forge.len())
        };
        writeln!(
            nav,
            r##"<div class="group"><h3><a href="#{g}">{g}</a> <span class="count">{count}</span></h3><ul>"##
        )
        .unwrap();
        writeln!(body, r#"<div class="group" id="{g}"><h2>{g}</h2>"#).unwrap();
        for entry in &in_group {
            let section = entry.section;
            let c = section.component;
            let id = format!("{g}-{c}");
            let own = !forge.is_empty() && !forge.contains(&c);
            let tag = if own {
                r#" <span class="tag">not in Forge</span>"#
            } else {
                ""
            };
            writeln!(nav, r##"<li><a href="#{id}">{c}</a>{tag}</li>"##).unwrap();

            let mut search = format!("{g} {c} {}", section.subtitle);
            for cell in &entry.cells {
                search.push(' ');
                search.push_str(cell.label);
            }
            let search = html_escape(&search);
            let subtitle = html_escape(section.subtitle);
            let file = &entry.file;
            let (w, h) = entry.size;
            writeln!(
                body,
                r##"<section id="{id}" data-search="{search}">
<h3><a href="#{id}">{c}</a> <span class="subtitle">{subtitle}</span>{tag}</h3>
<img src="{file}" style="--w:{w};--h:{h}" alt="{c}">"##
            )
            .unwrap();
            if !entry.cells.is_empty() {
                writeln!(
                    body,
                    "<details><summary>{} cells</summary><div class=\"grid\">",
                    entry.cells.len()
                )
                .unwrap();
                for cell in &entry.cells {
                    let label = html_escape(cell.label);
                    let file = &cell.file;
                    let (w, h) = cell.size;
                    writeln!(
                        body,
                        r#"<figure><img src="{file}" style="--w:{w};--h:{h}" alt="{label}"><figcaption>{label}</figcaption></figure>"#
                    )
                    .unwrap();
                }
                body.push_str("</div></details>\n");
            }
            body.push_str("</section>\n");
        }
        for name in &missing {
            writeln!(nav, r#"<li class="missing">{name}</li>"#).unwrap();
        }
        if !missing.is_empty() {
            writeln!(
                body,
                r#"<p class="missing">Missing Forge components: {}</p>"#,
                missing.join(", ")
            )
            .unwrap();
        }
        nav.push_str("</ul></div>\n");
        body.push_str("</div>\n");
    }

    let forge_total: usize = FORGE_COMPONENTS.iter().map(|(_, names)| names.len()).sum();
    let covered = forge_total - missing_total.len();
    let missing_list = missing_total
        .iter()
        .map(|(g, name)| format!(r##"<a href="#{g}">{g}</a>/{name}"##))
        .collect::<Vec<_>>()
        .join(", ");
    let summary = format!(
        r#"<p class="summary">Forge coverage: <b>{covered} of {forge_total}</b> components. Missing: {missing_list}</p>"#
    );

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>wgpu-gameui widget gallery</title>
<style>
:root {{ --zoom: 1; color-scheme: dark; }}
body {{ margin: 0; display: flex; background: #0b0d10; color: #c8d0d8;
  font: 13px/1.4 system-ui, sans-serif; }}
nav {{ position: sticky; top: 0; height: 100vh; overflow-y: auto; flex: none;
  width: 280px; padding: 12px; box-sizing: border-box; border-right: 1px solid #222; }}
nav input, nav select {{ width: 100%; box-sizing: border-box; margin-bottom: 8px; }}
nav h3 {{ font-size: 12px; margin: 12px 0 2px; text-transform: uppercase; letter-spacing: 0.08em; }}
nav h3 a {{ color: #c8d0d8; text-decoration: none; }}
nav ul {{ list-style: none; margin: 0; padding: 0; }}
nav li {{ display: flex; align-items: baseline; gap: 6px; }}
nav li a {{ padding: 1px 4px; color: #9ab; text-decoration: none; }}
nav li a:hover {{ background: #1a1f25; color: #def; }}
nav li.missing {{ padding: 1px 4px; color: #5a636c; text-decoration: line-through; }}
.count {{ color: #6a737b; font-weight: normal; letter-spacing: 0; }}
.tag {{ font-size: 10px; color: #8a7a55; border: 1px solid #4a4030; border-radius: 3px;
  padding: 0 4px; font-weight: normal; }}
main {{ flex: 1; padding: 12px 24px; min-width: 0; }}
.summary {{ color: #9ab; margin: 0 0 16px; }}
.summary a {{ color: #9ab; }}
h2 {{ font-size: 13px; margin: 24px 0 12px; padding-bottom: 4px; border-bottom: 1px solid #222;
  text-transform: uppercase; letter-spacing: 0.08em; color: #c8d0d8; scroll-margin-top: 12px; }}
section {{ margin-bottom: 28px; scroll-margin-top: 12px; }}
h3 {{ font-size: 15px; margin: 0 0 8px; }}
h3 a {{ color: #78b4ff; text-decoration: none; }}
.subtitle {{ color: #8a96a3; font-size: 12px; font-weight: normal; }}
p.missing {{ color: #6a737b; }}
main img {{ display: block; image-rendering: pixelated; cursor: zoom-in;
  width: calc(var(--w) * var(--zoom) * 1px); height: calc(var(--h) * var(--zoom) * 1px); }}
#lightbox {{ position: fixed; inset: 0; z-index: 10; display: flex; flex-direction: column;
  align-items: center; justify-content: center; gap: 8px; background: rgb(0 0 0 / 0.88);
  cursor: zoom-out; }}
#lightbox.hidden {{ display: none; }}
#lightbox img {{ image-rendering: pixelated; }}
#lightbox p {{ margin: 0; color: #9ab; }}
details {{ margin-top: 8px; }}
summary {{ cursor: pointer; color: #9ab; }}
.grid {{ display: flex; flex-wrap: wrap; gap: 16px; margin-top: 8px; align-items: flex-start; }}
figure {{ margin: 0; }}
figcaption {{ color: #8a96a3; font-size: 12px; margin-top: 4px; }}
.hidden {{ display: none; }}
</style>
</head>
<body>
<nav>
<input id="filter" type="search" placeholder="Filter components" autofocus>
<select id="zoom">
<option value="1">Zoom 1×</option><option value="2">Zoom 2×</option>
<option value="3">Zoom 3×</option><option value="4">Zoom 4×</option>
</select>
{nav}</nav>
<main>
{summary}
{body}</main>
<div id="lightbox" class="hidden"><img alt=""><p></p></div>
<script>
const filter = document.getElementById("filter");
filter.addEventListener("input", () => {{
  const query = filter.value.toLowerCase();
  for (const section of document.querySelectorAll("main section")) {{
    const hide = !section.dataset.search.toLowerCase().includes(query);
    section.classList.toggle("hidden", hide);
    document.querySelector(`nav a[href="#${{section.id}}"]`).parentElement
      .classList.toggle("hidden", hide);
  }}
  for (const item of document.querySelectorAll("nav li.missing")) {{
    item.classList.toggle("hidden", !item.textContent.toLowerCase().includes(query));
  }}
  // A group with nothing left to show folds away while filtering.
  for (const group of document.querySelectorAll(".group")) {{
    const empty = !group.querySelector("section:not(.hidden), li:not(.hidden)");
    group.classList.toggle("hidden", query !== "" && empty);
  }}
}});
document.getElementById("zoom").addEventListener("change", (event) => {{
  document.documentElement.style.setProperty("--zoom", event.target.value);
}});
// Clicking an image shows it big on this page (never a new tab). It is
// scaled to fit the window: whole-number steps when it fits at 1× or more so
// the pixels stay crisp, shrunk to fit otherwise. Click or Escape closes it.
const lightbox = document.getElementById("lightbox");
const lightboxImg = lightbox.querySelector("img");
const lightboxCaption = lightbox.querySelector("p");
function openLightbox(img) {{
  const w = img.naturalWidth, h = img.naturalHeight;
  let scale = Math.min((innerWidth - 48) / w, (innerHeight - 72) / h);
  if (scale >= 1) scale = Math.floor(scale);
  lightboxImg.src = img.src;
  lightboxImg.style.width = `${{w * scale}}px`;
  lightboxImg.style.height = `${{h * scale}}px`;
  lightboxCaption.textContent = `${{img.alt}} · ${{w}}×${{h}} at ${{Math.round(scale * 100)}}%`;
  lightbox.classList.remove("hidden");
}}
document.querySelector("main").addEventListener("click", (event) => {{
  if (event.target.tagName === "IMG") openLightbox(event.target);
}});
lightbox.addEventListener("click", () => lightbox.classList.add("hidden"));
document.addEventListener("keydown", (event) => {{
  if (event.key === "Escape") lightbox.classList.add("hidden");
}});
</script>
</body>
</html>
"##
    )
}

/// Size of the axis-aligned box around a `size` rect rotated by `angle`, plus
/// 2px for the anti-aliased edge — the cell a rotated sample needs.
fn rotated_bounds(size: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    let (s, c) = (s.abs(), c.abs());
    [
        size[0] * c + size[1] * s + 2.0,
        size[0] * s + size[1] * c + 2.0,
    ]
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
#[ignore = "needs a GPU adapter; writes PNGs and an index page for manual inspection"]
fn render_widget_gallery() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter available");

    // The canvas is rendered in pages of at most `PAGE_MAX` rows, so the
    // portable default limits are enough however tall the gallery grows.
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("gallery device"),
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
    let gallery_cells;

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
    // Hovered toolbars whose tooltips paint after the base layer, through the
    // toolbar's own overlay step (`draw_open_layer`) exactly as a host would —
    // so the items and states outlive the base scope. One docked left (tooltip
    // to the right), one docked top (tooltip below).
    #[cfg(feature = "phosphor-icons")]
    let tip_toolbar_items = [
        wgpu_gameui::ToolbarItem::tool(1, Icon::new(PhosphorIcon::Diamond), "Select", "Q"),
        wgpu_gameui::ToolbarItem::tool(2, Icon::new(PhosphorIcon::ArrowClockwise), "Move", "W"),
        wgpu_gameui::ToolbarItem::separator(),
        wgpu_gameui::ToolbarItem::toggle(
            5,
            Icon::new(PhosphorIcon::Eraser),
            "Snap to grid",
            "Shift+G",
        ),
    ];
    #[cfg(feature = "phosphor-icons")]
    let mut tip_toolbars = [
        wgpu_gameui::ToolbarState::new(wgpu_gameui::ToolbarEdge::Left),
        wgpu_gameui::ToolbarState::new(wgpu_gameui::ToolbarEdge::Top),
    ];
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
        flow.section(list, Category::Chrome, "MenuBar", "01-menu-bar.html states");

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

        flow.section(
            list,
            Category::Chrome,
            "MenuSheet",
            "02-menu-sheet.html, opened from the bar",
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
        flow.section(
            list,
            Category::Engine,
            "Primitives",
            "DrawList shapes, nine-slice, sprites",
        );

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

        // Rotated rounded rect: still one SDF instance — the shader applies the
        // rotation, so the edges stay anti-aliased. The cell fits the rotated
        // bounds so the shape doesn't cover its label.
        let (angle, size) = (0.18, [120.0, 44.0]);
        let bounds = rotated_bounds(size, angle);
        let r = flow.cell(list, "Rounded rect (rotated)", bounds[0], bounds[1]);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(angle);
        list.rounded_rect(
            Rect::new(-size[0] / 2.0, -size[1] / 2.0, size[0], size[1]),
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
            flow.section(list, Category::Foundations, "Icons", "Phosphor, MSDF");

            // The full curated set at a single readable size.
            for &icon in PhosphorIcon::ALL {
                let r = flow.cell(list, icon.name(), 32.0, 32.0);
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
        flow.section(
            list,
            Category::Foundations,
            "Text",
            "outline, shadow, glow, alignment",
        );

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
        flow.section(
            list,
            Category::Foundations,
            "Bidi",
            "RTL / bidi text and input",
        );

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
        flow.section(list, Category::Foundations, "VerticalText", "");

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
        flow.section(
            list,
            Category::Engine,
            "TextCentering",
            "vertical centering debug",
        );
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
        flow.section(list, Category::Foundations, "Fonts", "weights and styles");

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
        flow.section(
            list,
            Category::Foundations,
            "TextSpans",
            "span colour + underline",
        );

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
        flow.section(list, Category::Engine, "UiContext", "interactive verbs");
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
            let r = flow.cell(list, "pressable() + Image", 40.0, 40.0);
            list.push_transform();
            {
                let mut vstate = UiState::new();
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                // An image button: the "eye" icon key the gallery renderer has,
                // in a pressable.
                let _ = ui.pressable(Pressable::new(), 32.0, 34.0, |key, ctx| {
                    Image::key("eye")
                        .fit(ImageFit::Contain)
                        .natural_size(32.0, 32.0)
                        .draw(key.face.inset(6.0), ctx.draw_list);
                });
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

        flow.section(list, Category::Keys, "Button", "");

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

        // Forge Key tones: the accent and danger keys, enabled and disabled.
        for (label, caption, tone, enabled) in [
            ("Button (accent)", "Send", wgpu_gameui::Tone::Accent, true),
            ("Button (danger)", "Stop", wgpu_gameui::Tone::Danger, true),
            (
                "Button (accent, disabled)",
                "Send",
                wgpu_gameui::Tone::Accent,
                false,
            ),
            (
                "Button (danger, disabled)",
                "Stop",
                wgpu_gameui::Tone::Danger,
                false,
            ),
        ] {
            let r = flow.cell(list, label, 100.0, 32.0);
            Button::new(caption)
                .tone(tone)
                .enabled(enabled)
                .draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

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

        flow.section(list, Category::Forms, "Checkbox", "");
        let cb = Checkbox::new();
        let r = flow.cell(list, "Checkbox", 120.0, 20.0);
        cb.draw(false, "Off", r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Checkbox (checked)", 120.0, 20.0);
        cb.draw(true, "On", r, &mut ctx(list, &mut focus, &theme, &input));

        flow.section(list, Category::Forms, "Radio", "");
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

        flow.section(list, Category::Forms, "ProgressBar", "");
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

        // Busy with no known end: a sweep stepped by the caller's clock
        // (`indeterminate_step`); frozen at a step for the PNG.
        let r = flow.cell(list, "Progress (indeterminate)", 150.0, 7.0);
        ProgressBar::indeterminate(12).draw(r, list, &StyleResolver::new(&theme));

        flow.section(list, Category::Forms, "Slider", "");
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
        flow.section(list, Category::Windows, "DragHandle", "window mover");
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

        flow.section(list, Category::Layout, "SegmentedTabs", "Tabs");
        let r = flow.cell(list, "Tabs", 240.0, 30.0);
        Tabs::new(&["Tab A", "Tab B", "Tab C"]).draw(
            r,
            0,
            list,
            &StyleResolver::new(&theme),
            &input,
            None,
        );

        flow.section(list, Category::Forms, "TextField", "TextInput");
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
        flow.section(list, Category::Forms, "TextArea", "multiline TextInput");
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
        flow.section(list, Category::Forms, "NumberField", "NumberInput");
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
        flow.section(list, Category::Data, "Tree", "");
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
                let row = Rect::new(r.x, r.y + i as f32 * 22.0, r.width, 22.0);
                let mut tctx = DrawContext::new(list, &mut focus, &theme, &idle, W as f32, 600.0);
                TreeNode::new(label)
                    .with_leaf(*leaf)
                    .with_depth(*depth)
                    .with_leading(&leading)
                    .with_trailing(&trailing)
                    .draw(*id, row, &mut tree, &mut tctx);
            }
        }

        // The Forge tree: Phosphor glyphs per node, a trailing eye, the
        // accent selection ("Metal"), a disabled leaf ("locked.mat", with its
        // dim glyph) and the hover wash ("Foliage", under the pointer).
        #[cfg(feature = "phosphor-icons")]
        {
            use wgpu_gameui::{Ink, TreeIcon};
            let r = flow.cell(
                list,
                "Tree — glyphs · selected · disabled · hovered",
                220.0,
                132.0,
            );
            {
                list.quad(r.x, r.y, r.width, r.height, theme.panel);
                const VIS: u32 = 1;
                let mut tree = TreeState::new();
                tree.set_expanded(1, true);
                tree.select(3);
                let rows: [(u64, &str, bool, usize, bool); 6] = [
                    (1, "Materials", false, 0, false),
                    (2, "wood.mat", true, 1, false),
                    (3, "metal.mat", true, 1, false),
                    (4, "locked.mat", true, 1, true),
                    (5, "Foliage", false, 0, false),
                    (6, "stone.mat", true, 0, false),
                ];
                let hover = InputState {
                    mouse_x: r.x + 80.0,
                    mouse_y: r.y + 4.0 * 22.0 + 11.0,
                    ..InputState::default()
                };
                let eye_tint = StyleResolver::new(&theme).ink(Ink::Empty);
                let trailing = [TreeAction::phosphor(VIS, PhosphorIcon::Eye).with_tint(eye_tint)];
                for (i, &(id, label, leaf, depth, disabled)) in rows.iter().enumerate() {
                    let glyph = match (leaf, tree.is_expanded(id)) {
                        (true, _) => PhosphorIcon::File,
                        (false, true) => PhosphorIcon::FolderOpen,
                        (false, false) => PhosphorIcon::Folder,
                    };
                    let row = Rect::new(r.x, r.y + i as f32 * 22.0, r.width, 22.0);
                    let mut tctx =
                        DrawContext::new(list, &mut focus, &theme, &hover, W as f32, 600.0);
                    TreeNode::new(label)
                        .with_leaf(leaf)
                        .with_depth(depth)
                        .with_glyph(TreeIcon::Phosphor(glyph))
                        .with_disabled(disabled)
                        .with_slot_size(14.0)
                        .with_trailing(&trailing)
                        .draw(id, row, &mut tree, &mut tctx);
                }
            }
        }

        // The V2 Disk tab: project folders under "Projects" carry their
        // monogram thumb, the selected one ("agent-ui") in its accent look,
        // a disabled one faded.
        #[cfg(feature = "phosphor-icons")]
        {
            use wgpu_gameui::{Thumb, TreeIcon};
            let r = flow.cell(list, "Tree — project thumbs", 220.0, 110.0);
            list.quad(r.x, r.y, r.width, r.height, theme.panel);
            let mut tree = TreeState::new();
            tree.set_expanded(1, true);
            tree.select(2);
            let idle = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let rows: [(u64, &str, usize, bool, bool); 5] = [
                (1, "Projects", 0, false, false),
                (2, "agent-ui", 1, true, false),
                (3, "wgpu-gameui", 1, true, false),
                (4, "archived", 1, true, true),
                (5, ".dotfiles", 0, false, false),
            ];
            for (i, &(id, label, depth, project, disabled)) in rows.iter().enumerate() {
                let row = Rect::new(r.x, r.y + i as f32 * 22.0, r.width, 22.0);
                let mut tctx = DrawContext::new(list, &mut focus, &theme, &idle, W as f32, 600.0);
                let mut node = TreeNode::new(label)
                    .with_depth(depth)
                    .with_disabled(disabled);
                node = if project {
                    node.with_thumb(Thumb::new().name(label))
                } else {
                    node.with_glyph(TreeIcon::Phosphor(PhosphorIcon::Folder))
                };
                node.draw(id, row, &mut tree, &mut tctx);
            }
        }

        // Context menu state, shown over a viewport swatch. Its modal layer is
        // drawn after the base scope, matching the production integration path.
        flow.section(
            list,
            Category::Chrome,
            "ContextMenu",
            "05-context-menu.html",
        );
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
        flow.section(list, Category::Forms, "Dropdown", "");
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

        flow.section(list, Category::Layout, "ScrollArea", "ScrollView");
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
        flow.section(list, Category::Data, "Table", "");
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

        // Forge's `Key` is our `Pressable`: the key every clickable key is
        // built on, with the content drawn by the caller. An image button is
        // an `Image` in a pressable.
        flow.section(list, Category::Keys, "Key", "Pressable, holding an Image");
        {
            let image = |sprite| {
                move |key: &PressState, ctx: &mut DrawContext| {
                    Image::sprite(sprite)
                        .fit(ImageFit::Contain)
                        .natural_size(48.0, 48.0)
                        .draw(key.face.inset(4.0), ctx.draw_list);
                }
            };
            for (label, key, sprite) in [
                ("Default", Pressable::new(), duck),
                (
                    "Ghost",
                    Pressable::new().tone(wgpu_gameui::Tone::Ghost),
                    board,
                ),
                ("Held", Pressable::new().held(true), suitcase),
                ("Disabled", Pressable::new().enabled(false), suitcase),
                ("Bare", Pressable::new().bare(), board),
            ] {
                let r = flow.cell(list, label, 40.0, 42.0);
                key.draw(r, &mut ctx(list, &mut focus, &theme, &input), image(sprite));
            }
            // The pointer over the key: the face lightens, and a bare key
            // gets its wash over the image.
            for (label, key, sprite) in [
                ("Default, hovered", Pressable::new(), duck),
                ("Bare, hovered", Pressable::new().bare(), board),
            ] {
                let r = flow.cell(list, label, 40.0, 42.0);
                let hover = InputState {
                    mouse_x: r.x + 20.0,
                    mouse_y: r.y + 20.0,
                    ..InputState::default()
                };
                key.draw(r, &mut ctx(list, &mut focus, &theme, &hover), image(sprite));
            }
        }

        // ---- Lists / Grids (virtualized) --------------------------------
        flow.section(list, Category::Data, "List", "virtualized list and grid");

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
                        // Dark ink only on the focused accent fill; the held
                        // (unfocused) selection is a light wash.
                        let c = if it.selected && it.focused {
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
            state.scroll.snap_to(1, 420.0); // scrolled partway
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

        // ---- ListView (the design's dense sidebar list) -----------------
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(
                list,
                Category::Data,
                "ListView",
                "highlight · focus-aware selection · two-line · empty",
            );
            use wgpu_gameui::{ListRow, ListView};

            let projects = [
                ListRow::new("agent-ui"),
                ListRow::new("claude-code-ui"),
                ListRow::new("wgpu-gameui"),
                ListRow::new("magic-agents"),
                ListRow::new("old-agent-prototype").disabled(true),
                ListRow::new("serialexp"),
            ];
            let sessions = [
                ListRow::new("Fix websocket reconnect")
                    .subtitle("01a01043 · 38 msgs")
                    .meta("2h"),
                ListRow::new("Port the sidebar to gameui")
                    .subtitle("7c2e91f0 · 112 msgs")
                    .meta("1d"),
                ListRow::new("Untitled")
                    .subtitle("e41b77aa · 0 msgs")
                    .meta("3w"),
            ];
            let folders = [
                ListRow::new("crates")
                    .glyph(PhosphorIcon::Folder)
                    .mono(true),
                ListRow::new("docs").glyph(PhosphorIcon::Folder).mono(true),
                ListRow::new("Cargo.toml")
                    .glyph(PhosphorIcon::File)
                    .mono(true),
            ];
            // (label, rows, view, selected, hovered row)
            type Case<'a> = (
                &'a str,
                &'a [ListRow<'a>],
                ListView<'a>,
                Option<usize>,
                Option<usize>,
            );
            let cases: [Case; 5] = [
                (
                    "Focused · highlight \"ag\" · hover",
                    &projects,
                    ListView::new().highlight("ag").focused(true),
                    Some(0),
                    Some(3),
                ),
                (
                    "Unfocused (held selection)",
                    &projects,
                    ListView::new().highlight("ag"),
                    Some(0),
                    None,
                ),
                (
                    "Two-line (sessions)",
                    &sessions,
                    ListView::new().two_line(true).focused(true),
                    Some(1),
                    None,
                ),
                ("Glyphs · mono", &folders, ListView::new(), Some(0), None),
                (
                    "Empty",
                    &[],
                    ListView::new().empty("No projects match \"zz\""),
                    None,
                    None,
                ),
            ];
            for (label, rows, view, selected, hover) in cases {
                let r = flow.cell(list, label, 200.0, 140.0);
                list.rounded_rect(r, 2.0, [0.06, 0.07, 0.085, 1.0]);
                let row_h = view.row_height(&StyleResolver::new(&theme));
                let mut view_input = InputState {
                    mouse_x: hover.map_or(-1.0, |_| r.x + 40.0),
                    mouse_y: hover.map_or(-1.0, |i| r.y + (i as f32 + 0.5) * row_h),
                    ..InputState::default()
                };
                view.draw(
                    r,
                    rows.len(),
                    selected,
                    &mut ListState::new(),
                    list,
                    &StyleResolver::new(&theme),
                    &mut view_input,
                    |i| rows[i],
                );
            }
        }

        // ---- GroupList · Thumb · status dot · hue chip (V2 sidebar) -----
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(list, Category::Sidebar, "GroupList", "grouped sessions");
            use wgpu_gameui::{
                GroupHeader, GroupItem, GroupLayout, GroupList, GroupListState, GroupMore,
                GroupRow, Status, Thumb, status_dot,
            };
            let s = StyleResolver::new(&theme);
            let rows = [
                GroupRow::Header(
                    GroupHeader::new("agent-ui")
                        .count("3 / 5")
                        .thumb(Thumb::new().name("agent-ui")),
                ),
                GroupRow::Item(
                    GroupItem::new("Session context menu close option")
                        .subtitle("@merry-tiger · 447 msgs · 1 comp")
                        .chip("claude opus", 45.0)
                        .meta("now")
                        .status(Status::Running),
                ),
                GroupRow::Item(
                    GroupItem::new("README repo description")
                        .subtitle("@warm-otter · 6002 msgs · 15 comp")
                        .chip("claude opus", 45.0)
                        .meta("40m"),
                ),
                GroupRow::Item(
                    GroupItem::new("Async I/O audit (all kinds)")
                        .subtitle("@brisk-lynx · 1204 msgs · 3 comp")
                        .chip("gothab · codex", 255.0)
                        .meta("1h")
                        .status(Status::Unread),
                ),
                GroupRow::More(GroupMore::new("2 older", "3d – 8d")),
                GroupRow::Header(
                    GroupHeader::new("AJME-54")
                        .count("1 / 1")
                        .thumb(Thumb::new().name("AJME-54")),
                ),
                GroupRow::Item(
                    GroupItem::new("The human has a comment on PR 12")
                        .subtitle("@calm-fox · 1169 msgs · 9 comp")
                        .chip("claude opus", 45.0)
                        .meta("6m")
                        .status(Status::Waiting),
                ),
                GroupRow::Header(
                    GroupHeader::new("quiet projects")
                        .count("2")
                        .thumb(Thumb::new().glyph(PhosphorIcon::Rows))
                        .mono(false)
                        .dim(true),
                ),
                GroupRow::Header(
                    GroupHeader::new("claude-code-ui")
                        .count("1")
                        .open(true)
                        .depth(1)
                        .thumb(Thumb::new().name("claude-code-ui")),
                ),
                GroupRow::Item(
                    GroupItem::new("86d6a50a")
                        .subtitle("@wise-otter · 197 msgs")
                        .chip("claude opus", 45.0)
                        .meta("9d")
                        .dim(true)
                        .depth(1),
                ),
                GroupRow::Header(
                    GroupHeader::new("dotfiles")
                        .count("1")
                        .open(false)
                        .depth(1)
                        .thumb(Thumb::new().name("dotfiles")),
                ),
            ];
            let layout = GroupLayout::from_kinds(rows.iter().map(GroupRow::kind));
            // (label, selected, hovered row)
            let cases = [
                (
                    "Sidebar · selected · running · waiting · unread",
                    Some(2),
                    None,
                ),
                ("Hovered row (⋯ key)", Some(1), Some(3)),
            ];
            for (label, selected, hover) in cases {
                let r = flow.cell(list, label, 272.0, layout.height());
                list.rounded_rect(r, 0.0, [0.06, 0.07, 0.085, 1.0]);
                let mut input = InputState {
                    mouse_x: hover.map_or(-1.0, |_| r.x + 120.0),
                    mouse_y: hover.map_or(-1.0, |i| r.y + layout.top(i) + 10.0),
                    ..InputState::default()
                };
                GroupList::new().draw(
                    r,
                    &layout,
                    selected,
                    &mut GroupListState::new(),
                    list,
                    &s,
                    &mut input,
                    |i| rows[i],
                );
            }

            flow.section(
                list,
                Category::Data,
                "Thumb",
                "leading row visual; project thumbs also show under Tree",
            );
            let r = flow.cell(
                list,
                "Monogram 10/14/24 · swatch · glyph · slot",
                240.0,
                60.0,
            );
            let mut x = r.x;
            for size in [10.0, 14.0, 24.0] {
                Thumb::new()
                    .name("agent-ui")
                    .size(size)
                    .draw(x, r.y, list, &s);
                x += size + 8.0;
            }
            Thumb::new()
                .color([0.8, 0.4, 0.2, 1.0])
                .draw(x, r.y, list, &s);
            x += 22.0;
            Thumb::new()
                .glyph(PhosphorIcon::Rows)
                .draw(x, r.y, list, &s);
            x += 22.0;
            Thumb::new().size(24.0).draw(x, r.y, list, &s);
            // On an accent row.
            let accent = Rect::new(r.x, r.y + 34.0, 240.0, 22.0);
            list.quad(
                accent.x,
                accent.y,
                accent.width,
                accent.height,
                theme.accent,
            );
            let mut x = accent.x + 6.0;
            for thumb in [
                Thumb::new().name("sorry-pulumi2"),
                Thumb::new().glyph(PhosphorIcon::Folder),
                Thumb::new(),
            ] {
                thumb.selected(true).draw(x, accent.y + 4.0, list, &s);
                x += 22.0;
            }

            flow.section(
                list,
                Category::Feedback,
                "StatusDot",
                "session state on a row",
            );
            // Idle deliberately draws no dot; it holds the fourth slot empty.
            let r = flow.cell(list, "Running · waiting · unread · (idle)", 200.0, 16.0);
            let mut x = r.x + 4.0;
            for status in [
                Status::Running,
                Status::Waiting,
                Status::Unread,
                Status::Idle,
            ] {
                status_dot(list, &s, (x, r.y + 8.0), status);
                x += 16.0;
            }
        }

        // ---- Instanced chrome (SDF rounded-rect) ------------------------
        // Every `Button` already routes its background+border through the
        // instanced `chrome_rect` path; this section makes the batching
        // explicit (a strip of same-shape buttons collapses to one base mesh +
        // N instances) and shows rotated chrome staying one smooth instance.
        flow.section(
            list,
            Category::Engine,
            "InstancedChrome",
            "SDF rounded-rect batching",
        );

        for i in 0..6 {
            let r = flow.cell(list, "", 70.0, 30.0);
            Button::new(format!("#{i}")).draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

        // Rotated chrome: the instance carries the whole transform and the
        // shader rotates it, so edges stay anti-aliased and the gradient stays
        // inside the rounded corners. The cell fits the rotated bounds.
        let (angle, size) = (0.18, [80.0, 40.0]);
        let bounds = rotated_bounds(size, angle);
        let r = flow.cell(list, "Rotated, gradient", bounds[0], bounds[1]);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(angle);
        list.chrome_rect_gradient(
            Rect::new(-size[0] / 2.0, -size[1] / 2.0, size[0], size[1]),
            8.0,
            2.0,
            [0.40, 0.65, 0.45, 1.0],
            [0.20, 0.42, 0.26, 1.0],
            [0.80, 0.90, 0.80, 1.0],
        );
        list.pop_transform();

        // ---- Hit zone (draw-free sensor) --------------------------------
        // `HitZone` draws NOTHING — it only senses pointer interaction over a
        // rect (Teardown's UiMakeInteractive), for sensors over things the UI
        // didn't draw (3D viewports, world-projected regions). The gallery
        // can't show "nothing", so each cell paints its own outline + a caption
        // reporting the state `HitZone::test` returns for a synthetic pointer.
        flow.section(list, Category::Engine, "HitZone", "invisible sensor");

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
        flow.section(
            list,
            Category::Engine,
            "Styling",
            "theme, overlay and custom keys",
        );

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
        flow.section(
            list,
            Category::Engine,
            "Layout",
            "weighted HStack + Flow grid",
        );

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
        flow.section(list, Category::Engine, "Justify", "main-axis distribution");
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
        flow.section(list, Category::Layout, "Separator", "");
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
        flow.section(
            list,
            Category::Layout,
            "Splitter",
            "Forge dark chrome states",
        );
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
        flow.section(list, Category::Editors, "ColorPicker", "");
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
        flow.section(list, Category::Engine, "Gradients", "");
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
        flow.section(
            list,
            Category::Engine,
            "MeasuredLayout",
            "contextual measured layout",
        );
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
        flow.section(list, Category::Layout, "Group", "titled panel");
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
        // The new widgets ported from the "4a" UI design folder, one section
        // per component; every widget is drawn from its public API.
        let s = StyleResolver::new(&theme);

        flow.section(list, Category::Forms, "Switch", "Toggle");
        {
            let off = flow.cell(list, "Off", 60.0, 18.0);
            let on = flow.cell(list, "On, labeled", 90.0, 18.0);
            let mut tctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            Toggle::new().draw(false, off, &mut tctx);
            Toggle::new().label("shadows").draw(true, on, &mut tctx);
        }

        flow.section(
            list,
            Category::Feedback,
            "Badge",
            "status tones, any hue, compact",
        );
        {
            let tones = [
                ("queued", BadgeTone::Draft),
                ("exit 0", BadgeTone::Baked),
                ("killed", BadgeTone::Stale),
                ("exit 101", BadgeTone::Error),
                ("running", BadgeTone::Live),
            ];
            let hues = [
                ("claude opus", BadgeTone::Hue(45.0)),
                ("vertex", BadgeTone::Hue(230.0)),
                ("codex", BadgeTone::Hue(160.0)),
                ("gothab · codex", BadgeTone::Hue(255.0)),
            ];
            // A row of badges from the left of `r`, 6px apart.
            let row = |list: &mut DrawList,
                       r: Rect,
                       badges: &[(&str, BadgeTone)],
                       compact: bool,
                       on_accent: bool| {
                let mut x = r.x;
                for &(text, tone) in badges {
                    let mut badge = Badge::new(tone).on_accent(on_accent);
                    if compact {
                        badge = badge.compact();
                    }
                    x = badge.draw(list, &s, x, r.y, text).right() + 6.0;
                }
            };
            // Forge Badge's status tones, one cell each.
            let labels = [
                "Tone: draft",
                "Tone: baked",
                "Tone: stale",
                "Tone: error",
                "Tone: live",
            ];
            for (label, (text, tone)) in labels.into_iter().zip(tones) {
                let w = Badge::new(tone).width(list, &s, text);
                let r = flow.cell(list, label, w.max(70.0), BADGE_HEIGHT);
                Badge::new(tone).draw(list, &s, r.x, r.y, text);
            }
            let r = flow.cell(list, "Status tones, compact", 280.0, BADGE_HEIGHT);
            row(list, r, &tones, true, false);
            let r = flow.cell(list, "Any hue", 400.0, BADGE_HEIGHT);
            row(list, r, &hues, false, false);
            let r = flow.cell(list, "Any hue, compact", 300.0, BADGE_HEIGHT);
            row(list, r, &hues, true, false);
            // On a selected row the plate goes flat to read on the accent.
            let r = flow.cell(list, "On accent: regular, then compact", 460.0, 20.0);
            list.quad(r.x, r.y, r.width, r.height, theme.accent);
            let inner = Rect::new(r.x + 4.0, r.y + 2.5, r.width - 8.0, BADGE_HEIGHT);
            let pair = [hues[0], tones[4]];
            row(list, inner, &pair, false, true);
            let compact_x = inner.x + 240.0;
            let inner = Rect::new(compact_x, r.y + 3.5, r.right() - compact_x, BADGE_HEIGHT);
            row(list, inner, &pair, true, true);
        }

        flow.section(
            list,
            Category::Data,
            "CountBubble",
            "raised: an arriving quantity",
        );
        {
            for count in [3, 12, 128] {
                let w = CountBubble::new(count).width(list, &s);
                let r = flow.cell(list, &count.to_string(), w.max(40.0), COUNT_BUBBLE_HEIGHT);
                CountBubble::new(count).draw(list, &s, r.x, r.y);
            }
            // Beside a label, where it usually lives.
            let r = flow.cell(list, "Beside a row label", 140.0, 22.0);
            let mut label = s.sans_block("Problems", r.x, r.y, TextSize::Row, Ink::Row);
            let (label_w, label_h) = list.measure_block(&label);
            label.y = r.y + (r.height - label_h) * 0.5;
            list.text(label);
            let y = r.y + (r.height - COUNT_BUBBLE_HEIGHT) * 0.5;
            CountBubble::new(7).draw(list, &s, r.x + label_w + 6.0, y);
        }

        flow.section(
            list,
            Category::Feedback,
            "StatusIcon",
            "tone by glyph and colour · 22 px and 14 px",
        );
        {
            let tones = [
                ("Info", Severity::Info),
                ("Success", Severity::Success),
                ("Warning", Severity::Warning),
                ("Error", Severity::Error),
            ];
            for (label, tone) in tones {
                let r = flow.cell(list, label, 50.0, STATUS_ICON_SIZE);
                StatusIcon::new(tone).draw(list, &s, r.x, r.y);
                StatusIcon::new(tone).size(STATUS_ICON_INLINE_SIZE).draw(
                    list,
                    &s,
                    r.x + STATUS_ICON_SIZE + 8.0,
                    r.y + (STATUS_ICON_SIZE - STATUS_ICON_INLINE_SIZE) * 0.5,
                );
            }
            let r = flow.cell(list, "Inline in a 22 px row", 200.0, 22.0);
            let icon_y = r.y + (22.0 - STATUS_ICON_INLINE_SIZE) * 0.5;
            StatusIcon::new(Severity::Warning)
                .size(STATUS_ICON_INLINE_SIZE)
                .draw(list, &s, r.x, icon_y);
            let text = s.sans_block("3 missing textures", 0.0, 0.0, TextSize::Row, Ink::Row);
            let text_h = list.measure_block(&text).1;
            list.text(TextBlock {
                x: r.x + STATUS_ICON_INLINE_SIZE + 6.0,
                y: r.y + (22.0 - text_h) * 0.5,
                ..text
            });
        }

        flow.section(
            list,
            Category::Feedback,
            "Placeholder",
            "static stand-in: image · round · text",
        );
        {
            let r = flow.cell(list, "Image 16:9", 192.0, 108.0);
            Placeholder::image()
                .label("Level preview · 16:9")
                .draw(r, list, &s);
            let r = flow.cell(list, "Image, no caption", 64.0, 64.0);
            Placeholder::image().label("").draw(r, list, &s);
            let r = flow.cell(list, "Round", 64.0, 64.0);
            Placeholder::image().round(true).label("").draw(r, list, &s);
            let text = Placeholder::text(3).label("Description");
            let h = text.height(list, &s);
            let r = flow.cell(list, "Text, 3 lines", 180.0, h);
            text.draw(r, list, &s);
            let bare = Placeholder::text(5);
            let h = bare.height(list, &s);
            let r = flow.cell(list, "Text, 5 lines", 180.0, h);
            bare.draw(r, list, &s);
        }

        flow.section(
            list,
            Category::Data,
            "DropZone",
            "outline, not fill · idle / active",
        );
        {
            let (w, h) = DROP_ZONE_SIZE;
            let r = flow.cell(list, "Idle", w, h);
            DropZone::new("Drop to group").draw(r, list, &s);
            let r = flow.cell(list, "Active (drag over)", w, h);
            DropZone::new("Drop to group")
                .active(true)
                .draw(r, list, &s);
            let r = flow.cell(list, "Wide, over content", 260.0, h);
            Placeholder::text(4).draw(r.inset(6.0), list, &s);
            DropZone::new("Drop textures here to import them")
                .active(true)
                .draw(r, list, &s);
        }

        flow.section(
            list,
            Category::Forms,
            "FieldLabel",
            "mono caps above the well · optional readout",
        );
        {
            let label_h = FieldLabel::height(list, &s);
            let r = flow.cell(list, "Label", 160.0, label_h);
            FieldLabel::new("Albedo").draw(list, &s, r.x, r.y, r.width);
            let r = flow.cell(list, "With readout", 160.0, label_h);
            FieldLabel::new("Roughness")
                .value("0.42")
                .draw(list, &s, r.x, r.y, r.width);
            // Above its field, as it's used.
            let r = flow.cell(list, "Over a slider", 180.0, label_h + 4.0 + 16.0);
            let line = FieldLabel::new("Metallic")
                .value("0.80")
                .draw(list, &s, r.x, r.y, r.width);
            let slider_r = Rect::new(r.x, line.bottom() + 4.0, r.width, 16.0);
            let mut capture = DragCapture::default();
            Slider::new(0.0, 1.0).draw(
                0.8,
                0,
                &mut capture,
                slider_r,
                &mut ctx(list, &mut focus, &theme, &input),
            );
        }

        flow.section(
            list,
            Category::Layout,
            "Panel",
            "card on the app ground · mono caption",
        );
        {
            let r = flow.cell(list, "Captioned, with aside", 220.0, 110.0);
            let body = Panel::new().title("Physics").aside("3").draw(r, list, &s);
            for (i, (name, value)) in [("Mass", "72 kg"), ("Drag", "0.05"), ("Bounce", "0.3")]
                .into_iter()
                .enumerate()
            {
                let y = body.y + i as f32 * 20.0;
                list.text(s.sans_block(name, body.x, y, TextSize::Row, Ink::Row));
                let v = s.mono_block(value, 0.0, y, TextSize::Dense, Ink::Second);
                let vw = list.measure_block(&v).0;
                list.text(TextBlock {
                    x: body.right() - vw,
                    ..v
                });
            }
            let r = flow.cell(list, "No caption", 160.0, 110.0);
            let body = Panel::new().draw(r, list, &s);
            Placeholder::text(3).draw(body, list, &s);
        }

        flow.section(
            list,
            Category::Layout,
            "Sheet",
            "raised surface · header, body, footer keys",
        );
        {
            use wgpu_gameui::Tone;
            let keys = [
                SheetAction::new("Cancel"),
                SheetAction::new("Export").tone(Tone::Accent),
            ];
            let sheet = Sheet::new()
                .title("Export level")
                .description("Bakes lighting and packs every asset the level uses.")
                .body(46.0)
                .meta("~38 MB")
                .actions(&keys)
                .width(300.0);
            let h = sheet.height(300.0, list, &s);
            let r = sheet_cell(&mut flow, list, &s, "Body and meta", 300.0, h);
            sheet.draw_with(
                r,
                &mut ctx(list, &mut focus, &theme, &input),
                |_, body, ctx| {
                    let s = ctx.styles();
                    let label = FieldLabel::new("Target").value("PC · x64").draw(
                        ctx.draw_list,
                        &s,
                        body.x,
                        body.y,
                        body.width,
                    );
                    let rest = Rect::new(
                        body.x,
                        label.bottom() + 6.0,
                        body.width,
                        body.bottom() - label.bottom() - 6.0,
                    );
                    Placeholder::text(2).draw(rest, ctx.draw_list, &s);
                },
            );

            let keys = [
                SheetAction::new("Don't save").leading(true),
                SheetAction::new("Cancel"),
                SheetAction::new("Save").tone(Tone::Accent),
            ];
            let sheet = Sheet::new()
                .tone(Severity::Warning)
                .title("Unsaved changes")
                .description("city_block_07 has edits that haven't been saved.")
                .actions(&keys)
                .width(320.0);
            let h = sheet.height(320.0, list, &s);
            let r = sheet_cell(&mut flow, list, &s, "Tone, leading key", 320.0, h);
            sheet.draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

        flow.section(
            list,
            Category::Layout,
            "Modal",
            "sheet over a dimmed backdrop",
        );
        {
            let r = flow.cell(list, "Over a scene", 380.0, 230.0);
            dialog_stage(list, &s, r, "Level view");
            let keys = [
                SheetAction::new("Keep editing"),
                SheetAction::new("Discard").tone(wgpu_gameui::Tone::Danger),
            ];
            let mut state = ModalState::new();
            state.open();
            let mut modal_focus = FocusState::new();
            Modal::new()
                .tone(Severity::Warning)
                .title("Discard changes?")
                .message("Your terrain edits since the last save will be lost.")
                .actions(&keys)
                .focusable(900)
                .autofocus(None)
                .draw(
                    r,
                    &mut state,
                    &mut ctx(list, &mut modal_focus, &theme, &input),
                );
        }

        flow.section(
            list,
            Category::Dialogs,
            "AlertDialog",
            "one message, one key · mono detail",
        );
        {
            let r = flow.cell(list, "Error with detail", 340.0, 300.0);
            dialog_stage(list, &s, r, "AlertDialog · error");
            let mut state = AlertDialogState::new();
            state.open();
            let mut dialog_focus = FocusState::new();
            AlertDialog::new("Couldn't open level")
                .tone(Severity::Error)
                .message("Another editor has this level open. Close it there, then try again.")
                .detail("EBUSY: city_block_07.lvl\nlocked by pid 48213 (forge-editor)")
                .width(290.0)
                .draw(
                    910,
                    r,
                    &mut state,
                    &mut ctx(list, &mut dialog_focus, &theme, &input),
                );
        }

        flow.section(
            list,
            Category::Dialogs,
            "ConfirmDialog",
            "destructive · focus starts on Cancel",
        );
        {
            let r = flow.cell(list, "Destructive, don't ask", 340.0, 300.0);
            dialog_stage(list, &s, r, "ConfirmDialog · destructive");
            let mut state = ConfirmDialogState::new();
            state.open();
            let mut dialog_focus = FocusState::new();
            ConfirmDialog::new("Delete 3 prefabs?")
                .destructive(true)
                .message("Instances in open levels become unlinked meshes. This can't be undone.")
                .confirm_label("Delete")
                .dont_ask_label("Don't ask again")
                .width(300.0)
                .draw(
                    920,
                    r,
                    &mut state,
                    &mut ctx(list, &mut dialog_focus, &theme, &input),
                );
        }

        flow.section(
            list,
            Category::Dialogs,
            "PromptDialog",
            "one value · validated after the first edit",
        );
        {
            let taken = |v: &str| {
                let v = v.trim();
                if v != "Props" && ["Terrain", "Props", "Lighting"].contains(&v) {
                    Some("A layer with that name already exists".to_owned())
                } else if v.contains(['/', '\\']) {
                    Some("Names can't contain / or \\".to_owned())
                } else {
                    None
                }
            };
            let prompt = PromptDialog::new("Rename layer")
                .label("Name")
                .ok_label("Rename")
                .description("Scripts that look this layer up by name will need updating.")
                .hint("Shown in the outliner and layer menus.")
                .validate(&taken)
                .width(290.0);
            for (label, typed) in [("Opened", None), ("Invalid name", Some("Terrain"))] {
                let r = flow.cell(list, label, 340.0, 300.0);
                dialog_stage(list, &s, r, "PromptDialog · validated");
                let mut state = PromptDialogState::new();
                state.open("Props");
                let mut dialog_focus = FocusState::new();
                if let Some(text) = typed {
                    // A frame of typing, off-screen, so the error shows.
                    let mut scratch = DrawList::new();
                    let typing = InputState {
                        text_input: text.to_owned(),
                        ..InputState::default()
                    };
                    prompt.draw(
                        930,
                        r,
                        &mut state,
                        &mut ctx(&mut scratch, &mut dialog_focus, &theme, &typing),
                    );
                }
                prompt.draw(
                    930,
                    r,
                    &mut state,
                    &mut ctx(list, &mut dialog_focus, &theme, &input),
                );
            }
        }

        flow.section(list, Category::Keys, "Keycap", "");
        {
            let r = flow.cell(list, "⇧ · Ctrl · F", 150.0, 22.0);
            let mut kx = r.x;
            for cap in ["⇧", "Ctrl", "F"] {
                kx = keycap(list, &s, Rect::new(kx, r.y, 200.0, 22.0), cap, 18.0).right() + 4.0;
            }
        }

        flow.section(list, Category::Keys, "FilterChip", "chip");
        {
            // A filter row: first on, rest off.
            let r = flow.cell(list, "On · off · off", 200.0, 20.0);
            let mut cx = r.x;
            for (j, label) in ["info", "warn", "verbose"].iter().enumerate() {
                let _ = chip(
                    list,
                    &s,
                    Rect::new(cx, r.y, 70.0, 20.0),
                    label,
                    j == 0,
                    &input,
                );
                cx += 62.0;
            }
        }

        flow.section(list, Category::Layout, "Breadcrumb", "");
        {
            let r = flow.cell(list, "Three segments", 300.0, 20.0);
            let segs = ["World", "Region", "Forest"];
            Breadcrumb::new(&segs).draw(r, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });
        }

        flow.section(list, Category::Layout, "Pager", "");
        {
            let r = flow.cell(list, "Arrows", 150.0, 20.0);
            Pager::new().draw(2, 12, r, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });

            let r = flow.cell(list, "Numeric", 300.0, 18.0);
            Pager::new().numeric().draw(0, 4, r, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });
        }

        flow.section(list, Category::Chrome, "StatusBar", "06-status-bar.html");
        {
            let r = flow.cell(
                list,
                "Text cells, highlight, spacer",
                400.0,
                STATUS_BAR_HEIGHT,
            );
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

            // Zoned: dock toggles, clickable zones (one pressed), and a right
            // group of meters, a status dot and plain text.
            let toggles = [
                StatusToggle {
                    glyph: "◧",
                    on: true,
                },
                StatusToggle {
                    glyph: "◨",
                    on: false,
                },
            ];
            let row = s.ink(Ink::Row);
            let path = [StatusPart::Text("~/Projects/agent-ui")];
            let last = [StatusPart::Text("last:"), StatusPart::Tinted("send", row)];
            let bg = [
                StatusPart::Dot(Status::Running),
                StatusPart::Tinted("1 bg +2", row),
            ];
            let server = [
                StatusPart::Text("server"),
                StatusPart::Meter(36.0, MeterFill::neutral(0.3, &s)),
                StatusPart::Value("612 MB", row, 44.0),
            ];
            let connected = [
                StatusPart::Dot(Status::Running),
                StatusPart::Text("connected"),
            ];
            let left = [
                StatusZone::new(&path),
                StatusZone::new(&last),
                StatusZone::new(&bg).button(true),
            ];
            let right = [StatusZone::new(&server), StatusZone::new(&connected)];
            let r = flow.cell(
                list,
                "Zoned: toggles, zones, right group",
                760.0,
                STATUS_BAR_HEIGHT,
            );
            ZonedStatusBar::new(&toggles, &left, &right).draw(r, list, &s, &input);
        }

        flow.section(list, Category::Layout, "SpanTabs", "equal width");
        {
            let tabs = [
                SpanTab::new("Chat").glyph("›"),
                SpanTab::new("Forum").glyph("◫").count("3"),
                SpanTab::new("Files").glyph("◧").dirty(true),
                SpanTab::new("Terminal").glyph("⌗").disabled(true),
            ];
            let r = flow.cell(
                list,
                "Active · count · dirty · disabled",
                520.0,
                SPAN_TABS_HEIGHT,
            );
            SpanTabs::new(&tabs).draw(r, 0, list, &s, &input);
        }

        flow.section(list, Category::Feedback, "Meter", "not in Forge");
        {
            let r = flow.cell(list, "Inline: neutral", 60.0, 5.0);
            inline_meter(list, &s, r, MeterFill::neutral(0.07, &s));
            let r = flow.cell(list, "Inline: coloured, pace tick", 120.0, 6.0);
            inline_meter(
                list,
                &s,
                r,
                MeterFill::colored(0.44, s.color(StyleKey::Warning)).pace(0.3),
            );
            let r = flow.cell(list, "Stacked, auto-compact marker", 300.0, 6.0);
            let segments = [
                BarSegment {
                    fraction: 0.015,
                    color: [0.45, 0.62, 0.8, 1.0],
                },
                BarSegment {
                    fraction: 0.07,
                    color: [0.6, 0.6, 0.6, 1.0],
                },
                BarSegment {
                    fraction: 0.05,
                    color: [0.37, 0.7, 0.58, 1.0],
                },
                BarSegment {
                    fraction: 0.6,
                    color: [0.55, 0.5, 0.85, 1.0],
                },
            ];
            stacked_bar(
                list,
                &s,
                r,
                &segments,
                Some((0.8, s.color(StyleKey::Warning))),
            );
        }

        flow.section(list, Category::Keys, "WellChip", "not in Forge");
        {
            let meter = [
                WellChipPart::Meter(30.0, MeterFill::neutral(0.07, &s)),
                WellChipPart::Text("7%", None),
                WellChipPart::Text("5h", Some(Ink::Caption)),
            ];
            let context = [
                WellChipPart::Text("opus[1m]", None),
                WellChipPart::Text("79%", Some(Ink::Caption)),
            ];
            let r = flow.cell(list, "Meter and text", 90.0, WELL_CHIP_HEIGHT);
            WellChip::new(&meter).draw(r.x, r.y, list, &s, &input);
            let r = flow.cell(list, "Open (accent edge)", 110.0, WELL_CHIP_HEIGHT);
            WellChip::new(&context)
                .open(true)
                .draw(r.x, r.y, list, &s, &input);
        }

        flow.section(
            list,
            Category::Data,
            "Waffle",
            "share grid, hover fades the rest",
        );
        {
            let categories = [
                WaffleCategory {
                    name: "System prompt",
                    amount: "3k",
                    percent: "1.5%",
                    fill: WaffleFill::Solid([0.45, 0.62, 0.8, 1.0]),
                    aside: false,
                },
                WaffleCategory {
                    name: "Messages",
                    amount: "120k",
                    percent: "60.0%",
                    fill: WaffleFill::Solid([0.55, 0.5, 0.85, 1.0]),
                    aside: false,
                },
                WaffleCategory {
                    name: "Free space",
                    amount: "77k",
                    percent: "38.5%",
                    fill: WaffleFill::Hatched,
                    aside: true,
                },
            ];
            let mut cells = [2u8; 100];
            cells[..2].fill(0);
            cells[2..62].fill(1);
            let waffle = Waffle::new(&categories, &cells, 10);
            let r = flow.cell(list, "10×10 with legend", 340.0, 124.0);
            waffle.draw(r.x, r.y, r.width, list, &s, &input);
            let r = flow.cell(list, "Hover fades the rest", 340.0, 124.0);
            waffle
                .hovered(Some(1))
                .draw(r.x, r.y, r.width, list, &s, &input);
        }

        flow.section(list, Category::Chrome, "Band", "not in Forge");
        {
            let r = flow.cell(list, "Toolbar band, bottom edge", 300.0, 34.0);
            toolbar_band(list, &s, r, wgpu_gameui::Edge::Bottom);
            let r = flow.cell(list, "Toolbar band, top edge", 300.0, 34.0);
            toolbar_band(list, &s, r, wgpu_gameui::Edge::Top);
        }

        flow.section(list, Category::Forms, "VectorField", "");
        {
            let r_vec = flow.cell(list, "Position · rotation", 280.0, 46.0);
            let mut vctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let mut scrub: Option<VectorScrub> = None;
            let mut capture = DragCapture::new();
            let rows = [
                ("Position", [8.90f32, 12.90, 9.00]),
                ("Rotation", [0.00, 45.00, 0.00]),
            ];
            let _ = VectorField::new(&rows).draw(r_vec, &mut scrub, &mut capture, 900, &mut vctx);
        }

        flow.section(list, Category::Forms, "TagInput", "");
        {
            // Committed tags + an empty draft.
            let r_tags = flow.cell(list, "Tags", 220.0, 48.0);
            let mut tctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let tags = vec!["occlusion".to_string(), "static".to_string()];
            let mut draft = String::new();
            let _ = draw_tag_input(r_tags, &tags, &mut draft, false, &mut tctx);
        }

        flow.section(list, Category::Forms, "ComboBox", "combo trigger");
        {
            let r_combo = flow.cell(list, "Combo", 170.0, 24.0);
            let mut cctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let _ = draw_combo_trigger(r_combo, "Standard Lit", false, false, &mut cctx);
        }

        flow.section(list, Category::Layout, "DocumentTabs", "");
        {
            let r_docs = flow.cell(list, "Doc tabs", 300.0, 24.0);
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

        flow.section(list, Category::Data, "AssetGrid", "");
        {
            let assets = ["Crate_A", "Barrel", "Lamp_Post", "Bridge_A"];
            let r = flow.cell(list, "First selected", 380.0, 96.0);
            let _ = AssetGrid::new(&assets, "▣").draw(r, 0, &mut {
                DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
            });
        }

        flow.section(list, Category::Feedback, "Skeleton", "");
        let r = flow.cell(list, "Shimmer at 0.3", 120.0, 10.0);
        skeleton(list, &s, r, 0.3);

        flow.section(list, Category::Feedback, "Spinner", "");
        let r = flow.cell(list, "Spinner", 24.0, 24.0);
        spinner(list, &s, (r.x + 12.0, r.y + 12.0), 8.0, 0.4, 1.7);

        flow.section(list, Category::Feedback, "BusyDots", "dots");
        let r = flow.cell(list, "Dots", 40.0, 24.0);
        dots(list, &s, (r.x + 12.0, r.y + 12.0), 0.3);

        flow.section(list, Category::Feedback, "EmptyState", "");
        {
            // The design's own example, the agent-ui main area, and an icon
            // glyph.
            let r = flow.cell(
                list,
                "Empty state — hint · action",
                260.0,
                EmptyState::HEIGHT,
            );
            EmptyState::new()
                .title("No entities in selection")
                .hint("Select something in the viewport, or")
                .action("Create Entity")
                .draw(r, &mut ctx(list, &mut focus, &theme, &input));
            let r = flow.cell(list, "Empty state — glyph", 300.0, EmptyState::HEIGHT);
            EmptyState::new()
                .glyph("◫")
                .title("No session selected")
                .hint("Pick a session, or start a new one with +")
                .draw(r, &mut ctx(list, &mut focus, &theme, &input));
            #[cfg(feature = "phosphor-icons")]
            {
                let r = flow.cell(list, "Empty state — icon", 200.0, 120.0);
                EmptyState::new()
                    .icon(PhosphorIcon::MagnifyingGlass)
                    .title("Nothing matches \u{201c}zz\u{201d}")
                    .draw(r, &mut ctx(list, &mut focus, &theme, &input));
            }
        }

        flow.section(list, Category::Editors, "GradientRamp", "");
        {
            // Three stops, the middle one selected.
            let r_ramp = flow.cell(list, "Ramp (handles above)", 240.0, 42.0);
            let mut gctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let stops = [
                GradientStop::new(0.0, [0.05, 0.1, 0.12, 1.0]),
                GradientStop::new(0.45, [0.24, 0.75, 0.78, 1.0]),
                GradientStop::new(1.0, [0.95, 0.97, 0.98, 1.0]),
            ];
            let mut drag: Option<usize> = None;
            let mut capture = DragCapture::new();
            let _ = draw_gradient_ramp(r_ramp, &stops, 1, &mut drag, &mut capture, 901, &mut gctx);
        }

        flow.section(list, Category::Editors, "CurveEditor", "");
        {
            let r_curve = flow.cell(list, "Curve", 170.0, 110.0);
            let mut dctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
            let keys = [[0.0f32, 0.0], [0.35, 0.65], [0.7, 0.8], [1.0, 1.0]];
            let mut cdrag: Option<usize> = None;
            let mut ccapture = DragCapture::new();
            let _ = draw_curve_editor(r_curve, &keys, 1, &mut cdrag, &mut ccapture, 902, &mut dctx);
        }

        flow.section(
            list,
            Category::Layout,
            "Popover",
            "titled sheet above an anchor",
        );
        {
            // Drawn pointing up at an anchor stub. The cell is sized from the
            // measured sheet so the popover never covers the cell label.
            let popover_lines = ["Enter a new name for the selected entity."];
            let popover_width = 212.0;
            let popover_height = wgpu_gameui::measure_sheet_height(
                popover_width,
                "Rename",
                &popover_lines,
                list,
                &s,
            );
            let anchor_h = 20.0;
            let r_pop = flow.cell(
                list,
                "Popover",
                240.0,
                popover_height + 6.0 + anchor_h + 4.0,
            );
            let anchor = Rect::new(
                r_pop.x + 90.0,
                r_pop.bottom() - anchor_h - 4.0,
                60.0,
                anchor_h,
            );
            list.rounded_rect(anchor, 1.0, [0.16, 0.19, 0.22, 1.0]);
            list.text(
                TextBlock::new("Anchor", anchor.x + 8.0, anchor.y + 4.0)
                    .with_size(11.0)
                    .with_color(190, 200, 220),
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

            // The bare frame: title bar and close key, the content left to
            // the caller (here a meter row), opening below its anchor.
            let r = flow.cell(list, "Frame (custom content, below)", 240.0, 96.0);
            let frame = draw_popover_frame(
                Rect::new(r.x, r.y + 6.0, 240.0, 90.0),
                PopoverSide::Below,
                "rate limit usage",
                list,
                &s,
            );
            let c = frame.content;
            list.text(s.mono_block("5h", c.x, c.y, TextSize::Dense, Ink::Title));
            inline_meter(
                list,
                &s,
                Rect::new(c.x, c.y + 18.0, c.width, 6.0),
                MeterFill::neutral(0.07, &s).pace(0.4),
            );
        }

        // --- Banners & toasts ----------------------------------------------
        // Severity banners (info/success/warning/error) and a corner toast stack.
        // The toast stack normally anchors to the screen; here it lays out in
        // a reserved cell so it shows inline.
        flow.section(list, Category::Feedback, "Banner", "severities");
        {
            let style = StyleResolver::new(&theme);

            let banners: [(&str, Banner); 4] = [
                ("Info", Banner::info("A new version is available.")),
                (
                    "Success, titled",
                    Banner::success("Your settings were saved.").with_title("Saved"),
                ),
                ("Warning", Banner::warning("Low disk space (1.2 GB left).")),
                (
                    "Error, titled",
                    Banner::error("Connection lost. Retrying…").with_title("Error"),
                ),
            ];
            for (label, banner) in banners {
                // Size the cell to the banner's natural height so titled (two-line)
                // banners aren't clipped.
                let h = banner.measure_height(list, &style, 300.0);
                let r = flow.cell(list, label, 300.0, h);
                banner.draw(r, list, &style);
            }
        }

        flow.section(list, Category::Feedback, "Toast", "corner stack");
        {
            // Inline toast stack: a faint backdrop stands in for the screen,
            // and the cell is the area the stack lays out in.
            let r = flow.cell(list, "Toast stack (top-right)", 320.0, 250.0);
            list.quad(r.x, r.y, r.width, r.height, [0.09, 0.10, 0.13, 1.0]);
            list.rect_outline(r, 1.0, [0.25, 0.28, 0.34, 1.0]);
            let mut stack = ToastStack::new()
                .with_corner(Corner::TopRight)
                .with_width(232.0)
                .with_margin(10.0);
            stack.push(
                Toast::new(Severity::Success, "12 meshes written to /build")
                    .with_title("Export finished"),
            );
            stack.push(Toast::new(Severity::Info, "New update available (v1.2)"));
            stack.push(
                Toast::error("missing field `message_count` at line 1 column 187")
                    .with_title("Conversations")
                    .until_dismissed(),
            );
            let input = InputState::default();
            stack.draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

        // --- Toolbar ---------------------------------------------------------
        #[cfg(feature = "phosphor-icons")]
        {
            // One section for the toolbar; this block and the next two add
            // its key states, docked rails, and hover tooltips.
            flow.section(
                list,
                Category::Keys,
                "Toolbar",
                "03-toolbar.html: key states, docked rails, tooltips",
            );
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

        #[cfg(feature = "phosphor-icons")]
        {
            // Still the Toolbar section: docked rails.
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

        #[cfg(feature = "phosphor-icons")]
        {
            // Still the Toolbar section: a tooltip on hover.
            use wgpu_gameui::{DragCapture, Toolbar};

            let toolbar = Toolbar::new(&tip_toolbar_items);
            let btn = theme.toolbar_button_size;
            let pad = theme.toolbar_padding;
            // The hovered key is the second one (Move, as in the handoff). It
            // starts one key past the first key's preferred prefix.
            let prefix = Toolbar::new(&tip_toolbar_items[..1]);
            let labels = ["Docked left — hovering Move", "Docked top — hovering Snap"];
            for (index, state) in tip_toolbars.iter_mut().enumerate() {
                let edge = state.edge;
                let vertical = edge.is_vertical();
                let cross = toolbar.preferred_cross_for_edge(btn, pad, edge);
                let extent = toolbar.preferred_extent_for_edge(btn, pad, edge);
                // Reserve the tooltip's room on the strip's outward side so it
                // does not paint over the neighbouring cell.
                let (w, h) = if vertical {
                    (cross + 80.0, extent)
                } else {
                    (extent + 40.0, cross + 34.0)
                };
                let cell = flow.cell(list, labels[index], w, h);
                let rail = if vertical {
                    Rect::new(cell.x, cell.y, cross, extent)
                } else {
                    Rect::new(cell.x, cell.y, extent, cross)
                };
                let (hover_id, mouse) = if vertical {
                    let start = prefix.preferred_extent_for_edge(btn, pad, edge) - pad + 2.0;
                    (2, [rail.x + cross * 0.5, rail.y + start + btn * 0.5])
                } else {
                    // Snap is the last key; hover the middle of its face.
                    let end = extent - pad;
                    (5, [rail.x + end - btn * 0.5, rail.y + cross * 0.5])
                };
                let hover = InputState {
                    mouse_x: mouse[0],
                    mouse_y: mouse[1],
                    ..Default::default()
                };
                let mut capture = DragCapture::new();
                // The tooltip clamps into the screen; the gallery is far taller
                // than one window, so give it the whole canvas.
                let mut tctx =
                    DrawContext::new(list, &mut focus, &theme, &hover, W as f32, 100_000.0);
                let out = toolbar.draw(rail, state, &mut capture, 0x7C00 + index as u64, &mut tctx);
                assert_eq!(out.hovered, Some(hover_id), "{}", labels[index]);
            }
        }

        // --- App shell -------------------------------------------------------
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(list, Category::Chrome, "AppShell", "mini layout");
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
        flow.section(
            list,
            Category::Chrome,
            "DockPanel",
            "04-dock-panel.html tabs",
        );
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
            let dock_input = InputState {
                mouse_x: r.x + 120.0,
                mouse_y: r.y + theme.dock_tab_height * 0.5,
                ..InputState::default()
            };
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

        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(
                list,
                Category::Keys,
                "IconKey",
                "sizes 17 / 18 / 24 · tones · states",
            );
            use wgpu_gameui::{IconKey, Tone};

            // Each key gets its own synthetic pointer, so hover and pressed
            // show next to idle, held and disabled in the static gallery.
            let tones = [
                ("default", Tone::Default),
                ("ghost", Tone::Ghost),
                ("accent", Tone::Accent),
                ("sunken", Tone::Sunken),
            ];
            let states = ["idle", "hover", "pressed", "held", "disabled"];
            for size in [IconKey::HEADER, IconKey::STATUS, IconKey::TOOLBAR] {
                for (tone_name, tone) in tones {
                    let cell_w = states.len() as f32 * (size + 6.0);
                    let label = format!("{tone_name} {size}");
                    let r = flow.cell(list, &label, cell_w, size + 2.0);
                    for (i, state) in states.iter().enumerate() {
                        let key_rect =
                            Rect::new(r.x + i as f32 * (size + 6.0), r.y, size, size + 2.0);
                        let mut key_input = InputState::default();
                        if matches!(*state, "hover" | "pressed") {
                            key_input.mouse_x = key_rect.x + size * 0.5;
                            key_input.mouse_y = key_rect.y + size * 0.5;
                            key_input.mouse_down = *state == "pressed";
                        }
                        let mut kctx =
                            DrawContext::new(list, &mut focus, &theme, &key_input, W as f32, 600.0);
                        IconKey::new(PhosphorIcon::ArrowsClockwise, size)
                            .tone(tone)
                            .held(*state == "held")
                            .enabled(*state != "disabled")
                            .draw(key_rect, &mut kctx);
                    }
                }
            }

            // Text-glyph keys (Forge `IconKey glyph="■"`): the 15px inline
            // ghost keys of dock cards, travel 1.
            let glyphs = ["■", "›", "⋯", "+"];
            let r = flow.cell(
                list,
                "glyph keys, ghost 15",
                glyphs.len() as f32 * 21.0,
                16.0,
            );
            for (i, glyph) in glyphs.into_iter().enumerate() {
                let key_rect = Rect::new(r.x + i as f32 * 21.0, r.y, 15.0, 16.0);
                let mut kctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
                IconKey::glyph(glyph, 15.0)
                    .tone(Tone::Ghost)
                    .travel(1.0)
                    .draw(key_rect, &mut kctx);
            }
            let r = flow.cell(list, "glyph keys, default 24", 2.0 * 30.0, 26.0);
            for (i, glyph) in ["■", "›"].into_iter().enumerate() {
                let key_rect = Rect::new(r.x + i as f32 * 30.0, r.y, 24.0, 26.0);
                let mut kctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0);
                IconKey::glyph(glyph, 24.0).draw(key_rect, &mut kctx);
            }
        }

        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(
                list,
                Category::Forms,
                "SearchField",
                "empty · typed · focused · clear key hovered",
            );
            use wgpu_gameui::SearchField;

            const SEARCH_ID: u64 = 0x5EA2;
            let cases = [
                ("empty", "", false, false),
                ("typed", "agent", false, false),
                ("focused", "agent", true, false),
                ("clear key hovered", "agent-ui", false, true),
            ];
            for (label, text, focused, hover_clear) in cases {
                let r = flow.cell(list, label, 220.0, SearchField::HEIGHT);
                let mut field = TextInput::default()
                    .with_placeholder("Filter projects")
                    .with_value(text);
                let mut field_focus = FocusState::new();
                if focused {
                    field_focus.request(SEARCH_ID);
                }
                let mut field_input = InputState::default();
                if hover_clear {
                    // The clear key's centre: 4 px + border from the right edge.
                    field_input.mouse_x = r.x + r.width - 13.0;
                    field_input.mouse_y = r.y + r.height * 0.5;
                }
                let mut sctx = DrawContext::new(
                    list,
                    &mut field_focus,
                    &theme,
                    &field_input,
                    W as f32,
                    600.0,
                );
                SearchField::new().draw(&mut field, SEARCH_ID, r, &mut sctx);
            }
        }

        // ---- DockStack (the design's sidebar sections) ------------------
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(
                list,
                Category::Sidebar,
                "DockStack",
                "sections with toolbar · collapsed · hovered header · fixed · splitter hovered",
            );
            use wgpu_gameui::{
                DockSection, DockStack, DockStackState, ListRow, ListView, SearchField,
            };

            let projects = [
                "LuaJIT",
                "agent-ui",
                "citybuilder",
                "claude-code-ui",
                "codex-adventure",
                "cool-rust-terminal",
                "cross-notifier",
            ];
            let sessions = [
                ("Fix websocket reconnect loop", "01a01043 · 38 msgs", "2h"),
                ("Session list virtualisation", "01a01043 · 12 msgs", "5h"),
                ("Add dark scrollbar to composer", "01a0106b · 21 msgs", "1d"),
                ("Tauri build fails on Fedora", "3b504918 · 64 msgs", "2d"),
            ];
            let rescan = [PhosphorIcon::ArrowsClockwise];
            let plus = [PhosphorIcon::Plus];
            let sidebar = [
                DockSection::new("Projects")
                    .count(projects.len())
                    .toolbar(SearchField::HEIGHT)
                    .actions(&rescan),
                DockSection::new("Sessions")
                    .count(sessions.len())
                    .weight(1.3)
                    .actions(&plus),
            ];
            let sidebar_collapsed = [sidebar[0], sidebar[1].collapsed(true)];
            let with_details = [
                DockSection::new("Outliner").count(12),
                DockSection::new("Layers").count(3),
                DockSection::new("Details").fixed(36.0),
            ];

            const STACK_ID: u64 = 0xD0C5;
            const SEARCH_ID: u64 = 0x5EA3;
            // (label, sections, pointer)
            type Pointer = fn(Rect) -> (f32, f32);
            let cases: [(&str, &[DockSection], Pointer); 3] = [
                ("Projects + Sessions", &sidebar, |_| (-1.0, -1.0)),
                ("Collapsed · header hovered", &sidebar_collapsed, |r| {
                    (r.x + 60.0, r.y + 11.0)
                }),
                ("Fixed Details · splitter hovered", &with_details, |_| {
                    (-1.0, -1.0)
                }),
            ];
            for (i, (label, sections, pointer)) in cases.into_iter().enumerate() {
                let r = flow.cell(list, label, 244.0, 300.0);
                list.paint_background_opaque(r, theme.chrome.dock.body.background);
                let mut state = DockStackState::new(sections);
                let mut capture = DragCapture::new();
                let mut stack_focus = FocusState::new();
                let mut stack_input = InputState {
                    mouse_x: -1.0,
                    mouse_y: -1.0,
                    ..InputState::default()
                };
                (stack_input.mouse_x, stack_input.mouse_y) = pointer(r);
                if i == 2 {
                    // Point at the grip of the splitter between the first two.
                    let at =
                        DockStack::new(sections).layout(r, &state, &StyleResolver::new(&theme));
                    let bar = at[1].splitter_above.expect("a splitter");
                    stack_input.mouse_x = bar.x + bar.width * 0.5;
                    stack_input.mouse_y = bar.y + bar.height * 0.5;
                }
                let out = {
                    let mut sctx = DrawContext::new(
                        list,
                        &mut stack_focus,
                        &theme,
                        &stack_input,
                        W as f32,
                        600.0,
                    );
                    DockStack::new(sections).draw(STACK_ID, r, &mut state, &mut capture, &mut sctx)
                };

                if i < 2 {
                    if let Some(bar) = out.sections[0].toolbar {
                        let mut field = TextInput::default().with_placeholder("Search projects…");
                        let mut sctx = DrawContext::new(
                            list,
                            &mut stack_focus,
                            &theme,
                            &stack_input,
                            W as f32,
                            600.0,
                        );
                        SearchField::new().draw(&mut field, SEARCH_ID, bar, &mut sctx);
                    }
                    let mut idle = InputState {
                        mouse_x: -1.0,
                        mouse_y: -1.0,
                        ..InputState::default()
                    };
                    let resolver = StyleResolver::new(&theme);
                    if let Some(body) = out.sections[0].body {
                        ListView::new().draw(
                            body,
                            projects.len(),
                            Some(3),
                            &mut ListState::new(),
                            list,
                            &resolver,
                            &mut idle,
                            |k| ListRow::new(projects[k]).glyph(PhosphorIcon::Folder),
                        );
                    }
                    if let Some(body) = out.sections[1].body {
                        ListView::new().two_line(true).focused(true).draw(
                            body,
                            sessions.len(),
                            Some(0),
                            &mut ListState::new(),
                            list,
                            &resolver,
                            &mut idle,
                            |k| {
                                let (title, sub, meta) = sessions[k];
                                ListRow::new(title).subtitle(sub).meta(meta)
                            },
                        );
                    }
                }
            }
        }

        // ---- DockSection: one section on its own, per header state -------
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(
                list,
                Category::Sidebar,
                "DockSection",
                "22 px raised header · count · ghost keys · toolbar strip",
            );
            use wgpu_gameui::{DockSection, DockStack, DockStackState, SearchField};

            let plus = [PhosphorIcon::Plus];
            let rescan = [PhosphorIcon::ArrowsClockwise, PhosphorIcon::Plus];
            const SECTION_ID: u64 = 0xD5EC;
            const SECTION_SEARCH_ID: u64 = 0xD5EA;
            // (label, section, cell height, pointer on the header)
            let cases = [
                (
                    "Count · one action",
                    DockSection::new("Sessions").count(12).actions(&plus),
                    96.0,
                    false,
                ),
                (
                    "Toolbar strip · two actions",
                    DockSection::new("Projects")
                        .count(24)
                        .toolbar(SearchField::HEIGHT)
                        .actions(&rescan),
                    130.0,
                    false,
                ),
                (
                    "Collapsed",
                    DockSection::new("Layers").count(3).collapsed(true),
                    22.0,
                    false,
                ),
                (
                    "Header hovered",
                    DockSection::new("Outliner").count(128),
                    96.0,
                    true,
                ),
            ];
            for (label, section, h, hovered) in cases {
                let r = flow.cell(list, label, 244.0, h);
                list.paint_background_opaque(r, theme.chrome.dock.body.background);
                let sections = [section];
                let mut state = DockStackState::new(&sections);
                let mut capture = DragCapture::new();
                let mut section_focus = FocusState::new();
                let section_input = InputState {
                    mouse_x: if hovered { r.x + 80.0 } else { -1.0 },
                    mouse_y: if hovered { r.y + 11.0 } else { -1.0 },
                    ..InputState::default()
                };
                let mut sctx = DrawContext::new(
                    list,
                    &mut section_focus,
                    &theme,
                    &section_input,
                    W as f32,
                    600.0,
                );
                let out = DockStack::new(&sections).draw(
                    SECTION_ID,
                    r,
                    &mut state,
                    &mut capture,
                    &mut sctx,
                );
                if let Some(bar) = out.sections[0].toolbar {
                    let mut field = TextInput::default().with_placeholder("Search projects…");
                    SearchField::new().draw(&mut field, SECTION_SEARCH_ID, bar, &mut sctx);
                }
                if let Some(body) = out.sections[0].body {
                    Placeholder::text(4).draw(body.inset(8.0), sctx.draw_list, &s);
                }
            }
        }

        flow.section(
            list,
            Category::Windows,
            "FloatingWindow",
            "movable, closable, optional resize grip",
        );
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
            let window_input = InputState {
                mouse_x: key.x + key.width * 0.5,
                mouse_y: key.y + key.height * 0.5,
                ..InputState::default()
            };
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
        flow.section(list, Category::Engine, "BackdropBlur", "UiBlur");
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
        flow.section(list, Category::Engine, "HoverAnimation", "ease-out curve");
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
        // headroom below, overlapping no other widget. The reserve keeps the
        // popup inside the section's image.
        flow.section(list, Category::Feedback, "Tooltip", "TooltipLayer");
        let r = flow.cell(list, "Tooltip target", 120.0, 24.0);
        flow.reserve(90.0);
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
        gallery_cells = flow.cells;
    }

    // The laid-out canvas height, so the tooltip layer knows the real screen
    // height (it flips the popup up/left near the edges). Nothing is rendered
    // at this size: the canvas is drawn in pages (see `page_spans`).
    let h = (content_bottom.ceil() as u32).max(64);

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

    // Hovered toolbars: each paints its key's tooltip on a tooltip layer.
    #[cfg(feature = "phosphor-icons")]
    {
        let styles = StyleResolver::new(&theme);
        for state in &mut tip_toolbars {
            state.draw_open_layer(
                &mut layers,
                None,
                &tip_toolbar_items,
                &styles,
                &InputState::default(),
            );
        }
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

    // --- Backdrop-blur "scene" -------------------------------------------------
    // Stand in for the app's rendered game: a page-sized texture with vivid
    // colored stripes + text inside the reserved blur cell. `blur_backdrop` then
    // samples this and writes a blurred copy into the cell, with a crisp panel on
    // top — exactly the pause-menu flow (render scene → blur → draw UI).
    let mut scene_list = DrawList::new();
    {
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
    }

    // The crisp "PAUSED" panel drawn over the blurred cell — UI rendered after
    // the blur sits sharp.
    let mut panel_list = DrawList::new();
    {
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
    }

    // No full-canvas image: the canvas is drawn in pages of at most
    // `PAGE_MAX` rows, split between sections, and every section and cell
    // image is cut from the one page that holds it.
    let scene = GalleryScene {
        device: &device,
        queue: &queue,
        format,
        clear: ui.clear_color(theme.background),
        layers: &layers,
        backdrop: &scene_list,
        blur_rect,
        panel: &panel_list,
    };
    let pages: Vec<GalleryPage> = page_spans(&gallery_sections)
        .into_iter()
        .map(|(top, bottom)| render_gallery_page(&mut ui, &scene, top, bottom))
        .collect();
    save_gallery_images(&pages, &gallery_sections, &gallery_cells);

    // Sanity: at least some pixels are not the theme clear color.
    let [cr, cg, cb, _] = wgpu_gameui::color::to_rgba8(theme.background);
    let clear = [cr, cg, cb];
    let drew = pages.iter().flat_map(|page| page.img.pixels()).any(|p| {
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
