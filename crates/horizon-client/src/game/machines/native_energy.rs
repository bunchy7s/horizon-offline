//! Energy in a block's buffer for native server mods (docs/native-mods.md):
//! the buffer a mod block declared through `native_block_entity_energy`, read
//! through a side, filled, drained and moved to another block. Vanilla has no
//! energy; the contract is NeoForge's `EnergyHandler` (`SimpleEnergyHandler`
//! and `EnergyHandlerUtil.move`). Like the fluid helpers, they act where the
//! world is simulated, never on a client's copy, and only on the level in
//! place, which the readers name by their `dimension`
//! (`minecraft:overworld`), as `native_set_block` does.
use super::Game;
use horizon_client::containers::ModEnergyBuffer;
use horizon_core::geometry::Direction;
use horizon_world::nbt::{Compound, JavaString, Tag};
use std::sync::Arc;

/// A block's energy as a side reaches it (`native_energy`).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeEnergy {
    pub(crate) amount: i32,
    pub(crate) capacity: i32,
    /// The most one insert moves.
    pub(crate) max_insert: i32,
    /// The most one extract moves.
    pub(crate) max_extract: i32,
    /// Whether a fill may come through this side (and moves more than 0).
    pub(crate) can_insert: bool,
    /// Whether a drain may leave by this side (and moves more than 0).
    pub(crate) can_extract: bool,
}

/// The amount in `nbt`: the int `energy`, as `SimpleEnergyHandler` saves it;
/// anything else, or a negative, reads as empty.
fn read_energy(nbt: &Compound) -> i32 {
    match nbt.get(&JavaString::from("energy")) {
        Some(Tag::Int(energy)) => (*energy).max(0),
        _ => 0,
    }
}

/// `SimpleEnergyHandler.insert`: the room left (none when the amount is
/// over the capacity), the call's limit and what was offered, whichever is
/// least.
fn insert_amount(buffer: &ModEnergyBuffer, energy: i32, offered: i32) -> i32 {
    (buffer.capacity.saturating_sub(energy))
        .min(offered)
        .min(buffer.max_insert)
        .max(0)
}

