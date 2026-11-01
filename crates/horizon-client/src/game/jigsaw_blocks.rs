//! Jigsaw blocks on the server: `handleSetJigsawBlock` and
//! `handleJigsawGenerate` (`JigsawBlockEntity.generate`), and the joint a
//! placed jigsaw starts with.
use super::{Game, lan::LanHost};
use horizon_client::{
    containers::BlockEntity,
    jigsaw_block::{self, JigsawBlock, JointType},
};
use horizon_protocol::packets::play;
use horizon_world::{nbt::Tag, worldgen::generator::JigsawQuery};

type Pos = (i32, i32, i32);

impl Game<'_> {
    fn jigsaw_block_mut(&mut self, pos: Pos) -> Option<&mut JigsawBlock> {
        match self.world.block_entities.get_mut(&pos)? {
            BlockEntity::JigsawBlock(b) => Some(b),
            _ => None,
        }
    }

    /// The block's front facing, from its `orientation` (`JigsawBlock.getFrontFacing`).
    pub(super) fn jigsaw_front(&self, pos: Pos) -> Option<String> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let orientation = self
            .pack
            .assets
            .registry
            .state(state)?
            .properties
            .get("orientation")?;
        Some(jigsaw_block::front_of_orientation(orientation).to_owned())
    }

    /// `new JigsawBlockEntity(pos, state)`: the joint a block starts with
    /// follows its facing (`StructureTemplate.getDefaultJointType`).
    pub(super) fn jigsaw_block_placed(&mut self, pos: Pos) {
        let Some(front) = self.jigsaw_front(pos) else {
            return;
        };
        if let Some(b) = self.jigsaw_block_mut(pos) {
            b.joint = JointType::default_for_front(&front);
        }
    }

    /// `handleSetJigsawBlock` for a guest: only a creative player with game
    /// master commands edits the block.
    pub(super) fn lan_set_jigsaw_block(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::SetJigsawBlock,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest(host, conn, |game| {
            if allowed {
                game.apply_set_jigsaw_block(&p);
            }
        });
    }

    fn apply_set_jigsaw_block(&mut self, p: &play::SetJigsawBlock) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        let Some(b) = self.jigsaw_block_mut(pos) else {
            return;
        };
        b.name = p.name.clone();
        b.target = p.target.clone();
        b.pool = p.pool.clone();
        b.final_state = p.final_state.clone();
        // JointType.byName(name, ALIGNED).
        b.joint = JointType::of_serialized_name(&p.joint).unwrap_or(JointType::Aligned);
        b.placement_priority = p.placement_priority;
        b.selection_priority = p.selection_priority;
        self.mark_block_entity_updated(pos);
    }

    /// `handleJigsawGenerate` for a guest.
    pub(super) fn lan_jigsaw_generate(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::JigsawGenerate,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest(host, conn, |game| {
            if allowed {
                game.generate_jigsaw_block(&p);
            }
        });
    }

    /// `JigsawBlockEntity.generate`: the pool is built from the block's
    /// front neighbour, `levels` deep.
    fn generate_jigsaw_block(&mut self, p: &play::JigsawGenerate) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        let Some(BlockEntity::JigsawBlock(b)) = self.world.block_entities.get(&pos) else {
            return;
        };
        let (pool, target) = (b.pool.clone(), b.target.clone());
        let Some(front) = self.jigsaw_front(pos) else {
            return;
        };
        let step = super::facing_offset(&front);
        let position = [pos.0 + step[0], pos.1 + step[1], pos.2 + step[2]];
        // registry.getOrThrow(pool): an unknown pool throws, ending the packet.
        if !self
            .ui
            .registries
            .ids("minecraft:worldgen/template_pool")
            .contains(&pool)
        {
            return;
        }
        let Some(terrain) = self.end.locator() else {
            return;
        };
        let query = JigsawQuery {
            pool: &pool,
            target: &target,
            max_depth: p.levels,
            position,
        };
        let keep = p.keep_jigsaws;
        let (_, entities) = self.with_level(|b, level, ticks| {
            b.place_jigsaw(level, ticks, position, keep, || {
                terrain.jigsaw_pieces(&query)
            })
        });
        for entity in entities {
            self.spawn_entity_from_nbt(Tag::Compound(entity));
        }
        let _ = self.process_level_events();
    }
}
