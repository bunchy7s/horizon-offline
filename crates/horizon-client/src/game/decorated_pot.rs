//! Decorated pots, which Horizon keeps as loaded (the chunk's saved block
//! entities): the loot table a trial chamber pot holds, rolled into its item
//! the first time the item is read (`RandomizableContainer.unpackLootTable`),
//! and `DecoratedPotBlock.useItemOn` / `useWithoutItem`.
use super::{Game, spill};
use horizon_client::{
    first_person::Hand,
    items::ItemStack,
    mobs::Who,
    particles::{ParticleOptions, ParticleType},
    random_sequences::RandomSequences,
    save,
};
use horizon_core::random::LegacyRandom;
use horizon_world::{loot::LootData, nbt::Tag};

/// `DecoratedPotBlockEntity.WobbleStyle` ordinals.
const WOBBLE_POSITIVE: i32 = 0;
const WOBBLE_NEGATIVE: i32 = 1;

/// `RandomizableContainer.unpackLootTable(null)` on a pot's saved tag: the
/// loot table, if any, is removed and `LootTable.fill` puts its roll in
/// `item`. The roll draws from `LootTableSeed`, else the table's random
/// sequence, else the level random (`LootContext.Builder.create`).
pub(super) fn unpack_loot(
    tag: &mut Tag,
    loot: Option<&LootData>,
    random: &mut LegacyRandom,
    sequences: &mut RandomSequences,
    world_seed: i64,
) {
    let Tag::Compound(map) = tag else {
        return;
    };
    let Some(Tag::String(table)) = map.remove(&"LootTable".into()) else {
        return;
    };
    let seed = match map.remove(&"LootTableSeed".into()) {
        Some(Tag::Long(seed)) => seed,
        _ => 0,
    };
    map.remove(&"item".into());
    let Some(loot) = loot else {
        return;
    };
    let table = table.to_string_lossy();
    // No origin biome: the pot tables have no location checks.
    let slots = if seed != 0 {
        loot.fill(&table, seed, 1, None)
    } else if let Some(sequence) = loot.random_sequence(&table) {
        loot.fill_with(&table, sequences.get(sequence, world_seed), 1, None)
    } else {
        loot.fill_with(&table, random, 1, None)
    };
    if let Some(stack) = slots.into_iter().next().flatten() {
        let item = horizon_client::structure_blocks::loot_item_with_components(&stack);
        map.insert("item".into(), save::item_to_nbt(&item));
    }
}

impl Game<'_> {
    /// The pot's saved tag at `pos` with its loot unpacked (a new one for a
    /// pot without), or None when no pot is there.
    fn pot_tag(&mut self, pos: [i32; 3]) -> Option<Tag> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:decorated_pot" {
            return None;
        }
        let mut tag = spill::saved_entity(&self.world, pos).unwrap_or_else(|| {
            Tag::compound([
                ("id", Tag::String("minecraft:decorated_pot".into())),
                ("x", Tag::Int(pos[0])),
                ("y", Tag::Int(pos[1])),
                ("z", Tag::Int(pos[2])),
            ])
        });
        let loot = horizon_client::structure_blocks::loot_data(
            &self.options.resources,
            &self.pack_files(),
        );
        unpack_loot(
            &mut tag,
            loot.as_deref(),
            &mut self.world.random,
            &mut self.rand_sequences,
            self.level.seed,
        );
        Some(tag)
    }
    /// `DecoratedPotBlock.useItemOn` with `held`, then `useWithoutItem` on the
    /// main hand's turn: true when the pot took the click.
    pub(super) fn use_decorated_pot(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let Some(mut tag) = self.pot_tag(pos) else {
            return false;
        };
        let mut pot_item = tag.get("item").and_then(save::item_from_nbt);
        let insertable = held.filter(|stack| match &pot_item {
            None => true,
            Some(p) => p.same_item_same_components(stack) && p.count < self.max_stack(p),
        });
        let center = pos.map(|v| f64::from(v) + 0.5);
        let Some(stack) = insertable else {
            spill::put_saved_entity(&mut self.world, pos, tag);
            // TRY_WITH_EMPTY_HAND: only the main hand goes on to useWithoutItem.
            if self.hand != Hand::Main {
                return false;
            }
            self.audio.play(
                "minecraft:block.decorated_pot.insert_fail",
                Some(center),
                1.0,
                1.0,
            );
            self.block_entity_event(pos, 1, WOBBLE_NEGATIVE);
            self.post_block_game_event("minecraft:block_change", pos, Some(Who::Player), None);
            // ServerPlayerGameMode.useItemOn: useWithoutItem consumed the click.
            self.trigger_default_block_use(pos);
            return true;
        };
        let stack = stack.clone();
        self.block_entity_event(pos, 1, WOBBLE_POSITIVE);
        self.stats.add("used", &stack.item, 1);
        // consumeAndReturn(1): one item goes in (a creative player keeps theirs).
        let inserted = match &mut pot_item {
            Some(p) => {
                p.count += 1;
                p
            }
            None => pot_item.insert(ItemStack {
                count: 1,
                ..stack.clone()
            }),
        };
        let pitch_bend = inserted.count as f32 / self.max_stack(inserted) as f32;
        if let (Tag::Compound(map), Some(item)) = (&mut tag, &pot_item) {
            map.insert("item".into(), save::item_to_nbt(item));
        }
        spill::put_saved_entity(&mut self.world, pos, tag);
        // The sherds and item go to the guests (BlockEntityData).
        self.mark_block_entity_updated((pos[0], pos[1], pos[2]));
        self.consume_held();
        self.audio.play(
            "minecraft:block.decorated_pot.insert",
            Some(center),
            1.0,
            0.7 + 0.5 * pitch_bend,
        );
        self.particles.send_particles(
            &ParticleOptions::simple(ParticleType::DustPlume),
            [center[0], f64::from(pos[1]) + 1.2, center[2]],
            [0.0; 3],
            0.0,
            7,
        );
        self.post_block_game_event("minecraft:block_change", pos, Some(Who::Player), None);
        self.trigger_block("minecraft:item_used_on_block", pos, &stack);
        true
    }
}
