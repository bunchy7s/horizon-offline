//! Projectile impacts for native mods (`native_projectile_impact`, NeoForge's
//! `ProjectileImpactEvent`). `tick_projectiles` flies every projectile first and
//! collects the impacts; once its worlds are let go, the mods hear each one, and a
//! projectile whose impact a mod canceled flies the tick again from where it was,
//! through whatever it hit.
use super::{
    Game,
    creatures::{ProjWorld, wind_charge_hit},
    dragon::{CRYSTAL_IDS, SPECIAL},
    lan_pvp::meets_players,
    native_hooks::mob_kind,
};
use horizon_client::{
    arrows::ArrowEvent,
    mobs::Who,
    projectiles::{Hit, Projectile},
};
use horizon_core::geometry::Vec3;
use std::ops::Range;

/// A projectile's impact in one tick of `tick_projectiles`.
pub(super) struct Impact {
    /// Where the projectile was in the tick's list.
    pub index: usize,
    /// The projectile before the tick.
    pub before: Projectile,
    pub hit: Hit,
    /// Where its hit is in the tick's `hits`.
    pub hit_slot: usize,
    /// Where its arrow events are in the tick's `arrow_events`.
    pub events: Range<usize>,
}

/// The type the mods know a projectile by: a spectral arrow is its own.
fn projectile_type(p: &Projectile) -> &'static str {
    let spectral = p
        .arrow
        .as_ref()
        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow");
    if spectral {
        "minecraft:spectral_arrow"
    } else {
        p.kind.id()
    }
}

