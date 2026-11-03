//! Window sizing and frame pacing of 26.3: `Window.calculateScale` and
//! `setGuiScale`, `MouseHandler`'s scaled positions, `FramerateLimitTracker`,
//! `FramerateLimiter`, the vsync present-mode choice and the fullscreen
//! request/state pair.

/// `Window.calculateScale`: the largest scale that leaves at least 320x240
/// GUI pixels, capped by `max_scale` (0 = automatic); an odd result is bumped
/// to even when unicode is enforced.
pub fn calculate_scale(framebuffer: [i32; 2], max_scale: i32, enforce_unicode: bool) -> i32 {
    let [width, height] = framebuffer;
    let mut scale = 1;
    while scale != max_scale
        && scale < width
        && scale < height
        && width / (scale + 1) >= 320
        && height / (scale + 1) >= 240
    {
        scale += 1;
    }
    if enforce_unicode && scale % 2 != 0 {
        scale += 1;
    }
    scale
}

/// `Window.setGuiScale`: the GUI-scaled size of a framebuffer.
pub fn gui_scaled_size(framebuffer: [i32; 2], gui_scale: i32) -> [i32; 2] {
    let scaled = |v: i32| (f64::from(v) / f64::from(gui_scale)).ceil() as i32;
    [scaled(framebuffer[0]), scaled(framebuffer[1])]
}

/// `MouseHandler.getScaledXPos`/`getScaledYPos`.
pub fn scaled_pos(pos: f64, gui_scaled: i32, screen: i32) -> f64 {
    pos * f64::from(gui_scaled) / f64::from(screen)
}

/// `InactivityFpsLimit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InactivityFpsLimit {
    Minimized,
    Afk,
}

/// `FramerateLimitTracker.FramerateThrottleReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrottleReason {
    None,
    WindowIconified,
    LongAfk,
    ShortAfk,
    OutOfLevelMenu,
}

/// What `FramerateLimitTracker` looks at.
#[derive(Clone, Copy, Debug)]
pub struct FrameContext {
    pub inactivity: InactivityFpsLimit,
    pub iconified: bool,
    pub focused: bool,
    pub exclusive_fullscreen: bool,
    pub now_ms: i64,
    pub latest_input_ms: i64,
    pub has_level: bool,
    pub has_screen: bool,
    pub has_overlay: bool,
}

/// `getThrottleReason`.
pub fn throttle_reason(c: FrameContext) -> ThrottleReason {
    if !c.iconified && (c.focused || !c.exclusive_fullscreen) {
        if c.inactivity == InactivityFpsLimit::Afk {
            let afk = c.now_ms - c.latest_input_ms;
            if afk > 600_000 {
                return ThrottleReason::LongAfk;
            }
            if afk > 60_000 {
                return ThrottleReason::ShortAfk;
            }
        }
        if c.has_level || (!c.has_screen && !c.has_overlay) {
            ThrottleReason::None
        } else {
            ThrottleReason::OutOfLevelMenu
        }
    } else {
        ThrottleReason::WindowIconified
    }
}

/// `getFramerateLimit`.
pub fn framerate_limit(option_limit: i32, c: FrameContext) -> i32 {
    match throttle_reason(c) {
        ThrottleReason::None => option_limit,
        ThrottleReason::WindowIconified | ThrottleReason::LongAfk => 10,
        ThrottleReason::ShortAfk => option_limit.min(30),
        ThrottleReason::OutOfLevelMenu => 60,
    }
}

/// `isHeavilyThrottled`.
pub fn heavily_throttled(reason: ThrottleReason) -> bool {
    matches!(
        reason,
        ThrottleReason::WindowIconified | ThrottleReason::LongAfk
    )
}

/// Present modes the renderer can choose between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    Fifo,
    Mailbox,
    Immediate,
}

/// `GpuSurface.PresentMode.getSupportedVsyncMode`: vsync means Fifo only,
/// otherwise the first of Immediate, Mailbox, Fifo that is supported.
pub fn vsync_present_mode(supported: &[PresentMode], vsync: bool) -> Option<PresentMode> {
    let preferred: &[PresentMode] = if vsync {
        &[PresentMode::Fifo]
    } else {
        &[
            PresentMode::Immediate,
            PresentMode::Mailbox,
            PresentMode::Fifo,
        ]
    };
    preferred.iter().copied().find(|m| supported.contains(m))
}

