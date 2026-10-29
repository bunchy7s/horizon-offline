use super::*;
use crate::{noise::PerlinNoise, random::LegacyRandom};

#[test]
fn tail_writes_follow_the_selected_bulk_sampler() {
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([2, 3, 2], [-3, -4, -5], [1, 2, 1]).unwrap();
    let mut buffer = vec![17.0; volume.size() + 2];
    ConstantSampler(-0.0)
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert!(buffer.iter().all(|v| v.to_bits() == (-0.0_f32).to_bits()));
    GradientSampler::new(Axis::Y, TilingMode::ClampToEdge, -4, 0, -1.0, 1.0)
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert_eq!(&buffer[..3], &[-1.0, 0.0, 1.0]);
    assert_eq!(buffer[volume.size()].to_bits(), (-0.0_f32).to_bits());
    let noise = NoiseSampler {
        noise: Arc::new(PerlinNoise::new(&mut LegacyRandom::new(0))),
        xz_scale: 0.1,
        y_scale: 0.2,
    };
    noise
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert!(buffer[volume.size()..].iter().all(|v| v.to_bits() == 0));
}

#[test]
fn invalid_buffer_is_rejected_before_child_evaluation() {
    let volume = DensityVolume::new([1, 2, 1], [0; 3], [1; 3]).unwrap();
    let mut context = SamplerContext::default();
    let mut buffer = [19.0];
    let gradient = GradientSampler::new(Axis::X, TilingMode::Repeat, 0, 0, 1.0, 2.0);
    assert_eq!(
        gradient.sample_volume(&mut context, &mut buffer, volume),
        Err(SampleError::Volume(DensityError::BufferTooShort))
    );
    assert_eq!(buffer, [19.0]);
}

#[test]
fn scratch_returns_to_context_after_nested_sampling_errors() {
    let mut context = SamplerContext::default();
    let error = context.with_buffer(16, |context, _| {
        context.with_buffer(32, |_, _| Err::<(), _>(SampleError::ZeroGradientPeriod))
    });
    assert_eq!(error, Err(SampleError::ZeroGradientPeriod));
    assert_eq!(context.scratch.len(), 2);
    context
        .with_buffer(8, |context, outer| {
            outer.fill(17.0);
            context.with_buffer(8, |_, inner| {
                inner.fill(29.0);
                Ok(())
            })?;
            assert!(outer.iter().all(|v| *v == 17.0));
            Ok(())
        })
        .unwrap();
    assert_eq!(context.scratch.len(), 2);
    context.clear_scratch();
    assert!(context.scratch.is_empty());
}

