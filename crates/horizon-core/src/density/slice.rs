use super::*;

/// Coordinates fixed by the compiled single-axis or combined XZ slice node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceAxes {
    X(i32),
    Y(i32),
    Z(i32),
    Xz { x: i32, z: i32 },
}
impl SliceAxes {
    fn coordinates(self) -> [Option<i32>; 3] {
        match self {
            Self::X(x) => [Some(x), None, None],
            Self::Y(y) => [None, Some(y), None],
            Self::Z(z) => [None, None, Some(z)],
            Self::Xz { x, z } => [Some(x), None, Some(z)],
        }
    }
}

/// A y-fixed slice's plane: the slice node, the plane's corner (y fixed),
/// its x/z sizes and steps (`SamplerContext::with_planes`).
pub(super) type PlaneKey = (usize, [i32; 3], [usize; 2], [i32; 2]);

#[derive(Clone, Debug)]
pub struct SliceSampler {
    pub input: Arc<dyn DensitySampler>,
    pub axes: SliceAxes,
}
impl DensitySampler for SliceSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        mut position: [i32; 3],
    ) -> Result<f32, SampleError> {
        for (axis, fixed) in self.axes.coordinates().into_iter().enumerate() {
            if let Some(coordinate) = fixed {
                position[axis] = coordinate;
            }
        }
        self.input.sample_value(context, position)
    }

    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        let fixed = self.axes.coordinates();
        let mut sizes = volume.sizes().map(|size| size as i32);
        let mut minimum = volume.minimum();
        for (axis, coordinate) in fixed.into_iter().enumerate() {
            if let Some(coordinate) = coordinate {
                sizes[axis] = 1;
                minimum[axis] = coordinate;
            }
        }
        // Keep the original steps, even on fixed axes: cached children observe
        // the complete volume descriptor. An unchanged slice delegates directly
        // and therefore retains the child's tail and partial-write behavior.
        let reduced = DensityVolume::new(sizes, minimum, volume.steps())?;
        if reduced == volume {
            return self.input.sample_volume(context, buffer, volume);
        }
        let plane = (fixed[1].is_some() && context.planes.is_some()).then(|| {
            let [rx, _, rz] = reduced.sizes();
            let [step_x, _, step_z] = volume.steps();
            (
                self as *const Self as usize,
                reduced.minimum(),
                [rx, rz],
                [step_x, step_z],
            )
        });
        if let Some(key) = &plane
            && let Some(samples) = context.planes.as_ref().and_then(|p| p.get(key))
        {
            spread(fixed, volume, reduced, samples, buffer);
            return Ok(());
        }
        context.with_buffer(reduced.size(), |context, samples| {
            self.input.sample_volume(context, samples, reduced)?;
            spread(fixed, volume, reduced, samples, buffer);
            if let (Some(key), Some(planes)) = (plane, &mut context.planes) {
                planes.insert(key, samples[..reduced.size()].to_vec());
            }
            Ok(())
        })
    }
}
/// Copies the `reduced` samples along the fixed axes of `volume`.
fn spread(
    fixed: [Option<i32>; 3],
    volume: DensityVolume,
    reduced: DensityVolume,
    samples: &[f32],
    buffer: &mut [f32],
) {
    let [sx, sy, sz] = volume.sizes();
    let [rx, ry, _] = reduced.sizes();
    for z in 0..sz {
        let source_z = if fixed[2].is_some() { 0 } else { z };
        for x in 0..sx {
            let source_x = if fixed[0].is_some() { 0 } else { x };
            let source = (source_x + source_z * rx) * ry;
            let target = (x + z * sx) * sy;
            let column = &mut buffer[target..target + sy];
            if fixed[1].is_some() {
                column.fill(samples[source]);
            } else {
                column.copy_from_slice(&samples[source..source + sy]);
            }
        }
    }
}
