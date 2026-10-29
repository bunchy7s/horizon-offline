//! Splitscreen's layout, pure data tested without a window: where each player's view goes in
//! its window (`place`).
use winit::dpi::PhysicalSize;

/// Players at most: P1 and fifteen more.
pub const MAX_PLAYERS: usize = 16;

/// A player's view: its window (an index into the windows), its corner there and its size.
pub type View = (usize, [u32; 2], PhysicalSize<u32>);

/// Rows of five to sixteen players sharing a window, widest first (columns on a tall window): every
/// view stays near a screen's shape, where a full grid would leave a long thin one.
const ROWS: [&[u32]; MAX_PLAYERS - 4] = [
    &[3, 2],
    &[3, 3],
    &[3, 2, 2],
    &[3, 3, 2],
    &[3, 3, 3],
    &[4, 3, 3],
    &[4, 4, 3],
    &[4, 4, 4],
    &[4, 3, 3, 3],
    &[4, 4, 3, 3],
    &[4, 4, 4, 3],
    &[4, 4, 4, 4],
];

/// `order` made a whole order of `n` players: those it names (once, if there are), then the
/// others, P1 first.
pub fn ordered(order: &[usize], n: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(n);
    for &i in order {
        if i < n && !out.contains(&i) {
            out.push(i);
        }
    }
    out.extend((0..n).filter(|i| !order.contains(i)));
    out
}

/// `place` with the players sharing a window in `order` (`ordered`): the first left or on top.
/// The views are still by player.
pub fn place_ordered(
    windows: &[usize],
    order: &[usize],
    sizes: &[PhysicalSize<u32>],
    two: u8,
    three: u8,
) -> Vec<View> {
    let order = ordered(order, windows.len());
    let in_order: Vec<usize> = order.iter().map(|&i| windows[i]).collect();
    let placed = place(&in_order, sizes, two, three);
    let mut views = placed.clone();
    for (k, &i) in order.iter().enumerate() {
        views[i] = placed[k];
    }
    views
}

/// Each player's view, given the window each is in and the windows' sizes. Players in one window
/// share it in the order they come: two take halves (`two`: 0 side by side when wider than
/// tall, 1 side by side, 2 top and bottom); three (`three`: 0 the first wide on top and two below,
/// 1 the first tall on the left and two on the right, 2 three columns); four its quarters; five
/// to sixteen in balanced rows (`ROWS`).
pub fn place(windows: &[usize], sizes: &[PhysicalSize<u32>], two: u8, three: u8) -> Vec<View> {
    windows
        .iter()
        .enumerate()
        .map(|(i, &window)| {
            let here = windows.iter().filter(|&&w| w == window).count();
            let slot = windows[..i].iter().filter(|&&w| w == window).count() as u32;
            let size = sizes
                .get(window)
                .copied()
                .unwrap_or(PhysicalSize::new(1, 1));
            let (w, h) = (size.width, size.height);
            let side_by_side = match two {
                1 => true,
                2 => false,
                _ => w >= h,
            };
            // The view's cell as fractions of the window: (x0, x1, y0, y1) in halves or thirds.
            let cell = |x0: u32, x1: u32, y0: u32, y1: u32, of: u32| {
                [w * x0 / of, w * x1 / of, h * y0 / of, h * y1 / of]
            };
            let [x0, x1, y0, y1] = match (here, slot) {
                (1, _) => cell(0, 1, 0, 1, 1),
                (2, s) if side_by_side => cell(s, s + 1, 0, 2, 2),
                (2, s) => cell(0, 2, s, s + 1, 2),
                (3, s) => match (three, s) {
                    (2, s) => cell(s, s + 1, 0, 3, 3),
                    (1, 0) => cell(0, 1, 0, 2, 2),
                    (1, s) => cell(1, 2, s - 1, s, 2),
                    (_, 0) => cell(0, 2, 0, 1, 2),
                    (_, s) => cell(s - 1, s, 1, 2, 2),
                },
                (4, s) => cell(s % 2, s % 2 + 1, s / 2, s / 2 + 1, 2),
                (n, s) => rows(n, s, w, h),
            };
            (
                window,
                [x0, y0],
                PhysicalSize::new((x1 - x0).max(1), (y1 - y0).max(1)),
            )
        })
        .collect()
}