#[test]
fn scalar_shortcuts_and_bulk_arithmetic_keep_distinct_results() {
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    let multiply = BinarySampler {
        left: Arc::new(ConstantSampler(-0.0)),
        right: Arc::new(ConstantSampler(f32::NAN)),
        operation: BinaryOp::Multiply,
    };
    assert_eq!(
        multiply
            .sample_value(&mut context, [0; 3])
            .unwrap()
            .to_bits(),
        0
    );
    let mut buffer = [17.0];
    multiply
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert!(buffer[0].is_nan());
    let minimum = BinarySampler {
        left: Arc::new(ConstantSampler(0.0)),
        right: Arc::new(ConstantSampler(-0.0)),
        operation: BinaryOp::Minimum {
            right_min: f32::NEG_INFINITY,
        },
    };
    assert_eq!(
        minimum
            .sample_value(&mut context, [0; 3])
            .unwrap()
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    minimum
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert_eq!(buffer[0].to_bits(), 0);
}

#[test]
fn bulk_choice_evaluates_in_range_before_a_failing_selector() {
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    let choice = RangeChoiceSampler {
        input: Arc::new(GradientSampler::new(
            Axis::X,
            TilingMode::Repeat,
            0,
            0,
            0.0,
            1.0,
        )),
        min_inclusive: 0.0,
        max_exclusive: 1.0,
        when_in_range: Arc::new(ConstantSampler(3.0)),
        when_out_of_range: Arc::new(ConstantSampler(7.0)),
    };
    let mut buffer = [19.0];
    assert_eq!(
        choice.sample_volume(&mut context, &mut buffer, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(buffer, [3.0]);
    assert_eq!(context.scratch.len(), 1);
    let mut too_long = [19.0; 2];
    assert_eq!(
        choice.sample_volume(&mut context, &mut too_long, volume),
        Err(SampleError::ExactBufferSizeRequired)
    );
    assert_eq!(too_long, [19.0; 2]);
}

#[test]
fn interval_branch_errors_keep_selector_output_and_return_scratch() {
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    let selector = IntervalSelectSampler::new(
        Arc::new(ConstantSampler(-3.0)),
        vec![0.0],
        vec![
            Arc::new(ConstantSampler(7.0)),
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::Repeat,
                0,
                0,
                0.0,
                1.0,
            )),
        ],
    )
    .unwrap();
    assert_eq!(selector.sample_value(&mut context, [0; 3]), Ok(7.0));
    let mut buffer = [19.0];
    assert_eq!(
        selector.sample_volume(&mut context, &mut buffer, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(buffer, [-3.0]);
    assert_eq!(context.scratch.len(), 1);
    assert_eq!(context.scratch[0].len(), 2);
    assert!(IntervalSelectSampler::new(Arc::new(ConstantSampler(0.0)), vec![], vec![]).is_err());
}

#[test]
fn range_arithmetic_and_zero_equality_match_reference_special_cases() {
    let zero = Interval::exact(0.0).unwrap();
    assert_eq!(zero.multiply_range(Interval::INFINITE), zero);
    assert!(zero.reciprocal().is_nai());
    assert!(
        Interval::exact(f32::INFINITY)
            .unwrap()
            .add_range(Interval::exact(f32::NEG_INFINITY).unwrap())
            .is_nai()
    );
    let cached = Interval::new(-0.0, 1.0).unwrap();
    assert_eq!(cached.min().to_bits(), 0);
    let mixed_zero = Interval::new(-0.0, 0.0).unwrap();
    assert_eq!(mixed_zero, zero);
    assert_ne!(mixed_zero.java_hash_code(), zero.java_hash_code());
    assert_eq!(Interval::NAI, Interval::NAI);
    assert_eq!(
        Interval::encapsulating(&[Interval::NAI, cached]),
        Ok(cached)
    );
    assert_eq!(Interval::encapsulating(&[]), Err(IntervalError::EmptyInput));
}

#[test]
fn monotonic_mapping_skips_nai_and_rejects_nan_after_both_endpoints() {
    let mut calls = Vec::new();
    assert_eq!(
        Interval::NAI.map_monotonic(|v| {
            calls.push(v);
            v
        }),
        Ok(Interval::NAI)
    );
    assert!(calls.is_empty());
    assert_eq!(
        Interval::new(-1.0, 1.0).unwrap().map_monotonic(|v| {
            calls.push(v);
            v.sqrt()
        }),
        Err(IntervalError::NanMappedBound)
    );
    assert_eq!(calls, [-1.0, 1.0]);
    assert_eq!(
        Interval::exact(0.0)
            .unwrap()
            .lerp_values(f32::INFINITY, f32::NEG_INFINITY),
        Interval::exact(f32::INFINITY).unwrap()
    );
}

#[test]
fn shift_b_repeats_each_horizontal_sample_and_preserves_tail() {
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([2, 3, 4], [-17, -64, 32], [4, 8, 16]).unwrap();
    let source = Arc::new(PerlinNoise::new(&mut LegacyRandom::new(0)));
    let sampler = compile_shift_noise(source, ShiftNoiseKind::ShiftB);
    let mut buffer = vec![19.0; volume.size() + 2];
    sampler
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    for column in buffer[..volume.size()].chunks_exact(3) {
        assert!(column.iter().all(|v| v.to_bits() == column[0].to_bits()));
    }
    assert_eq!(&buffer[volume.size()..], &[19.0, 19.0]);
    assert_eq!(context.scratch.len(), 1);
    assert_eq!(context.scratch[0].len(), 8);
    let mut short = [23.0; 2];
    assert_eq!(
        sampler.sample_volume(&mut context, &mut short, volume),
        Err(SampleError::Volume(DensityError::BufferTooShort))
    );
    assert_eq!(short, [23.0; 2]);
}

#[test]
fn blended_fbm_work_bound_rejects_before_random_draws() {
    let mut random = LegacyRandom::new(0);
    let mut expected = random.clone();
    assert!(matches!(
        create_blended_fbm(&mut random, i32::MIN, 1.0, 1.0),
        Err(crate::noise::NoiseBuildError::TooManyOctaves)
    ));
    assert!(matches!(
        create_blended_fbm(&mut random, 1, 1.0, 1.0),
        Err(crate::noise::NoiseBuildError::PositiveLegacyOctaves)
    ));
    use crate::random::RandomSource;
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn volume_lookup_requires_exact_lattice_and_preserves_java_overflow() {
    let volume = DensityVolume::new([2, 3, 2], [i32::MAX, -64, 32], [4, 1, 4]).unwrap();
    assert_eq!(volume.index_of_block([i32::MIN + 3, -63, 36]), Some(10));
    assert_eq!(volume.index_of_block([i32::MIN, -64, 32]), None);
    let overflow = DensityVolume::new([2, 1, 1], [0; 3], [i32::MAX, 1, 1]).unwrap();
    assert_eq!(overflow.index_of_block([0; 3]), None);
}

#[test]
fn cache_returns_filled_failure_buffer_on_the_next_volume_request() {
    let mut context = SamplerContext::cached();
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    let input = RangeChoiceSampler {
        input: Arc::new(GradientSampler::new(
            Axis::X,
            TilingMode::Repeat,
            0,
            0,
            0.0,
            1.0,
        )),
        min_inclusive: 0.0,
        max_exclusive: 1.0,
        when_in_range: Arc::new(ConstantSampler(7.0)),
        when_out_of_range: Arc::new(ConstantSampler(9.0)),
    };
    let cached = CachingDensitySampler::new(0, Arc::new(input)).unwrap();
    let mut buffer = [19.0];
    assert_eq!(
        cached.sample_volume(&mut context, &mut buffer, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(buffer, [19.0]);
    cached
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert_eq!(buffer, [7.0]);
    assert_eq!(cached.sample_value(&mut context, [0; 3]), Ok(7.0));
    context.clear_caches();
    assert_eq!(
        cached.sample_value(&mut context, [0; 3]),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert!(matches!(
        CachingDensitySampler::new(65536, Arc::new(ConstantSampler(0.0))),
        Err(SampleError::CacheIdOutOfRange)
    ));
}

#[test]
fn slices_delegate_matching_volumes_and_isolate_reduced_child_failures() {
    let mut context = SamplerContext::default();
    let slice = SliceSampler {
        input: Arc::new(ConstantSampler(-0.0)),
        axes: SliceAxes::Xz { x: 7, z: 9 },
    };
    for (sizes, minimum, delegated) in [
        ([1, 2, 1], [7, -64, 9], true),
        ([1, 2, 1], [8, -64, 9], false),
        ([2, 2, 1], [7, -64, 9], false),
        ([1, 2, 2], [7, -64, 9], false),
    ] {
        let volume = DensityVolume::new(sizes, minimum, [4, 8, 16]).unwrap();
        let mut output = vec![19.0; volume.size() + 2];
        slice
            .sample_volume(&mut context, &mut output, volume)
            .unwrap();
        assert!(
            output[..volume.size()]
                .iter()
                .all(|v| v.to_bits() == (-0.0_f32).to_bits())
        );
        let expected_tail = if delegated { -0.0_f32 } else { 19.0 };
        assert!(
            output[volume.size()..]
                .iter()
                .all(|v| v.to_bits() == expected_tail.to_bits())
        );
    }
    let failing = SliceSampler {
        axes: SliceAxes::Y(3),
        input: Arc::new(RangeChoiceSampler {
            input: Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::Repeat,
                0,
                0,
                0.0,
                1.0,
            )),
            min_inclusive: 0.0,
            max_exclusive: 1.0,
            when_in_range: Arc::new(ConstantSampler(7.0)),
            when_out_of_range: Arc::new(ConstantSampler(9.0)),
        }),
    };
    for (y, expected) in [(2, 19.0), (3, 7.0)] {
        let volume = DensityVolume::new([1; 3], [0, y, 0], [4, 8, 16]).unwrap();
        let mut output = [19.0];
        assert_eq!(
            failing.sample_volume(&mut context, &mut output, volume),
            Err(SampleError::ZeroGradientPeriod)
        );
        assert_eq!(output, [expected]);
    }
    assert!(!context.scratch.is_empty());
}

#[test]
fn slices_preserve_reduced_volume_steps_in_cached_children() {
    let input = Arc::new(
        CachingDensitySampler::new(
            0,
            Arc::new(DistanceToPointSampler {
                point: [0; 3],
                metric: DistanceMetric::Manhattan,
            }),
        )
        .unwrap(),
    );
    let mut context = SamplerContext::cached();
    let slice = SliceSampler {
        input: input.clone(),
        axes: SliceAxes::X(7),
    };
    let volume = DensityVolume::new([3, 2, 2], [-17, -64, 32], [4, 8, 16]).unwrap();
    let mut output = vec![0.0; volume.size()];
    slice
        .sample_volume(&mut context, &mut output, volume)
        .unwrap();
    for z in 0..2 {
        for x in 0..3 {
            assert_eq!(
                &output[(x + z * 3) * 2..(x + z * 3) * 2 + 2],
                &[103.0 + z as f32 * 16.0, 95.0 + z as f32 * 16.0]
            );
        }
    }
    // The following matching request must hit the cached reduced descriptor.
    // A short logical output is still rejected before touching that child.
    let reduced = DensityVolume::new([1, 2, 2], [7, -64, 32], [4, 8, 16]).unwrap();
    let mut reduced_output = [0.0; 4];
    input
        .sample_volume(&mut context, &mut reduced_output, reduced)
        .unwrap();
    assert_eq!(reduced_output, [103.0, 95.0, 119.0, 111.0]);
    let mut short = [19.0];
    assert_eq!(
        slice.sample_volume(&mut context, &mut short, volume),
        Err(SampleError::Volume(DensityError::BufferTooShort))
    );
    assert_eq!(short, [19.0]);
}

#[test]
fn grid_interpolation_keeps_incremental_bulk_rounding() {
    let input = Arc::new(GradientSampler::new(
        Axis::Y,
        TilingMode::ClampToEdge,
        0,
        10,
        0.0,
        1.0,
    ));
    let sampler = InterpolatedSampler::new(input, 4, 10).unwrap();
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([1, 10, 1], [0; 3], [1; 3]).unwrap();
    let mut output = [19.0; 11];
    sampler
        .sample_volume(&mut context, &mut output, volume)
        .unwrap();
    assert_eq!(output[7].to_bits(), 0x3f333334);
    assert_eq!(
        sampler
            .sample_value(&mut context, [0, 7, 0])
            .unwrap()
            .to_bits(),
        0x3f333333
    );
    assert_eq!(output[10], 19.0);
    let strided = DensityVolume::new([1, 5, 1], [0; 3], [1, 2, 1]).unwrap();
    let mut selected = [0.0; 5];
    sampler
        .sample_volume(&mut context, &mut selected, strided)
        .unwrap();
    for (i, value) in selected.iter().enumerate() {
        assert_eq!(value.to_bits(), output[i * 2].to_bits());
    }
}

#[test]
fn grid_interpolation_uses_child_bulk_corners_and_preserves_aligned_delegation() {
    #[derive(Debug, Default)]
    struct Traced {
        volumes: std::sync::Mutex<Vec<DensityVolume>>,
        points: std::sync::Mutex<Vec<[i32; 3]>>,
    }
    impl DensitySampler for Traced {
        fn sample_value(
            &self,
            _: &mut SamplerContext,
            position: [i32; 3],
        ) -> Result<f32, SampleError> {
            self.points.lock().unwrap().push(position);
            Ok(-0.0)
        }
        fn sample_volume(
            &self,
            _: &mut SamplerContext,
            buffer: &mut [f32],
            volume: DensityVolume,
        ) -> Result<(), SampleError> {
            self.volumes.lock().unwrap().push(volume);
            buffer.fill(-0.0);
            Ok(())
        }
    }
    let traced = Arc::new(Traced::default());
    let sampler = InterpolatedSampler::new(traced.clone(), 4, 8).unwrap();
    let mut context = SamplerContext::default();
    assert_eq!(
        sampler
            .sample_value(&mut context, [-4, -8, 4])
            .unwrap()
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    sampler.sample_value(&mut context, [-1, -1, -1]).unwrap();
    assert_eq!(*traced.points.lock().unwrap(), [[-4, -8, 4]]);
    assert_eq!(
        *traced.volumes.lock().unwrap(),
        [DensityVolume::new([2; 3], [-4, -8, -4], [4, 8, 4]).unwrap()]
    );
    let volume = DensityVolume::new([2, 1, 2], [-4, -8, 4], [4, 1, 4]).unwrap();
    let mut output = [19.0; 6];
    sampler
        .sample_volume(&mut context, &mut output, volume)
        .unwrap();
    assert!(output.iter().all(|v| v.to_bits() == (-0.0_f32).to_bits()));
    assert_eq!(traced.volumes.lock().unwrap()[1], volume);
    let distant = DensityVolume::new([16, 49, 16], [-8192, -64, -8192], [1024, 8, 1024]).unwrap();
    let mut output = vec![19.; distant.size()];
    sampler
        .sample_volume(&mut SamplerContext::sparse(), &mut output, distant)
        .unwrap();
    assert_eq!(
        traced.volumes.lock().unwrap()[2],
        distant,
        "far samples must not expand to a block-resolution bounding box"
    );
    let unaligned = DensityVolume::new([2; 3], [-5, -9, -7], [1024, 8, 1024]).unwrap();
    let mut output = [19.; 8];
    sampler
        .sample_volume(&mut SamplerContext::sparse(), &mut output, unaligned)
        .unwrap();
    assert!(output.iter().all(|&v| v == 0.));
    assert!(
        traced.volumes.lock().unwrap()[3..]
            .iter()
            .all(|v| v.size() == 8),
        "unaligned far points use bounded corner queries"
    );
    assert!(InterpolatedSampler::new(traced.clone(), 0, 8).is_err());
    assert!(InterpolatedSampler::new(traced, 4, -1).is_err());
}

#[test]
fn surface_search_fixes_upper_bound_y_and_visits_descending_cell_boundaries() {
    #[derive(Debug, Default)]
    struct DensityTrace(std::sync::Mutex<Vec<[i32; 3]>>);
    impl DensitySampler for DensityTrace {
        fn sample_value(
            &self,
            _: &mut SamplerContext,
            position: [i32; 3],
        ) -> Result<f32, SampleError> {
            self.0.lock().unwrap().push(position);
            Ok(match position[1] {
                12 => f32::NAN,
                8 => -0.0,
                _ => 1.0,
            })
        }
    }
    let density = Arc::new(DensityTrace::default());
    let upper = Arc::new(GradientSampler::new(
        Axis::Y,
        TilingMode::ClampToEdge,
        0,
        10,
        14.0,
        54.0,
    ));
    let sampler = compile_surface_search(density.clone(), upper, -64, 4).unwrap();
    assert_eq!(
        sampler.sample_value(&mut SamplerContext::default(), [7, 1000, -9]),
        Ok(4.0)
    );
    assert_eq!(
        *density.0.lock().unwrap(),
        [[7, 12, -9], [7, 8, -9], [7, 4, -9]]
    );
    density.0.lock().unwrap().clear();
    let shortcut =
        FindTopSurfaceSampler::new(density.clone(), Arc::new(ConstantSampler(-63.0)), -64, 8)
            .unwrap();
    assert_eq!(
        shortcut.sample_value(&mut SamplerContext::default(), [0; 3]),
        Ok(-64.0)
    );
    assert!(density.0.lock().unwrap().is_empty());
    let negative_infinity = FindTopSurfaceSampler::new(
        density.clone(),
        Arc::new(ConstantSampler(f32::NEG_INFINITY)),
        -64,
        1,
    )
    .unwrap();
    assert_eq!(
        negative_infinity.sample_value(&mut SamplerContext::default(), [0; 3]),
        Ok(-64.0)
    );
    assert!(density.0.lock().unwrap().is_empty());
}

#[test]
fn surface_bulk_preserves_upper_bound_writes_and_sliced_error_isolation() {
    #[derive(Debug)]
    struct FailSecondColumn;
    impl DensitySampler for FailSecondColumn {
        fn sample_value(
            &self,
            _: &mut SamplerContext,
            position: [i32; 3],
        ) -> Result<f32, SampleError> {
            if position[0] == 1 {
                Err(SampleError::ZeroGradientPeriod)
            } else {
                Ok(if position[1] <= 4 { 1.0 } else { 0.0 })
            }
        }
    }
    let raw = Arc::new(
        FindTopSurfaceSampler::new(
            Arc::new(FailSecondColumn),
            Arc::new(ConstantSampler(14.0)),
            -64,
            4,
        )
        .unwrap(),
    );
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([2, 1, 1], [0; 3], [1; 3]).unwrap();
    let mut output = [19.0; 3];
    assert_eq!(
        raw.sample_volume(&mut context, &mut output, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(output, [4.0, 14.0, 14.0]);
    let volume = DensityVolume::new([2, 2, 1], [0; 3], [1; 3]).unwrap();
    let mut output = [19.0; 5];
    assert_eq!(
        raw.sample_volume(&mut context, &mut output, volume),
        Err(SampleError::SurfaceRequiresSingleLayer)
    );
    assert_eq!(output, [19.0; 5]);
    let sliced = SliceSampler {
        input: raw,
        axes: SliceAxes::Y(0),
    };
    assert_eq!(
        sliced.sample_volume(&mut context, &mut output, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(output, [19.0; 5]);
    assert!(!context.scratch.is_empty());
}

#[test]
fn spline_coordinate_cache_retries_nan_scalars_but_reuses_nan_volumes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Counted {
        value: f32,
        scalar: AtomicUsize,
        bulk: AtomicUsize,
    }
    impl DensitySampler for Counted {
        fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
            self.scalar.fetch_add(1, Ordering::Relaxed);
            Ok(self.value)
        }
        fn sample_volume(
            &self,
            _: &mut SamplerContext,
            output: &mut [f32],
            _: DensityVolume,
        ) -> Result<(), SampleError> {
            self.bulk.fetch_add(1, Ordering::Relaxed);
            output.fill(self.value);
            Ok(())
        }
    }
    let inner = CubicSpline::multipoint(
        1,
        vec![SplinePoint {
            location: 0.0,
            derivative: -0.0,
            value: CubicSpline::Constant(7.0),
        }],
    )
    .unwrap();
    let root = CubicSpline::multipoint(
        1,
        vec![
            SplinePoint {
                location: -1.0,
                derivative: 0.0,
                value: inner.clone(),
            },
            SplinePoint {
                location: 1.0,
                derivative: 0.0,
                value: inner,
            },
        ],
    )
    .unwrap();
    for (value, scalar_calls) in [(0.0, 1), (f32::NAN, 2)] {
        let input = Arc::new(Counted {
            value,
            scalar: AtomicUsize::new(0),
            bulk: AtomicUsize::new(0),
        });
        let sampler =
            SplineSampler::new(&root, &[Arc::new(ConstantSampler(99.0)), input.clone()]).unwrap();
        assert_eq!(sampler.coordinate_count(), 1);
        let mut context = SamplerContext::default();
        assert_eq!(sampler.sample_value(&mut context, [0; 3]), Ok(7.0));
        assert_eq!(input.scalar.load(Ordering::Relaxed), scalar_calls);
        let volume = DensityVolume::new([2, 3, 2], [0; 3], [1; 3]).unwrap();
        let mut output = [19.0; 12];
        sampler
            .sample_volume(&mut context, &mut output, volume)
            .unwrap();
        assert_eq!(output, [7.0; 12]);
        assert_eq!(input.bulk.load(Ordering::Relaxed), 1);
        assert_eq!(input.scalar.load(Ordering::Relaxed), scalar_calls);
    }
    assert!(matches!(
        SplineSampler::new(&root, &[]),
        Err(SplineError::MissingCoordinate)
    ));
    assert!(matches!(
        CubicSpline::multipoint(0, vec![]),
        Err(SplineError::EmptyPoints)
    ));
}

#[test]
fn spline_lazy_branch_failure_preserves_completed_output_and_returns_scratch() {
    let fail = CubicSpline::multipoint(
        1,
        vec![SplinePoint {
            location: 0.0,
            derivative: 0.0,
            value: CubicSpline::Constant(11.0),
        }],
    )
    .unwrap();
    let root = CubicSpline::multipoint(
        0,
        vec![
            SplinePoint {
                location: -1.0,
                derivative: 0.0,
                value: CubicSpline::Constant(7.0),
            },
            SplinePoint {
                location: 1.0,
                derivative: 0.0,
                value: fail,
            },
        ],
    )
    .unwrap();
    let inputs: Vec<Arc<dyn DensitySampler>> = vec![
        Arc::new(GradientSampler::new(
            Axis::X,
            TilingMode::ClampToEdge,
            0,
            1,
            -2.0,
            0.0,
        )),
        Arc::new(GradientSampler::new(
            Axis::X,
            TilingMode::Repeat,
            0,
            0,
            0.0,
            1.0,
        )),
    ];
    let sampler = SplineSampler::new(&root, &inputs).unwrap();
    let mut context = SamplerContext::default();
    let volume = DensityVolume::new([2, 1, 1], [0; 3], [1; 3]).unwrap();
    let mut output = [19.0; 2];
    assert_eq!(
        sampler.sample_volume(&mut context, &mut output, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(output, [7.0, 19.0]);
    assert_eq!(context.scratch.len(), 2);
    assert_eq!(sampler.sample_value(&mut context, [0; 3]), Ok(7.0));
}

#[test]
fn reset_retains_scratch_but_discards_graph_caches_and_world_bindings() {
    let binding = Arc::new(ConstantSampler(17.0));
    let lifetime = Arc::downgrade(&binding);
    let mut context = SamplerContext::cached()
        .with_world_fields(WorldFields::default().with_field(WorldField::Beardifier, binding));
    let cached = CachingDensitySampler::new(
        0,
        Arc::new(ContextBoundSampler::new(WorldField::Beardifier)),
    )
    .unwrap();
    let volume = DensityVolume::new([2; 3], [0; 3], [1; 3]).unwrap();
    let mut output = [0.0; 8];
    cached
        .sample_volume(&mut context, &mut output, volume)
        .unwrap();
    assert_eq!(output, [17.0; 8]);
    assert_eq!(cached.sample_value(&mut context, [9; 3]), Ok(17.0));
    context
        .with_buffer(128, |_, buffer| {
            buffer.fill(99.0);
            Ok(())
        })
        .unwrap();
    let allocation = context.scratch[0].as_ptr();
    let capacity = context.scratch[0].capacity();
    context.reset();
    assert!(lifetime.upgrade().is_none());
    assert!(context.caches.as_ref().unwrap().is_empty());
    assert_eq!(context.scratch[0].as_ptr(), allocation);
    assert_eq!(context.scratch[0].capacity(), capacity);
    // Same coordinates and cache ID must resolve the new context's fallback.
    assert_eq!(cached.sample_value(&mut context, [9; 3]), Ok(0.0));
    cached
        .sample_volume(&mut context, &mut output, volume)
        .unwrap();
    assert_eq!(output, [0.0; 8]);
    // A different graph may reuse IDs without inheriting scalar/volume results.
    context.reset();
    let replacement = CachingDensitySampler::new(0, Arc::new(ConstantSampler(3.0))).unwrap();
    assert_eq!(replacement.sample_value(&mut context, [9; 3]), Ok(3.0));
    replacement
        .sample_volume(&mut context, &mut output, volume)
        .unwrap();
    assert_eq!(output, [3.0; 8]);
}

#[test]
fn reset_after_sampling_error_drops_blender_and_preserves_uncached_mode() {
    let blender = Arc::new(EmptyBlender);
    let lifetime = Arc::downgrade(&blender);
    let mut context =
        SamplerContext::default().with_world_fields(WorldFields::default().with_blender(blender));
    let result: Result<(), SampleError> = context.with_buffer(64, |_, buffer| {
        buffer.fill(42.0);
        Err(SampleError::RecursiveCache)
    });
    assert_eq!(result, Err(SampleError::RecursiveCache));
    let allocation = context.scratch[0].as_ptr();
    context.reset();
    assert!(lifetime.upgrade().is_none());
    assert!(context.caches.is_none());
    context
        .with_buffer(32, |_, buffer| {
            assert_eq!(buffer.as_ptr(), allocation);
            Ok(())
        })
        .unwrap();
}

#[test]
fn unary_bulk_keeps_child_failure_writes_and_full_buffer_tail() {
    #[derive(Debug)]
    struct PartialInput;
    impl DensitySampler for PartialInput {
        fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
            Err(SampleError::ZeroGradientPeriod)
        }
        fn sample_volume(
            &self,
            _: &mut SamplerContext,
            buffer: &mut [f32],
            _: DensityVolume,
        ) -> Result<(), SampleError> {
            buffer[0] = -3.0;
            Err(SampleError::ZeroGradientPeriod)
        }
    }
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    let mut context = SamplerContext::default();
    let mut buffer = [17.0; 3];
    let sampler = UnarySampler {
        input: Arc::new(PartialInput),
        operation: UnaryOp::Square,
    };
    assert_eq!(
        sampler.sample_volume(&mut context, &mut buffer, volume),
        Err(SampleError::ZeroGradientPeriod)
    );
    assert_eq!(buffer, [-3.0, 17.0, 17.0]);
    let sampler = UnarySampler {
        input: Arc::new(ConstantSampler(-3.0)),
        operation: UnaryOp::Square,
    };
    sampler
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert_eq!(buffer, [9.0; 3]);
}

#[test]
fn binary_bulk_preserves_child_failure_order_and_output() {
    #[derive(Debug)]
    struct Input {
        name: &'static str,
        value: f32,
        fail: bool,
        calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }
    impl DensitySampler for Input {
        fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
            unreachable!("bulk-only fixture")
        }
        fn sample_volume(
            &self,
            _: &mut SamplerContext,
            buffer: &mut [f32],
            _: DensityVolume,
        ) -> Result<(), SampleError> {
            self.calls.lock().unwrap().push(self.name);
            buffer.fill(self.value);
            if self.fail {
                Err(SampleError::ZeroGradientPeriod)
            } else {
                Ok(())
            }
        }
    }
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    for left_fails in [true, false] {
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sampler = BinarySampler {
            left: Arc::new(Input {
                name: "left",
                value: 3.0,
                fail: left_fails,
                calls: calls.clone(),
            }),
            right: Arc::new(Input {
                name: "right",
                value: 7.0,
                fail: true,
                calls: calls.clone(),
            }),
            operation: BinaryOp::Multiply,
        };
        let mut context = SamplerContext::default();
        let mut wrong_size = [17.0; 2];
        assert!(
            sampler
                .sample_volume(&mut context, &mut wrong_size, volume)
                .is_err()
        );
        assert_eq!(wrong_size, [17.0; 2]);
        assert!(calls.lock().unwrap().is_empty());
        let mut output = [17.0];
        assert_eq!(
            sampler.sample_volume(&mut context, &mut output, volume),
            Err(SampleError::ZeroGradientPeriod)
        );
        assert_eq!(output, [3.0]);
        assert_eq!(
            *calls.lock().unwrap(),
            if left_fails {
                vec!["left"]
            } else {
                vec!["left", "right"]
            }
        );
    }
}
/// A y-fixed slice's plane, kept by `SamplerContext::with_planes`, gives the
/// values fresh sampling gives at other heights and vertical steps, and is
/// sampled once for one x/z lattice.
#[test]
fn kept_planes_match_fresh_slices_across_vertical_lattices() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Counted(NoiseSampler, AtomicUsize);
    impl DensitySampler for Counted {
        fn sample_value(&self, c: &mut SamplerContext, p: [i32; 3]) -> Result<f32, SampleError> {
            self.0.sample_value(c, p)
        }
        fn sample_volume(
            &self,
            c: &mut SamplerContext,
            out: &mut [f32],
            v: DensityVolume,
        ) -> Result<(), SampleError> {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.sample_volume(c, out, v)
        }
    }
    let input = Arc::new(Counted(
        NoiseSampler {
            noise: Arc::new(PerlinNoise::new(&mut LegacyRandom::new(5))),
            xz_scale: 0.3,
            y_scale: 0.7,
        },
        AtomicUsize::new(0),
    ));
    let slice = slice::SliceSampler {
        input: input.clone(),
        axes: slice::SliceAxes::Y(0),
    };
    let lattices = [
        DensityVolume::new([5, 9, 4], [-8, -64, 12], [4, 64, 4]).unwrap(),
        DensityVolume::new([5, 17, 4], [-8, 0, 12], [4, 8, 4]).unwrap(),
    ];
    let mut kept = SamplerContext::sparse().with_planes();
    for (i, volume) in lattices.into_iter().enumerate() {
        let mut fresh = vec![0.0; volume.size()];
        slice
            .sample_volume(&mut SamplerContext::sparse(), &mut fresh, volume)
            .unwrap();
        let mut reused = vec![1.0; volume.size()];
        slice.sample_volume(&mut kept, &mut reused, volume).unwrap();
        assert_eq!(fresh, reused, "lattice {i}");
    }
    // Two fresh evaluations and one kept plane.
    assert_eq!(input.1.load(Ordering::Relaxed), 3);
}
