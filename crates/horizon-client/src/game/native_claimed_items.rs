//! A client mod's view and its own items (docs/native-mods.md, "A client mod's view and items"): the
//! first block along the view, across the distant terrain past the loaded chunks, and items whose
//! right-click is the mod's alone, heard as use events the mod polls.
use super::Game;
use std::collections::BTreeMap;

/// A stack as a client mod's use events carry it.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeHeldItem {
    pub id: String,
    pub count: u32,
    /// The inventory slot: 0 to 8, the hotbar (a mod use is the main hand's).
    pub slot: u32,
    /// `minecraft:custom_name` as SNBT, when the stack has one.
    pub custom_name: Option<String>,
    /// `minecraft:custom_data` as SNBT, when the stack has some.
    pub custom_data: Option<String>,
}
/// The block a ray along the view meets.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeHit {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    /// Where the ray meets the block.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The block's id (`minecraft:stone`).
    pub block: String,
}
/// What happened to a mod's claimed item, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeModEvent {
    /// A right-click with the item began its use: the stack, and 0 ticks.
    UseStart(NativeHeldItem, u32),
    /// The use ended: the stack it began with, and the ticks it was held. It
    /// ends when the button comes up, the hand's item changes or leaves the
    /// claim, the player respawns, or the game joins a server.
    UseStop(NativeHeldItem, u32),
}
/// The farthest a ray along the view reaches, in blocks.
pub(crate) const RAYCAST_MAX: f32 = 1024.0;
/// The ids a mod may claim.
const CLAIMS_MAX: usize = 64;
/// Events kept for a mod between two polls.
const EVENTS_MAX: usize = 256;

/// A claimed item's use under way.
struct ModUse {
    mod_id: String,
    item: NativeHeldItem,
    /// The player's `using` while it is this use: its slot and ticks.
    slot: usize,
    ticks: u32,
}

/// The items client mods claim, their events waiting for a poll, and the use under way.
#[derive(Default)]
pub(super) struct ModInput {
    claims: BTreeMap<String, Vec<String>>,
    events: BTreeMap<String, Vec<NativeModEvent>>,
    using: Option<ModUse>,
}

impl ModInput {
    fn claim(&mut self, mod_id: &str, ids: &[String]) {
        let ids = ids.iter().take(CLAIMS_MAX).cloned().collect();
        self.claims.insert(mod_id.to_owned(), ids);
    }
    /// The mod that claims `item`, the first in id order.
    pub(super) fn claimant(&self, item: &str) -> Option<&str> {
        self.claims
            .iter()
            .find(|(_, ids)| ids.iter().any(|i| i == item))
            .map(|(id, _)| id.as_str())
    }
    fn push(&mut self, mod_id: &str, event: NativeModEvent) {
        let events = self.events.entry(mod_id.to_owned()).or_default();
        if events.len() < EVENTS_MAX {
            events.push(event);
        }
    }
    fn poll(&mut self, mod_id: &str) -> Vec<NativeModEvent> {
        self.events.remove(mod_id).unwrap_or_default()
    }
    /// Mod `mod_id`'s use of `item` in `slot` begins, ending any other first.
    fn start(&mut self, mod_id: &str, item: NativeHeldItem, slot: usize) {
        self.end();
        self.push(mod_id, NativeModEvent::UseStart(item.clone(), 0));
        self.using = Some(ModUse {
            mod_id: mod_id.to_owned(),
            item,
            slot,
            ticks: 0,
        });
    }
    /// Whether the player's use (`Game.using`) is the mod use under way. A
    /// use the game reset and another item's begun since are not.
    fn owns(&self, using: Option<(usize, u32)>) -> bool {
        self.using
            .as_ref()
            .is_some_and(|u| using == Some((u.slot, u.ticks)))
    }
    fn tick(&mut self) {
        if let Some(u) = &mut self.using {
            u.ticks += 1;
        }
    }
    /// The use under way ends: its mod hears `UseStop`.
    fn end(&mut self) {
        if let Some(u) = self.using.take() {
            self.push(&u.mod_id, NativeModEvent::UseStop(u.item, u.ticks));
        }
    }
}

