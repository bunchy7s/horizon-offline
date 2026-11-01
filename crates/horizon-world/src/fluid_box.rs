//! Entity fluid scan bounds and the boat passenger waterline restriction.
use horizon_core::geometry::{Aabb, Vec3};
pub trait FluidBoxWorld {
    type Error;
    type Vehicle: Copy;
    fn bounding_box(&self) -> Aabb;
    fn vehicle(&mut self) -> Result<Option<Self::Vehicle>, Self::Error>;
    fn modify_passenger_box(
        &mut self,
        vehicle: Self::Vehicle,
        bounds: Aabb,
    ) -> Result<Option<Aabb>, Self::Error>;
}
pub fn interaction_box<W: FluidBoxWorld>(world: &mut W) -> Result<Option<Aabb>, W::Error> {
    let bounds = world.bounding_box().deflate(Vec3::new(0.001, 0.001, 0.001));
    match world.vehicle()? {
        None => Ok(Some(bounds)),
        Some(vehicle) => world.modify_passenger_box(vehicle, bounds),
    }
}
pub trait BoatFluidBoxWorld {
    type Error;
    fn is_under_water(&mut self) -> Result<bool, Self::Error>;
    fn boat_bounds(&self) -> Aabb;
}
/// Submerged boats preserve the passenger bounds. Otherwise only the part
/// strictly above the boat's top can interact with fluid; it may be absent.
pub fn boat_passenger_box<W: BoatFluidBoxWorld>(
    world: &mut W,
    bounds: Aabb,
) -> Result<Option<Aabb>, W::Error> {
    if world.is_under_water()? {
        return Ok(Some(bounds));
    }
    let boat_top = world.boat_bounds().max_position().y;
    let mut min = bounds.min_position();
    let max = bounds.max_position();
    if boat_top >= max.y {
        return Ok(None);
    }
    min.y = java_max(min.y, boat_top);
    Ok(Some(Aabb::new(min, max)))
}
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else {
        a.max(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Boat {
        under_water: bool,
        bounds: Aabb,
    }
    impl BoatFluidBoxWorld for Boat {
        type Error = std::convert::Infallible;
        fn is_under_water(&mut self) -> Result<bool, Self::Error> {
            Ok(self.under_water)
        }
        fn boat_bounds(&self) -> Aabb {
            self.bounds
        }
    }

    fn boxed(min_y: f64, max_y: f64) -> Aabb {
        Aabb::new(Vec3::new(-0.3, min_y, -0.3), Vec3::new(0.3, max_y, 0.3))
    }

    /// A submerged boat leaves the passenger's box alone.
    #[test]
    fn a_submerged_boat_keeps_the_passenger_box() {
        let passenger = boxed(0.0, 1.8);
        let mut boat = Boat {
            under_water: true,
            bounds: boxed(0.0, 0.6),
        };
        assert_eq!(
            boat_passenger_box(&mut boat, passenger),
            Ok(Some(passenger))
        );
    }

    /// A boat reaching the passenger's top leaves no fluid interaction at all.
    #[test]
    fn a_boat_reaching_the_passenger_top_removes_the_box() {
        let passenger = boxed(0.0, 1.8);
        for top in [1.8, 2.0] {
            let mut boat = Boat {
                under_water: false,
                bounds: boxed(0.0, top),
            };
            assert_eq!(boat_passenger_box(&mut boat, passenger), Ok(None));
        }
    }

    /// Otherwise only the part above the boat's top interacts with fluid.
    #[test]
    fn a_floating_boat_trims_the_passenger_box_to_its_top() {
        let mut boat = Boat {
            under_water: false,
            bounds: boxed(0.0, 0.6),
        };
        assert_eq!(
            boat_passenger_box(&mut boat, boxed(0.0, 1.8)),
            Ok(Some(boxed(0.6, 1.8)))
        );
        // A boat entirely below the passenger raises nothing.
        let mut low = Boat {
            under_water: false,
            bounds: boxed(-2.0, -1.0),
        };
        assert_eq!(
            boat_passenger_box(&mut low, boxed(0.0, 1.8)),
            Ok(Some(boxed(0.0, 1.8)))
        );
    }
}
