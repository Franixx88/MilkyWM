/// Layout modes for tiling windows within a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// One window fills the entire screen (current behaviour — sun always full-screen).
    Monocle,
    /// Windows share the screen width equally in a horizontal row.
    HorizSplit,
    /// Windows share the screen height equally in a vertical stack.
    VertSplit,
}

/// A simple integer rectangle in screen-space (pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}

/// Compute tile rectangles for `count` windows within `area`.
///
/// Returns `count` non-overlapping rectangles that together fill `area`.
/// If `count == 0` an empty Vec is returned.
pub fn compute_tiles(count: usize, area: Rect, mode: LayoutMode) -> Vec<Rect> {
    if count == 0 {
        return vec![];
    }
    if count == 1 || mode == LayoutMode::Monocle {
        return vec![area];
    }

    match mode {
        LayoutMode::Monocle => vec![area], // unreachable, handled above
        LayoutMode::HorizSplit => {
            let tile_w = area.w / count as i32;
            let remainder = area.w - tile_w * count as i32;
            (0..count)
                .map(|i| {
                    let extra = if i == count - 1 { remainder } else { 0 };
                    Rect::new(
                        area.x + tile_w * i as i32,
                        area.y,
                        tile_w + extra,
                        area.h,
                    )
                })
                .collect()
        }
        LayoutMode::VertSplit => {
            let tile_h = area.h / count as i32;
            let remainder = area.h - tile_h * count as i32;
            (0..count)
                .map(|i| {
                    let extra = if i == count - 1 { remainder } else { 0 };
                    Rect::new(
                        area.x,
                        area.y + tile_h * i as i32,
                        area.w,
                        tile_h + extra,
                    )
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horiz_split_two() {
        let area = Rect::new(0, 0, 1920, 1080);
        let tiles = compute_tiles(2, area, LayoutMode::HorizSplit);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0], Rect::new(0, 0, 960, 1080));
        assert_eq!(tiles[1], Rect::new(960, 0, 960, 1080));
    }

    #[test]
    fn vert_split_three() {
        let area = Rect::new(0, 0, 1920, 1080);
        let tiles = compute_tiles(3, area, LayoutMode::VertSplit);
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[0].h, 360);
        assert_eq!(tiles[1].h, 360);
        assert_eq!(tiles[2].h + tiles[2].y, 1080); // last tile fills remainder
    }

    #[test]
    fn monocle_ignores_count() {
        let area = Rect::new(0, 0, 1920, 1080);
        let tiles = compute_tiles(5, area, LayoutMode::Monocle);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0], area);
    }
}
