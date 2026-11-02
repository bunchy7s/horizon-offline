//! Project a draft arrangement onto the Setup screen, without opening a window.
use crate::split_layout::{Draft, Monitor};

/// Map area: below the title and above the move row and footer.
pub const TOP: f32 = 30.0;
pub const BOTTOM: f32 = 80.0;
pub type Rect = [f32; 4];
pub type Map = (Vec<Rect>, Vec<(u8, Rect)>, Vec<(u8, Rect)>);

/// Monitor, window and player rectangles in UI coordinates, in their drawing order.
pub fn project(draft: &Draft, monitors: &[Monitor], main: &str, w: f32, h: f32) -> Map {
    let desk: Vec<[f64; 4]> = monitors.iter().map(Monitor::desktop).collect();
    // A window's size on the desktop: its pixels at the scale of the monitor it is on.
    let rects = draft.rects(monitors, main);
    let on_desk: Vec<(u8, [f64; 4])> = draft
        .windows
        .iter()
        .zip(&rects)
        .filter_map(|(win, rect)| {
            let ([x, y], [pw, ph]) = (*rect)?;
            let (x, y) = (f64::from(x), f64::from(y));
            let scale = monitors
                .iter()
                .find(|m| {
                    let [mx, my, mw, mh] = m.desktop();
                    x >= mx && x < mx + mw && y >= my && y < my + mh
                })
                .or_else(|| monitors.iter().find(|m| m.name == main))
                .map_or(1.0, |m| m.desktop()[2] / f64::from(m.size[0]));
            Some((
                win.number,
                [x, y, f64::from(pw) * scale, f64::from(ph) * scale],
            ))
        })
        .collect();
    // The monitors, and windows past them (Wayland's, whose place is unknown) still shown.
    let all = || desk.iter().chain(on_desk.iter().map(|(_, d)| d));
    let x0 = all().map(|d| d[0]).fold(f64::MAX, f64::min);
    let y0 = all().map(|d| d[1]).fold(f64::MAX, f64::min);
    let x1 = all().map(|d| d[0] + d[2]).fold(f64::MIN, f64::max);
    let y1 = all().map(|d| d[1] + d[3]).fold(f64::MIN, f64::max);
    if desk.is_empty() || x1 <= x0 || y1 <= y0 {
        return Default::default();
    }
    let (aw, ah) = (
        f64::from(w) - 40.0,
        // Room under the monitors for their names, and always for the move row: the map
        // doesn't jump when a player is picked (a drop back to where its tile was misses).
        (f64::from(h - TOP - BOTTOM) - 18.0).max(10.0),
    );
    let k = (aw / (x1 - x0)).min(ah / (y1 - y0));
    let (ox, oy) = (
        (f64::from(w) - (x1 - x0) * k) / 2.0,
        f64::from(TOP) + 4.0 + (ah - (y1 - y0) * k) / 2.0,
    );
    let to_map = |[x, y, dw, dh]: [f64; 4]| {
        [
            (ox + (x - x0) * k) as f32,
            (oy + (y - y0) * k) as f32,
            (dw * k) as f32,
            (dh * k) as f32,
        ]
    };
    let mapped_monitors = desk.iter().copied().map(to_map).collect();
    let windows: Vec<(u8, [f32; 4])> = on_desk.iter().map(|&(n, d)| (n, to_map(d))).collect();
    // In their own order, emptied ones too: a window a player was just dragged out of stays
    // where it was until Apply closes it, so the player can be dragged back into it.
    let tiles = draft
        .preview(monitors, main)
        .iter()
        .map(|&(i, [vx, vy], size)| {
            let number = draft.windows.get(i).map_or(1, |w| w.number);
            let Some((_, [wx, wy, ww, wh])) = windows.iter().find(|(n, _)| *n == number) else {
                return (number, [0.0; 4]);
            };
            let [pw, ph] = rects[i].map_or([1, 1], |r| r.1).map(|v| v.max(1) as f32);
            (
                number,
                [
                    wx + vx as f32 / pw * ww,
                    wy + vy as f32 / ph * wh,
                    size.width as f32 / pw * ww,
                    size.height as f32 / ph * wh,
                ],
            )
        })
        .collect();
    (mapped_monitors, windows, tiles)
}

/// The largest visible side of a tile after later windows cover it.
pub fn uncovered(mut r: [f32; 4], covers: impl Iterator<Item = [f32; 4]>) -> [f32; 4] {
    for [cx, cy, cw, ch] in covers {
        let [x, y, w, h] = r;
        if cx >= x + w || cx + cw <= x || cy >= y + h || cy + ch <= y {
            continue;
        }
        let sides = [
            [x, y, cx - x, h],
            [cx + cw, y, x + w - (cx + cw), h],
            [x, y, w, cy - y],
            [x, cy + ch, w, y + h - (cy + ch)],
        ];
        r = sides
            .into_iter()
            .map(|[sx, sy, sw, sh]| [sx, sy, sw.max(0.0), sh.max(0.0)])
            .max_by(|a, b| (a[2] * a[3]).total_cmp(&(b[2] * b[3])))
            .unwrap_or([x, y, 0.0, 0.0]);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split_layout::{Placement, Spot};

    #[test]
    fn preview_tiles_share_a_window_and_fallback_when_a_monitor_disappears() {
        let monitors = vec![
            Monitor {
                name: "Desk".into(),
                at: [0, 0],
                size: [2000, 1200],
                scale: 1.0,
            },
            Monitor {
                name: "TV".into(),
                at: [-1920, 0],
                size: [1920, 1080],
                scale: 1.0,
            },
        ];
        let mut draft =
            Draft::everyone_here(2, Placement::Free(Some(([0, 0], [2000, 1200]))), 0, 0);
        let (_, windows, tiles) = project(&draft, &monitors, "Desk", 427.0, 300.0);
        assert_eq!(windows.len(), 1);
        let (a, b) = (tiles[0].1, tiles[1].1);
        assert!((a[2] - b[2]).abs() < 0.01);
        assert!((a[0] + a[2] - b[0]).abs() < 0.01);
        draft.place_player(1, Spot::Fill("TV".into()));
        let (screens, windows, tiles) = project(&draft, &monitors, "Desk", 427.0, 300.0);
        assert!(screens[1][0] < screens[0][0]);
        assert_eq!(windows.len(), 2);
        assert_eq!((tiles[0].0, tiles[1].0), (1, 2));
        let (_, windows, tiles) = project(&draft, &monitors[..1], "Desk", 427.0, 300.0);
        assert_eq!(windows.len(), 1);
        assert_eq!((tiles[0].0, tiles[1].0), (1, 1));
        assert!(project(&draft, &[], "Desk", 427.0, 300.0).0.is_empty());
    }

    #[test]
    fn covered_tiles_only_expose_a_visible_side_for_clicks() {
        let tile = [0.0, 0.0, 100.0, 100.0];
        assert_eq!(
            uncovered(tile, [[0.0, 0.0, 60.0, 100.0]].into_iter()),
            [60.0, 0.0, 40.0, 100.0]
        );
        let hidden = uncovered(tile, [tile].into_iter());
        assert_eq!(hidden[2] * hidden[3], 0.0);
        assert_eq!(
            uncovered(tile, [[110.0, 0.0, 50.0, 50.0]].into_iter()),
            tile
        );
    }
}
