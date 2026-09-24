//! Fixed-size values for composable surface backgrounds, borders, and edge lines.

use crate::DrawList;
use crate::layout::Rect;
use crate::shadow::{BoxShadow, CornerRadii};

/// Axis of a two-stop linear background gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientAxis {
    /// Gradient runs from the left edge to the right edge.
    Horizontal,
    /// Gradient runs from the top edge to the bottom edge.
    Vertical,
}

/// A fixed-size quad background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// One straight sRGB-encoded RGBA color (see [`crate::color`]).
    Solid([f32; 4]),
    /// A two-stop linear gradient spanning the quad along `axis`.
    LinearGradient {
        /// Color at the left or top edge.
        start: [f32; 4],
        /// Color at the right or bottom edge.
        end: [f32; 4],
        /// Direction in which the two colors are interpolated.
        axis: GradientAxis,
    },
}

impl Default for Background {
    fn default() -> Self {
        Self::Solid([0.0; 4])
    }
}

/// Border widths in clockwise edge order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EdgeWidths {
    /// Top width, growing inward.
    pub top: f32,
    /// Right width, growing inward.
    pub right: f32,
    /// Bottom width, growing inward.
    pub bottom: f32,
    /// Left width, growing inward.
    pub left: f32,
}

impl EdgeWidths {
    /// Construct four independent inward-growing edge widths.
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Use one width for every edge.
    pub const fn uniform(width: f32) -> Self {
        Self::new(width, width, width, width)
    }

    /// Return widths in top, right, bottom, left order.
    pub const fn as_array(self) -> [f32; 4] {
        [self.top, self.right, self.bottom, self.left]
    }
}

impl From<f32> for EdgeWidths {
    fn from(value: f32) -> Self {
        Self::uniform(value)
    }
}

/// One explicit structural edge line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeStyle {
    /// Line thickness in local logical pixels.
    pub thickness: f32,
    /// Straight sRGB-encoded RGBA color.
    pub color: [f32; 4],
}

/// A rectangle edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Top edge.
    Top,
    /// Right edge.
    Right,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
}

/// Fixed-size style for one composable quad.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuadStyle {
    /// Surface background.
    pub background: Background,
    /// Border widths, each growing inward from its edge.
    pub border_widths: EdgeWidths,
    /// Straight sRGB-encoded RGBA border color shared by all edges.
    pub border_color: [f32; 4],
    /// Outer corner radii, clockwise from the top-left.
    pub corner_radii: CornerRadii,
}

/// One structural line painted after a surface border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StructuralLine {
    /// Edge from which the line's inward offset is measured.
    pub edge: Edge,
    /// Distance inward from `edge`, in local logical pixels.
    pub offset: f32,
    /// Thickness and color of the line.
    pub style: EdgeStyle,
}

/// Finite menu-bar chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuBarChrome {
    /// Bar surface and border.
    pub surface: QuadStyle,
    /// Authored outer elevation.
    pub shadows: [BoxShadow; 1],
    /// Top highlight and bottom edge.
    pub lines: [StructuralLine; 2],
}

/// Finite menu-sheet and context-menu chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuSheetChrome {
    /// Sheet surface and border.
    pub surface: QuadStyle,
    /// Broad and contact shadows in CSS declaration order.
    pub shadows: [BoxShadow; 2],
    /// Top highlight and bottom shade.
    pub lines: [StructuralLine; 2],
    /// Dark/light separator pair.
    pub separator: [EdgeStyle; 2],
}