/// The present mode for the VSync option and Horizon's present-mode setting
/// (`None` for Auto). VSync on always means Fifo, in a window and in
/// fullscreen alike, as in vanilla; the setting chooses only while VSync is
/// off, when the surface supports it, and otherwise vanilla's choice applies.
pub fn chosen_present_mode(
    supported: &[PresentMode],
    vsync: bool,
    setting: Option<PresentMode>,
) -> Option<PresentMode> {
    setting
        .filter(|m| !vsync && supported.contains(m))
        .or_else(|| vsync_present_mode(supported, vsync))
}

/// `FramerateLimiter.limitDisplayFPS` with an injectable clock so the sleep
/// and spin decisions are testable.
#[derive(Clone, Debug)]
pub struct FramerateLimiter {
    last_frame_time: i64,
    average_overshoot_ns: i64,
    last_framerate_limit: i32,
}

/// Time source of the limiter.
pub trait LimiterClock {
    fn nanos(&mut self) -> i64;
    /// `LockSupport.parkNanos`.
    fn park(&mut self, nanos: i64);
    /// `Thread.onSpinWait`.
    fn spin(&mut self);
    /// `Thread.interrupted`: an interrupted limiter skips that sleep.
    fn interrupted(&mut self) -> bool {
        false
    }
}

/// The real clock: `System.nanoTime` from a fixed origin, sleeping for
/// `LockSupport.parkNanos` and a CPU spin hint for `Thread.onSpinWait`.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    origin: std::time::Instant,
}
impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}
impl LimiterClock for SystemClock {
    fn nanos(&mut self) -> i64 {
        i64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(i64::MAX)
    }
    fn park(&mut self, nanos: i64) {
        let end = self.nanos().saturating_add(nanos);
        park_in_steps(
            end,
            || self.nanos(),
            |step| {
                std::thread::sleep(std::time::Duration::from_nanos(step as u64));
            },
        );
    }
    fn spin(&mut self) {
        std::hint::spin_loop();
    }
}

/// The longest single sleep of [`SystemClock::park`].
const PARK_STEP_NS: i64 = 1_000_000;

/// Sleeps until `now()` reaches `end`, at most [`PARK_STEP_NS`] at a time. macOS lets
/// a sleep run late by a share of its length (timer coalescing: 1.8-3.5 ms on a 14 ms
/// sleep, 0.25 ms on 1 ms), more than the limiter's overshoot margin can learn (capped
/// at 2 ms): a 60 FPS cap gave frames of 17-20 ms. Short steps keep the lateness to
/// the last step's.
fn park_in_steps(end: i64, mut now: impl FnMut() -> i64, mut sleep: impl FnMut(i64)) {
    loop {
        let left = end - now();
        if left <= 0 {
            return;
        }
        sleep(left.min(PARK_STEP_NS));
    }
}

