//! The debug charts (26.3 `debugchart`): F3+2 shows the frame times and, in
//! single player, the integrated server's tick times; F3+3 the ping and, on a
//! remote server, the bandwidth. Each chart keeps the last 240 samples
//! (`LocalSampleLogger`) and draws one bar per sample over a 60 pixel high
//! box (`AbstractDebugChart`).
use crate::{
    font::Font,
    hud_layout::{GuiDraw, argb, fill},
    text::Text,
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// `LocalSampleLogger.CAPACITY`.
pub const CAPACITY: usize = 240;
/// `AbstractDebugChart.CHART_HEIGHT`.
pub const CHART_HEIGHT: i32 = 60;
pub const TICK_SERVER_METHOD: usize = 1;
pub const SCHEDULED_TASKS: usize = 2;
pub const IDLE: usize = 3;

/// `LocalSampleLogger`: the last [`CAPACITY`] samples, oldest first; a
/// sample has up to four values (`TpsDebugDimensions`), the first the main.
#[derive(Clone, Debug, Default)]
pub struct SampleLogger {
    samples: VecDeque<[i64; 4]>,
}
impl SampleLogger {
    /// `logSample`.
    pub fn log(&mut self, value: i64) {
        self.log_full([value, 0, 0, 0]);
    }
    /// `logFullSample`.
    pub fn log_full(&mut self, sample: [i64; 4]) {
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
    pub fn reset(&mut self) {
        self.samples.clear();
    }
    fn get(&self, index: usize, dimension: usize) -> i64 {
        self.samples[index][dimension]
    }
}

/// `DebugScreenOverlay`'s chart switches and the samples behind them.
#[derive(Clone, Debug, Default)]
pub struct DebugCharts {
    /// `renderFpsCharts` (F3+2).
    pub fps: bool,
    /// `renderNetworkCharts` (F3+3).
    pub network: bool,
    /// `renderLightmapTexture` (F3+4).
    pub lightmap: bool,
    /// `frameTimeLogger`: nanoseconds between frames.
    pub frame_times: SampleLogger,
    /// `tickTimeLogger`: the integrated server's ticks.
    pub tick_times: SampleLogger,
    /// `pingLogger`: milliseconds.
    pub ping: SampleLogger,
    /// `bandwidthLogger`: bytes received per tick.
    pub bandwidth: SampleLogger,
    /// `MinecraftServer.smoothedTickTimeMillis`.
    pub smoothed_tick_ms: f32,
    /// Connection's smoothed packets per second, sent then received.
    pub packet_rates: [f32; 2],
    packet_totals: [i64; 2],
    packet_tick: u8,
    previous_tick: Option<Instant>,
    bytes_seen: Option<i64>,
}
impl DebugCharts {
    /// `toggleFpsCharts`; `overlay` is the debug overlay's visibility.
    pub fn toggle_fps(&mut self, overlay: &mut bool) {
        self.fps = !*overlay || !self.fps;
        if self.fps {
            *overlay = true;
            self.network = false;
            self.lightmap = false;
        }
    }
    /// `toggleNetworkCharts`.
    pub fn toggle_network(&mut self, overlay: &mut bool) {
        self.network = !*overlay || !self.network;
        if self.network {
            *overlay = true;
            self.fps = false;
            self.lightmap = false;
        }
    }
    /// `toggleLightmapTexture`.
    pub fn toggle_lightmap(&mut self, overlay: &mut bool) {
        self.lightmap = !*overlay || !self.lightmap;
        if self.lightmap {
            *overlay = true;
            self.fps = false;
            self.network = false;
        }
    }
    /// `reset`: leaving a world.
    pub fn reset(&mut self) {
        self.tick_times.reset();
        self.ping.reset();
        self.bandwidth.reset();
        self.previous_tick = None;
        self.bytes_seen = None;
        self.smoothed_tick_ms = 0.0;
        self.packet_rates = [0.0; 2];
        self.packet_totals = [0; 2];
        self.packet_tick = 0;
    }
    /// One integrated server tick that started at `started` and ran its
    /// tick method for `method` (`MinecraftServer.logFullTickTime` and
    /// `logTickMethodTime`): the full tick runs from one start to the next,
    /// and what the tick method leaves of it is idle.
    pub fn log_tick(&mut self, started: Instant, method: Duration) {
        let method = method.as_nanos() as i64;
        let full = self
            .previous_tick
            .map_or(method, |p| (started - p).as_nanos() as i64)
            .max(method);
        self.previous_tick = Some(started);
        self.tick_times.log_full([full, method, 0, full - method]);
        self.smoothed_tick_ms = self.smoothed_tick_ms * 0.8 + method as f32 / 1e6 * 0.199_999_99;
    }
    /// `BandwidthDebugMonitor.tick`: the bytes received since the last
    /// tick, from the connection's running total.
    pub fn log_bandwidth(&mut self, total: i64) {
        if let Some(seen) = self.bytes_seen {
            self.bandwidth.log(total - seen);
        }
        self.bytes_seen = Some(total);
    }
    /// `Connection.tickSecond`: sampled on the first tick, then every 20 ticks.
    pub fn log_packets(&mut self, totals: [i64; 2]) {
        if self.packet_tick == 0 {
            for (i, total) in totals.into_iter().enumerate() {
                let count = (total - self.packet_totals[i]).max(0) as f32;
                self.packet_rates[i] = count + 0.75 * (self.packet_rates[i] - count);
            }
            self.packet_totals = totals;
        }
        self.packet_tick = (self.packet_tick + 1) % 20;
    }
    /// Whether `draw` (or the lightmap view) puts something in the bottom
    /// left and the bottom right corner, for a `remote` server or not.
    pub fn corners(&self, remote: bool) -> [bool; 2] {
        [
            self.fps || (self.network && remote),
            (self.fps && !self.tick_times.is_empty()) || self.network || self.lightmap,
        ]
    }
    /// The charts along the bottom of the screen (`extractRenderState`):
    /// frame times left and tick times right, or bandwidth left (remote
    /// servers only) and ping right. `mspt` is the level's milliseconds
    /// per tick.
    pub fn draw(
        &self,
        font: &Font,
        width: i32,
        height: i32,
        framerate_limit: u32,
        mspt: f32,
        remote: bool,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let max_width = width / 2;
        let chart_width = Chart::width(max_width);
        if self.fps {
            let fps = Chart::Fps { framerate_limit };
            out.extend(fps.draw(font, &self.frame_times, 0, chart_width, height));
            if !self.tick_times.is_empty() {
                let tps = Chart::Tps { mspt };
                out.extend(tps.draw(
                    font,
                    &self.tick_times,
                    width - chart_width,
                    chart_width,
                    height,
                ));
            }
        }
        if self.network {
            if remote {
                out.extend(Chart::Bandwidth.draw(font, &self.bandwidth, 0, chart_width, height));
            }
            out.extend(Chart::Ping.draw(
                font,
                &self.ping,
                width - chart_width,
                chart_width,
                height,
            ));
        }
        out
    }
}

/// The lightmap view (`showLightmapTexture`): the 16 by 16 texture at 64
/// by 64 in the bottom right corner on a black border, sky light up and
/// block light right (the blit flips the texture's rows). `pixels` are RGBA
/// rows of sky level by block level.
pub fn lightmap(pixels: &[u8; 16 * 16 * 4], width: i32, height: i32) -> Vec<GuiDraw> {
    let (x, y) = (width - 64 - 2, height - 64 - 2);
    let mut out = vec![fill(x - 1, y - 1, x + 64 + 1, y + 64 + 1, 0xFF00_0000)];
    for (i, p) in pixels.as_chunks::<4>().0.iter().enumerate() {
        let (sky, block) = ((i / 16) as i32, (i % 16) as i32);
        let color = argb(255, u32::from(p[0]), u32::from(p[1]), u32::from(p[2]));
        let (cx, cy) = (x + block * 4, y + (15 - sky) * 4);
        out.push(fill(cx, cy, cx + 4, cy + 4, color));
    }
    out
}

/// The four charts and what each needs to scale its bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chart {
    /// `FpsDebugChart`: frame durations in nanoseconds; the frame rate limit
    /// option (`maxFps`).
    Fps { framerate_limit: u32 },
    /// `TpsDebugChart`: tick samples in nanoseconds; the level's
    /// milliseconds per tick.
    Tps { mspt: f32 },
    /// `PingDebugChart`: round trips in milliseconds.
    Ping,
    /// `BandwidthDebugChart`: bytes received per tick.
    Bandwidth,
}

/// The labels' colour and the backdrop of the box and shaded labels.
const LABEL: u32 = 0xFFE0_E0E0;
const BACKDROP: u32 = 0x9050_5050;
const WHITE: u32 = 0xFFFF_FFFF;
const GREEN: u32 = 0xFF00_FF00;
const YELLOW: u32 = 0xFFFF_FF00;
const RED: u32 = 0xFFFF_0000;
const CYAN: u32 = 0xFF00_FFFF;
/// `BandwidthDebugChart.MID_COLOR`.
const LAVENDER: u32 = 0xFFA0_A0FF;
/// `TpsDebugChart`'s bar parts.
const TICK_METHOD_COLOR: u32 = 0xFF99_1111;
const TASK_COLOR: u32 = 0xFFBA_995F;
const OTHER_COLOR: u32 = 0xFF5F_0E8C;
const MEGABYTE: f64 = 1_048_576.0;

impl Chart {
    /// `AbstractDebugChart.getWidth`.
    pub fn width(max_width: i32) -> i32 {
        (CAPACITY as i32 + 2).min(max_width)
    }
    /// `AbstractDebugChart.extractRenderState`.
    pub fn draw(
        self,
        font: &Font,
        samples: &SampleLogger,
        left: i32,
        width: i32,
        bottom: i32,
    ) -> Vec<GuiDraw> {
        let mut out = vec![fill(
            left,
            bottom - CHART_HEIGHT,
            left + width,
            bottom,
            BACKDROP,
        )];
        let (mut sum, mut min, mut max) = (0i64, i64::from(i32::MAX), i64::from(i32::MIN));
        let start = CAPACITY.saturating_sub((width - 2).max(0) as usize);
        let count = samples.len().saturating_sub(start);
        for i in 0..count {
            let x = left + i as i32 + 1;
            let index = start + i;
            let value = self.aggregated(samples, index);
            min = min.min(value);
            max = max.max(value);
            sum += value;
            self.bars(&mut out, samples, bottom, x, index);
        }
        horizontal_line(
            &mut out,
            left,
            left + width - 1,
            bottom - CHART_HEIGHT,
            WHITE,
        );
        horizontal_line(&mut out, left, left + width - 1, bottom - 1, WHITE);
        vertical_line(&mut out, left, bottom - CHART_HEIGHT, bottom, WHITE);
        vertical_line(
            &mut out,
            left + width - 1,
            bottom - CHART_HEIGHT,
            bottom,
            WHITE,
        );
        if count > 0 {
            let y = bottom - CHART_HEIGHT - 9;
            let min_text = format!("{} min", self.display(min as f64));
            let avg_text = format!("{} avg", self.display(sum as f64 / count as f64));
            let max_text = format!("{} max", self.display(max as f64));
            text(&mut out, min_text, left + 2, y, LABEL, true);
            let avg_x = left + width / 2 - font.width(&avg_text) as i32 / 2;
            text(&mut out, avg_text, avg_x, y, LABEL, true);
            let max_x = left + width - font.width(&max_text) as i32 - 2;
            text(&mut out, max_text, max_x, y, LABEL, true);
        }
        self.lines_and_labels(&mut out, font, left, width, bottom);
        out
    }
    fn bars(
        self,
        out: &mut Vec<GuiDraw>,
        samples: &SampleLogger,
        bottom: i32,
        x: i32,
        index: usize,
    ) {
        let value = samples.get(index, 0);
        let height = self.height(value as f64);
        out.push(fill(x, bottom - height, x + 1, bottom, self.color(value)));
        if let Chart::Tps { .. } = self {
            let method = samples.get(index, TICK_SERVER_METHOD);
            let method_h = self.height(method as f64);
            out.push(fill(x, bottom - method_h, x + 1, bottom, TICK_METHOD_COLOR));
            let tasks = samples.get(index, SCHEDULED_TASKS);
            let tasks_h = self.height(tasks as f64);
            out.push(fill(
                x,
                bottom - method_h - tasks_h,
                x + 1,
                bottom - method_h,
                TASK_COLOR,
            ));
            let other = value - samples.get(index, IDLE) - method - tasks;
            let other_h = self.height(other as f64);
            out.push(fill(
                x,
                bottom - other_h - tasks_h - method_h,
                x + 1,
                bottom - tasks_h - method_h,
                OTHER_COLOR,
            ));
        }
    }
    /// `getValueForAggregation`: a tick's time without its idle part.
    fn aggregated(self, samples: &SampleLogger, index: usize) -> i64 {
        match self {
            Chart::Tps { .. } => samples.get(index, 0) - samples.get(index, IDLE),
            _ => samples.get(index, 0),
        }
    }
    /// `extractAdditionalLinesAndLabels`.
    fn lines_and_labels(
        self,
        out: &mut Vec<GuiDraw>,
        font: &Font,
        left: i32,
        width: i32,
        bottom: i32,
    ) {
        match self {
            Chart::Fps { framerate_limit } => {
                shaded(out, font, "30 fps", left + 1, bottom - CHART_HEIGHT + 1);
                shaded(out, font, "60 fps", left + 1, bottom - 30 + 1);
                horizontal_line(out, left, left + width - 1, bottom - 30, WHITE);
                if framerate_limit > 0 && framerate_limit <= 250 {
                    let y = bottom - self.height(1.0e9 / f64::from(framerate_limit)) - 1;
                    horizontal_line(out, left, left + width - 1, y, CYAN);
                }
            }
            Chart::Tps { mspt } => {
                let tps = 1000.0 / mspt;
                shaded(
                    out,
                    font,
                    &format!("{tps:.1} tps"),
                    left + 1,
                    bottom - CHART_HEIGHT + 1,
                );
            }
            Chart::Ping => shaded(out, font, "500 ms", left + 1, bottom - CHART_HEIGHT + 1),
            Chart::Bandwidth => {
                for bytes in [64.0, 1024.0, 16384.0] {
                    let y = bottom - bandwidth_height(bytes);
                    shaded(out, font, &bandwidth_display(bytes), left + 1, y + 1);
                    horizontal_line(out, left, left + width - 1, y, WHITE);
                }
                let y = bottom - bandwidth_height(MEGABYTE) + 1;
                shaded(out, font, &bandwidth_display(MEGABYTE), left + 1, y);
            }
        }
    }
    /// `toDisplayString`.
    fn display(self, sample: f64) -> String {
        match self {
            Chart::Fps { .. } | Chart::Tps { .. } => format!("{} ms", java_round(sample / 1.0e6)),
            Chart::Ping => format!("{} ms", java_round(sample)),
            Chart::Bandwidth => bandwidth_display(sample * 20.0),
        }
    }
    /// `getSampleHeight`.
    fn height(self, sample: f64) -> i32 {
        match self {
            Chart::Fps { .. } => java_round(sample / 1.0e6 * 60.0 / 33.333333333333336) as i32,
            Chart::Tps { mspt } => java_round(sample / 1.0e6 * 60.0 / f64::from(mspt)) as i32,
            Chart::Ping => java_round(sample * 60.0 / 500.0) as i32,
            Chart::Bandwidth => bandwidth_height(sample * 20.0),
        }
    }
    /// `getSampleColor`.
    fn color(self, sample: i64) -> u32 {
        let s = sample as f64;
        match self {
            Chart::Fps { .. } => gradient(s / 1.0e6, 0.0, GREEN, 28.0, YELLOW, 56.0, RED),
            Chart::Tps { mspt } => {
                let m = f64::from(mspt);
                gradient(s / 1.0e6, m, GREEN, m * 1.125, YELLOW, m * 1.25, RED)
            }
            Chart::Ping => gradient(s, 0.0, GREEN, 250.0, YELLOW, 500.0, RED),
            Chart::Bandwidth => gradient(s * 20.0, 0.0, CYAN, 8192.0, LAVENDER, 1.048576e7, RED),
        }
    }
}

/// `BandwidthDebugChart.toDisplayStringInternal`.
fn bandwidth_display(bytes_per_second: f64) -> String {
    if bytes_per_second >= MEGABYTE {
        format!("{:.1} MiB/s", bytes_per_second / MEGABYTE)
    } else if bytes_per_second >= 1024.0 {
        format!("{:.1} KiB/s", bytes_per_second / 1024.0)
    } else {
        format!("{} B/s", bytes_per_second.floor() as i64)
    }
}
/// `BandwidthDebugChart.getSampleHeightInternal`: logarithmic, 1 MiB/s at
/// the top.
fn bandwidth_height(bytes_per_second: f64) -> i32 {
    java_round((bytes_per_second + 1.0).ln() * 60.0 / MEGABYTE.ln()) as i32
}
/// `Math.round(double)`: halves round up.
fn java_round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}
/// `AbstractDebugChart.getSampleColor(sample, min, minColor, mid, ...)`.
fn gradient(
    sample: f64,
    min: f64,
    min_color: u32,
    mid: f64,
    mid_color: u32,
    max: f64,
    max_color: u32,
) -> u32 {
    let sample = sample.clamp(min, max);
    if sample < mid {
        srgb_lerp(((sample - min) / (mid - min)) as f32, min_color, mid_color)
    } else {
        srgb_lerp(((sample - mid) / (max - mid)) as f32, mid_color, max_color)
    }
}
/// `ARGB.srgbLerp`: each channel by `Mth.lerpInt`.
fn srgb_lerp(alpha: f32, p0: u32, p1: u32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = (((p0 >> shift) & 255) as i32, ((p1 >> shift) & 255) as i32);
        (a + (alpha * (b - a) as f32).floor() as i32) as u32
    };
    argb(channel(24), channel(16), channel(8), channel(0))
}
/// `GuiGraphicsExtractor.horizontalLine`.
fn horizontal_line(out: &mut Vec<GuiDraw>, x0: i32, x1: i32, y: i32, color: u32) {
    out.push(fill(x0.min(x1), y, x0.max(x1) + 1, y + 1, color));
}
/// `GuiGraphicsExtractor.verticalLine`.
fn vertical_line(out: &mut Vec<GuiDraw>, x: i32, y0: i32, y1: i32, color: u32) {
    out.push(fill(x, y0.min(y1) + 1, x + 1, y0.max(y1), color));
}
fn text(out: &mut Vec<GuiDraw>, s: String, x: i32, y: i32, color: u32, shadow: bool) {
    out.push(GuiDraw::Text {
        text: Text::literal(s),
        x,
        y,
        color,
        shadow,
    });
}
/// `extractStringWithShade`: a label on a dark backdrop, without shadow.
fn shaded(out: &mut Vec<GuiDraw>, font: &Font, s: &str, x: i32, y: i32) {
    out.push(fill(x, y, x + font.width(s) as i32 + 1, y + 9, BACKDROP));
    text(out, s.to_owned(), x + 1, y + 1, LABEL, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logger_keeps_the_last_240_samples() {
        let mut l = SampleLogger::default();
        for i in 0..300 {
            l.log(i);
        }
        assert_eq!(l.len(), CAPACITY);
        assert_eq!(l.get(0, 0), 60);
        assert_eq!(l.get(CAPACITY - 1, 0), 299);
    }

    #[test]
    fn chart_toggles_show_the_overlay_and_exclude_each_other() {
        let mut c = DebugCharts::default();
        let mut overlay = false;
        c.toggle_fps(&mut overlay);
        assert!(overlay && c.fps);
        c.toggle_network(&mut overlay);
        assert!(c.network && !c.fps);
        c.toggle_network(&mut overlay);
        assert!(!c.network && overlay);
        c.toggle_lightmap(&mut overlay);
        c.toggle_fps(&mut overlay);
        assert!(c.fps && !c.lightmap);
        // With the overlay hidden, the key shows it again with the chart on.
        c.toggle_fps(&mut overlay);
        overlay = false;
        c.toggle_fps(&mut overlay);
        assert!(overlay && c.fps);
    }

    #[test]
    fn corners_are_where_the_charts_are_drawn() {
        let mut c = DebugCharts::default();
        assert_eq!(c.corners(true), [false, false]);
        // Frame times left; tick times right once the server logged a tick.
        c.fps = true;
        assert_eq!(c.corners(false), [true, false]);
        c.tick_times.log(1);
        assert_eq!(c.corners(false), [true, true]);
        // Ping right; bandwidth left on a remote server only.
        c.fps = false;
        c.network = true;
        assert_eq!(c.corners(false), [false, true]);
        assert_eq!(c.corners(true), [true, true]);
        // The lightmap view, bottom right.
        c.network = false;
        c.lightmap = true;
        assert_eq!(c.corners(true), [false, true]);
    }

    #[test]
    fn the_lightmap_puts_full_sky_light_at_the_top_left() {
        let mut pixels = [0u8; 16 * 16 * 4];
        // Sky 15, block 0: the last row's first texel.
        pixels[15 * 64..15 * 64 + 4].copy_from_slice(&[10, 20, 30, 255]);
        let draws = lightmap(&pixels, 427, 240);
        assert_eq!(draws[0], fill(360, 173, 426, 239, 0xFF00_0000));
        assert_eq!(draws.len(), 257);
        assert!(draws.contains(&fill(361, 174, 365, 178, 0xFF0A_141E)));
    }

    #[test]
    fn packet_rates_follow_connection_smoothing_and_reset_between_worlds() {
        let mut charts = DebugCharts::default();
        charts.log_packets([80, 400]);
        assert_eq!(charts.packet_rates, [20.0, 100.0]);
        for _ in 1..20 {
            charts.log_packets([180, 800]);
        }
        assert_eq!(charts.packet_rates, [20.0, 100.0]);
        charts.log_packets([180, 800]);
        assert_eq!(charts.packet_rates, [40.0, 175.0]);
        for _ in 0..20 {
            charts.log_packets([180, 800]);
        }
        assert_eq!(charts.packet_rates, [30.0, 131.25]);
        charts.reset();
        assert_eq!(charts.packet_rates, [0.0, 0.0]);
        charts.log_packets([4, 8]);
        assert_eq!(charts.packet_rates, [1.0, 2.0]);
    }

    #[test]
    fn ticks_and_bandwidth_log_their_differences() {
        let mut c = DebugCharts::default();
        let t0 = Instant::now();
        c.log_tick(t0, Duration::from_millis(5));
        c.log_tick(t0 + Duration::from_millis(50), Duration::from_millis(7));
        assert_eq!(
            c.tick_times.samples[1],
            [50_000_000, 7_000_000, 0, 43_000_000]
        );
        c.log_bandwidth(1000);
        c.log_bandwidth(1600);
        assert_eq!(c.bandwidth.len(), 1);
        assert_eq!(c.bandwidth.get(0, 0), 600);
    }

    #[test]
    fn heights_match_vanilla_scales() {
        let fps = Chart::Fps {
            framerate_limit: 260,
        };
        // 33.3 ms (30 fps) fills the chart; 16.7 ms half of it.
        assert_eq!(fps.height(33_333_333.0), 60);
        assert_eq!(fps.height(16_666_667.0), 30);
        let tps = Chart::Tps { mspt: 50.0 };
        assert_eq!(tps.height(25e6), 30);
        assert_eq!(Chart::Ping.height(250.0), 30);
        // 1 MiB/s at the top; 52 bytes per tick is 1 KiB/s.
        assert_eq!(bandwidth_height(MEGABYTE), 60);
        assert_eq!(Chart::Bandwidth.height(51.2), 30);
    }

    #[test]
    fn labels_match_vanilla_formats() {
        assert_eq!(
            Chart::Fps { framerate_limit: 0 }.display(16_500_000.0),
            "17 ms"
        );
        assert_eq!(Chart::Ping.display(12.4), "12 ms");
        assert_eq!(Chart::Bandwidth.display(3.0), "60 B/s");
        assert_eq!(Chart::Bandwidth.display(100.0), "2.0 KiB/s");
        assert_eq!(bandwidth_display(MEGABYTE), "1.0 MiB/s");
    }

    #[test]
    fn colours_go_green_yellow_red() {
        let fps = Chart::Fps { framerate_limit: 0 };
        assert_eq!(fps.color(0), GREEN);
        assert_eq!(fps.color(28_000_000), YELLOW);
        assert_eq!(fps.color(100_000_000), RED);
        // Halfway between green and yellow: Mth.lerpInt floors.
        assert_eq!(fps.color(14_000_000), 0xFF7F_FF00);
        let tps = Chart::Tps { mspt: 50.0 };
        assert_eq!(tps.color(10_000_000), GREEN);
        assert_eq!(Chart::Bandwidth.color(0), CYAN);
    }
}