/// Finite toolbar rail, tool-face, separator, and popup chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarChrome {
    /// Rail gradient colors; widgets select the axis from their dock edge.
    pub rail_colors: [[f32; 4]; 2],
    /// Dock-edge rule.
    pub dock_edge: EdgeStyle,
    /// Inner highlight next to the dock edge.
    pub dock_highlight: EdgeStyle,
    /// Rail elevation away from the dock edge.
    pub rail_shadow: BoxShadow,
    /// Resting tool face.
    pub tool_idle: QuadStyle,
    /// Hovered tool face.
    pub tool_hover: QuadStyle,
    /// Pressed tool face.
    pub tool_pressed: QuadStyle,
    /// Latched tool face.
    pub tool_latched: QuadStyle,
    /// Per-state authored inset shadows: idle, hover, pressed, latched.
    pub tool_insets: [[BoxShadow; 2]; 4],
    /// Resting, hovered, and dragging grip colors.
    pub grip_colors: [[f32; 4]; 3],
    /// Grip counter-edge.
    pub grip_counter_edge: EdgeStyle,
    /// Toolbar popup surface, shadows, and inset edge lines. The authored dock
    /// chooser uses the same CSS sheet recipe as menus.
    pub popup: MenuSheetChrome,
    /// Dark/light separator pair.
    pub separator: [EdgeStyle; 2],
}

/// Finite dock-panel body, header, and tab chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockChrome {
    /// Panel body.
    pub body: QuadStyle,
    /// Header surface.
    pub header: QuadStyle,
    /// Header and tab structural rules.
    pub lines: [StructuralLine; 3],
    /// Active tab face.
    pub active_tab: QuadStyle,
    /// Hover wash for an inactive tab or header key.
    pub tab_hover: QuadStyle,
    /// Authored inset on the active tab face.
    pub active_tab_inset: BoxShadow,
}

/// Finite splitter track, grip, and dragging glow chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterChrome {
    /// Track gradient colors; widgets select the axis from orientation.
    pub track_colors: [[f32; 4]; 2],
    /// Dark outside edges.
    pub outer_edges: EdgeStyle,
    /// Inner highlight.
    pub inner_highlight: EdgeStyle,
    /// Idle grip color.
    pub grip_idle: [f32; 4],
    /// Hovered grip color.
    pub grip_hover: [f32; 4],
    /// Dragging grip color.
    pub grip_dragging: [f32; 4],
    /// Grip counter-edge.
    pub grip_counter_edge: EdgeStyle,
    /// Symmetric authored dragging glow.
    pub dragging_glow: BoxShadow,
}

/// Finite status-bar surface and divider chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusBarChrome {
    /// Status-bar surface.
    pub surface: QuadStyle,
    /// Top dark edge and highlight.
    pub lines: [StructuralLine; 2],
    /// Dark/light cell divider pair.
    pub divider: [EdgeStyle; 2],
    /// Authored inset for a latched status toggle.
    pub latched_inset: BoxShadow,
}

/// One floating surface with exactly one authored elevation shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingSurfaceChrome {
    /// Floating surface and border.
    pub surface: QuadStyle,
    /// Authored elevation.
    pub shadow: BoxShadow,
    /// Structural edge lines (e.g. a top inset highlight). Up to two;
    /// unused slots have zero-alpha color and are effectively no-ops.
    pub lines: [StructuralLine; 2],
}

/// Movable in-app window chrome: a floating surface with a title strip, a
/// close key, and an optional bottom-right resize grip. See
/// [`Window`](crate::Window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowChrome {
    /// Window surface and border (the whole outer rect).
    pub surface: QuadStyle,
    /// Authored elevation.
    pub shadow: BoxShadow,
    /// Structural lines on the outer surface (e.g. a top inset highlight).
    pub lines: [StructuralLine; 2],
    /// Title-strip surface, painted inside the outer border.
    pub header: QuadStyle,
    /// Title-strip structural rules (highlight and the divider under it).
    pub header_lines: [StructuralLine; 3],
    /// Hover wash behind the close key.
    pub close_hover: QuadStyle,
    /// Resting and hovered/dragging colors of the resize grip marks.
    pub grip_colors: [[f32; 4]; 2],
}