/// The real clock without busy waiting: where vanilla spins on
/// `Thread.onSpinWait` for the last half millisecond, this sleeps in short
/// steps, so a capped frame rate leaves the core idle.
#[derive(Clone, Copy, Debug, Default)]
pub struct SleepingClock(pub SystemClock);
impl LimiterClock for SleepingClock {
    fn nanos(&mut self) -> i64 {
        self.0.nanos()
    }
    fn park(&mut self, nanos: i64) {
        self.0.park(nanos);
    }
    fn spin(&mut self) {
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
}

/// `Minecraft.runTick`'s frame-limiter step: only limits below 260
/// (the "unlimited" setting) pace frames.
pub fn end_frame(
    limiter: &mut FramerateLimiter,
    framerate_limit: i32,
    clock: &mut impl LimiterClock,
) {
    if framerate_limit < 260 {
        limiter.limit_display_fps(framerate_limit, clock);
    }
}

/// [`end_frame`] on a fixed grid of frame slots ([`FramerateLimiter::limit_on_grid`]):
/// Horizon's Efficient pacing.
pub fn end_frame_on_grid(
    limiter: &mut FramerateLimiter,
    framerate_limit: i32,
    clock: &mut impl LimiterClock,
) {
    if framerate_limit < 260 {
        limiter.limit_on_grid(framerate_limit, clock);
    }
}

impl FramerateLimiter {
    pub fn new(now_ns: i64) -> Self {
        Self::with_state(now_ns, 0, 0)
    }
    /// The limiter's static state: `lastFrameTime`, `averageOvershootNs`
    /// and `lastFramerateLimit`.
    pub fn with_state(
        last_frame_time: i64,
        average_overshoot_ns: i64,
        last_framerate_limit: i32,
    ) -> Self {
        Self {
            last_frame_time,
            average_overshoot_ns,
            last_framerate_limit,
        }
    }
    pub fn average_overshoot_ns(&self) -> i64 {
        self.average_overshoot_ns
    }
    pub fn last_framerate_limit(&self) -> i32 {
        self.last_framerate_limit
    }
    pub fn limit_display_fps(&mut self, framerate_limit: i32, clock: &mut impl LimiterClock) {
        let target_time_per_frame = 1_000_000_000 / i64::from(framerate_limit);
        let target_time_ns = self.last_frame_time + target_time_per_frame;
        if framerate_limit != self.last_framerate_limit {
            self.average_overshoot_ns = 0;
            self.last_framerate_limit = framerate_limit;
        }
        self.wait_until(target_time_ns, clock);
        self.last_frame_time = clock.nanos();
    }
    /// Horizon's Efficient pacing: [`Self::limit_display_fps`]'s wait, but each frame's
    /// slot starts where the last one's ended (its deadline), not when the thread woke
    /// up. Vanilla's way adds every late wake-up to the frame (on the Mac, 50 us sleeps
    /// running 0.2 ms late made a 144 FPS cap 139); on a fixed grid the cap holds on
    /// average. A frame that ends past its slot, or wakes up more than half a frame
    /// late, starts the grid again from now: no frame is rushed out to catch up.
    pub fn limit_on_grid(&mut self, framerate_limit: i32, clock: &mut impl LimiterClock) {
        let period = 1_000_000_000 / i64::from(framerate_limit);
        if framerate_limit != self.last_framerate_limit {
            self.average_overshoot_ns = 0;
            self.last_framerate_limit = framerate_limit;
        }
        let deadline = self.last_frame_time + period;
        if clock.nanos() >= deadline {
            self.last_frame_time = clock.nanos();
            return;
        }
        self.wait_until(deadline, clock);
        let woke = clock.nanos();
        self.last_frame_time = if woke - deadline > period / 2 {
            woke
        } else {
            deadline
        };
    }
    /// `limitDisplayFPS`'s wait: sleep to within the learned overshoot and half a
    /// millisecond of `target_time_ns`, then spin.
    fn wait_until(&mut self, target_time_ns: i64, clock: &mut impl LimiterClock) {
        loop {
            let remaining = target_time_ns - clock.nanos();
            if remaining <= 0 {
                break;
            }
            if remaining > self.average_overshoot_ns + 500_000 {
                let sleep_start = clock.nanos();
                let expected = remaining - self.average_overshoot_ns - 500_000;
                if !clock.interrupted() {
                    clock.park(expected);
                    let duration = clock.nanos() - sleep_start;
                    let overshoot = duration - expected;
                    if overshoot > 0 && overshoot < 25_000_000 {
                        self.average_overshoot_ns = (0.1 * overshoot as f64
                            + 0.9 * self.average_overshoot_ns as f64)
                            as i64;
                        self.average_overshoot_ns = self.average_overshoot_ns.min(2_000_000);
                    }
                }
            } else {
                clock.spin();
            }
        }
    }
    pub fn last_frame_time(&self) -> i64 {
        self.last_frame_time
    }
}

/// blaze3d `Window`'s `fullscreenRequested` / `fullscreen` pair. The window
/// system has the last word: a change it makes on its own (a compositor
/// shortcut, a window rule) is adopted by the Fullscreen option
/// (`Window.updateFullscreenState` -> `Minecraft.fullscreenStateChanged`), and
/// the window is only asked to change when the option itself changes
/// (`Window.updateFullscreenIfChanged`), never re-forced to the option.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FullscreenState {
    /// The state the window system last reported (`Window.fullscreen`).
    actual: bool,
    /// The state last asked of the window system, or adopted from it.
    requested: bool,
    /// `Window.exclusiveFullscreen`.
    exclusive: bool,
    /// `Window.preferredFullscreenVideoMode`.
    mode: Option<VideoMode>,
    /// `Window.dirty`: the preferred mode changed and the screen has not been left yet.
    mode_dirty: bool,
}

