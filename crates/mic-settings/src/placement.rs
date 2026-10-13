#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Physical rect for the 280×84 logical toast. An unknown position sits at the bottom center.
pub fn toast_rect(area: WorkArea, scale: f64, position: &str) -> ToastRect {
    let width = (280.0 * scale).round() as i32;
    let height = (84.0 * scale).round() as i32;
    let margin = (24.0 * scale).round() as i32;
    let left = area.x;
    let top = area.y;
    let right = left + area.width as i32 - width;
    let bottom = top + area.height as i32 - height;
    let center_x = left + (area.width as i32 - width) / 2;
    let center_y = top + (area.height as i32 - height) / 2;
    let (x, y) = match position {
        "top-left" => (left + margin, top + margin),
        "top" => (center_x, top + margin),
        "top-right" => (right - margin, top + margin),
        "left" => (left + margin, center_y),
        "center" => (center_x, center_y),
        "right" => (right - margin, center_y),
        "bottom-left" => (left + margin, bottom - margin),
        "bottom-right" => (right - margin, bottom - margin),
        _ => (center_x, bottom - margin),
    };
    ToastRect {
        x,
        y,
        width: width as u32,
        height: height as u32,
    }
}

/// The monitor under the cursor, otherwise the first area. `None` when the list is empty.
pub fn work_area_at(cursor: Option<(i32, i32)>, areas: &[WorkArea]) -> Option<WorkArea> {
    if let Some((x, y)) = cursor {
        for area in areas {
            let inside = x >= area.x
                && y >= area.y
                && x < area.x + area.width as i32
                && y < area.y + area.height as i32;
            if inside {
                return Some(*area);
            }
        }
    }
    areas.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> WorkArea {
        WorkArea {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        }
    }

    #[test]
    fn positions_use_scale_and_unknown_is_bottom() {
        let area = screen();
        let bottom = toast_rect(area, 1.0, "bottom");
        assert_eq!(bottom.width, 280);
        assert_eq!(bottom.height, 84);
        assert_eq!(bottom.x, (1000 - 280) / 2);
        assert_eq!(bottom.y, 800 - 84 - 24);

        let center = toast_rect(area, 1.0, "center");
        assert_eq!(center.x, bottom.x);
        assert_eq!(center.y, (800 - 84) / 2);

        let top_left = toast_rect(area, 2.0, "top-left");
        assert_eq!(top_left.width, 560);
        assert_eq!(top_left.x, 48);
        assert_eq!(top_left.y, 48);

        assert_eq!(toast_rect(area, 1.0, "nope"), bottom);
    }

    #[test]
    fn cursor_picks_the_monitor_that_contains_it() {
        let areas = [
            screen(),
            WorkArea {
                x: 1000,
                y: 0,
                width: 800,
                height: 600,
            },
        ];
        assert_eq!(work_area_at(Some((1200, 10)), &areas), Some(areas[1]));
        assert_eq!(work_area_at(Some((-1, 0)), &areas), Some(areas[0]));
        assert_eq!(work_area_at(None, &areas), Some(areas[0]));
        assert_eq!(work_area_at(Some((0, 0)), &[]), None);
    }
}