/// All finite component chrome families in the default design language.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromeTheme {
    /// Menu-bar chrome.
    pub menu_bar: MenuBarChrome,
    /// Menu-sheet and context-menu chrome.
    pub menu_sheet: MenuSheetChrome,
    /// Toolbar chrome.
    pub toolbar: ToolbarChrome,
    /// Dock-panel chrome.
    pub dock: DockChrome,
    /// Splitter chrome.
    pub splitter: SplitterChrome,
    /// Status-bar chrome.
    pub status_bar: StatusBarChrome,
    /// Dropdown popup chrome.
    pub dropdown: FloatingSurfaceChrome,
    /// Popover chrome.
    pub popover: FloatingSurfaceChrome,
    /// Tooltip chrome.
    pub tooltip: FloatingSurfaceChrome,
    /// Toast chrome.
    pub toast: FloatingSurfaceChrome,
    /// Curve-editor key chrome.
    pub curve_key: FloatingSurfaceChrome,
    /// Movable window chrome.
    pub window: WindowChrome,
}

/// Allocation-free staged painter for one explicitly boxed component surface.
///
/// [`Self::paint_pre_content`] records reverse-stacked outset shadows, the
/// background, then reverse-stacked inset shadows and returns the padding box.
/// After content is recorded, [`Self::paint_post_content`] records the border
/// and structural lines.
pub struct SurfacePainter<'a> {
    list: &'a mut DrawList,
    border_box: Rect,
    padding_box: Rect,
    padding_radii: CornerRadii,
    style: QuadStyle,
    shadows: &'a [BoxShadow],
    lines: &'a [StructuralLine],
}

impl<'a> SurfacePainter<'a> {
    /// Create a painter with explicit border-box and padding-box geometry.
    pub fn new(
        list: &'a mut DrawList,
        border_box: Rect,
        padding_box: Rect,
        padding_radii: CornerRadii,
        style: QuadStyle,
        shadows: &'a [BoxShadow],
        lines: &'a [StructuralLine],
    ) -> Self {
        Self {
            list,
            border_box,
            padding_box,
            padding_radii,
            style,
            shadows,
            lines,
        }
    }

    /// Paint pre-content layers and return the explicit padding/content box.
    pub fn paint_pre_content(&mut self) -> Rect {
        self.list
            .box_shadows_outset(self.border_box, self.style.corner_radii, self.shadows);
        self.list.paint_quad_background(
            self.border_box,
            self.style.background,
            self.style.corner_radii,
        );
        self.list
            .box_shadows_inset(self.padding_box, self.padding_radii, self.shadows);
        self.padding_box
    }

    /// Like [`paint_pre_content`](Self::paint_pre_content) but renders the
    /// background as opaque triangle soup instead of the SDF rounded-rect
    /// pipeline.  Use for shell chrome surfaces (menu bar, toolbar rail,
    /// dock panels) that tile edge-to-edge and must not have semi-transparent
    /// boundary pixels.
    ///
    /// Outset and inset shadows still use the analytic SDF path — they paint
    /// on top of the opaque background, so their antialiased falloff blends
    /// against the solid surface, not the canvas.
    pub fn paint_pre_content_opaque(&mut self) -> Rect {
        self.list
            .box_shadows_outset(self.border_box, self.style.corner_radii, self.shadows);
        self.list
            .paint_background_opaque(self.border_box, self.style.background);
        self.list
            .box_shadows_inset(self.padding_box, self.padding_radii, self.shadows);
        self.padding_box
    }

    /// Borrow the target draw list to record content between the two stages.
    pub fn draw_list(&mut self) -> &mut DrawList {
        self.list
    }

    /// Paint the border followed by structural lines.
    pub fn paint_post_content(self) {
        self.list.paint_quad_border(
            self.border_box,
            self.style.border_widths,
            self.style.border_color,
            self.style.corner_radii,
        );
        for line in self.lines {
            let rect = inset_edge(self.border_box, line.edge, line.offset);
            self.list
                .edge_line(rect, line.edge, line.style.thickness, line.style.color);
        }
    }
}

fn inset_edge(rect: Rect, edge: Edge, amount: f32) -> Rect {
    let amount = amount.max(0.0);
    match edge {
        Edge::Top => Rect::new(
            rect.x,
            rect.y + amount,
            rect.width,
            (rect.height - amount).max(0.0),
        ),
        Edge::Right => Rect::new(rect.x, rect.y, (rect.width - amount).max(0.0), rect.height),
        Edge::Bottom => Rect::new(rect.x, rect.y, rect.width, (rect.height - amount).max(0.0)),
        Edge::Left => Rect::new(
            rect.x + amount,
            rect.y,
            (rect.width - amount).max(0.0),
            rect.height,
        ),
    }
}

