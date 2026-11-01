//! Mouse look and scrolling of 26.3: `MouseHandler` (accumulated movement,
//! sensitivity curve, cinematic smoothing, inversion), `SmoothDouble`,
//! `ScrollWheelHandler` and `Entity.turn`.

/// `SmoothDouble`.
#[derive(Clone, Debug, Default)]
pub struct SmoothDouble {
    target_value: f64,
    remaining_value: f64,
    last_amount: f64,
}

impl SmoothDouble {
    pub fn get_new_delta_value(&mut self, target_delta: f64, time: f64) -> f64 {
        self.target_value += target_delta;
        let mut delta = self.target_value - self.remaining_value;
        let new_last_amount = self.last_amount + 0.5 * (delta - self.last_amount);
        let delta_sign = java_signum(delta);
        if delta_sign * delta > delta_sign * self.last_amount {
            delta = new_last_amount;
        }
        self.last_amount = new_last_amount;
        self.remaining_value += delta * time;
        delta * time
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// `Math.signum(double)`.
pub fn java_signum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() {
        v
    } else {
        v.signum()
    }
}

/// `ScrollWheelHandler`.
#[derive(Clone, Debug, Default)]
pub struct ScrollWheelHandler {
    accumulated_x: f64,
    accumulated_y: f64,
}

impl ScrollWheelHandler {
    /// `onMouseScroll`: whole wheel steps `(x, y)` once enough has accumulated.
    pub fn on_mouse_scroll(&mut self, x: f64, y: f64) -> (i32, i32) {
        if self.accumulated_x != 0.0 && java_signum(x) != java_signum(self.accumulated_x) {
            self.accumulated_x = 0.0;
        }
        if self.accumulated_y != 0.0 && java_signum(y) != java_signum(self.accumulated_y) {
            self.accumulated_y = 0.0;
        }
        self.accumulated_x += x;
        self.accumulated_y += y;
        // Java's double-to-int cast saturates and maps NaN to zero, like Rust's.
        let wheel_x = self.accumulated_x as i32;
        let wheel_y = self.accumulated_y as i32;
        if wheel_x == 0 && wheel_y == 0 {
            return (0, 0);
        }
        self.accumulated_x -= f64::from(wheel_x);
        self.accumulated_y -= f64::from(wheel_y);
        (wheel_x, wheel_y)
    }

    /// `getNextScrollWheelSelection`.
    pub fn next_selection(wheel: f64, current_selected: i32, limit: i32) -> i32 {
        let step = java_signum(wheel) as i32;
        let mut current = (current_selected - step).max(-1);
        while current < 0 {
            current += limit;
        }
        while current >= limit {
            current -= limit;
        }
        current
    }
}

/// `MouseHandler.onScroll` with the spectator menu closed. Only vertical whole
/// wheel steps adjust `Abilities.flyingSpeed`; horizontal scrolling does not.
pub fn spectator_flying_speed(speed: f32, vertical_steps: i32) -> f32 {
    (speed + vertical_steps as f32 * 0.005).clamp(0.0, 0.2)
}

/// Entity rotation as `Entity.turn` keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation {
    pub x_rot: f32,
    pub y_rot: f32,
    pub x_rot_o: f32,
    pub y_rot_o: f32,
}

impl Rotation {
    fn set_x_rot(&mut self, x_rot: f32) {
        if x_rot.is_finite() {
            self.x_rot = (x_rot % 360.0).clamp(-90.0, 90.0);
        }
    }
    fn set_y_rot(&mut self, y_rot: f32) {
        if y_rot.is_finite() {
            self.y_rot = y_rot;
        }
    }
    /// `Entity.turn(xo, yo)`: mouse deltas in, 0.15 degrees per unit, pitch clamped.
    pub fn turn(&mut self, xo: f64, yo: f64) {
        let x_delta = (yo as f32) * 0.15;
        let y_delta = (xo as f32) * 0.15;
        self.set_x_rot(self.x_rot + x_delta);
        self.set_y_rot(self.y_rot + y_delta);
        self.set_x_rot(mth_clamp(self.x_rot, -90.0, 90.0));
        self.x_rot_o += x_delta;
        self.y_rot_o += y_delta;
        self.x_rot_o = mth_clamp(self.x_rot_o, -90.0, 90.0);
    }
}

