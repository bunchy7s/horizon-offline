//! Synchronous mod-block contacts from the server's simulation code.
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug)]
pub struct Contact<'a> {
    pub entity: i32,
    pub kind: &'a str,
    pub block: &'a str,
    pub state: BlockStateId,
    pub position: BlockPos,
}

#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    StepOn(Contact<'a>),
    Inside(Contact<'a>),
    FallOn(Contact<'a>, f64),
}

pub static HOOK: OnceLock<fn(&str, Event<'_>, &mut f32)> = OnceLock::new();
/// Whether the loaded blocks hold one outside `minecraft:` (a content mod's).
/// Without one no contact can dispatch, so the item and projectile sweeps
/// skip their per-tick work: every item and thrown thing on a server would
/// otherwise sweep its blocks each tick for nothing.
static MOD_BLOCKS: AtomicBool = AtomicBool::new(false);

/// Called when the block tables load.
pub fn set_mod_blocks(any: bool) {
    MOD_BLOCKS.store(any, Ordering::Relaxed);
}
thread_local! {
    static LEVEL: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Scope authority to this thread and restore it even on an error or panic.
/// Nested guest actions may temporarily enter a different dimension.
pub fn in_level<R>(dimension: Option<&'static str>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static str>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LEVEL.set(self.0);
        }
    }
    let _restore = Restore(LEVEL.replace(dimension));
    f()
}

pub fn emit(event: Event<'_>) {
    dispatch(event, &mut 0.0);
}

/// Damage is the calculated fall damage, before hurt handling and fall sounds.
pub fn fall(contact: Contact<'_>, distance: f64, mut damage: f32) -> f32 {
    dispatch(Event::FallOn(contact, distance), &mut damage);
    damage
}

fn dispatch(event: Event<'_>, damage: &mut f32) {
    let contact = match event {
        Event::StepOn(c) | Event::Inside(c) | Event::FallOn(c, _) => c,
    };
    if contact.block.starts_with("minecraft:") {
        return;
    }
    if let Some(dimension) = LEVEL.get()
        && let Some(hook) = HOOK.get()
    {
        let original = *damage;
        hook(dimension, event, damage);
        if !damage.is_finite() {
            *damage = original;
        }
        *damage = damage.max(0.0);
    }
}

/// Reuse the entity block sweep for item and projectile ticks without an effect collector.
pub fn swept_inside<'a>(
    entity: i32,
    kind: &str,
    from: horizon_core::geometry::Vec3,
    to: horizon_core::geometry::Vec3,
    bounds: horizon_core::geometry::Aabb,
    mut block: impl FnMut(BlockPos) -> (BlockStateId, &'a str),
) {
    if !MOD_BLOCKS.load(Ordering::Relaxed) || LEVEL.get().is_none() || HOOK.get().is_none() {
        return;
    }
    let bounds = bounds.deflate(horizon_core::geometry::Vec3::new(1e-5, 1e-5, 1e-5));
    let mut visited = std::collections::HashSet::new();
    let _ =
        horizon_core::geometry::sweep_intersected_blocks(from, to, bounds, 1_000_000, |p, step| {
            if step >= 16 {
                return false;
            }
            if visited.insert(p) {
                let (state, name) = block(p);
                emit(Event::Inside(Contact {
                    entity,
                    kind,
                    block: name,
                    state,
                    position: p,
                }));
            }
            true
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    thread_local! {
        static SEEN: Cell<usize> = const { Cell::new(0) };
    }
    fn hook(dimension: &str, event: Event<'_>, damage: &mut f32) {
        assert_eq!(dimension, "minecraft:the_nether");
        let c = match event {
            Event::FallOn(c, distance) => {
                assert_eq!(distance, 12.0);
                *damage = match *damage {
                    99.0 => f32::NAN,
                    98.0 => -1.0,
                    _ => *damage * 0.2,
                };
                c
            }
            Event::Inside(c) | Event::StepOn(c) => c,
        };
        assert_eq!(
            (c.entity, c.kind, c.state, c.position),
            (
                17,
                "minecraft:zombie",
                BlockStateId(9),
                BlockPos::new(1, 2, 3)
            )
        );
        SEEN.set(SEEN.get() + 1);
    }
    #[test]
    fn contacts_require_server_scope_and_mod_blocks_and_restore_nested_levels() {
        HOOK.set(hook).unwrap();
        let c = Contact {
            entity: 17,
            kind: "minecraft:zombie",
            block: "test:hay",
            state: BlockStateId(9),
            position: BlockPos::new(1, 2, 3),
        };
        assert_eq!(fall(c, 12.0, 10.0), 10.0);
        in_level(Some("minecraft:the_nether"), || {
            emit(Event::StepOn(c));
            emit(Event::Inside(c));
            assert_eq!(fall(c, 12.0, 10.0), 2.0);
            in_level(None, || assert_eq!(fall(c, 12.0, 10.0), 10.0));
            assert_eq!(fall(c, 12.0, 0.0), 0.0);
            emit(Event::Inside(Contact {
                block: "minecraft:hay_block",
                ..c
            }));
        });
        assert_eq!(fall(c, 12.0, 10.0), 10.0);
        assert_eq!(SEEN.get(), 4);
        in_level(Some("minecraft:the_nether"), || {
            assert_eq!(fall(c, 12.0, 99.0), 99.0);
            assert_eq!(fall(c, 12.0, 98.0), 0.0);
            use horizon_core::geometry::{Aabb, Vec3};
            let to = Vec3::new(2.5, 2.0, 3.5);
            let bounds = Aabb::new(Vec3::new(2.25, 2.0, 3.25), Vec3::new(2.75, 2.5, 3.75));
            let block = |p| {
                (
                    BlockStateId(9),
                    if p == c.position {
                        "test:hay"
                    } else {
                        "minecraft:air"
                    },
                )
            };
            set_mod_blocks(true);
            swept_inside(17, c.kind, Vec3::new(0.5, 2.0, 3.5), to, bounds, block);
            assert_eq!(SEEN.get(), 7, "a block crossed mid-move is notified once");
            let at = Vec3::new(1.5, 2.0, 3.5);
            let bounds = bounds.move_by(Vec3::new(-1.0, 0.0, 0.0));
            swept_inside(17, c.kind, at, at, bounds, block);
            assert_eq!(SEEN.get(), 8, "stationary overlap still notifies");
        });
        let _ =
            std::panic::catch_unwind(|| in_level(Some("minecraft:the_nether"), || panic!("test")));
        assert_eq!(fall(c, 12.0, 10.0), 10.0);
    }
}
