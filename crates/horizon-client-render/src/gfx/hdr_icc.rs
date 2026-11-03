//! The ICC profile of HDR screenshots, for programs that don't read PNG's `cICP`: BT.2020
//! primaries and the PQ curve, with HDR reference white (203 nits) as the profile's white.
//! An ICC profile has no values above white, so those programs show a correct SDR picture
//! with its highlights clipped, where without it they took PQ's values for sRGB's and showed
//! them washed out. Programs that read `cICP` ignore this (PNG's third edition).

/// HDR reference white in nits (ITU-R BT.2408): where the curve reaches the profile's white.
const WHITE: f64 = 203.0;
/// PQ's values from 0 to 1, in the curve's steps.
const STEPS: usize = 4096;
/// BT.2020's primaries, adapted to the profile connection space's D50 (Bradford).
const RED: [f64; 3] = [0.67348, 0.27904, -0.00193];
const GREEN: [f64; 3] = [0.16567, 0.67534, 0.02998];
const BLUE: [f64; 3] = [0.12505, 0.04561, 0.79685];
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// PQ's EOTF (SMPTE ST 2084): a signal from 0 to 1 in nits.
fn pq_nits(signal: f64) -> f64 {
    let (m1, m2) = (2610.0 / 16384.0, 2523.0 / 32.0);
    let (c1, c2, c3) = (3424.0 / 4096.0, 2413.0 / 128.0, 2392.0 / 128.0);
    let p = signal.powf(1.0 / m2);
    10_000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
}

fn s15_fixed16(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz(v: [f64; 3]) -> Vec<u8> {
    let mut tag = b"XYZ \0\0\0\0".to_vec();
    tag.extend(v.iter().flat_map(|&c| s15_fixed16(c)));
    tag
}

fn curve() -> Vec<u8> {
    let mut tag = b"curv\0\0\0\0".to_vec();
    tag.extend((STEPS as u32).to_be_bytes());
    for i in 0..STEPS {
        let y = (pq_nits(i as f64 / (STEPS - 1) as f64) / WHITE).min(1.0);
        tag.extend(((y * 65535.0).round() as u16).to_be_bytes());
    }
    tag
}

/// `textDescriptionType` (ICC v2), with its empty Unicode and ScriptCode parts.
fn description(text: &str) -> Vec<u8> {
    let mut tag = b"desc\0\0\0\0".to_vec();
    tag.extend((text.len() as u32 + 1).to_be_bytes());
    tag.extend(text.as_bytes());
    tag.extend([0; 1 + 4 + 4 + 2 + 1 + 67]);
    tag
}

/// An ICC v2 matrix-shaper display profile.
pub(super) fn profile() -> Vec<u8> {
    let trc = curve();
    let copyright = b"text\0\0\0\0Public domain\0".to_vec();
    let tags: [(&[u8; 4], &[u8]); 9] = [
        (b"desc", &description("BT.2020 PQ, 203 nits = white")),
        (b"cprt", &copyright),
        (b"wtpt", &xyz(D50)),
        (b"rXYZ", &xyz(RED)),
        (b"gXYZ", &xyz(GREEN)),
        (b"bXYZ", &xyz(BLUE)),
        (b"rTRC", &trc),
        (b"gTRC", &trc),
        (b"bTRC", &trc),
    ];
    let data_start = 128 + 4 + 12 * tags.len();
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    let mut data = Vec::new();
    let mut trc_at = None;
    for (name, tag) in tags {
        // The three channels share one curve.
        let is_trc = tag == &trc[..];
        let at = match trc_at.filter(|_| is_trc) {
            Some(at) => at,
            None => {
                let at = data_start + data.len();
                data.extend(tag);
                data.resize(data.len().next_multiple_of(4), 0);
                at
            }
        };
        if is_trc {
            trc_at = Some(at);
        }
        table.extend(name);
        table.extend((at as u32).to_be_bytes());
        table.extend((tag.len() as u32).to_be_bytes());
    }
    let mut out = vec![0; 128];
    let size = data_start + data.len();
    out[0..4].copy_from_slice(&(size as u32).to_be_bytes());
    out[8..12].copy_from_slice(&[2, 0x40, 0, 0]);
    out[12..16].copy_from_slice(b"mntr");
    out[16..20].copy_from_slice(b"RGB ");
    out[20..24].copy_from_slice(b"XYZ ");
    out[36..40].copy_from_slice(b"acsp");
    // Relative colorimetric, and the connection space's illuminant.
    out[64..68].copy_from_slice(&1u32.to_be_bytes());
    for (i, &c) in D50.iter().enumerate() {
        out[68 + 4 * i..72 + 4 * i].copy_from_slice(&s15_fixed16(c));
    }
    out.extend(table);
    out.extend(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag<'a>(profile: &'a [u8], name: &[u8; 4]) -> &'a [u8] {
        let u32_at = |i: usize| u32::from_be_bytes(profile[i..i + 4].try_into().unwrap()) as usize;
        let at = (0..u32_at(128))
            .map(|i| 132 + 12 * i)
            .find(|&e| &profile[e..e + 4] == name)
            .unwrap();
        &profile[u32_at(at + 4)..u32_at(at + 4) + u32_at(at + 8)]
    }

    #[test]
    fn the_profile_is_whole() {
        let p = profile();
        assert_eq!(
            u32::from_be_bytes(p[0..4].try_into().unwrap()) as usize,
            p.len()
        );
        assert_eq!(&p[36..40], b"acsp");
        assert_eq!(p.len() % 4, 0);
        assert_eq!(&tag(&p, b"desc")[0..4], b"desc");
        // One curve, shared: the profile stays small.
        assert_eq!(tag(&p, b"rTRC").as_ptr(), tag(&p, b"bTRC").as_ptr());
        assert!(p.len() < 10_000, "{}", p.len());
    }

    #[test]
    fn the_primaries_add_up_to_white() {
        for i in 0..3 {
            assert!((RED[i] + GREEN[i] + BLUE[i] - D50[i]).abs() < 0.0001);
        }
    }

    #[test]
    fn the_curve_reaches_white_at_reference_white_and_stays() {
        let p = profile();
        let trc = tag(&p, b"rTRC");
        let value = |signal: f64| {
            let i = 12 + 2 * (signal * (STEPS - 1) as f64).round() as usize;
            u16::from_be_bytes([trc[i], trc[i + 1]])
        };
        assert_eq!(value(0.0), 0);
        // 203 nits in PQ is a signal of 0.5807.
        assert!(value(0.57) < 65535);
        assert_eq!(value(0.59), 65535);
        assert_eq!(value(1.0), 65535);
    }
}