/// `Mth.clamp(float, float, float)`.
pub fn mth_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value.is_nan() || max.is_nan() {
        f32::NAN
    } else {
        value.min(max)
    }
}

/// The options `turnPlayer` reads.
#[derive(Clone, Copy, Debug)]
pub struct LookOptions {
    pub sensitivity: f64,
    pub smooth_camera: bool,
    /// First-person camera and the player is scoping (spyglass).
    pub scoping: bool,
    pub invert_x: bool,
    pub invert_y: bool,
}

/// The input state of `MouseHandler` that drives the camera and the hotbar.
#[derive(Clone, Debug)]
pub struct MouseLook {
    xpos: f64,
    ypos: f64,
    ignore_first_move: bool,
    smooth_turn_x: SmoothDouble,
    smooth_turn_y: SmoothDouble,
    accumulated_dx: f64,
    accumulated_dy: f64,
    pub scroll: ScrollWheelHandler,
    last_handle_movement_time: f64,
}

impl Default for MouseLook {
    fn default() -> Self {
        Self {
            xpos: 0.0,
            ypos: 0.0,
            ignore_first_move: true,
            smooth_turn_x: SmoothDouble::default(),
            smooth_turn_y: SmoothDouble::default(),
            accumulated_dx: 0.0,
            accumulated_dy: 0.0,
            scroll: ScrollWheelHandler::default(),
            // Double.MIN_VALUE
            last_handle_movement_time: 4.9e-324,
        }
    }
}

impl MouseLook {
    /// `setIgnoreFirstMove`.
    pub fn set_ignore_first_move(&mut self) {
        self.ignore_first_move = true;
    }

    /// `onMove`: relative motion counts while the pointer is grabbed, absolute
    /// motion otherwise.
    pub fn on_move(
        &mut self,
        xpos: f64,
        ypos: f64,
        xrel: f64,
        yrel: f64,
        window_active: bool,
        grabbed: bool,
    ) {
        if self.ignore_first_move {
            self.xpos = xpos;
            self.ypos = ypos;
            self.ignore_first_move = false;
        } else {
            if window_active {
                if grabbed {
                    self.accumulated_dx += xrel;
                    self.accumulated_dy += yrel;
                } else {
                    self.accumulated_dx += xpos - self.xpos;
                    self.accumulated_dy += ypos - self.ypos;
                }
            }
            self.xpos = xpos;
            self.ypos = ypos;
        }
    }

    /// `onMove` with the pointer grabbed, for relative motion an agent makes
    /// (`horizon-ctl mouse-move`): added to the frame's accumulated movement.
    pub fn add_motion(&mut self, dx: f64, dy: f64) {
        self.accumulated_dx += dx;
        self.accumulated_dy += dy;
    }

    /// Movement accumulated since the last frame (for screens' mouse-moved events).
    pub fn accumulated(&self) -> (f64, f64) {
        (self.accumulated_dx, self.accumulated_dy)
    }

    /// `handleAccumulatedMovement` for a frame at `time` seconds: the
    /// `(xo, yo)` passed to `Entity.turn` when the pointer is grabbed and the
    /// window is active. The accumulators are cleared either way.
    pub fn handle_accumulated_movement(
        &mut self,
        time: f64,
        window_active: bool,
        grabbed_with_player: bool,
        look: LookOptions,
    ) -> Option<(f64, f64)> {
        let mousea = time - self.last_handle_movement_time;
        self.last_handle_movement_time = time;
        let turn = (window_active && grabbed_with_player).then(|| self.turn_player(mousea, look));
        self.accumulated_dx = 0.0;
        self.accumulated_dy = 0.0;
        turn
    }