impl Game<'_> {
    /// Whether this game is a server that flies projectiles for the mods (the
    /// server thread or a dedicated server, never the deprecated in-frame path
    /// nor a client) and a mod asked to hear their impacts
    /// (`native_hears_projectile_impacts`). Without one, the tick keeps no copy
    /// of its projectiles: it costs nothing.
    pub(super) fn native_hears_impacts(&self) -> bool {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return false;
        }
        let mut hears = false;
        self.native_hears_projectile_impacts(&mut hears);
        hears
    }

    /// The entity id of the one who shot `p`: a mob's, a player's, else 0.
    fn native_impact_owner(&self, p: &Projectile) -> u32 {
        match p.owner {
            Some(Who::Mob(id)) => id,
            Some(Who::Player) => self.native_thrower(p.guest),
            None => 0,
        }
    }

    /// The entity id of the player who threw something: the guest `guest`, or
    /// the host.
    fn native_thrower(&self, guest: Option<u32>) -> u32 {
        match guest {
            Some(conn) => self
                .lan_guest_entities()
                .into_iter()
                .find(|&(c, _)| c == conn)
                .map_or(0, |(_, id)| id),
            None => self.lan.as_ref().map_or(0, |host| host.host_entity as u32),
        }
    }

    /// `native_projectile_impact` for a player's thrown trident, which
    /// `tick_tridents` flies rather than `tick_projectiles`: it hit the mob
    /// `mob`, or with `mob` None the block at `block` on its `face`, at `at`.
    /// Whether a mod canceled it.
    pub(super) fn native_trident_impact(
        &mut self,
        trident: u32,
        guest: Option<u32>,
        mob: Option<u32>,
        block: [i32; 3],
        face: [i32; 3],
        at: Vec3,
    ) -> bool {
        if !self.native_hears_impacts() {
            return false;
        }
        let owner = self.native_thrower(guest);
        let (entity, entity_kind) = match mob {
            Some(id) => {
                let (entity, kind) = self.native_impact_entity(id);
                (Some(entity), kind)
            }
            None => (None, String::new()),
        };
        let dimension = self.dim.id();
        let mut canceled = false;
        self.native_projectile_impact(
            dimension,
            trident,
            "minecraft:trident",
            owner,
            entity,
            &entity_kind,
            block,
            face,
            [at.x, at.y, at.z],
            &mut canceled,
        );
        canceled
    }

    /// The entity id and type a projectile's hit on the reserved or ordinary
    /// `id` stands for: a dragon part is the dragon, an End crystal its own id.
    fn native_impact_entity(&self, id: u32) -> (u32, String) {
        if id >= SPECIAL {
            return if id < CRYSTAL_IDS {
                let dragon = self.dragon.as_ref().map_or(0, |d| d.id);
                (dragon, "minecraft:ender_dragon".into())
            } else {
                let crystal = self.crystals.get((id - CRYSTAL_IDS) as usize);
                (crystal.map_or(0, |c| c.id), "minecraft:end_crystal".into())
            };
        }
        if let Some(mob) = self.mobs.iter().find(|m| m.id == id) {
            return (id, mob_kind(mob));
        }
        if let Some(cart) = self.carts.iter().find(|c| c.id == id) {
            return (id, cart.kind.id().into());
        }
        if self.lan_guest_entities().iter().any(|&(_, e)| e == id) {
            return (id, "minecraft:player".into());
        }
        (id, String::new())
    }

    /// `native_projectile_impact` for each of `impacts`: whether the mods
    /// canceled it. A hit on a wind charge is not told.
    fn native_projectile_impacts(&mut self, impacts: &[Impact]) -> Vec<bool> {
        let dimension = self.dim.id();
        let mut canceled = Vec::with_capacity(impacts.len());
        for impact in impacts {
            let p = &impact.before;
            let (entity, entity_kind, block, face, at) = match impact.hit {
                Hit::Block { pos, at, face } => (None, String::new(), pos, face, at),
                Hit::Entity {
                    who: Who::Mob(id),
                    at,
                } => {
                    if wind_charge_hit(id).is_some() {
                        canceled.push(false);
                        continue;
                    }
                    let (entity, kind) = self.native_impact_entity(id);
                    (Some(entity), kind, [0; 3], [0; 3], at)
                }
                Hit::Entity {
                    who: Who::Player,
                    at,
                } => {
                    let Some(host) = self.lan.as_ref().map(|h| h.host_entity as u32) else {
                        canceled.push(false);
                        continue;
                    };
                    (Some(host), "minecraft:player".into(), [0; 3], [0; 3], at)
                }
                Hit::None => {
                    canceled.push(false);
                    continue;
                }
            };
            let owner = self.native_impact_owner(p);
            let mut stop = false;
            self.native_projectile_impact(
                dimension,
                p.id,
                projectile_type(p),
                owner,
                entity,
                &entity_kind,
                block,
                face,
                [at.x, at.y, at.z],
                &mut stop,
            );
            canceled.push(stop);
        }
        canceled
    }

    /// The end of a tick of `tick_projectiles` that met `impacts`, with
    /// `projectiles` in place (those it keeps, as `listed` says of the tick's
    /// list): the mods hear each, and a projectile one canceled flies the tick
    /// again from before it, through what it hit. Its hit and its arrow events are
    /// taken out of `hits` and `arrow_events`, and it goes back among the
    /// projectiles. One a mod removed in its hook stays removed.
    pub(super) fn native_settle_impacts(
        &mut self,
        projectiles: &mut Vec<Projectile>,
        listed: &[bool],
        impacts: Vec<Impact>,
        hits: &mut Vec<(Projectile, Hit)>,
        arrow_events: &mut Vec<(Vec3, ArrowEvent)>,
    ) {
        // The mods see the projectiles where they will be, as `projectile_hit` does.
        self.creatures.projectiles = std::mem::take(projectiles);
        let canceled = self.native_projectile_impacts(&impacts);
        *projectiles = std::mem::take(&mut self.creatures.projectiles);
        if !canceled.contains(&true) {
            return;
        }
        let mut drop_hits = vec![false; hits.len()];
        let mut drop_events = vec![false; arrow_events.len()];
        let mut flown = Vec::new();
        for (impact, _) in impacts.into_iter().zip(canceled).filter(|(_, c)| *c) {
            drop_hits[impact.hit_slot] = true;
            drop_events[impact.events.clone()].fill(true);
            let present = projectiles.iter().position(|q| q.id == impact.before.id);
            if present.is_none() && listed[impact.index] {
                continue;
            }
            let (p, keep, events) = self.native_projectile_through(impact.before);
            flown.extend(events);
            match (present, keep) {
                (Some(i), true) => projectiles[i] = p,
                (Some(i), false) => {
                    projectiles.remove(i);
                }
                (None, true) => projectiles.push(p),
                (None, false) => {}
            }
        }
        let mut n = 0;
        hits.retain(|_| {
            n += 1;
            !drop_hits[n - 1]
        });
        let mut n = 0;
        arrow_events.retain(|_| {
            n += 1;
            !drop_events[n - 1]
        });
        arrow_events.extend(flown);
    }

    /// `p`, as it was before its tick, flown that tick through what it hit
    /// (`Projectile::tick_through`, `arrows::tick_through`): the projectile after
    /// it, whether it stays, and the arrow events it made.
    fn native_projectile_through(
        &self,
        mut p: Projectile,
    ) -> (Projectile, bool, Vec<(Vec3, ArrowEvent)>) {
        let mob_shot = !matches!(p.owner, Some(Who::Player));
        let pvp = !mob_shot && meets_players(&p.kind);
        // A guest's shot flies from that guest, as in `tick_projectiles`.
        let shooter = p
            .guest
            .filter(|_| pvp)
            .and_then(|c| self.lan_guest_entities().into_iter().find(|&(g, _)| g == c))
            .map(|(_, id)| Who::Mob(id));
        if shooter.is_some() {
            p.owner = shooter;
        }
        let mut targets = self.projectile_targets();
        if p.arrow.is_some() {
            targets.extend(self.carts.iter().map(|c| (Who::Mob(c.id), c.bounds())));
        }
        if mob_shot || pvp {
            targets.extend(self.lan_projectile_targets());
        }
        let rain = self.arrow_rain();
        let scene = self.arrow_scene(&rain);
        let (keep, _) = if p.arrow.is_some() {
            horizon_client::arrows::tick_through(&mut p, &scene, &targets)
        } else {
            let w = self.mob_world_base(&[]);
            p.tick_through(&ProjWorld { w: &w, targets })
        };
        if shooter.is_some() {
            p.owner = Some(Who::Player);
        }
        let at = p.position;
        let events = p
            .arrow
            .as_mut()
            .map(|a| a.events.drain(..).map(|e| (at, e)).collect())
            .unwrap_or_default();
        (p, keep, events)
    }
}