/// `SimpleEnergyHandler.extract`: what is stored, the call's limit and what
/// was asked, whichever is least.
fn extract_amount(buffer: &ModEnergyBuffer, energy: i32, asked: i32) -> i32 {
    energy.min(asked).min(buffer.max_extract).max(0)
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The buffer of the mod block at `pos` in the level in place,
    /// `dimension`, and the energy it holds (its block entity's, and not the
    /// placeholder standing in while it ticks).
    fn native_energy_at(
        &self,
        dimension: &str,
        pos: [i32; 3],
    ) -> Option<(Arc<ModEnergyBuffer>, i32)> {
        if !self.native_simulates() || dimension != self.dim.id() {
            return None;
        }
        let key = (pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let buffer = self.world.block_entities.mod_energy(state.0)?;
        // The block entity is out of the world while it ticks.
        match self.world.block_entities.get(&key) {
            Some(horizon_client::containers::BlockEntity::Mod(m)) if !m.id.is_empty() => {
                Some((Arc::clone(buffer), read_energy(&m.nbt)))
            }
            _ => None,
        }
    }
    /// Writes `energy` into the block entity at `pos`, saved with its chunk.
    fn native_energy_store(&mut self, pos: [i32; 3], energy: i32) {
        let key = (pos[0], pos[1], pos[2]);
        if let Some(horizon_client::containers::BlockEntity::Mod(m)) =
            self.world.block_entities.get_mut(&key)
        {
            m.nbt
                .insert(JavaString::from("energy"), Tag::Int(energy.max(0)));
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
    }
    /// What the buffer at `pos` takes (`insert`) or gives through `face` of
    /// up to `amount`, written unless `simulate`. Nothing where the side
    /// doesn't reach the buffer for that.
    fn native_energy_transfer(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        face: Direction,
        amount: i32,
        insert: bool,
        simulate: bool,
    ) -> i32 {
        if amount <= 0 {
            return 0;
        }
        let Some((buffer, energy)) = self.native_energy_at(dimension, pos) else {
            return 0;
        };
        let moved = if insert {
            if !buffer.insert_faces[face as usize] {
                return 0;
            }
            insert_amount(&buffer, energy, amount)
        } else {
            if !buffer.extract_faces[face as usize] {
                return 0;
            }
            extract_amount(&buffer, energy, amount)
        };
        if moved > 0 && !simulate {
            let after = if insert {
                energy.saturating_add(moved)
            } else {
                energy - moved
            };
            self.native_energy_store(pos, after);
        }
        moved
    }
    /// The energy buffer of the block at `pos` in the level in place,
    /// `dimension`, as its side `face` reaches it: what it holds, its
    /// capacity and its limits per call. None where no buffer is: another
    /// dimension, a chunk that isn't loaded, a block without one, a side
    /// that reaches neither a fill nor a drain, or the mod block entity
    /// ticking now (it's out of the world). `Capabilities.Energy.BLOCK`.
    pub(in crate::game) fn native_energy(
        &self,
        dimension: &str,
        pos: [i32; 3],
        face: Direction,
    ) -> Option<NativeEnergy> {
        let (buffer, amount) = self.native_energy_at(dimension, pos)?;
        let (fill, drain) = (
            buffer.insert_faces[face as usize],
            buffer.extract_faces[face as usize],
        );
        (fill || drain).then(|| NativeEnergy {
            amount,
            capacity: buffer.capacity,
            max_insert: buffer.max_insert,
            max_extract: buffer.max_extract,
            can_insert: fill && buffer.max_insert > 0,
            can_extract: drain && buffer.max_extract > 0,
        })
    }
    /// `EnergyHandler.insert`: up to `amount` into the buffer of the block at
    /// `pos` through its side `face`, in the level in place: no more than
    /// its room and its limit per call. With `simulate` nothing changes and
    /// the answer says what would. The amount that went in.
    pub(in crate::game) fn native_energy_insert(
        &mut self,
        pos: [i32; 3],
        face: Direction,
        amount: i32,
        simulate: bool,
    ) -> i32 {
        let dimension = self.dim.id();
        self.native_energy_transfer(dimension, pos, face, amount, true, simulate)
    }
    /// `EnergyHandler.extract`: up to `amount` out of the buffer of the block
    /// at `pos` in the level in place, `dimension`, through its side `face`:
    /// no more than it holds and its limit per call. With `simulate` nothing
    /// changes and the answer says what would. The amount that came out.
    pub(in crate::game) fn native_energy_extract(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        face: Direction,
        amount: i32,
        simulate: bool,
    ) -> i32 {
        self.native_energy_transfer(dimension, pos, face, amount, false, simulate)
    }
    /// `EnergyHandlerUtil.move`: up to `max` from the block at `from`,
    /// through its side `from_face`, into the one at `to`, through
    /// `to_face`, in the level in place, `dimension`. It asks what the
    /// source can give, offers that to the target, and takes from the source
    /// exactly what the target accepted, so none is lost. The amount moved;
    /// 0 where either has no buffer for it, and within one block.
    pub(in crate::game) fn native_move_energy(
        &mut self,
        dimension: &str,
        from: [i32; 3],
        from_face: Direction,
        to: [i32; 3],
        to_face: Direction,
        max: i32,
    ) -> i32 {
        if from == to || max <= 0 {
            return 0;
        }
        let offered = self.native_energy_transfer(dimension, from, from_face, max, false, true);
        let accepted = self.native_energy_transfer(dimension, to, to_face, offered, true, true);
        if accepted == 0 {
            return 0;
        }
        let taken = self.native_energy_transfer(dimension, from, from_face, accepted, false, false);
        let put = self.native_energy_transfer(dimension, to, to_face, taken, true, false);
        if put < taken {
            // Both were just simulated and nothing ran between; a buffer
            // that answers differently is a bug, and its energy goes back.
            log::warn!("Horizon: an energy move lost {} energy", taken - put);
            let _ =
                self.native_energy_transfer(dimension, from, from_face, taken - put, true, false);
        }
        put
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(capacity: i32, max_insert: i32, max_extract: i32) -> ModEnergyBuffer {
        ModEnergyBuffer {
            capacity,
            max_insert,
            max_extract,
            insert_faces: [true; 6],
            extract_faces: [true; 6],
        }
    }

    #[test]
    fn an_insert_stops_at_the_room_the_limit_and_the_offer() {
        let b = buffer(1000, 100, 50);
        assert_eq!(insert_amount(&b, 0, 40), 40);
        assert_eq!(insert_amount(&b, 0, 500), 100);
        assert_eq!(insert_amount(&b, 950, 500), 50);
        assert_eq!(insert_amount(&b, 1000, 500), 0);
        // Over the capacity after a mod update shrank it: nothing goes in.
        assert_eq!(insert_amount(&b, 1500, 500), 0);
        assert_eq!(insert_amount(&b, 0, i32::MAX), 100);
        assert_eq!(
            insert_amount(&buffer(i32::MAX, i32::MAX, 0), 5, i32::MAX),
            i32::MAX - 5
        );
    }

    #[test]
    fn an_extract_stops_at_what_is_stored_the_limit_and_the_ask() {
        let b = buffer(1000, 100, 50);
        assert_eq!(extract_amount(&b, 800, 30), 30);
        assert_eq!(extract_amount(&b, 800, 500), 50);
        assert_eq!(extract_amount(&b, 20, 500), 20);
        assert_eq!(extract_amount(&b, 0, 500), 0);
        assert_eq!(extract_amount(&buffer(1000, 100, 0), 800, 500), 0);
    }

    #[test]
    fn the_amount_reads_as_the_saved_int_and_anything_else_as_empty() {
        let mut nbt = Compound::new();
        assert_eq!(read_energy(&nbt), 0);
        nbt.insert(JavaString::from("energy"), Tag::Int(750));
        assert_eq!(read_energy(&nbt), 750);
        nbt.insert(JavaString::from("energy"), Tag::Int(-5));
        assert_eq!(read_energy(&nbt), 0);
        nbt.insert(
            JavaString::from("energy"),
            Tag::String(JavaString::from("lots")),
        );
        assert_eq!(read_energy(&nbt), 0);
    }
}
