//! Toolbar band — the raised strip an app puts above or below its content
//! (Forge app headers and composers: `--surface-toolbar`, a hard
//! `--edge-sheet` line on the side facing the content, and the lit
//! `--hi-bar` line along its top).

use crate::chrome::Edge;
use crate::layout::Rect;
use crate::style::StyleResolver;

use super::DrawList;

/// `--edge-sheet`.
const EDGE_SHEET: [f32; 4] = [0.0, 0.0, 0.0, 0.75];
/// `--hi-bar`.
const HI_BAR: [f32; 4] = [1.0, 1.0, 1.0, 0.11];

/// Paint a toolbar band filling `rect`, with its hard edge on `edge` (the
/// side facing the content: `Bottom` for a header, `Top` for a composer).
/// The lit line runs along the top, inside the edge when that is the top.
pub fn toolbar_band(list: &mut DrawList, s: &StyleResolver, rect: Rect, edge: Edge) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let [top, bottom] = s.toolbar().rail_colors;
    list.vertical_gradient(rect, top, bottom);
    let hi_y = if edge == Edge::Top {
        rect.y + 1.0
    } else {
        rect.y
    };
    list.quad(rect.x, hi_y, rect.width, 1.0, HI_BAR);
    list.edge_line(rect, edge, 1.0, EDGE_SHEET);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    fn rects_of(list: &DrawList, color: [f32; 4]) -> Vec<[f32; 4]> {
        list.chrome_instances()
            .filter(|c| c.bg == color)
            .map(|c| c.rect)
            .collect()
    }

    #[test]
    fn a_header_band_has_its_edge_below_and_its_light_on_top() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        toolbar_band(
            &mut list,
            &s,
            Rect::new(0.0, 10.0, 300.0, 34.0),
            Edge::Bottom,
        );
        assert_eq!(rects_of(&list, HI_BAR), vec![[0.0, 10.0, 300.0, 1.0]]);
        assert_eq!(
            rects_of(&list, EDGE_SHEET),
            vec![[0.0, 43.0, 300.0, 1.0]],
            "a bottom edge"
        );
    }

    #[test]
    fn a_composer_band_has_its_edge_on_top_and_the_light_under_it() {
        let theme = Theme::default();
        let s = StyleResolver::new(&theme);
        let mut list = DrawList::new();
        toolbar_band(&mut list, &s, Rect::new(0.0, 10.0, 300.0, 80.0), Edge::Top);
        assert_eq!(rects_of(&list, HI_BAR), vec![[0.0, 11.0, 300.0, 1.0]]);
        assert_eq!(rects_of(&list, EDGE_SHEET), vec![[0.0, 10.0, 300.0, 1.0]]);
    }
}
