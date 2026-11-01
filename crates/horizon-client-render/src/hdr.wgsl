// Horizon's HDR output pass: the frame, drawn as without HDR in vanilla's gamma
// space, decoded to linear light at the paper white and written to the swapchain:
// HDR10 (BT.2020, PQ) or scRGB (1.0 = 80 nits). Bright pixels of light sources that still show the
// world as it was before the hand and HUD are expanded towards the peak.
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var world: texture_2d<f32>;

struct Params {
    // Paper white in scRGB units (nits / 80).
    paper: f32,
    // Peak brightness relative to paper white; 1 expands nothing.
    peak: f32,
    // 1 when this frame drew a world.
    world: f32,
    // 1 greys what isn't expanded and tints what is (HORIZON_HDR_DEBUG=1).
    debug: f32,
    // 1 for an HDR10 swapchain, 0 for scRGB.
    pq: f32,
    // The display's black level in nits; 0 leaves the picture as it is.
    black: f32,
    // The HUD and menus' brightness as a share of paper white.
    ui: f32,
    // Per tier, low to very bright (hdr.rs `Tier`): the share of the headroom
    // the brightest pixels get, and the brightness where the lift starts and
    // where it is full.
    share: vec4<f32>,
    start: vec4<f32>,
    full: vec4<f32>,
    // HDR calibration (hdr_calibration.rs): x its step (0 none, 1 black level,
    // 2 peak, 3 paper white), y the slider in nits, z where its controls start,
    // a share of the height.
    calibration: vec4<f32>,
};
@group(0) @binding(2) var<uniform> params: Params;
// Horizon's logo, 1 inside, 0 outside, for calibration's test pictures.
@group(0) @binding(3) var logo: texture_2d<f32>;

// Light sources are lifted evenly rather than by a highlight curve: vanilla's
// textures were drawn for SDR, so a curve on their brightest pixels blows out
// near-white spots (glowstone) and leaves the rest (lava, a jack o'lantern's
// face) nearly as it was. Each tier's pixels brighter than its `full` get its
// `share` of the headroom, dimmer ones less down to none at its `start`
// (lava's crust, a lantern's metal), and no channel goes past the peak.
// Brightness is the mean of luminance and the brightest channel, so saturated
// light counts as bright.

// Whether `c` still shows the world pixel `w`: the same, or darkened evenly
// across its channels, as the vignette and other screen-wide tints darken it.
// The hand, HUD and text replace pixels instead, so they never pass. The
// channels are held to one factor, the ratio of their sums, so a channel at 0
// stays 0; a step of 8-bit rounding is allowed on each.
fn shows_world(c: vec3<f32>, w: vec3<f32>) -> bool {
    if all(c == w) {
        return true;
    }
    let f = (c.r + c.g + c.b) / max(w.r + w.g + w.b, 1.0 / 255.0);
    return f <= 1.01 && f > 0.5 && all(abs(c - f * w) <= 0.03 * w + 1.0 / 255.0);
}

// Linear BT.709 to BT.2020 (ITU-R BT.2087), columns first.
const BT709_TO_BT2020 = mat3x3<f32>(
    vec3(0.6274, 0.0691, 0.0164),
    vec3(0.3293, 0.9195, 0.0880),
    vec3(0.0433, 0.0114, 0.8956),
);

// SMPTE ST 2084: nits (up to 10000) to the PQ signal.
fn pq(nits: vec3<f32>) -> vec3<f32> {
    let y = pow(clamp(nits / 10000.0, vec3(0.0), vec3(1.0)), vec3(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3(78.84375));
}

// ST 2084's inverse: the PQ signal to nits.
fn pq_nits(e: vec3<f32>) -> vec3<f32> {
    let p = pow(clamp(e, vec3(0.0), vec3(1.0)), vec3(1.0 / 78.84375));
    let y = max(p - 0.8359375, vec3(0.0)) / (18.8515625 - 18.6875 * p);
    return 10000.0 * pow(y, vec3(1.0 / 0.1593017578125));
}

// ITU-R BT.2390's black level lift (its EETF without the knee) on the PQ signal
// `e`: black moves up to the display's black level, the peak stays, and the lift
// fades out by the fourth power on the way up, so only the shadows move. A display
// that can't go darker than its black level would crush everything below it.
fn lift_black(e: vec3<f32>) -> vec3<f32> {
    let top = pq(vec3(params.peak * params.paper * 80.0)).x;
    let n = min(e / top, vec3(1.0));
    return e + pq(vec3(params.black)).x * pow(vec3(1.0) - n, vec3(4.0));
}

// Whether `p` falls on the logo drawn `size` pixels wide around `centre`.
fn on_logo(p: vec2<f32>, centre: vec2<f32>, size: f32) -> bool {
    let uv = (p - centre) / size + 0.5;
    if any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) {
        return false;
    }
    let texels = vec2<f32>(textureDimensions(logo));
    return textureLoad(logo, vec2<i32>(uv * texels), 0).r > 0.5;
}

