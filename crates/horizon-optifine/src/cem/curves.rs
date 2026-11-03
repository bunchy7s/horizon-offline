//! The curves behind CEM's `ease*`, spline and `keyframe` functions: easings.net's easings
//! and the splines EMF documents, on plain `f32`s.

const PI: f32 = std::f32::consts::PI;

/// The family of an `ease*` function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Quad,
    Cubic,
    Quart,
    Quint,
    Sine,
    Expo,
    Circ,
    Back,
    Elastic,
    Bounce,
}

/// Which end of the curve eases.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    In,
    Out,
    InOut,
}

impl Curve {
    pub fn named(name: &str) -> Option<Curve> {
        use Curve::*;
        Some(match name {
            "quad" => Quad,
            "cubic" => Cubic,
            "quart" => Quart,
            "quint" => Quint,
            "sine" => Sine,
            "expo" => Expo,
            "circ" => Circ,
            "back" => Back,
            "elastic" => Elastic,
            "bounce" => Bounce,
            _ => return None,
        })
    }

    /// The ease-in curve at `t`.
    fn ease_in(self, t: f32) -> f32 {
        use Curve::*;
        let c1 = 1.70158;
        match self {
            Quad => t * t,
            Cubic => t.powi(3),
            Quart => t.powi(4),
            Quint => t.powi(5),
            Sine => 1.0 - (t * PI / 2.0).cos(),
            Expo if t == 0.0 => 0.0,
            Expo => 2f32.powf(10.0 * t - 10.0),
            Circ => 1.0 - (1.0 - t * t).sqrt(),
            Back => (c1 + 1.0) * t.powi(3) - c1 * t * t,
            Elastic if t == 0.0 || t == 1.0 => t,
            Elastic => {
                -(2f32.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * (2.0 * PI / 3.0)).sin()
            }
            Bounce => 1.0 - bounce_out(1.0 - t),
        }
    }

    /// The curve at `t`, 0 at 0 and 1 at 1. Ease-out mirrors ease-in; ease-in-out runs ease-in
    /// at double speed, then its mirror, except back and elastic, which have their own.
    pub fn at(self, ease: Ease, t: f32) -> f32 {
        let (c2, c5) = (1.70158 * 1.525, 2.0 * PI / 4.5);
        match (ease, self) {
            (Ease::In, _) => self.ease_in(t),
            (Ease::Out, _) => 1.0 - self.ease_in(1.0 - t),
            (Ease::InOut, Curve::Back) if t < 0.5 => {
                (2.0 * t).powi(2) * ((c2 + 1.0) * 2.0 * t - c2) / 2.0
            }
            (Ease::InOut, Curve::Back) => {
                ((2.0 * t - 2.0).powi(2) * ((c2 + 1.0) * (t * 2.0 - 2.0) + c2) + 2.0) / 2.0
            }
            (Ease::InOut, Curve::Elastic) if t == 0.0 || t == 1.0 => t,
            (Ease::InOut, Curve::Elastic) if t < 0.5 => {
                -(2f32.powf(20.0 * t - 10.0) * ((20.0 * t - 11.125) * c5).sin()) / 2.0
            }
            (Ease::InOut, Curve::Elastic) => {
                2f32.powf(-20.0 * t + 10.0) * ((20.0 * t - 11.125) * c5).sin() / 2.0 + 1.0
            }
            (Ease::InOut, _) if t < 0.5 => self.ease_in(2.0 * t) / 2.0,
            (Ease::InOut, _) => 1.0 - self.ease_in(2.0 - 2.0 * t) / 2.0,
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984375
    }
}

pub fn lerp(k: f32, x: f32, y: f32) -> f32 {
    x + k * (y - x)
}

/// `keyframe(k, frames...)`: frame `i` at `k = i`, the last one past the end; `keyframeloop`
/// goes from the last frame back to the first. A negative `k` counts as positive.
pub fn keyframe(k: f32, frames: &[f32], looped: bool) -> f32 {
    let n = frames.len();
    let mut k = k.abs();
    if looped {
        k %= n as f32;
    } else if k >= (n - 1) as f32 {
        return frames[n - 1];
    }
    // `k` is below `n` (below `n - 1` without the loop) or NaN, which converts to 0.
    let i = (k.floor() as usize).min(n - 1);
    lerp(k - i as f32, frames[i], frames[(i + 1) % n])
}

/// `Mth.catmullrom(k, p0, p1, p2, p3)`: from p1 to p2.
pub fn catmullrom(k: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
    0.5 * (2.0 * p1
        + (p2 - p0) * k
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * k * k
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * k * k * k)
}

/// From x to y with tangents m0 and m1.
pub fn hermite(k: f32, x: f32, y: f32, m0: f32, m1: f32) -> f32 {
    let (k2, k3) = (k * k, k * k * k);
    (2.0 * k3 - 3.0 * k2 + 1.0) * x
        + (k3 - 2.0 * k2 + k) * m0
        + (-2.0 * k3 + 3.0 * k2) * y
        + (k3 - k2) * m1
}

/// From x to y through control points z and w.
pub fn cubic_bezier(k: f32, x: f32, y: f32, z: f32, w: f32) -> f32 {
    let j = 1.0 - k;
    j * j * j * x + 3.0 * j * j * k * z + 3.0 * j * k * k * w + k * k * k * y
}

/// From x to y through control point z.
pub fn quad_bezier(k: f32, x: f32, y: f32, z: f32) -> f32 {
    let j = 1.0 - k;
    j * j * x + 2.0 * j * k * z + k * k * y
}
