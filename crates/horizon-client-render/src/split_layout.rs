//! Splitscreen's layout, pure data tested without a window: where each player's view goes in
//! its window (`place`), and the setup screen's draft (`Draft`): the windows, where each is, who
//! plays in each, before any of it is applied.
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

/// A screen as the system reports it: its name, where it is on the desktop and its size (both
/// physical pixels), and its scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub at: [i32; 2],
    pub size: [u32; 2],
    pub scale: f64,
}

impl Monitor {
    /// Where it is on the desktop and its size, as windows are placed: macOS places them in
    /// points, each monitor at its own scale; Windows and X11 in desktop pixels.
    pub fn desktop(&self) -> [f64; 4] {
        let k = if cfg!(target_os = "macos") {
            self.scale
        } else {
            1.0
        };
        [
            f64::from(self.at[0]) / k,
            f64::from(self.at[1]) / k,
            f64::from(self.size[0]) / k,
            f64::from(self.size[1]) / k,
        ]
    }
}

/// Where a window is.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// A normal window: its corner on the desktop (`Monitor::desktop`'s units) and its size in
    /// pixels (`None`: a new one, where the system opens it, `NEW_WINDOW` big).
    Free(Option<([i32; 2], [u32; 2])>),
    /// Borderless over the monitor of this name.
    Fill(String),
}

/// A new free window's size, in logical pixels.
pub const NEW_WINDOW: [f64; 2] = [854.0, 480.0];

/// How far window `number` opens right and down of the centre, in logical pixels: Window 2
/// centred, each next one a step further, as systems cascade new windows (back to the centre
/// after six), so new windows never open exactly on one another.
pub fn cascade(number: u8) -> f64 {
    f64::from(number.saturating_sub(2) % 6) * 32.0
}

/// A window: its number (1 is the game's own) and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub number: u8,
    pub placement: Placement,
}

/// Where a player can go: a window there is (by number), a new free one, or a new one filling a
/// monitor (by name).
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    Window(u8),
    New,
    Fill(String),
}

/// Something to tell before Apply.
#[derive(Clone, Debug, PartialEq)]
pub enum Warning {
    /// Nobody plays in this window: it won't open (Window 1 stays, blank).
    Empty(u8),
    /// This window fills a monitor that isn't there: its players play in Window 1.
    Unplugged(u8, String),
    /// This window fills the monitor Window 1 is on: its players play in Window 1.
    OnWindow1(u8),
}

/// The setup screen's arrangement until Apply: the windows, each player's window (by number, P1
/// first) and the shapes two or three players sharing a window take (`place`'s `two`, `three`).
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub windows: Vec<Window>,
    pub players: Vec<u8>,
    pub two: u8,
    pub three: u8,
    /// The players (indices) in the order their views fill a window shared with others: first
    /// left or on top. Empty, or missing some: P1, P2... (`ordered`).
    pub order: Vec<usize>,
}

impl Draft {
    /// Everyone in Window 1 (where it is now).
    pub fn everyone_here(players: usize, window1: Placement, two: u8, three: u8) -> Self {
        Self {
            windows: vec![Window {
                number: 1,
                placement: window1,
            }],
            players: vec![1; players],
            two,
            three,
            order: Vec::new(),
        }
    }

    /// Players `i` and `j` trade places: their windows and their places in them.
    pub fn swap(&mut self, i: usize, j: usize) {
        let n = self.players.len();
        if i >= n || j >= n || i == j {
            return;
        }
        self.players.swap(i, j);
        let mut order = ordered(&self.order, n);
        let (a, b) = (
            order.iter().position(|&p| p == i),
            order.iter().position(|&p| p == j),
        );
        if let (Some(a), Some(b)) = (a, b) {
            order.swap(a, b);
        }
        self.order = order;
    }