impl FullscreenState {
    /// A window created windowed or, when `requested`, asked to be fullscreen.
    pub fn new(requested: bool) -> Self {
        Self {
            actual: false,
            requested,
            exclusive: false,
            mode: None,
            mode_dirty: false,
        }
    }

    /// `Window.updateFullscreenIfChanged` with the exclusive option and preferred mode: asks the
    /// window to enter, leave or change its fullscreen only when one of them changed.
    /// `in_video_screen` holds a changed mode back until the Video Settings screen closes
    /// (`changeFullscreenVideoMode`).
    pub fn apply(
        &mut self,
        window: &winit::window::Window,
        wanted: bool,
        exclusive: bool,
        preferred: Option<VideoMode>,
        in_video_screen: bool,
    ) {
        let request = self.request(wanted);
        let reapply = self.set_exclusive(exclusive);
        self.set_preferred_mode(preferred);
        let mode_change = !in_video_screen && self.take_mode_change();
        match request {
            Some(false) => window.set_fullscreen(None),
            Some(true) => window.set_fullscreen(Some(fullscreen_request(
                window,
                exclusive,
                preferred.as_ref(),
            ))),
            None if reapply || mode_change => window.set_fullscreen(Some(fullscreen_request(
                window,
                exclusive,
                preferred.as_ref(),
            ))),
            None => {}
        }
    }

    /// `Window.setExclusiveFullscreen`: true when the option changed while fullscreen is
    /// requested, so the window has to enter the other kind of fullscreen at once.
    pub fn set_exclusive(&mut self, exclusive: bool) -> bool {
        let changed = self.exclusive != exclusive;
        self.exclusive = exclusive;
        changed && self.requested
    }

    /// `Window.setPreferredFullscreenVideoMode`: a changed mode waits for `take_mode_change`.
    pub fn set_preferred_mode(&mut self, mode: Option<VideoMode>) {
        if self.mode != mode {
            self.mode = mode;
            self.mode_dirty = true;
        }
    }

    /// `Window.changeFullscreenVideoMode` (the Video Settings screen closing): true once when
    /// a changed mode has to be applied to a fullscreen window.
    pub fn take_mode_change(&mut self) -> bool {
        let apply = self.requested && self.mode_dirty;
        if apply {
            self.mode_dirty = false;
        }
        apply
    }

    /// `Window.updateFullscreenIfChanged`: the state to ask of the window
    /// when the option `wanted` differs from what was last asked or adopted.
    pub fn request(&mut self, wanted: bool) -> Option<bool> {
        (wanted != self.requested).then(|| {
            self.requested = wanted;
            wanted
        })
    }

    /// `Window.updateFullscreenState`: the window now reports `now`; returns
    /// it when it changed, for the option to adopt.
    pub fn observe(&mut self, now: bool) -> Option<bool> {
        (now != self.actual).then(|| {
            self.actual = now;
            self.requested = now;
            now
        })
    }
}

/// `VideoMode`: one display mode of a monitor, as `fullscreenResolution` in options.txt keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMode {
    pub width: i32,
    pub height: i32,
    pub red_bits: i32,
    pub green_bits: i32,
    pub blue_bits: i32,
    pub refresh_rate: f32,
}

impl VideoMode {
    /// `VideoMode.read`: the JSON object `write` makes; any other string is None.
    pub fn read(text: &str) -> Option<Self> {
        let number = |key: &str| -> Option<f64> {
            let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
            let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')))
                .unwrap_or(rest.len());
            rest[..end].parse().ok()
        };
        if !text.trim_start().starts_with('{') {
            return None;
        }
        Some(Self {
            width: number("width")? as i32,
            height: number("height")? as i32,
            red_bits: number("red_bits")? as i32,
            green_bits: number("green_bits")? as i32,
            blue_bits: number("blue_bits")? as i32,
            refresh_rate: number("refresh_rate")? as f32,
        })
    }

