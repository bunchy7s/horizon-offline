//! `PortalProcessor` and an entity's portal cooldown
//! (`Entity.setAsInsidePortal` / `handlePortal`) for nether portals.
use horizon_core::position::BlockPos;

/// `Entity.getDimensionChangingDelay`: the portal cooldown of an entity that
/// went through a portal.
pub const ENTITY_DELAY: i32 = 300;

/// `PortalProcessor` for a nether portal.
#[derive(Clone, Copy, Debug)]
pub struct Process {
    pub entry: BlockPos,
    pub time: i32,
    pub inside: bool,
}
impl Process {
    /// `processPortalTeleportation`: whether the entity goes through now.
    pub fn process(&mut self, allowed: bool, transition_time: i32) -> bool {
        if !self.inside {
            // decayTick
            self.time = (self.time - 4).max(0);
            return false;
        }
        self.inside = false;
        if !allowed {
            return false;
        }
        let time = self.time;
        self.time += 1;
        time >= transition_time
    }
}

/// An entity's `portalCooldown` and `portalProcess`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PortalState {
    pub cooldown: i32,
    pub process: Option<Process>,
}
impl PortalState {
    /// `Entity.setAsInsidePortal` for a nether portal block at `pos`.
    pub fn set_inside(&mut self, pos: BlockPos, delay: i32) {
        if self.cooldown > 0 {
            self.cooldown = delay;
            return;
        }
        match &mut self.process {
            None => {
                self.process = Some(Process {
                    entry: pos,
                    time: 0,
                    inside: true,
                })
            }
            Some(p) if !p.inside => {
                p.entry = pos;
                p.inside = true;
            }
            Some(_) => {}
        }
    }
    /// `Entity.handlePortal`: the entry block when the entity goes through
    /// (its cooldown then set to `delay`).
    pub fn handle(&mut self, allowed: bool, transition_time: i32, delay: i32) -> Option<BlockPos> {
        // processPortalCooldown
        if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        let process = self.process.as_mut()?;
        if process.process(allowed, transition_time) {
            self.cooldown = delay;
            return Some(process.entry);
        }
        // hasExpired
        if process.time <= 0 {
            self.process = None;
        }
        None
    }
}