    /// Player `i` to place `k` among the others in its window (0: before them all; past the
    /// last: after them), the others keeping their order: the bumpers' "Window 1 (2 of 3)".
    pub fn put_at(&mut self, i: usize, k: usize) {
        let n = self.players.len();
        if i >= n {
            return;
        }
        let mine = self.players[i];
        let mut order: Vec<usize> = ordered(&self.order, n)
            .into_iter()
            .filter(|&p| p != i)
            .collect();
        let others: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&p| self.players[p] == mine)
            .collect();
        let at = match others.get(k) {
            Some(j) => order.iter().position(|p| p == j).unwrap_or(order.len()),
            None => others
                .last()
                .and_then(|j| order.iter().position(|p| p == j))
                .map_or(order.len(), |p| p + 1),
        };
        order.insert(at, i);
        self.order = order;
    }

    /// The players but `i` in window `number`, in their order.
    pub fn sharing(&self, i: usize, number: u8) -> Vec<usize> {
        ordered(&self.order, self.players.len())
            .into_iter()
            .filter(|&p| p != i && self.players[p] == number)
            .collect()
    }

    /// A new free window, numbered with the lowest number free; its number.
    pub fn add_window(&mut self) -> u8 {
        self.open(Placement::Free(None))
    }

    fn open(&mut self, placement: Placement) -> u8 {
        let number = (2..=u8::MAX)
            .find(|n| self.windows.iter().all(|w| w.number != *n))
            .unwrap_or(u8::MAX);
        self.windows.push(Window { number, placement });
        number
    }

    /// Player `i` to `spot`: a new window opens for it. A window it leaves empty stays in the
    /// draft until `drop_empty` (at Apply), so numbers hold still while players move. A window
    /// number the draft doesn't have is ignored.
    pub fn place_player(&mut self, i: usize, spot: Spot) {
        if i >= self.players.len() {
            return;
        }
        let number = match spot {
            Spot::Window(n) if self.windows.iter().any(|w| w.number == n) => n,
            Spot::Window(_) => return,
            Spot::New => self.add_window(),
            Spot::Fill(name) => self.open(Placement::Fill(name)),
        };
        self.players[i] = number;
    }

    /// The windows nobody plays in, gone, but Window 1 and those in `keep`. At Apply, all of
    /// them; while a draft is edited, the ones it added (the windows open now stay until Apply,
    /// so their numbers hold still as players move).
    pub fn drop_empty(&mut self, keep: &[u8]) {
        let players = &self.players;
        self.windows
            .retain(|w| w.number == 1 || players.contains(&w.number) || keep.contains(&w.number));
    }

    /// Whether nobody plays in window `number`.
    pub fn empty(&self, number: u8) -> bool {
        !self.players.contains(&number)
    }

    /// What to tell before Apply, in window order.
    pub fn warnings(&self, monitors: &[Monitor], main: &str) -> Vec<Warning> {
        let mut out = Vec::new();
        for w in &self.windows {
            if let Placement::Fill(name) = &w.placement
                && !monitors.iter().any(|m| &m.name == name)
            {
                out.push(Warning::Unplugged(w.number, name.clone()));
            } else if w.number != 1 && w.placement == Placement::Fill(main.to_owned()) {
                out.push(Warning::OnWindow1(w.number));
            } else if !self.players.contains(&w.number) {
                out.push(Warning::Empty(w.number));
            }
        }
        out
    }

    /// Where each window would be: its corner on the desktop (`Monitor::desktop`'s units) and its
    /// size in pixels, in `windows` order: free ones where they were (new ones at `main`'s
    /// corner), the others over their monitor; a window over a missing monitor or Window 1's
    /// (its players play in Window 1, as the game does): `None`. Window 1 always has one: give
    /// it its real rect (`Free(Some)`); else it is taken as a new window would open.
    pub fn rects(&self, monitors: &[Monitor], main: &str) -> Vec<Option<([i32; 2], [u32; 2])>> {
        let home = monitors
            .iter()
            .find(|m| m.name == main)
            .or(monitors.first());
        // Centred on Window 1's monitor, as the game opens a new window.
        let new = |number: u8| {
            let (at, scale, k) = home.map_or(([0.0; 4], 1.0, 1.0), |m| {
                let at = m.desktop();
                (at, m.scale, at[2] / f64::from(m.size[0].max(1)))
            });
            let size = NEW_WINDOW.map(|v| (v * scale).round() as u32);
            let step = cascade(number) * scale * k;
            let x = at[0] + (at[2] - f64::from(size[0]) * k) / 2.0 + step;
            let y = at[1] + (at[3] - f64::from(size[1]) * k) / 2.0 + step;
            ([x as i32, y as i32], size)
        };
        self.windows
            .iter()
            .map(|w| {
                let rect = match &w.placement {
                    Placement::Free(Some(rect)) => Some(*rect),
                    Placement::Free(None) => Some(new(w.number)),
                    Placement::Fill(name) if w.number != 1 && name == main => None,
                    Placement::Fill(name) => monitors.iter().find(|m| &m.name == name).map(|m| {
                        let at = m.desktop();
                        ([at[0] as i32, at[1] as i32], m.size)
                    }),
                };
                // Everyone with nowhere else plays in Window 1: it never has no size.
                if w.number == 1 {
                    rect.or_else(|| Some(new(1)))
                } else {
                    rect
                }
            })
            .collect()
    }

    /// Every player's view as Apply would make it (`place`, as the game tiles them), with
    /// `View`'s window an index into `windows`; players of a window over a missing monitor in
    /// Window 1's.
    pub fn preview(&self, monitors: &[Monitor], main: &str) -> Vec<View> {
        let rects = self.rects(monitors, main);
        let index = |number: u8| self.windows.iter().position(|w| w.number == number);
        let window1 = index(1).unwrap_or(0);
        let windows: Vec<usize> = self
            .players
            .iter()
            .map(|&n| index(n).filter(|&i| rects[i].is_some()).unwrap_or(window1))
            .collect();
        let sizes: Vec<PhysicalSize<u32>> = rects
            .iter()
            .map(|r| {
                r.map_or(PhysicalSize::new(1, 1), |(_, [w, h])| {
                    PhysicalSize::new(w, h)
                })
            })
            .collect();
        place_ordered(&windows, &self.order, &sizes, self.two, self.three)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_1_always_has_a_size_and_unknown_windows_take_nobody() {
        let monitors = [Monitor {
            name: "Main".into(),
            at: [0, 0],
            size: [1920, 1080],
            scale: 1.0,
        }];
        let mut d = Draft::everyone_here(2, Placement::Fill("Gone".into()), 0, 0);
        assert!(d.rects(&monitors, "Main")[0].is_some());
        assert!(d.preview(&monitors, "Main").iter().all(|v| v.2.width > 1));
        d.place_player(1, Spot::Window(9));
        assert_eq!(d.players, [1, 1], "no Window 9 in the draft");
    }

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
        let mut draft = here(16);
        draft.place_player(15, Spot::New);
        let views = draft.preview(&monitors(), "Built-in");
        assert_eq!(views.len(), 16);
        assert_ne!(views[0].0, views[15].0);
    }

    fn monitors() -> Vec<Monitor> {
        vec![
            Monitor {
                name: "Built-in".into(),
                at: [0, 0],
                size: [3024, 1964],
                scale: 2.0,
            },
            Monitor {
                name: "TV".into(),
                at: [3024, 0],
                size: [3840, 2160],
                scale: 1.0,
            },
        ]
    }

    /// P1 in Window 1, everyone else in Window 2 filling the TV.
    trait DeskAndTv {
        fn desk_and_tv(self, monitors: &[Monitor], main: &str) -> Option<Draft>;
    }
    impl DeskAndTv for Draft {
        fn desk_and_tv(mut self, monitors: &[Monitor], main: &str) -> Option<Draft> {
            let tv = monitors.iter().find(|m| m.name != main)?;
            self.windows.push(Window {
                number: 2,
                placement: Placement::Fill(tv.name.clone()),
            });
            for p in self.players.iter_mut().skip(1) {
                *p = 2;
            }
            Some(self)
        }
    }

    fn here(players: usize) -> Draft {
        Draft::everyone_here(
            players,
            Placement::Free(Some(([100, 100], [2000, 1200]))),
            0,
            0,
        )
    }

    #[test]
    fn presets() {
        let m = monitors();
        // Everyone here: four quarters of Window 1.
        let views = here(4).preview(&m, "Built-in");
        assert_eq!(views[3], (0, [1000, 600], size(1000, 600)));
        // P1 alone, the others share the TV in rows.
        let tv = here(4).desk_and_tv(&m, "Built-in").unwrap();
        assert_eq!(tv.players, [1, 2, 2, 2]);
        assert_eq!(tv.windows[1].placement, Placement::Fill("TV".into()));
        let views = tv.preview(&m, "Built-in");
        assert_eq!(views[0], (0, [0, 0], size(2000, 1200)));
        assert_eq!(views[1], (1, [0, 0], size(3840, 1080)));
    }

    #[test]
    fn new_windows_cascade_instead_of_stacking() {
        let m = monitors();
        let mut d = here(3);
        d.place_player(1, Spot::New);
        d.place_player(2, Spot::New);
        let rects = d.rects(&m, "Built-in");
        let ((a, _), (b, _)) = (rects[1].unwrap(), rects[2].unwrap());
        // Window 3 a step right and down of Window 2, on the same diagonal.
        assert!(b[0] > a[0] && b[0] - a[0] == b[1] - a[1]);
        assert_eq!((cascade(2), cascade(3), cascade(8)), (0.0, 32.0, 0.0));
    }

    #[test]
    fn put_at_a_place_among_a_window_s_players() {
        let mut d = here(3);
        d.put_at(2, 0);
        assert_eq!(d.order, [2, 0, 1]);
        d.put_at(2, 5); // past the last: after them
        assert_eq!(d.order, [0, 1, 2]);
        // Players in another window keep their places.
        d.players = vec![1, 2, 1];
        d.put_at(2, 0);
        assert_eq!(d.sharing(2, 1), [0]);
        assert_eq!(d.order, [2, 0, 1]);
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

    #[test]
    fn two_players_swap_in_a_window_and_between_windows() {
        let m = monitors();
        // In one window: P1 and P2 trade sides.
        let mut d = here(2);
        d.swap(0, 1);
        assert_eq!(d.order, [1, 0]);
        let views = d.preview(&m, "Built-in");
        assert!(views[1].1[0] < views[0].1[0], "P2 left of P1");
        // Between windows: P2 on the TV, P3 in Window 1; they trade windows.
        let mut d = here(3);
        d.place_player(1, Spot::Fill("TV".into()));
        d.swap(1, 2);
        assert_eq!(d.players, [1, 1, 2]);
    }

    #[test]
    fn players_placed() {
        let m = monitors();
        let mut d = here(3);
        d.place_player(1, Spot::Fill("TV".into()));
        d.place_player(2, Spot::Window(2));
        assert_eq!(d.players, [1, 2, 2]);
        d.place_player(2, Spot::New);
        assert_eq!(d.players, [1, 2, 3]);
        assert!(d.warnings(&m, "Built-in").is_empty());
        // The window left behind stays while the draft is edited (numbers hold still), and
        // closes at Apply; Window 1 never.
        d.place_player(1, Spot::Window(1));
        let numbers = |d: &Draft| d.windows.iter().map(|w| w.number).collect::<Vec<_>>();
        assert_eq!(numbers(&d), [1, 2, 3]);
        assert!(d.empty(2));
        d.drop_empty(&[]);
        assert_eq!(numbers(&d), [1, 3]);
        assert!(d.warnings(&m, "Built-in").is_empty());
        d.place_player(0, Spot::Window(3));
        d.place_player(1, Spot::Window(3));
        assert_eq!(d.warnings(&m, "Built-in"), [Warning::Empty(1)]);
    }

    #[test]
    fn windows_numbered_lowest_free() {
        let mut d = here(3);
        assert_eq!(d.add_window(), 2);
        assert_eq!(d.add_window(), 3);
        d.windows.retain(|w| w.number != 2);
        assert_eq!(d.add_window(), 2);
        // Nobody to place: no window opens for it.
        d.place_player(9, Spot::New);
        assert_eq!(d.windows.len(), 3);
    }

    #[test]
    fn a_window_over_window_1s_monitor() {
        let m = monitors();
        let mut d = here(2);
        d.place_player(1, Spot::Fill("Built-in".into()));
        assert_eq!(d.warnings(&m, "Built-in"), [Warning::OnWindow1(2)]);
        // As the game does: P2 beside P1 in Window 1.
        assert_eq!(d.preview(&m, "Built-in")[1].0, 0);
    }

    #[test]
    fn warnings_and_an_unplugged_monitor() {
        let m = monitors();
        let mut d = here(2).desk_and_tv(&m, "Built-in").unwrap();
        let three = d.add_window();
        assert_eq!(d.warnings(&m, "Built-in"), [Warning::Empty(three)]);
        // The TV unplugged: its player plays in Window 1 beside P1.
        assert_eq!(
            d.warnings(&m[..1], "Built-in"),
            [Warning::Unplugged(2, "TV".into()), Warning::Empty(three)]
        );
        let views = d.preview(&m[..1], "Built-in");
        assert_eq!(views[1], (0, [1000, 0], size(1000, 1200)));
        // P1 elsewhere: Window 1 is blank.
        d.players = vec![2, 2];
        assert_eq!(d.warnings(&m, "Built-in")[0], Warning::Empty(1));
    }

    #[test]
    fn eight_players_two_windows() {
        let m = monitors();
        let d = here(8).desk_and_tv(&m, "Built-in").unwrap();
        let views = d.preview(&m, "Built-in");
        // Seven on the TV: rows of 3, 2 and 2, the whole TV covered.
        let area: u64 = views[1..]
            .iter()
            .map(|v| u64::from(v.2.width) * u64::from(v.2.height))
            .sum();
        assert_eq!(area, 3840 * 2160);
        assert!(views[1..].iter().all(|v| v.0 == 1));
    }
}