// A calibration step's test picture at `p`, in nits, the way display calibration
// tools draw them: nothing in it is bright over a large area, so the display
// doesn't dim itself while it is judged.
fn calibration_nits(p: vec2<f32>, size: vec2<f32>) -> f32 {
    let top = size.y * params.calibration.z;
    let centre = vec2(size.x, top) / 2.0;
    let v = params.calibration.y;
    switch u32(params.calibration.x) {
        // Black level: the logo at the slider on 0. Lowered until it disappears,
        // it sits on the darkest the display shows.
        case 1u: {
            return select(0.0, v, on_logo(p, centre, top * 0.4));
        }
        // Peak: a tenth of the screen at 10 000 nits, which the display clips at
        // its brightest, the logo inside at the slider. Raised until it
        // disappears, it sits on that brightest.
        case 2u: {
            let side = sqrt(0.1 * size.x * size.y);
            if all(abs(p - centre) < vec2(side / 2.0)) {
                return select(10000.0, v, on_logo(p, centre, side * 0.7));
            }
            return 0.0;
        }
        // Paper white: a white card at the slider, on a dim grey, to match a
        // sheet of paper held beside the screen.
        default: {
            let card = vec2(top * 0.9, top * 0.6);
            return select(3.0, v, all(abs(p - centre) < card / 2.0));
        }
    }
}

@vertex fn vs_hdr(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

// Hdr::keep_world: once the world is kept, the frame's alpha starts at 0 again.
@fragment fn fs_clear_alpha() -> @location(0) vec4<f32> {
    return vec4(0.0);
}

@fragment fn fs_hdr(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    // Calibration: above its controls the test picture, straight in nits, past
    // paper white, the lift and the black level.
    let size = vec2<f32>(textureDimensions(frame));
    if params.calibration.x > 0.5 && pos.y < size.y * params.calibration.z {
        let nits = vec3(calibration_nits(pos.xy, size));
        if params.pq > 0.5 {
            return vec4(pq(nits), 1.0);
        }
        return vec4(nits / 80.0, 1.0);
    }
    let f = textureLoad(frame, at, 0);
    let c = f.rgb;
    // What covers the world here (Hdr::keep_world): nothing at 0, the hand at
    // 1/255 (Hdr::mark_hand: drawn at the world's brightness), else the HUD or a
    // menu, by how much.
    let covered = f.a > 0.0;
    let ui = select(f.a, 0.0, f.a < 1.5 / 255.0);
    // Gamma 2.2, as SDR displays and the compositors' SDR paths decode it.
    var light = pow(max(c, vec3(0.0)), vec3(2.2));
    let w = textureLoad(world, at, 0);
    // The world's alpha is below 0.75 on light sources, the sun and the moon
    // (terrain.wgsl, the celestial pipeline): only those shine, by the tier it
    // holds, the lower the brighter, and only where nothing was drawn over them
    // since: grey text over the white sun still looks like the sun darkened, but
    // it raised the frame's alpha.
    var lifted = false;
    if params.world > 0.5 && params.peak > 1.0 && w.a < 0.75 && !covered
        && shows_world(c, w.rgb) {
        // Brightness of the world pixel, before the vignette darkens the frame
        // toward its edges: a light source shines the same anywhere on screen.
        let wl = pow(max(w.rgb, vec3(0.0)), vec3(2.2));
        let y = 0.5 * (dot(wl, vec3(0.2126, 0.7152, 0.0722)) + max(wl.r, max(wl.g, wl.b)));
        let tier = u32(clamp(round(4.0 - 4.0 * w.a / 0.7), 1.0, 4.0)) - 1u;
        let lift = smoothstep(params.start[tier], params.full[tier], y);
        if lift > 0.0 {
            let top = max(light.r, max(light.g, light.b));
            let gain = min(1.0 + (params.peak - 1.0) * params.share[tier] * lift, params.peak / max(top, 1e-4));
            light *= max(gain, 1.0);
            lifted = true;
        }
    }
    // HORIZON_HDR_DEBUG=1: what isn't lifted turns grey, what is keeps its
    // colour with a magenta tint, so a lift shows on any colour.
    if params.debug > 0.5 && params.world > 0.5 {
        if lifted {
            light *= vec3(1.0, 0.6, 1.0);
        } else {
            light = vec3(dot(light, vec3(0.2126, 0.7152, 0.0722)));
        }
    }
    // The HUD and menus at their own brightness, by how much they cover.
    light *= mix(1.0, params.ui, ui);
    let scrgb = light * params.paper;
    if params.pq > 0.5 {
        let e = pq(BT709_TO_BT2020 * scrgb * 80.0);
        return vec4(select(e, lift_black(e), params.black > 0.0), 1.0);
    }
    if params.black > 0.0 {
        return vec4(pq_nits(lift_black(pq(scrgb * 80.0))) / 80.0, 1.0);
    }
    return vec4(scrgb, 1.0);
}