/// Five to sixteen players: `slot`'s cell `[x0, x1, y0, y1]` in balanced rows (`ROWS`), or
/// in columns when the window is taller than wide.
fn rows(n: usize, slot: u32, w: u32, h: u32) -> [u32; 4] {
    let counts = ROWS[n.clamp(5, MAX_PLAYERS) - 5];
    let bands = counts.len() as u32;
    let (mut band, mut k) = (0, slot);
    while band + 1 < bands && k >= counts[band as usize] {
        k -= counts[band as usize];
        band += 1;
    }
    let across = counts[band as usize];
    // Along: the long side, divided between a band's views; down: the bands.
    let (along, down) = if w >= h { (w, h) } else { (h, w) };
    let (a0, a1) = (along * k / across, along * (k + 1) / across);
    let (d0, d1) = (down * band / bands, down * (band + 1) / bands);
    if w >= h {
        [a0, a1, d0, d1]
    } else {
        [d0, d1, a0, a1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: u32, h: u32) -> PhysicalSize<u32> {
        PhysicalSize::new(w, h)
    }

    #[test]
    fn players_on_one_screen_share_it() {
        let wide = [size(1920, 1080)];
        assert_eq!(
            place(&[0, 0], &wide, 0, 0)[1],
            (0, [960, 0], size(960, 1080))
        );
        assert_eq!(
            place(&[0, 0], &wide, 2, 0)[1],
            (0, [0, 540], size(1920, 540))
        );
        // Auto on a tall screen: top and bottom.
        assert_eq!(place(&[0, 0], &[size(1080, 1920)], 0, 0)[1].1, [0, 960]);
        assert_eq!(
            place(&[0; 4], &wide, 0, 0)[3],
            (0, [960, 540], size(960, 540))
        );
    }

    #[test]
    fn desk_and_couch() {
        // P1 alone at the desk; P2 and P3 share the TV.
        let screens = [size(3024, 1964), size(3840, 2160)];
        let views = place(&[0, 1, 1], &screens, 0, 0);
        assert_eq!(views[0], (0, [0, 0], size(3024, 1964)));
        assert_eq!(views[1], (1, [0, 0], size(1920, 2160)));
        assert_eq!(views[2], (1, [1920, 0], size(1920, 2160)));
    }

    #[test]
    fn three_on_a_screen() {
        let tv = [size(1920, 1080)];
        // One wide on top, two below.
        let top = place(&[0; 3], &tv, 0, 0);
        assert_eq!(top[0], (0, [0, 0], size(1920, 540)));
        assert_eq!(top[2], (0, [960, 540], size(960, 540)));
        // One tall on the left, two stacked on the right.
        let left = place(&[0; 3], &tv, 0, 1);
        assert_eq!(left[0], (0, [0, 0], size(960, 1080)));
        assert_eq!(left[2], (0, [960, 540], size(960, 540)));
        // Three columns.
        assert_eq!(
            place(&[0; 3], &tv, 0, 2)[2],
            (0, [1280, 0], size(640, 1080))
        );
    }

    #[test]
    fn two_windows_two_players_each() {
        // P1 and P2 at the desk, P3 and P4 on the TV: halves in each window.
        let screens = [size(3024, 1964), size(3840, 2160)];
        let views = place(&[0, 0, 1, 1], &screens, 0, 0);
        assert_eq!(views[1], (0, [1512, 0], size(1512, 1964)));
        assert_eq!(views[2], (1, [0, 0], size(1920, 2160)));
        assert_eq!(views[3], (1, [1920, 0], size(1920, 2160)));
    }

    #[test]
    fn five_to_eight_in_rows_of_three_and_two() {
        let tv = [size(1920, 1080)];
        let eight = place(&[0; 8], &tv, 0, 0);
        assert_eq!(eight[0], (0, [0, 0], size(640, 360)));
        assert_eq!(eight[5], (0, [1280, 360], size(640, 360)));
        assert_eq!(eight[7], (0, [960, 720], size(960, 360)));
        let seven = place(&[0; 7], &tv, 0, 0);
        assert_eq!(seven[3], (0, [0, 360], size(960, 360)));
        // Every count tiles the window exactly: no gap, no overlap.
        for n in 5..=MAX_PLAYERS {
            for screen in [size(1920, 1080), size(1080, 1920), size(1711, 997)] {
                let views = place(&vec![0; n], &[screen], 0, 0);
                for (i, (_, [x, y], size)) in views.iter().enumerate() {
                    assert!(x + size.width <= screen.width && y + size.height <= screen.height);
                    for (_, [ox, oy], other) in &views[..i] {
                        assert!(
                            x + size.width <= *ox
                                || ox + other.width <= *x
                                || y + size.height <= *oy
                                || oy + other.height <= *y,
                            "overlapping views for {n} in {screen:?}"
                        );
                    }
                }
                let area: u64 = views
                    .iter()
                    .map(|v| u64::from(v.2.width) * u64::from(v.2.height))
                    .sum();
                assert_eq!(
                    area,
                    u64::from(screen.width) * u64::from(screen.height),
                    "{n} in {screen:?}"
                );
            }
        }
        // A tall window: columns.
        let tall = place(&[0; 5], &[size(1080, 1920)], 0, 0);
        assert_eq!(tall[0], (0, [0, 0], size(540, 640)));
        assert_eq!(tall[4], (0, [540, 960], size(540, 960)));
    }

    #[test]
    fn sixteen_players_form_a_four_by_four_grid() {
        let views = place(&[0; 16], &[size(3840, 2160)], 0, 0);
        for (i, view) in views.iter().enumerate() {
            assert_eq!(
                *view,
                (
                    0,
                    [(i as u32 % 4) * 960, (i as u32 / 4) * 540],
                    size(960, 540)
                )
            );
        }
    }

    #[test]
    fn an_order_names_each_player_once_and_the_others_come_last() {
        assert_eq!(ordered(&[], 3), [0, 1, 2]);
        assert_eq!(ordered(&[2, 0], 3), [2, 0, 1]);
        // Gone players and repeats are dropped.
        assert_eq!(ordered(&[5, 1, 1], 3), [1, 0, 2]);
    }

    #[test]
    fn players_sharing_a_window_fill_it_in_their_order() {
        let wide = [size(2000, 1000)];
        // P2 first: P2 on the left, P1 on the right.
        let views = place_ordered(&[0, 0], &[1, 0], &wide, 0, 0);
        assert_eq!(views[1].1, [0, 0]);
        assert_eq!(views[0].1, [1000, 0]);
        // P3 first of three: the wide view on top is P3's.
        let views = place_ordered(&[0, 0, 0], &[2], &wide, 0, 0);
        assert_eq!(views[2].2, size(2000, 500));
    }
}