    /// `VideoMode.write`: the Codec's JSON through Gson (a float prints as `60.0`).
    pub fn write(&self) -> String {
        let rate = if self.refresh_rate == self.refresh_rate.trunc() {
            format!("{:.1}", self.refresh_rate)
        } else {
            self.refresh_rate.to_string()
        };
        format!(
            "{{\"width\":{},\"height\":{},\"red_bits\":{},\"green_bits\":{},\"blue_bits\":{},\"refresh_rate\":{rate}}}",
            self.width, self.height, self.red_bits, self.green_bits, self.blue_bits
        )
    }

    /// `VideoMode.refreshRateLabel`: a whole rate without decimals, any other with two.
    pub fn refresh_rate_label(&self) -> String {
        if self.refresh_rate == self.refresh_rate.round_ties_even() {
            (self.refresh_rate as i32).to_string()
        } else {
            format!("{:.2}", self.refresh_rate)
        }
    }

    /// The colour depth `options.fullscreen.entry` shows.
    pub fn bits(&self) -> i32 {
        self.red_bits + self.green_bits + self.blue_bits
    }

    /// A winit mode: its depth is the total, split evenly over the channels.
    pub fn from_winit(mode: &winit::monitor::VideoModeHandle) -> Self {
        let depth = i32::from(mode.bit_depth());
        let size = mode.size();
        Self {
            width: size.width as i32,
            height: size.height as i32,
            red_bits: depth / 3,
            green_bits: depth / 3,
            blue_bits: depth - 2 * (depth / 3),
            refresh_rate: mode.refresh_rate_millihertz() as f32 / 1000.0,
        }
    }
}

/// `Monitor.videoModes`: the modes a monitor offers.
pub fn monitor_video_modes(monitor: &winit::monitor::MonitorHandle) -> Vec<VideoMode> {
    monitor
        .video_modes()
        .map(|m| VideoMode::from_winit(&m))
        .collect()
}

/// The video modes of the window's monitor, listed once per monitor. Listing them takes tens of
/// milliseconds on Windows, and settings apply at every step of a dragged slider: listing them
/// each time made the slider stutter.
#[derive(Debug)]
pub struct VideoModeList<M = winit::monitor::MonitorHandle> {
    listed: Option<M>,
}

impl<M> Default for VideoModeList<M> {
    fn default() -> Self {
        Self { listed: None }
    }
}

impl<M: PartialEq> VideoModeList<M> {
    /// The modes of `monitor` when it is not the monitor listed last, else `None` (the list
    /// stands). No monitor lists none.
    pub fn refresh(
        &mut self,
        monitor: Option<M>,
        list: impl FnOnce(&M) -> Vec<VideoMode>,
    ) -> Option<Vec<VideoMode>> {
        if monitor.is_some() && monitor == self.listed {
            return None;
        }
        let modes = monitor.as_ref().map(list).unwrap_or_default();
        self.listed = monitor;
        Some(modes)
    }
}

/// `Monitor.currentMode`: the monitor's desktop mode; winit has no such query, so the mode of
/// the monitor's own size with the highest refresh rate and depth.
fn monitor_current_mode(
    monitor: &winit::monitor::MonitorHandle,
) -> Option<winit::monitor::VideoModeHandle> {
    let size = monitor.size();
    monitor
        .video_modes()
        .filter(|m| m.size() == size)
        .max_by_key(|m| (m.refresh_rate_millihertz(), m.bit_depth()))
}

/// `Window.applyFullscreen`: the fullscreen the window asks for. Exclusive fullscreen takes
/// `Monitor.getPreferredVideoMode` (the preferred mode if the monitor has it, else the desktop
/// mode); a monitor with no usable mode falls back to borderless, as vanilla does.
pub fn fullscreen_request(
    window: &winit::window::Window,
    exclusive: bool,
    preferred: Option<&VideoMode>,
) -> winit::window::Fullscreen {
    use winit::window::Fullscreen;
    if !exclusive {
        return Fullscreen::Borderless(None);
    }
    let Some(monitor) = window
        .current_monitor()
        .or_else(|| window.primary_monitor())
    else {
        return Fullscreen::Borderless(None);
    };
    let wanted = preferred.and_then(|p| {
        monitor
            .video_modes()
            .find(|m| VideoMode::from_winit(m) == *p)
    });
    match wanted.or_else(|| monitor_current_mode(&monitor)) {
        Some(mode) => Fullscreen::Exclusive(mode),
        None => Fullscreen::Borderless(None),
    }
}

