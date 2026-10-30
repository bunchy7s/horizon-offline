//! End gateway travel for the entities that step into one:
//! `EndGatewayBlock.entityInside` (the gateway takes the entity in and starts
//! its 40 tick cooldown), `Entity.setAsInsidePortal` and `Entity.handlePortal`
//! (the entity goes through on the next tick), and
//! `TheEndGatewayBlockEntity.getPortalPosition` (where it lands).
use crate::nether_portal::{ENTITY_DELAY, PortalState};
use horizon_core::{geometry::Vec3, position::BlockPos};

/// The End gateway travel of one entity.
#[derive(Clone, Copy, Debug, Default)]
pub struct Traveller {
    /// The entity's portal process and cooldown (`Entity.portalProcess`).
    state: PortalState,
}

impl Traveller {
    /// `Entity.setAsInsidePortal`: the entity is in the gateway and goes
    /// through it on the next tick.
    pub fn enter(&mut self, gateway: BlockPos) {
        self.state.set_inside(gateway, ENTITY_DELAY);
    }

    /// `Entity.handlePortal`: the entity goes through the gateway it entered
    /// (the End gateway has no transition time), or stays where it is.
    pub fn travel(&mut self, allowed: bool) -> Option<BlockPos> {
        self.state.handle(allowed, 0, ENTITY_DELAY)
    }

    /// Whether the entity's travel is over: it has no portal process left and
    /// its portal cooldown has run out.
    pub fn is_done(&self) -> bool {
        self.state.cooldown == 0 && self.state.process.is_none()
    }

    /// The entity's portal cooldown, the ticks it stays out of a portal.
    pub fn cooldown(&self) -> i32 {
        self.state.cooldown
    }
}

/// The End gateway block an entity's box swept through
/// (`Entity.checkInsideBlocks`, which calls `EndGatewayBlock.entityInside`).
pub fn swept_gateway(
    from: Vec3,
    to: Vec3,
    width: f32,
    height: f32,
    mut is_gateway: impl FnMut(BlockPos) -> bool,
) -> Option<BlockPos> {
    let half = f64::from(width) / 2.0;
    let bounds = horizon_core::geometry::Aabb::new(
        Vec3::new(to.x - half, to.y, to.z - half),
        Vec3::new(to.x + half, to.y + f64::from(height), to.z + half),
    )
    .deflate(Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
    let mut found = None;
    let _ =
        horizon_core::geometry::sweep_intersected_blocks(from, to, bounds, 1_000_000, |p, _| {
            if is_gateway(p) {
                found = Some(p);
                return false;
            }
            true
        });
    found
}

/// The bottom centre of a block (`Vec3.atBottomCenterOf`): where a traveller
/// lands at a gateway's exit.
pub fn bottom_center(p: [i32; 3]) -> Vec3 {
    Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]),
        f64::from(p[2]) + 0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gateway of the issue: a block at [702, 100, 0] in the End.
    const GATEWAY: [i32; 3] = [702, 100, 0];

    fn at(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(x, y, z)
    }

    fn is_gateway(p: BlockPos) -> bool {
        [p.x, p.y, p.z] == GATEWAY
    }

    #[test]
    fn an_item_and_a_mob_in_a_gateway_are_found_in_it() {
        // A dropped item, 0.25 wide, standing in the gateway block.
        let item = at(702.5, 100.5, 0.5);
        assert_eq!(
            swept_gateway(item, item, 0.25, 0.25, is_gateway),
            Some(BlockPos::new(702, 100, 0))
        );
        // A cow, 0.9 wide and 1.4 tall, in the same block.
        let cow = at(702.5, 100.0, 0.5);
        assert_eq!(
            swept_gateway(cow, cow, 0.9, 1.4, is_gateway),
            Some(BlockPos::new(702, 100, 0))
        );
    }

    #[test]
    fn an_entity_out_of_the_gateway_is_not_in_it() {
        // Two blocks above the gateway: nothing to travel through.
        let above = at(702.5, 102.5, 0.5);
        assert_eq!(swept_gateway(above, above, 0.25, 0.25, is_gateway), None);
        // Beside the gateway, outside its block.
        let beside = at(704.5, 100.5, 0.5);
        assert_eq!(swept_gateway(beside, beside, 0.9, 1.4, is_gateway), None);
    }

    #[test]
    fn an_entity_falling_through_the_gateway_goes_through_it() {
        // A cow falling past the gateway block passes through it
        // (Entity.checkInsideBlocks visits the blocks along the way).
        let from = at(702.5, 103.0, 0.5);
        let to = at(702.5, 99.0, 0.5);
        assert_eq!(
            swept_gateway(from, to, 0.9, 1.4, is_gateway),
            Some(BlockPos::new(702, 100, 0))
        );
    }
    #[test]
    fn an_entity_goes_through_the_gateway_on_the_tick_after_it_entered() {
        let gateway = BlockPos::new(702, 100, 0);
        let mut traveller = Traveller::default();
        // Nothing happens while the entity is outside the gateway.
        assert_eq!(traveller.travel(true), None);
        // It enters the gateway, and goes through on the next tick.
        traveller.enter(gateway);
        assert_eq!(traveller.travel(true), Some(gateway));
        // Its portal cooldown keeps it from going back at once.
        assert_eq!(traveller.cooldown(), ENTITY_DELAY);
        assert!(!traveller.is_done());
    }

    #[test]
    fn an_entity_that_may_not_use_portals_stays_in_the_gateway() {
        let gateway = BlockPos::new(702, 100, 0);
        let mut traveller = Traveller::default();
        traveller.enter(gateway);
        // A mob that cannot use a portal does not go through.
        assert_eq!(traveller.travel(false), None);
    }

    #[test]
    fn a_traveller_lands_at_the_bottom_centre_of_the_exit() {
        // TheEndGatewayBlockEntity.getPortalPosition: the exit's bottom
        // centre, the same spot the player arrives at.
        assert_eq!(bottom_center([8, 100, 0]), at(8.5, 100.0, 0.5));
    }
}