    fn turn_player(&mut self, mousea: f64, look: LookOptions) -> (f64, f64) {
        let (sensitivity_mod, sens) = sensitivity_scales(look.sensitivity);
        let (xo, yo) = if look.smooth_camera {
            (
                self.smooth_turn_x
                    .get_new_delta_value(self.accumulated_dx * sens, mousea * sens),
                self.smooth_turn_y
                    .get_new_delta_value(self.accumulated_dy * sens, mousea * sens),
            )
        } else if look.scoping {
            self.smooth_turn_x.reset();
            self.smooth_turn_y.reset();
            (
                self.accumulated_dx * sensitivity_mod,
                self.accumulated_dy * sensitivity_mod,
            )
        } else {
            self.smooth_turn_x.reset();
            self.smooth_turn_y.reset();
            (self.accumulated_dx * sens, self.accumulated_dy * sens)
        };
        (
            if look.invert_x { -xo } else { xo },
            if look.invert_y { -yo } else { yo },
        )
    }

    /// `onScroll` for the in-game case (no screen, player present): the wheel
    /// value `wheel` the hotbar or spectator speed reacts to, and the raw
    /// wheel steps. `None` when no whole step has accumulated.
    pub fn on_scroll(
        &mut self,
        xoffset: f64,
        yoffset: f64,
        discrete: bool,
        wheel_sensitivity: f64,
    ) -> Option<(i32, (i32, i32))> {
        let scale = |v: f64| (if discrete { java_signum(v) } else { v }) * wheel_sensitivity;
        let (x, y) = self.scroll.on_mouse_scroll(scale(xoffset), scale(yoffset));
        if x == 0 && y == 0 {
            return None;
        }
        let wheel = if y == 0 { -x } else { y };
        Some((wheel, (x, y)))
    }
}

/// `MouseHandler.turnPlayer`'s scales: `sensitivityMod = (s*0.6F+0.2F)^3`
/// and `sens = sensitivityMod * 8`.
fn sensitivity_scales(sensitivity: f64) -> (f64, f64) {
    let ss = sensitivity * f64::from(0.6f32) + f64::from(0.2f32);
    let sensitivity_mod = ss * ss * ss;
    (sensitivity_mod, sensitivity_mod * 8.0)
}

/// Degrees `Entity.turn` turns per unit of mouse motion: 0.15 x `sens`
/// (`sensitivityMod` while scoping). With the cinematic camera the same total
/// is reached, spread over the following frames.
pub fn degrees_per_count(look: LookOptions) -> f64 {
    let (sensitivity_mod, sens) = sensitivity_scales(look.sensitivity);
    let scale = if look.scoping && !look.smooth_camera {
        sensitivity_mod
    } else {
        sens
    };
    0.15 * scale
}

/// The mouse motion that turns the view by `dyaw` and `dpitch` degrees with
/// these options, the inverted axes included.
pub fn motion_for_turn(dyaw: f64, dpitch: f64, look: LookOptions) -> (f64, f64) {
    let per_count = degrees_per_count(look);
    let flip = |inverted: bool| if inverted { -1.0 } else { 1.0 };
    (
        dyaw / per_count * flip(look.invert_x),
        dpitch / per_count * flip(look.invert_y),
    )
}

/// Ease-in-out (a half cosine) over `t` in 0..=1: how much of a spread-out
/// turn is done at that point of its time.
pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    0.5 - 0.5 * (std::f64::consts::PI * t).cos()
}

/// `MouseHandler.grabMouse` / `releaseMouse`: the mouse position is the
/// window's centre whenever the pointer's grab state changes, so a screen
/// opened from gameplay starts with the cursor in the middle. A release of
/// an already free pointer (or a grab of an already grabbed one) leaves the
/// position where it is, as vanilla does.
pub fn mouse_position_after_grab_change(
    grab: bool,
    was_grabbed: bool,
    size: [u32; 2],
    scale: f32,
) -> Option<(f32, f32)> {
    if grab == was_grabbed {
        None
    } else {
        Some((size[0] as f32 / 2.0 / scale, size[1] as f32 / 2.0 / scale))
    }
}