/// Horizon's icon for the window's title bar and the task bar (X11 and Windows; Wayland takes
/// it from the `horizon.desktop` entry, macOS from the app bundle). `None` if it can't be read.
pub fn icon() -> Option<winit::window::Icon> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(horizon_theme::ICON_PNG));
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    bytes.truncate(info.buffer_size());
    winit::window::Icon::from_rgba(bytes, info.width, info.height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_mode_round_trips_the_options_json() {
        let mode = VideoMode {
            width: 1920,
            height: 1080,
            red_bits: 8,
            green_bits: 8,
            blue_bits: 8,
            refresh_rate: 60.0,
        };
        let text = mode.write();
        assert_eq!(
            text,
            r#"{"width":1920,"height":1080,"red_bits":8,"green_bits":8,"blue_bits":8,"refresh_rate":60.0}"#
        );
        assert_eq!(VideoMode::read(&text), Some(mode));
        assert_eq!(VideoMode::read("1920x1080"), None);
        assert_eq!(VideoMode::read(r#"{"width":1}"#), None);
    }
    #[test]
    fn refresh_rate_label_is_whole_or_two_decimals() {
        let mut mode = VideoMode::read(
            r#"{"width":1,"height":1,"red_bits":8,"green_bits":8,"blue_bits":8,"refresh_rate":144.0}"#,
        )
        .unwrap();
        assert_eq!(mode.refresh_rate_label(), "144");
        mode.refresh_rate = 59.94;
        assert_eq!(mode.refresh_rate_label(), "59.94");
        assert_eq!(mode.bits(), 24);
    }
    #[test]
    fn the_window_icon_decodes() {
        assert!(icon().is_some());
    }

    #[test]
    fn exclusive_fullscreen_reapplies_only_while_fullscreen_is_requested() {
        let mut f = FullscreenState::new(false);
        assert!(!f.set_exclusive(true));
        assert!(!f.set_exclusive(true));
        assert_eq!(f.request(true), Some(true));
        assert!(f.set_exclusive(false));
        assert!(!f.set_exclusive(false));
    }
    #[test]
    fn a_changed_video_mode_applies_once_when_fullscreen() {
        let mode = VideoMode::read(
            r#"{"width":1,"height":1,"red_bits":8,"green_bits":8,"blue_bits":8,"refresh_rate":60.0}"#,
        );
        let mut f = FullscreenState::new(false);
        f.set_preferred_mode(mode);
        assert!(!f.take_mode_change());
        f.request(true);
        f.set_preferred_mode(mode);
        assert!(f.take_mode_change());
        assert!(!f.take_mode_change());
    }
    #[test]
    fn fullscreen_follows_the_option_only_when_it_changes() {
        let mut f = FullscreenState::new(false);
        // Settings applied again with nothing changed: the window is left alone.
        assert_eq!(f.request(false), None);
        // The compositor makes the window fullscreen on its own: the option adopts it,
        // and applying the settings afterwards does not undo it.
        assert_eq!(f.observe(true), Some(true));
        assert_eq!(f.request(true), None);
        assert_eq!(f.observe(true), None);
        // F11: one request, not repeated while the compositor catches up.
        assert_eq!(f.request(false), Some(false));
        assert_eq!(f.request(false), None);
        assert_eq!(f.observe(false), Some(false));
        assert_eq!(f.request(false), None);
    }

    #[test]
    fn fullscreen_requests_changed_before_the_compositor_answers() {
        // Created fullscreen from options.txt.
        let mut f = FullscreenState::new(true);
        assert_eq!(f.request(true), None);
        assert_eq!(f.observe(true), Some(true));
        // Toggled twice before any configure: the second request cancels the first.
        assert_eq!(f.request(false), Some(false));
        assert_eq!(f.request(true), Some(true));
        assert_eq!(f.observe(true), None);
        // A compositor that refuses leaves the option where the player put it.
        assert_eq!(f.request(false), Some(false));
        assert_eq!(f.request(false), None);
    }

    struct Virtual {
        now: i64,
        oversleep: i64,
    }
    impl LimiterClock for Virtual {
        fn nanos(&mut self) -> i64 {
            self.now
        }
        fn park(&mut self, nanos: i64) {
            self.now += nanos + self.oversleep;
        }
        fn spin(&mut self) {
            self.now += 1000;
        }
    }

    #[test]
    fn scale_matches_the_320_by_240_rule() {
        assert_eq!(calculate_scale([854, 480], 0, false), 2);
        assert_eq!(calculate_scale([1920, 1080], 0, false), 4);
        assert_eq!(calculate_scale([1920, 1080], 3, false), 3);
        assert_eq!(calculate_scale([1920, 1080], 3, true), 4);
        assert_eq!(gui_scaled_size([1921, 1081], 4), [481, 271]);
    }
    /// A clock whose sleeps run late by a quarter of their length, as macOS's do,
    /// sleeping as [`SystemClock::park`] does.
    struct Coalescing {
        now: std::cell::Cell<i64>,
    }
    impl LimiterClock for Coalescing {
        fn nanos(&mut self) -> i64 {
            self.now.get()
        }
        fn park(&mut self, nanos: i64) {
            let now = &self.now;
            park_in_steps(
                now.get() + nanos,
                || now.get(),
                |step| {
                    now.set(now.get() + step + step / 4);
                },
            );
        }
        fn spin(&mut self) {
            self.now.set(self.now.get() + 1000);
        }
    }
    #[test]
    fn a_frame_cap_holds_when_sleeps_run_late() {
        let mut clock = Coalescing {
            now: std::cell::Cell::new(0),
        };
        let mut limiter = FramerateLimiter::new(0);
        let mut last = 0;
        for frame in 0..120 {
            // The frame's own work.
            clock.now.set(clock.now.get() + 2_000_000);
            limiter.limit_display_fps(60, &mut clock);
            let period = clock.now.get() - last;
            last = clock.now.get();
            assert!(
                (16_666_666..16_666_666 + 100_000).contains(&period),
                "frame {frame}: {period} ns"
            );
        }
    }
    /// A clock whose sleeps and spins each run up to `late` ns late (a different
    /// share each time), as a loaded machine's do.
    struct Late {
        now: i64,
        late: i64,
        seed: u64,
    }
    impl Late {
        fn new(late: i64) -> Self {
            Self {
                now: 0,
                late,
                seed: 0x9e37_79b9_7f4a_7c15,
            }
        }
        fn lateness(&mut self) -> i64 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 7;
            self.seed ^= self.seed << 17;
            (self.seed % 1000) as i64 * self.late / 1000
        }
    }
    impl LimiterClock for Late {
        fn nanos(&mut self) -> i64 {
            self.now
        }
        fn park(&mut self, nanos: i64) {
            self.now += nanos + self.lateness();
        }
        fn spin(&mut self) {
            self.now += 50_000 + self.lateness();
        }
    }
    /// Frame periods of 144 FPS frames whose work takes `work(frame)` ns.
    fn grid_periods(late: i64, work: impl Fn(usize) -> i64) -> Vec<i64> {
        let mut clock = Late::new(late);
        let mut limiter = FramerateLimiter::new(0);
        let mut ends = Vec::new();
        for frame in 0..300 {
            clock.now += work(frame);
            limiter.limit_on_grid(144, &mut clock);
            ends.push(clock.now);
        }
        ends.windows(2).map(|w| w[1] - w[0]).collect()
    }
    #[test]
    fn the_grid_holds_the_cap_when_wake_ups_run_late() {
        let period = 1_000_000_000 / 144;
        for late in [0, 50_000, 200_000, 1_000_000] {
            let periods = grid_periods(late, |_| 2_000_000);
            let mean = periods[10..].iter().sum::<i64>() / (periods.len() - 10) as i64;
            assert!(
                (mean - period).abs() < 10_000,
                "{late} ns late: mean period {mean} ns"
            );
            // Each slot waits at least half a frame: the grid never rushes one out.
            assert!(
                periods.iter().all(|&p| p > period / 2),
                "{late} ns late: {periods:?}"
            );
        }
        // Vanilla's way adds the lateness to the frames: the cap isn't held.
        let mut clock = Late::new(400_000);
        let mut limiter = FramerateLimiter::new(0);
        for _ in 0..300 {
            clock.now += 2_000_000;
            limiter.limit_display_fps(144, &mut clock);
        }
        assert!(
            clock.now / 300 > period + 50_000,
            "vanilla: {} ns",
            clock.now / 300
        );
    }
    #[test]
    fn a_long_frame_starts_the_grid_again_without_a_burst() {
        let period = 1_000_000_000 / 144;
        // Frame 100 works 30 ms: four slots go by.
        let periods = grid_periods(
            50_000,
            |frame| if frame == 100 { 30_000_000 } else { 2_000_000 },
        );
        assert!(periods[99] >= 30_000_000);
        for (i, &p) in periods.iter().enumerate().skip(100) {
            // On the grid a slot is a late wake-up (50 us at most) short or long.
            assert!(
                (period - 100_000..period + 100_000).contains(&p),
                "frame {}: {p} ns",
                i + 1
            );
        }
    }
    #[test]
    fn limiter_paces_frames_at_the_target() {
        let mut limiter = FramerateLimiter::new(0);
        let mut clock = Virtual {
            now: 1_000_000,
            oversleep: 200_000,
        };
        limiter.limit_display_fps(100, &mut clock);
        assert!(clock.now >= 10_000_000);
        assert!(clock.now < 10_000_000 + 1_000_000);
    }
    #[test]
    fn vsync_only_uses_fifo() {
        let all = [
            PresentMode::Mailbox,
            PresentMode::Fifo,
            PresentMode::Immediate,
        ];
        assert_eq!(vsync_present_mode(&all, true), Some(PresentMode::Fifo));
        assert_eq!(
            vsync_present_mode(&all, false),
            Some(PresentMode::Immediate)
        );
        assert_eq!(
            vsync_present_mode(&[PresentMode::Fifo], false),
            Some(PresentMode::Fifo)
        );
        // Regression guard: GLES capabilities report [Fifo, Immediate] without Mailbox.
        assert_eq!(
            vsync_present_mode(&[PresentMode::Fifo, PresentMode::Immediate], false),
            Some(PresentMode::Immediate)
        );
    }
    #[test]
    fn vsync_on_waits_for_the_display_whatever_the_present_mode() {
        use PresentMode::*;
        let all = [Mailbox, Fifo, Immediate];
        for setting in [None, Some(Fifo), Some(Mailbox), Some(Immediate)] {
            assert_eq!(chosen_present_mode(&all, true, setting), Some(Fifo));
        }
        assert_eq!(chosen_present_mode(&all, false, None), Some(Immediate));
        assert_eq!(
            chosen_present_mode(&all, false, Some(Mailbox)),
            Some(Mailbox)
        );
        assert_eq!(chosen_present_mode(&all, false, Some(Fifo)), Some(Fifo));
        // Metal has no Mailbox: vanilla's choice instead.
        assert_eq!(
            chosen_present_mode(&[Fifo, Immediate], false, Some(Mailbox)),
            Some(Immediate)
        );
    }

    #[test]
    fn video_modes_are_listed_again_only_for_another_monitor() {
        let mode = |width| VideoMode {
            width,
            height: 1080,
            red_bits: 8,
            green_bits: 8,
            blue_bits: 8,
            refresh_rate: 60.0,
        };
        let mut list = VideoModeList::default();
        let mut listings = 0;
        let mut modes = |monitor: &i32| {
            listings += 1;
            vec![mode(*monitor)]
        };
        assert_eq!(list.refresh(Some(1), &mut modes), Some(vec![mode(1)]));
        assert_eq!(list.refresh(Some(1), &mut modes), None);
        assert_eq!(list.refresh(Some(2), &mut modes), Some(vec![mode(2)]));
        assert_eq!(list.refresh(None, &mut modes), Some(vec![]));
        assert_eq!(list.refresh(Some(2), &mut modes), Some(vec![mode(2)]));
        assert_eq!(listings, 3);
    }
}
