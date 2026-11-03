//! `BlockBehaviour.getInteractionShape` of the four blocks that override it in
//! 26.3, for the player's block pick (`BlockGetter.clipWithInteractionOverride`):
//! a cauldron, a composter, a hopper and scaffolding. When the ray enters the
//! interaction shape before it enters the outline, the hit keeps the outline's
//! point but takes the interaction shape's face, so a bowl is targeted from
//! above where its inside wall is the outline's first hit.

/// The interaction shape's boxes of the block named `name` with `facing`, as
/// `[min x, min y, min z, max x, max y, max z]` within the block; none for any
/// other block.
pub(super) fn boxes(name: &str, facing: Option<&str>) -> Vec<[f64; 6]> {
    const P: f64 = 1.0 / 16.0;
    let scale = |b: [f64; 6]| b.map(|v| v * P);
    match name {
        // `AbstractCauldronBlock.SHAPE_INSIDE`: `Block.column(12, 4, 16)`.
        "minecraft:cauldron"
        | "minecraft:water_cauldron"
        | "minecraft:lava_cauldron"
        | "minecraft:powder_snow_cauldron" => vec![scale([2.0, 4.0, 2.0, 14.0, 16.0, 14.0])],
        // `Shapes.block()`.
        "minecraft:composter" | "minecraft:scaffolding" => vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
        "minecraft:hopper" => {
            // `inside` is `Block.column(12, 11, 16)`; a hopper pointing sideways adds its
            // spout, `Block.boxZ(4, 8, 10, 0, 4)` turned to face.
            let mut boxes = vec![scale([2.0, 11.0, 2.0, 14.0, 16.0, 14.0])];
            match facing {
                Some("north") => boxes.push(scale([6.0, 6.0, 0.0, 10.0, 10.0, 4.0])),
                Some("south") => boxes.push(scale([6.0, 6.0, 12.0, 10.0, 10.0, 16.0])),
                Some("west") => boxes.push(scale([0.0, 6.0, 6.0, 4.0, 10.0, 10.0])),
                Some("east") => boxes.push(scale([12.0, 6.0, 6.0, 16.0, 10.0, 10.0])),
                _ => {}
            }
            boxes
        }
        _ => Vec::new(),
    }
}

/// The nearest entry of the ray `origin + dir * t` (`t` up to `reach`) into one
/// of `boxes` (world coordinates): its `t` and the normal of the face it
/// enters through, `inside` (the face the ray entered the block through) when it
/// starts in the box.
pub(super) fn entry(
    origin: [f64; 3],
    dir: [f64; 3],
    boxes: &[[f64; 6]],
    reach: f64,
    inside: [i32; 3],
) -> Option<(f64, [i32; 3])> {
    let mut best: Option<(f64, [i32; 3])> = None;
    for b in boxes {
        let (mut t0, mut t1, mut axis_in, mut sign) = (0.0f64, f64::INFINITY, None, 0);
        let mut miss = false;
        for a in 0..3 {
            if dir[a] == 0.0 {
                miss |= origin[a] < b[a] || origin[a] > b[a + 3];
                continue;
            }
            let (lo, hi) = ((b[a] - origin[a]) / dir[a], (b[a + 3] - origin[a]) / dir[a]);
            let (near, far) = if lo < hi { (lo, hi) } else { (hi, lo) };
            if near > t0 {
                t0 = near;
                axis_in = Some(a);
                sign = if dir[a] > 0.0 { -1 } else { 1 };
            }
            t1 = t1.min(far);
        }
        if miss || t0 > t1 || t0 > reach {
            continue;
        }
        let mut normal = inside;
        if let Some(a) = axis_in {
            normal = [0; 3];
            normal[a] = sign;
        }
        if best.is_none_or(|(bt, _)| t0 < bt) {
            best = Some((t0, normal));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report: standing at eyes [0.5, 101.27, 0.5], a ray pitched 7 degrees down crosses the
    /// cauldron at [0, 100, 2] at the height of its rim, inside the inside shape's top plane:
    /// the inside shape is entered through its top face.
    #[test]
    fn a_ray_over_a_cauldron_enters_its_inside_through_the_top() {
        let boxes: Vec<[f64; 6]> = boxes("minecraft:cauldron", None)
            .iter()
            .map(|b| {
                [
                    b[0],
                    b[1] + 100.0,
                    b[2] + 2.0,
                    b[3],
                    b[4] + 100.0,
                    b[5] + 2.0,
                ]
            })
            .collect();
        let pitch = 7.0f64.to_radians();
        let dir = [0.0, -pitch.sin(), pitch.cos()];
        let (_, normal) = entry([0.5, 101.27, 0.5], dir, &boxes, 5.0, [0, 0, -1]).unwrap();
        assert_eq!(normal, [0, 1, 0]);
    }

    #[test]
    fn only_the_four_blocks_have_one() {
        assert!(boxes("minecraft:stone", None).is_empty());
        assert_eq!(
            boxes("minecraft:composter", None),
            [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
        );
        assert_eq!(boxes("minecraft:hopper", Some("down")).len(), 1);
        // A hopper pointing east: the spout on its east side.
        assert_eq!(boxes("minecraft:hopper", Some("east")).len(), 2);
    }
}