/// `MacosUtil.setCtrlClickEmulatesRightClick`: with the option on (and only on
/// macOS), a left click made with Control held is a right click, and so is
/// its release, whatever Control does in between.
#[derive(Clone, Copy, Debug, Default)]
pub struct RightClickEmulation {
    emulating: bool,
}
impl RightClickEmulation {
    /// Whether the left button's `pressed` or released event is a right click.
    /// `enabled` is the option on macOS.
    pub fn is_right_click(&mut self, pressed: bool, control: bool, enabled: bool) -> bool {
        if pressed {
            self.emulating = enabled && control;
        }
        let right = self.emulating;
        if !pressed {
            self.emulating = false;
        }
        right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: LookOptions = LookOptions {
        sensitivity: 0.5,
        smooth_camera: false,
        scoping: false,
        invert_x: false,
        invert_y: false,
    };

    /// One frame of motion `(dx, dy)` through `onMove`'s grabbed path,
    /// `handleAccumulatedMovement` and `Entity.turn`.
    fn frame(look: &mut MouseLook, rot: &mut Rotation, time: f64, d: (f64, f64), o: LookOptions) {
        look.add_motion(d.0, d.1);
        if let Some((xo, yo)) = look.handle_accumulated_movement(time, true, true, o) {
            rot.turn(xo, yo);
        }
    }

    fn rotation() -> Rotation {
        Rotation {
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
        }
    }

    #[test]
    fn turn_player_matches_the_vanilla_formula() {
        // MouseHandler.turnPlayer: ss = sensitivity*0.6F + 0.2F, sens = ss^3 * 8,
        // then Entity.turn: yRot += (float)xo * 0.15F, xRot += (float)yo * 0.15F.
        for sensitivity in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for (dx, dy) in [(1.0, 0.0), (37.0, -12.5), (-400.0, 250.0)] {
                let o = LookOptions {
                    sensitivity,
                    ..PLAIN
                };
                let ss = sensitivity * f64::from(0.6f32) + f64::from(0.2f32);
                let sens = ss * ss * ss * 8.0;
                let mut look = MouseLook::default();
                let mut rot = rotation();
                frame(&mut look, &mut rot, 1.0, (dx, dy), o);
                let yaw = (dx * sens) as f32 * 0.15;
                let pitch = ((dy * sens) as f32 * 0.15).clamp(-90.0, 90.0);
                assert_eq!(
                    (rot.y_rot, rot.x_rot),
                    (yaw, pitch),
                    "s {sensitivity} d {dx},{dy}"
                );
            }
        }
    }

    #[test]
    fn inverted_and_scoping_turns() {
        let mut look = MouseLook::default();
        let mut rot = rotation();
        let o = LookOptions {
            invert_y: true,
            ..PLAIN
        };
        frame(&mut look, &mut rot, 1.0, (0.0, 10.0), o);
        assert!(rot.x_rot < 0.0, "inverted Y looks up: {}", rot.x_rot);
        // Scoping drops the factor 8.
        let scoped = LookOptions {
            scoping: true,
            ..PLAIN
        };
        assert!((degrees_per_count(PLAIN) / degrees_per_count(scoped) - 8.0).abs() < 1e-12);
    }

    #[test]
    fn motion_for_turn_round_trips_through_turn_player() {
        for o in [
            PLAIN,
            LookOptions {
                sensitivity: 0.9,
                invert_x: true,
                invert_y: true,
                ..PLAIN
            },
            LookOptions {
                scoping: true,
                ..PLAIN
            },
        ] {
            let (dx, dy) = motion_for_turn(73.5, -21.25, o);
            let mut look = MouseLook::default();
            let mut rot = rotation();
            frame(&mut look, &mut rot, 1.0, (dx, dy), o);
            assert!((rot.y_rot - 73.5).abs() < 1e-3, "{o:?}: {}", rot.y_rot);
            assert!((rot.x_rot + 21.25).abs() < 1e-3, "{o:?}: {}", rot.x_rot);
        }
    }