#[allow(dead_code)]
impl Game<'_> {
    /// The stack in inventory slot `slot` as a mod's events carry it.
    fn mod_held(&self, slot: usize) -> Option<NativeHeldItem> {
        let stack = self.stack_in(slot)?;
        let extra = stack.extra.as_deref();
        Some(NativeHeldItem {
            id: stack.item.to_string(),
            count: stack.count,
            slot: slot as u32,
            custom_name: extra
                .and_then(|e| e.custom_name.as_ref())
                .map(horizon_client::snbt::to_snbt),
            custom_data: extra
                .and_then(|e| e.other("minecraft:custom_data"))
                .map(horizon_client::snbt::to_snbt),
        })
    }
    /// The first block along the view within `distance` (at most 1024), past the loaded chunks over the distant terrain. API 116.
    pub(super) fn native_raycast(&self, distance: f32) -> Option<NativeHit> {
        let (d, hit) = self.native_view_hit()?;
        (d <= distance.clamp(0.0, RAYCAST_MAX)).then_some(hit)
    }
    /// The first block along the view within `RAYCAST_MAX`, and its distance. API 116.
    pub(super) fn native_view_hit(&self) -> Option<(f32, NativeHit)> {
        self.player.as_ref()?;
        let eye = self.eye_position()?;
        let dir = super::view_vector(self.yaw, self.pitch);
        let reach = f64::from(RAYCAST_MAX);
        let (pos, point, state) = match self.clip_ray(eye, dir, reach) {
            Some(hit) => (
                hit.pos,
                hit.point,
                self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]),
            ),
            None => self.lod_ray(eye, dir, reach)?,
        };
        let distance = (0..3)
            .map(|i| (point[i] - eye[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        Some((
            distance as f32,
            NativeHit {
                block_x: pos[0],
                block_y: pos[1],
                block_z: pos[2],
                x: point[0],
                y: point[1],
                z: point[2],
                block: self.res.facts().get(state).name.to_string(),
            },
        ))
    }
    /// A ray over the distant terrain (`Lod::ground`) past the loaded chunks, a block at a time.
    fn lod_ray(
        &self,
        eye: [f64; 3],
        dir: [f64; 3],
        reach: f64,
    ) -> Option<([i32; 3], [f64; 3], horizon_world::registry::BlockStateId)> {
        let lod = self.lod.as_ref()?;
        let mut t = 0.0;
        while t <= reach {
            let p = [
                eye[0] + dir[0] * t,
                eye[1] + dir[1] * t,
                eye[2] + dir[2] * t,
            ];
            let (x, z) = (p[0].floor() as i32, p[2].floor() as i32);
            if !self.world.loaded(x, z)
                && let Some((height, state)) = lod.ground(x, z)
                && p[1] <= f64::from(height)
            {
                return Some(([x, height - 1, z], [p[0], f64::from(height), p[2]], state));
            }
            t += 1.0;
        }
        None
    }
    /// Right-clicks with these items (at most 64 ids) are mod `mod_id`'s; a later call replaces the list. API 116.
    pub(super) fn native_claim_items(&mut self, mod_id: &str, ids: &[String]) {
        self.native_input.claim(mod_id, ids);
    }
    /// Mod `mod_id`'s events since its last poll (at most 256 wait). API 116.
    pub(super) fn native_poll(&mut self, mod_id: &str) -> Vec<NativeModEvent> {
        self.native_input.poll(mod_id)
    }
    /// `use_hand`: a right-click with an item a mod claims starts its use for
    /// that mod and nothing else happens. True when a mod took it.
    pub(super) fn mod_use_start(&mut self) -> bool {
        if self.guest_use.is_some() || self.using.is_some() {
            return false;
        }
        // With no use, a mod use the game reset without telling it is over.
        self.native_input.end();
        let slot = self.held_slot();
        let Some(item) = self.mod_held(slot) else {
            return false;
        };
        let Some(id) = self.native_input.claimant(&item.id).map(str::to_owned) else {
            return false;
        };
        self.native_input.start(&id, item, slot);
        self.using = Some((slot, 0));
        true
    }
    /// `tick_used_item` for a claimed item: counts the ticks held and ends
    /// the use when the button comes up. True when the player's use is a
    /// mod's; a mod use the game reset ends, and the player's own use goes on.
    pub(super) fn mod_use_tick(&mut self, slot: usize, ticks: u32, released: bool) -> bool {
        if !self.native_input.owns(Some((slot, ticks))) {
            self.native_input.end();
            return false;
        }
        let still_claimed = self.native_input.using.as_ref().is_some_and(|u| {
            self.stack_in(slot)
                .is_some_and(|s| self.native_input.claimant(&s.item) == Some(u.mod_id.as_str()))
        });
        if released || !still_claimed {
            self.native_input.end();
            self.using = None;
        } else {
            self.native_input.tick();
            self.using = Some((slot, ticks + 1));
        }
        true
    }
    /// The player's use was reset (a respawn, a join, the hand's item
    /// changed): a mod use under way ends and its mod hears `UseStop`.
    pub(super) fn end_mod_use(&mut self) {
        self.native_input.end();
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    fn stick(slot: u32) -> NativeHeldItem {
        NativeHeldItem {
            id: "minecraft:stick".into(),
            count: 1,
            slot,
            custom_name: None,
            custom_data: Some("{orbital:1b}".into()),
        }
    }

    #[test]
    fn items_are_claimed_by_the_first_mod_in_id_order() {
        let mut input = ModInput::default();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        input.claim("zeta", &ids(&["orbital:remote", "minecraft:stick"]));
        input.claim("alpha", &ids(&["minecraft:stick"]));
        assert_eq!(input.claimant("minecraft:stick"), Some("alpha"));
        assert_eq!(input.claimant("orbital:remote"), Some("zeta"));
        assert_eq!(input.claimant("minecraft:dirt"), None);
        input.claim("alpha", &[]);
        assert_eq!(input.claimant("minecraft:stick"), Some("zeta"));
        let many: Vec<String> = (0..100).map(|i| format!("m:item{i}")).collect();
        input.claim("alpha", &many);
        assert_eq!(input.claimant("m:item63"), Some("alpha"));
        assert_eq!(input.claimant("m:item64"), None, "64 at most");
    }

    #[test]
    fn events_wait_for_their_mod_up_to_a_limit() {
        let mut input = ModInput::default();
        for _ in 0..EVENTS_MAX + 10 {
            input.push("a", NativeModEvent::UseStart(stick(0), 0));
        }
        input.push("b", NativeModEvent::UseStop(stick(0), 3));
        assert_eq!(input.poll("a").len(), EVENTS_MAX);
        assert!(input.poll("a").is_empty());
        assert_eq!(input.poll("b"), vec![NativeModEvent::UseStop(stick(0), 3)]);
    }

    #[test]
    fn a_claimed_use_is_the_mods_while_it_runs() {
        let mut input = ModInput::default();
        input.start("orbital", stick(2), 2);
        assert!(input.owns(Some((2, 0))));
        input.tick();
        input.tick();
        assert!(input.owns(Some((2, 2))));
        input.end();
        assert!(!input.owns(Some((2, 2))));
        assert_eq!(
            input.poll("orbital"),
            vec![
                NativeModEvent::UseStart(stick(2), 0),
                NativeModEvent::UseStop(stick(2), 2),
            ]
        );
    }

    #[test]
    fn an_interrupted_claimed_use_leaves_the_next_use_alone() {
        // A respawn ends the use: the mod hears it stop, and a vanilla use
        // begun next in the same slot is not taken for the mod's.
        let mut input = ModInput::default();
        input.start("orbital", stick(0), 0);
        input.tick();
        input.end();
        assert!(!input.owns(Some((0, 0))));
        assert_eq!(
            input.poll("orbital").last(),
            Some(&NativeModEvent::UseStop(stick(0), 1))
        );
        // A reset the mod use never heard of (`Game.using` cleared alone):
        // the player's next use, from 0 ticks or in another slot, is still
        // not the mod's, and ending it then tells the mod once.
        input.start("orbital", stick(0), 0);
        input.tick();
        input.tick();
        assert!(!input.owns(None));
        assert!(!input.owns(Some((0, 0))), "a vanilla use from 0 ticks");
        assert!(!input.owns(Some((1, 2))), "another slot");
        input.end();
        input.end();
        let events = input.poll("orbital");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1], NativeModEvent::UseStop(stick(0), 2));
    }
}