impl Default for ChromeTheme {
    fn default() -> Self {
        use crate::color::rgb8;

        let gradient = |top, bottom| Background::LinearGradient {
            start: rgb8(top),
            end: rgb8(bottom),
            axis: GradientAxis::Vertical,
        };
        let quad = |background| QuadStyle {
            background,
            border_widths: EdgeWidths::default(),
            border_color: [0.0; 4],
            corner_radii: CornerRadii::uniform(1.0),
        };
        let bordered = |background, border| QuadStyle {
            background,
            border_widths: EdgeWidths::uniform(1.0),
            border_color: border,
            corner_radii: CornerRadii::uniform(1.0),
        };
        let line = |edge, offset, color| StructuralLine {
            edge,
            offset,
            style: EdgeStyle {
                thickness: 1.0,
                color,
            },
        };
        let shadow = |y, blur, color, inset| BoxShadow {
            offset: [0.0, y],
            blur,
            spread: 0.0,
            color,
            inset,
        };
        let no_lines: [StructuralLine; 2] = [
            StructuralLine {
                edge: Edge::Top,
                offset: 0.0,
                style: EdgeStyle {
                    thickness: 0.0,
                    color: [0.0; 4],
                },
            },
            StructuralLine {
                edge: Edge::Bottom,
                offset: 0.0,
                style: EdgeStyle {
                    thickness: 0.0,
                    color: [0.0; 4],
                },
            },
        ];
        let floating = |surface, y, blur, alpha| FloatingSurfaceChrome {
            surface,
            shadow: shadow(y, blur, [0.0, 0.0, 0.0, alpha], false),
            lines: no_lines,
        };
        let panel = bordered(
            Background::Solid(crate::color::rgba8([0x16, 0x19, 0x1d], 0.95)),
            [0.0, 0.0, 0.0, 0.7],
        );
        let menu_sheet = bordered(
            gradient([0x1d, 0x22, 0x27], [0x14, 0x18, 0x1c]),
            rgb8([0x03, 0x04, 0x05]),
        );
        Self {
            menu_bar: MenuBarChrome {
                surface: quad(gradient([0x25, 0x2b, 0x31], [0x17, 0x1b, 0x1f])),
                shadows: [shadow(2.0, 8.0, [0.0, 0.0, 0.0, 0.4], false)],
                lines: [
                    line(Edge::Top, 0.0, rgb8([0x3d, 0x42, 0x47])),
                    line(Edge::Bottom, 0.0, rgb8([0x03, 0x05, 0x06])),
                ],
            },
            menu_sheet: MenuSheetChrome {
                surface: menu_sheet,
                shadows: [
                    shadow(16.0, 40.0, [0.0, 0.0, 0.0, 0.7], false),
                    shadow(2.0, 6.0, [0.0, 0.0, 0.0, 0.5], false),
                ],
                lines: [
                    line(Edge::Top, 1.0, rgb8([0x38, 0x3d, 0x41])),
                    line(Edge::Bottom, 1.0, rgb8([0x0a, 0x0c, 0x0e])),
                ],
                separator: [
                    EdgeStyle {
                        thickness: 1.0,
                        color: rgb8([0x0a, 0x0c, 0x0d]),
                    },
                    EdgeStyle {
                        thickness: 1.0,
                        color: rgb8([0x26, 0x2b, 0x2f]),
                    },
                ],
            },
            toolbar: ToolbarChrome {
                rail_colors: [rgb8([0x23, 0x28, 0x2e]), rgb8([0x16, 0x1a, 0x1e])],
                dock_edge: EdgeStyle {
                    thickness: 1.0,
                    color: [0.0, 0.0, 0.0, 0.7],
                },
                dock_highlight: EdgeStyle {
                    thickness: 1.0,
                    color: [1.0, 1.0, 1.0, 0.05],
                },
                // Direction is supplied by the widget from the dock edge; this
                // token preserves the authored 2px elevation and 8px blur.
                rail_shadow: shadow(2.0, 8.0, [0.0, 0.0, 0.0, 0.35], false),
                tool_idle: bordered(
                    gradient([0x41, 0x44, 0x48], [0x2a, 0x2e, 0x33]),
                    rgb8([0x10, 0x12, 0x15]),
                ),
                tool_hover: bordered(
                    gradient([0x53, 0x56, 0x5a], [0x35, 0x39, 0x3e]),
                    rgb8([0x10, 0x12, 0x15]),
                ),
                tool_pressed: bordered(
                    gradient([0x31, 0x35, 0x39], [0x24, 0x29, 0x2d]),
                    rgb8([0x0b, 0x0d, 0x0f]),
                ),
                tool_latched: bordered(
                    gradient([0x4a, 0x8a, 0x9c], [0x5f, 0xa3, 0xb6]),
                    rgb8([0x0b, 0x0d, 0x0f]),
                ),
                tool_insets: [
                    [
                        shadow(1.0, 0.0, rgb8([0x63, 0x66, 0x69]), true),
                        BoxShadow::default(),
                    ],
                    [
                        shadow(1.0, 0.0, rgb8([0x83, 0x85, 0x88]), true),
                        BoxShadow::default(),
                    ],
                    [
                        shadow(1.0, 0.0, rgb8([0x3d, 0x41, 0x45]), true),
                        shadow(2.0, 3.0, [0.0, 0.0, 0.0, 0.4], true),
                    ],
                    [
                        shadow(2.0, 4.0, rgb8([0x1c, 0x46, 0x53]), true),
                        shadow(1.0, 0.0, rgb8([0x3f, 0x7a, 0x8b]), true),
                    ],
                ],
                grip_colors: [
                    [1.0, 1.0, 1.0, 0.28],
                    [1.0, 1.0, 1.0, 0.55],
                    rgb8([0x65, 0xbd, 0xca]),
                ],
                grip_counter_edge: EdgeStyle {
                    thickness: 1.0,
                    color: [0.0, 0.0, 0.0, 0.5],
                },
                popup: MenuSheetChrome {
                    surface: menu_sheet,
                    shadows: [
                        shadow(16.0, 40.0, [0.0, 0.0, 0.0, 0.7], false),
                        shadow(2.0, 6.0, [0.0, 0.0, 0.0, 0.5], false),
                    ],
                    lines: [
                        line(Edge::Top, 1.0, rgb8([0x38, 0x3d, 0x41])),
                        line(Edge::Bottom, 1.0, rgb8([0x0a, 0x0c, 0x0e])),
                    ],
                    separator: [
                        EdgeStyle {
                            thickness: 1.0,
                            color: rgb8([0x0a, 0x0c, 0x0d]),
                        },
                        EdgeStyle {
                            thickness: 1.0,
                            color: rgb8([0x26, 0x2b, 0x2f]),
                        },
                    ],
                },
                separator: [
                    EdgeStyle {
                        thickness: 1.0,
                        color: rgb8([0x0b, 0x0d, 0x0f]),
                    },
                    EdgeStyle {
                        thickness: 1.0,
                        color: rgb8([0x3d, 0x41, 0x45]),
                    },
                ],
            },
            dock: DockChrome {
                body: quad(gradient([0x15, 0x19, 0x1d], [0x0f, 0x12, 0x15])),
                header: quad(gradient([0x23, 0x27, 0x2b], [0x18, 0x1c, 0x20])),
                lines: [
                    line(Edge::Top, 0.0, rgb8([0x37, 0x3b, 0x3e])),
                    line(Edge::Bottom, 0.0, rgb8([0x0a, 0x0b, 0x0d])),
                    line(Edge::Bottom, 1.0, rgb8([0x21, 0x25, 0x29])),
                ],
                active_tab: bordered(
                    gradient([0x40, 0x43, 0x46], [0x2a, 0x2e, 0x31]),
                    rgb8([0x0f, 0x11, 0x13]),
                ),
                tab_hover: QuadStyle {
                    background: Background::Solid(rgb8([0x2e, 0x31, 0x35])),
                    border_widths: EdgeWidths::default(),
                    border_color: [0.0; 4],
                    corner_radii: CornerRadii::uniform(1.0),
                },
                active_tab_inset: shadow(1.0, 0.0, rgb8([0x6a, 0x6c, 0x6f]), true),
            },
            splitter: SplitterChrome {
                track_colors: [rgb8([0x16, 0x19, 0x1b]), rgb8([0x0e, 0x11, 0x13])],
                outer_edges: EdgeStyle {
                    thickness: 1.0,
                    color: rgb8([0x08, 0x09, 0x0a]),
                },
                inner_highlight: EdgeStyle {
                    thickness: 1.0,
                    color: rgb8([0x1e, 0x21, 0x22]),
                },
                grip_idle: rgb8([0x50, 0x52, 0x53]),
                grip_hover: rgb8([0x94, 0x96, 0x97]),
                grip_dragging: rgb8([0x8f, 0xd6, 0xe4]),
                grip_counter_edge: EdgeStyle {
                    thickness: 1.0,
                    color: rgb8([0x08, 0x09, 0x0a]),
                },
                dragging_glow: {
                    BoxShadow {
                        offset: [0.0, 0.0],
                        blur: 7.0,
                        spread: 0.0,
                        color: crate::color::rgba8([0x79, 0xc6, 0xd8], 0.65),
                        inset: false,
                    }
                },
            },
            status_bar: StatusBarChrome {
                surface: quad(gradient([0x1e, 0x23, 0x28], [0x13, 0x17, 0x1b])),
                lines: [
                    line(Edge::Top, 0.0, rgb8([0x04, 0x06, 0x08])),
                    line(Edge::Top, 1.0, rgb8([0x2f, 0x34, 0x38])),
                ],
                divider: [
                    EdgeStyle {
                        thickness: 1.0,
                        color: rgb8([0x0a, 0x0c, 0x0d]),
                    },
                    EdgeStyle {
                        thickness: 1.0,
                        color: rgb8([0x26, 0x2a, 0x2f]),
                    },
                ],
                latched_inset: shadow(2.0, 3.0, rgb8([0x1c, 0x46, 0x53]), true),
            },
            dropdown: floating(panel, 10.0, 26.0, 0.6),
            popover: floating(
                QuadStyle {
                    border_color: [0.0, 0.0, 0.0, 0.75],
                    ..panel
                },
                14.0,
                44.0,
                0.6,
            ),
            tooltip: FloatingSurfaceChrome {
                surface: bordered(
                    Background::Solid(rgb8([0x1b, 0x20, 0x25])),
                    [0.0, 0.0, 0.0, 0.7],
                ),
                shadow: shadow(6.0, 18.0, [0.0, 0.0, 0.0, 0.6], false),
                lines: [line(Edge::Top, 1.0, [1.0, 1.0, 1.0, 0.11]), no_lines[1]],
            },
            toast: floating(
                QuadStyle {
                    corner_radii: CornerRadii::uniform(3.0),
                    ..panel
                },
                12.0,
                30.0,
                0.6,
            ),
            curve_key: floating(panel, 1.0, 3.0, 0.6),
            // Stand-in until the window design handoff lands: the popover's
            // floating surface and elevation with the dock header's strip,
            // rules, and key hover.
            window: WindowChrome {
                surface: QuadStyle {
                    border_color: [0.0, 0.0, 0.0, 0.75],
                    ..panel
                },
                shadow: shadow(14.0, 44.0, [0.0, 0.0, 0.0, 0.6], false),
                lines: no_lines,
                header: quad(gradient([0x23, 0x27, 0x2b], [0x18, 0x1c, 0x20])),
                header_lines: [
                    line(Edge::Top, 0.0, rgb8([0x37, 0x3b, 0x3e])),
                    line(Edge::Bottom, 0.0, rgb8([0x0a, 0x0b, 0x0d])),
                    line(Edge::Bottom, 1.0, rgb8([0x21, 0x25, 0x29])),
                ],
                close_hover: quad(Background::Solid(rgb8([0x2e, 0x31, 0x35]))),
                grip_colors: [rgb8([0x50, 0x52, 0x53]), rgb8([0x94, 0x96, 0x97])],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::PaintCmd;

    #[test]
    fn defaults_keep_authored_shadow_recipes_distinct() {
        let chrome = ChromeTheme::default();
        assert_eq!(chrome.menu_sheet.shadows[0].blur, 40.0);
        assert_eq!(chrome.menu_sheet.shadows[1].blur, 6.0);
        assert_eq!(chrome.toolbar.rail_shadow.offset, [0.0, 2.0]);
        assert_eq!(chrome.toolbar.rail_shadow.blur, 8.0);
        assert_eq!(chrome.toolbar.popup.shadows, chrome.menu_sheet.shadows);
        assert_eq!(chrome.dropdown.shadow.blur, 26.0);
        assert_eq!(chrome.popover.shadow.blur, 44.0);
        assert_eq!(chrome.tooltip.shadow.blur, 18.0);
        assert_eq!(chrome.toast.shadow.blur, 30.0);
        assert_eq!(chrome.curve_key.shadow.blur, 3.0);
        assert_eq!(chrome.window.shadow.blur, 44.0);
        assert_eq!(chrome.splitter.dragging_glow.offset, [0.0, 0.0]);
        assert_eq!(chrome.splitter.dragging_glow.blur, 7.0);
        assert_eq!(chrome.splitter.dragging_glow.spread, 0.0);
    }

    #[test]
    fn surface_painter_stages_css_groups_content_border_and_lines() {
        let mut list = DrawList::new();
        let style = QuadStyle {
            background: Background::Solid([0.2, 0.3, 0.4, 1.0]),
            border_widths: EdgeWidths::uniform(1.0),
            border_color: [0.8; 4],
            corner_radii: CornerRadii::uniform(3.0),
        };
        let shadows = [
            BoxShadow {
                color: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
            BoxShadow {
                color: [0.0, 1.0, 0.0, 1.0],
                ..Default::default()
            },
            BoxShadow {
                color: [0.0, 0.0, 1.0, 1.0],
                inset: true,
                ..Default::default()
            },
            BoxShadow {
                color: [1.0, 1.0, 0.0, 1.0],
                inset: true,
                ..Default::default()
            },
        ];
        let lines = [StructuralLine {
            edge: Edge::Top,
            offset: 2.0,
            style: EdgeStyle {
                thickness: 1.0,
                color: [1.0; 4],
            },
        }];
        let border_box = Rect::new(0.0, 0.0, 30.0, 20.0);
        let padding_box = Rect::new(2.0, 3.0, 26.0, 14.0);
        let mut painter = SurfacePainter::new(
            &mut list,
            border_box,
            padding_box,
            CornerRadii::uniform(1.0),
            style,
            &shadows,
            &lines,
        );
        assert_eq!(painter.paint_pre_content(), padding_box);
        painter.draw_list().quad(5.0, 6.0, 2.0, 2.0, [0.5; 4]);
        painter.paint_post_content();

        assert_eq!(list.shadow_instance(0).unwrap().color, shadows[1].color);
        assert_eq!(list.shadow_instance(1).unwrap().color, shadows[0].color);
        assert_eq!(
            list.shadow_instance(2).unwrap().element_rect,
            [2.0, 3.0, 26.0, 14.0]
        );
        assert_eq!(list.shadow_instance(2).unwrap().color, shadows[3].color);
        assert_eq!(list.shadow_instance(3).unwrap().color, shadows[2].color);
        assert_eq!(list.paint_commands().len(), 1);
        assert!(matches!(
            list.paint_commands()[0],
            PaintCmd::Analytic { ref instances } if instances == &(0..8)
        ));
        assert_eq!(list.chrome_instance(1).unwrap().rect, [5.0, 6.0, 2.0, 2.0]);
        assert_eq!(list.chrome_instance(2).unwrap().widths, [1.0; 4]);
        assert_eq!(
            list.chrome_instances().last().unwrap().rect,
            [0.0, 2.0, 30.0, 1.0]
        );
    }
}