    #[test]
    fn cinematic_camera_spreads_the_same_motion_over_frames() {
        // SmoothDouble: the first frame turns part of the way, later frames
        // without new motion carry on towards the same total.
        let o = LookOptions {
            smooth_camera: true,
            ..PLAIN
        };
        let (dx, _) = motion_for_turn(30.0, 0.0, o);
        let mut look = MouseLook::default();
        let mut rot = rotation();
        frame(&mut look, &mut rot, 0.0, (0.0, 0.0), o);
        frame(&mut look, &mut rot, 1.0 / 60.0, (dx, 0.0), o);
        let first = rot.y_rot;
        assert!(first > 0.0 && first < 30.0, "{first}");
        for i in 2..600 {
            frame(&mut look, &mut rot, f64::from(i) / 60.0, (0.0, 0.0), o);
        }
        assert!((rot.y_rot - 30.0).abs() < 0.05, "{}", rot.y_rot);
    }

    #[test]
    fn ease_in_out_starts_and_ends_slowly() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-12);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-12);
        assert!(ease_in_out(0.1) < 0.1 && ease_in_out(0.9) > 0.9);
        assert_eq!(ease_in_out(2.0), 1.0);
    }
    #[test]
    fn default_sensitivity_turns_point_fifteen_degrees_per_count_at_one() {
        let mut look = MouseLook::default();
        look.on_move(0.0, 0.0, 0.0, 0.0, true, true);
        look.on_move(1.0, 0.0, 1.0, 0.0, true, true);
        let (xo, _) = look
            .handle_accumulated_movement(
                1.0,
                true,
                true,
                LookOptions {
                    sensitivity: 0.5,
                    smooth_camera: false,
                    scoping: false,
                    invert_x: false,
                    invert_y: false,
                },
            )
            .unwrap();
        let mut rot = Rotation {
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
        };
        rot.turn(xo, 0.0);
        assert!((rot.y_rot - 0.15 * 1.0).abs() < 1e-6, "{}", rot.y_rot);
    }
    #[test]
    fn scroll_accumulates_fractions() {
        let mut look = MouseLook::default();
        assert_eq!(look.on_scroll(0.0, 0.4, false, 1.0), None);
        assert_eq!(look.on_scroll(0.0, 0.7, false, 1.0), Some((1, (0, 1))));
    }
    #[test]
    fn the_mouse_position_is_the_centre_when_the_grab_changes() {
        // A grab of a free pointer and a release of a grabbed one both put
        // the position at the window's centre (`MouseHandler.grabMouse`,
        // `MouseHandler.releaseMouse`).
        assert_eq!(
            mouse_position_after_grab_change(true, false, [1920, 1080], 1.0),
            Some((960.0, 540.0))
        );
        assert_eq!(
            mouse_position_after_grab_change(false, true, [1920, 1080], 2.0),
            Some((480.0, 270.0))
        );
        // Without a change of the grab state the position stays where it is.
        assert_eq!(
            mouse_position_after_grab_change(false, false, [1920, 1080], 1.0),
            None
        );
        assert_eq!(
            mouse_position_after_grab_change(true, true, [1920, 1080], 1.0),
            None
        );
    }

    #[test]
    fn control_click_is_a_right_click_until_released() {
        let mut e = RightClickEmulation::default();
        // The option off, or no Control: a left click stays one.
        assert!(!e.is_right_click(true, true, false));
        assert!(!e.is_right_click(false, true, false));
        assert!(!e.is_right_click(true, false, true));
        assert!(!e.is_right_click(false, true, true));
        // Control held at the press: the press and the release are right clicks,
        // even when Control was let go before the release.
        assert!(e.is_right_click(true, true, true));
        assert!(e.is_right_click(false, false, true));
        // The next left click is its own.
        assert!(!e.is_right_click(true, false, true));
        assert!(!e.is_right_click(false, false, true));
    }
}
