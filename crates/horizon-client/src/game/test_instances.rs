//! Test instance blocks on the server: `handleTestInstanceBlockAction` and
//! the `TestInstanceBlockEntity` operations behind it (reset, save, export,
//! the structure's placement), over the game tests the registry holds.
use super::{
    Game,
    game_tests::Info,
    lan::LanHost,
    structure_blocks::{create_seed, template_size},
    test_entities::{aabb_of, inflate},
};
use horizon_client::{
    block_entity_render::vertex_light,
    component::name_text,
    containers::BlockEntity,
    hud::HudBatch,
    mesher::Vertex,
    structure_block::rotation_from_ordinal,
    test_instance_block::{Data, Status, TestInstanceBlock},
    text::{self, Text},
};
use horizon_core::{position::BlockPos, ticks::TickArea};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::{
    level::{TemplateSettings, flags},
    worldgen::structure::{BoundingBox, Mirror, Rotation},
};

type Pos = (i32, i32, i32);

/// `GameTestBlockHighlightRenderer.highlightPos`'s colour, 1610678016: ARGB 0x6000FF00.
const HIGHLIGHT_FILL: u32 = 0x6000_FF00;

/// `ServerboundTestInstanceBlockActionPacket.Action`.
const INIT: i32 = 0;
const QUERY: i32 = 1;
const RESET: i32 = 3;
const SAVE: i32 = 4;
const EXPORT: i32 = 5;
const RUN: i32 = 6;

/// The `GameTestInstance` a registry entry describes: `TestData` and what
/// `describe()` prints.
#[derive(Clone)]
pub(super) struct TestDef {
    pub(super) structure: String,
    pub(super) rotation: Rotation,
    pub(super) padding: i32,
    pub(super) required: bool,
    pub(super) sky_access: bool,
    /// `BlockBasedTestInstance` (else a `FunctionGameTestInstance`).
    pub(super) block_based: bool,
    /// The function a function test runs.
    pub(super) function: Option<String>,
    /// The test environment's id (`batch()`).
    pub(super) environment: String,
    pub(super) max_ticks: i32,
    pub(super) setup_ticks: i32,
    /// `TestData.maxAttempts` and `requiredSuccesses`.
    pub(super) max_attempts: i32,
    pub(super) required_successes: i32,
    /// The dimension the test runs in (`TestData.dimension`).
    pub(super) dimension: String,
    description: Text,
}

#[cfg(test)]
impl TestDef {
    /// A test with the timing and type a runner test needs.
    pub(super) fn sample(block_based: bool, max_ticks: i32, setup_ticks: i32) -> Self {
        Self {
            structure: "minecraft:empty".into(),
            rotation: Rotation::None,
            padding: 0,
            required: true,
            sky_access: false,
            block_based,
            function: None,
            environment: "minecraft:default".into(),
            dimension: "minecraft:overworld".into(),
            max_ticks,
            setup_ticks,
            max_attempts: 1,
            required_successes: 1,
            description: Text::literal(""),
        }
    }
}

/// `GameTestInstance.descriptionRow`: the row's value in blue, then a line break.
fn row(key: &str, value: &str) -> Text {
    Text::translate(key, &[Text::literal(value).with_color(0x5555FF)]).append(Text::literal("\n"))
}

fn red(key: &str, args: &[Text]) -> Text {
    Text::translate(key, args).with_color(text::RED)
}

impl Game<'_> {
    /// `registryAccess().get(testKey)`: the test instance from the game's
    /// data (`data/<namespace>/test_instance/<path>.json`).
    pub(super) fn test_def(&self, id: &str) -> Option<TestDef> {
        if let Some(known) = self.test_defs.borrow().get(id) {
            return known.clone();
        }
        let def = self.read_test_def(id);
        self.test_defs
            .borrow_mut()
            .insert(id.to_owned(), def.clone());
        def
    }

    fn read_test_def(&self, id: &str) -> Option<TestDef> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let file = self
            .options
            .resources
            .join("data")
            .join(ns)
            .join("test_instance")
            .join(format!("{path}.json"));
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let string = |key: &str| json.get(key).and_then(|v| v.as_str());
        let structure = string("structure")?.to_owned();
        let environment = string("environment")?.to_owned();
        let dimension = string("dimension").unwrap_or("minecraft:overworld");
        let function = string("function");
        // `ExtraCodecs.POSITIVE_INT`, 1 when absent.
        let positive = |key: &str| {
            json.get(key)
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .clamp(1, i64::from(i32::MAX)) as i32
        };
        // GameTestInstance.describe: the type, the function (function tests
        // only), then the structure, batch and dimension.
        let block_based = string("type") == Some("minecraft:block_based");
        let mut description = Text::translate(
            "test_instance.description.type",
            &[Text::translate(
                if block_based {
                    "test_instance.type.block_based"
                } else {
                    "test_instance.type.function"
                },
                &[],
            )
            .with_color(0x5555FF)],
        )
        .append(Text::literal("\n"));
        if !block_based && let Some(function) = function {
            description = description.append(row("test_instance.description.function", function));
        }
        description = description
            .append(row("test_instance.description.structure", &structure))
            .append(row("test_instance.description.batch", &environment))
            .append(row("test_instance.description.dimension", dimension));
        Some(TestDef {
            structure,
            rotation: string("rotation")
                .and_then(Rotation::from_name)
                .unwrap_or(Rotation::None),
            padding: json
                .get("padding")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(0, 128) as i32,
            required: json
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            sky_access: json
                .get("sky_access")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            block_based,
            function: function.map(str::to_owned),
            environment: environment.clone(),
            dimension: dimension.to_owned(),
            max_ticks: json
                .get("max_ticks")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .clamp(1, i64::from(i32::MAX)) as i32,
            setup_ticks: json
                .get("setup_ticks")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(0, i64::from(i32::MAX)) as i32,
            max_attempts: positive("max_attempts"),
            required_successes: positive("required_successes"),
            description,
        })
    }

    pub(super) fn test_instance_at(&self, pos: Pos) -> Option<&TestInstanceBlock> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::TestInstanceBlock(b) => Some(b),
            _ => None,
        }
    }

    pub(super) fn test_instance_mut(&mut self, pos: Pos) -> Option<&mut TestInstanceBlock> {
        match self.world.block_entities.get_mut(&pos)? {
            BlockEntity::TestInstanceBlock(b) => Some(b),
            _ => None,
        }
    }

    /// `getTestHolder`.
    pub(super) fn test_holder(&self, pos: Pos) -> Option<TestDef> {
        let key = self.test_instance_at(pos)?.data.test.clone()?;
        self.test_def(&key)
    }

    /// The padding of the block's test, 0 without one.
    pub(super) fn test_holder_padding(&self, pos: Pos) -> i32 {
        self.test_holder(pos).map_or(0, |t| t.padding)
    }

    /// `ClientPacketListener.handleGameTestHighlightPos`:
    /// `GameTestBlockHighlightRenderer.highlightPos` shows the block for ten
    /// seconds with its position relative to the test.
    pub(super) fn highlight_test_pos(&mut self, absolute: [i32; 3], relative: [i32; 3]) {
        let text = format!("{}, {}, {}", relative[0], relative[1], relative[2]);
        let until = std::time::Instant::now() + std::time::Duration::from_millis(10_000);
        self.test_highlights
            .insert((absolute[0], absolute[1], absolute[2]), (text, until));
    }

    /// `TestInstanceRenderer.submit`'s error markers, for any player within
    /// the beacon's view distance of the block: each marked block in a
    /// translucent red box (`out`, blending) and its text as a billboard over
    /// everything (`see_through`).
    pub(super) fn test_instance_marker_geometry(
        &mut self,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
        see_through: &mut Vec<Vertex>,
    ) {
        use horizon_client::gizmos as g;
        // `GameTestBlockHighlightRenderer.emitGizmos`: expired highlights go,
        // the rest are a green box with the relative position over it.
        let now = std::time::Instant::now();
        self.test_highlights.retain(|_, (_, until)| now <= *until);
        let mut markers: Vec<(_, _, u32)> = self
            .test_highlights
            .iter()
            .map(|(&pos, (text, _))| ([pos.0, pos.1, pos.2], text.clone(), HIGHLIGHT_FILL))
            .collect();
        for (pos, entity) in &self.world.block_entities {
            let BlockEntity::TestInstanceBlock(b) = entity else {
                continue;
            };
            // BeaconRenderer.getViewDistance: 256, by the horizontal distance.
            let dx = f64::from(pos.0) + 0.5 - camera.x;
            let dz = f64::from(pos.2) + 0.5 - camera.z;
            if dx * dx + dz * dz >= 256.0 * 256.0 {
                continue;
            }
            for marker in &b.error_markers {
                markers.push((
                    marker.pos,
                    name_text(&marker.text).plain(),
                    g::ERROR_MARKER_FILL,
                ));
            }
        }
        if markers.is_empty() {
            return;
        }
        let light = vertex_light(0xFF);
        self.mark_translucent_entity(out.len(), "minecraft:test_instance_block");
        let mut patches = Vec::new();
        for (pos, text, color) in markers {
            // A plain colour: the atlas's white pixel at the fill's colour.
            let mut fill = HudBatch::new([1, 1]);
            fill.fill(&self.gui.atlas, 0.0, 0.0, 1.0, 1.0, color);
            let Some(v) = fill.vertices.first().copied() else {
                continue;
            };
            let (min, max, text_at) = g::error_marker_shape(pos);
            for quad in g::cuboid_fill_quads(min, max) {
                for corner in quad {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: (glam::DVec3::from_array(corner) - camera)
                            .as_vec3()
                            .to_array(),
                        uv: v.uv,
                        layer: 0,
                        color: v.color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
            // `Gizmos.billboardText(..).setAlwaysOnTop()`: white, centred,
            // without a background.
            patches.extend(self.name_tag_quads(
                glam::DVec3::from_array(text_at) - camera,
                &[(Text::literal(text), 0.0)],
                0xFF,
                false,
                2,
                0,
                g::billboard_text_unit(g::ERROR_MARKER_TEXT_SCALE),
                see_through,
            ));
        }
        self.map_patches.extend(patches);
    }

    /// `getRotation`: the test's rotation turned by the block's.
    pub(super) fn test_rotation(&self, pos: Pos) -> Rotation {
        let own = self
            .test_instance_at(pos)
            .map_or(Rotation::None, |b| b.data.rotation);
        self.test_holder(pos)
            .map_or(Rotation::None, |t| t.rotation)
            .get_rotated(own)
    }

    /// `getTestName`.
    pub(super) fn test_name(&self, pos: Pos) -> Text {
        match self.test_instance_at(pos).and_then(|b| b.data.test.clone()) {
            Some(key) => Text::literal(key),
            None => Text::translate("test_instance_block.invalid_test", &[]),
        }
    }

    /// `ServerGamePacketListenerImpl.handleTestInstanceBlockAction` for a
    /// guest: only a creative player with game master commands. A query or
    /// the screen's first look answers with a status packet.
    pub(super) fn lan_test_instance_action(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::TestInstanceBlockAction,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        let reply = self
            .as_guest_messaged(host, conn, |game| {
                allowed
                    .then(|| game.apply_test_instance_action(&p))
                    .flatten()
            })
            .flatten();
        if let Some((status, size)) = reply {
            host.hub.send_to(
                conn,
                &C::TestInstanceBlockStatus(play::TestInstanceBlockStatus {
                    status: horizon_client::lan::data::component_nbt(&status.component()),
                    size,
                }),
            );
        }
    }

    fn apply_test_instance_action(
        &mut self,
        p: &play::TestInstanceBlockAction,
    ) -> Option<(Text, Option<[i32; 3]>)> {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        self.test_instance_at(pos)?;
        // `ByIdMap.continuous(.., ZERO)`: an unknown action is INIT.
        let action = if (0..=RUN).contains(&p.action) {
            p.action
        } else {
            INIT
        };
        if action != QUERY && action != INIT {
            // `blockEntity.set(packet.data())`.
            let data = Data {
                test: p.data.test.clone(),
                size: p.data.size,
                rotation: rotation_from_ordinal(p.data.rotation).unwrap_or(Rotation::None),
                ignore_entities: p.data.ignore_entities,
                status: Status::by_index(p.data.status),
                error_message: p.data.error_message.clone(),
            };
            if let Some(b) = self.test_instance_mut(pos) {
                b.data = data;
            }
            match action {
                RESET => self.reset_test(pos),
                SAVE => {
                    self.save_test(pos);
                }
                EXPORT => self.export_test(pos),
                RUN => self.run_test(pos),
                _ => {}
            }
            self.mark_block_entity_updated(pos);
            return None;
        }
        let def = p.data.test.as_deref().and_then(|key| self.test_def(key));
        let status = def.map_or_else(
            || red("test_instance.description.no_test", &[]),
            |d| d.description,
        );
        // QUERY: the size of the test's structure.
        let size = (action == QUERY)
            .then(|| {
                let def = p.data.test.as_deref().and_then(|key| self.test_def(key))?;
                let entry = self.template_entry(&def.structure)?;
                Some(template_size(&entry.tag))
            })
            .flatten();
        Some((status, size))
    }

    /// `getStructureBoundingBox`.
    pub(super) fn test_structure_box(&self, pos: Pos) -> Option<BoundingBox> {
        let padding = self.test_holder(pos).map_or(0, |t| t.padding);
        let rotation = self.test_rotation(pos);
        Some(
            self.test_instance_at(pos)?
                .structure_box([pos.0, pos.1, pos.2], padding, rotation),
        )
    }

    /// `processStructureBoundary`: the floor and walls (and the ceiling,
    /// unless the test has sky access) of the box around the structure.
    pub(super) fn structure_boundary(&self, pos: Pos) -> Vec<Pos> {
        let Some(bounds) = self.test_structure_box(pos) else {
            return Vec::new();
        };
        let ceiling = !self.test_holder(pos).is_some_and(|t| t.sky_access);
        let low = bounds.min.map(|v| v - 1);
        // BlockPos.containing(maxX...) of the AABB, one past the box.
        let high = bounds.max.map(|v| v + 1);
        let mut out = Vec::new();
        for x in low[0]..=high[0] {
            for y in low[1]..=high[1] {
                for z in low[2]..=high[2] {
                    let edge =
                        x == low[0] || x == high[0] || z == low[2] || z == high[2] || y == low[1];
                    if edge || (y == high[1] && ceiling) {
                        out.push((x, y, z));
                    }
                }
            }
        }
        out
    }

    /// `setBlockAndUpdate(pos, state)`.
    pub(super) fn set_test_block_state(&mut self, at: Pos, name: &str, flags: i32) {
        let Some(id) = self.pack.assets.registry.default_state(name) else {
            return;
        };
        let pos = BlockPos::new(at.0, at.1, at.2);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, id, flags));
    }

    /// `removeBarriers`.
    pub(super) fn remove_test_barriers(&mut self, pos: Pos) {
        let barrier = self.pack.assets.registry.default_state("minecraft:barrier");
        for at in self.structure_boundary(pos) {
            if Some(self.world.block(at.0, at.1, at.2)) == barrier {
                self.set_test_block_state(at, "minecraft:air", flags::UPDATE_ALL);
            }
        }
        let _ = self.process_level_events();
    }

    /// `clearErrorMarkers`.
    pub(super) fn clear_error_markers(&mut self, pos: Pos) {
        if let Some(b) = self.test_instance_mut(pos)
            && !b.error_markers.is_empty()
        {
            b.error_markers.clear();
        }
    }

    /// `resetTest`.
    pub(super) fn reset_test(&mut self, pos: Pos) {
        self.remove_test_barriers(pos);
        self.clear_error_markers(pos);
        if self.place_test_structure(pos) {
            let name = self.test_name(pos);
            self.system_message(
                Text::translate("test_instance_block.reset_success", &[name])
                    .with_color(text::GREEN),
            );
        }
        // `data.withStatus(CLEARED)`: the error message goes too.
        if let Some(b) = self.test_instance_mut(pos) {
            b.data.status = Status::Cleared;
            b.data.error_message = None;
        }
    }

    /// `placeStructure()`: the test's structure, cleared space first.
    pub(super) fn place_test_structure(&mut self, pos: Pos) -> bool {
        let Some(def) = self.test_holder(pos) else {
            return false;
        };
        let Some(entry) = self.template_entry(&def.structure) else {
            return false;
        };
        let rotation = self.test_rotation(pos);
        let padding = def.padding;
        let Some(block) = self.test_instance_at(pos).cloned() else {
            return false;
        };
        let origin = [pos.0, pos.1, pos.2];
        // `forceLoadChunks`, then `StructureUtils.clearSpaceForStructure(
        // getTestBoundingBox())`.
        if let Some(inner) = self.test_structure_box(pos) {
            self.force_load_test_chunks(&inner);
            let area = BoundingBox::new(
                inner.min.map(|v| v - padding),
                inner.max.map(|v| v + padding),
            );
            self.clear_test_space(&area);
            // removeEntities: `getTestBounds()`, the structure's box inflated.
            self.discard_entities_in(inflate(aabb_of(&inner), f64::from(padding)));
        }
        let corner = block.start_corner(origin, padding, rotation);
        let settings = TemplateSettings {
            mirror: Mirror::None,
            rotation,
            ignore_entities: block.data.ignore_entities,
            known_shape: true,
            integrity: 1.0,
            seed: create_seed(0),
            flags: 818,
        };
        self.place_template_entry(
            &entry,
            BlockPos::new(corner[0], corner[1], corner[2]),
            settings,
            Mirror::None,
            rotation,
        );
        true
    }

    /// `StructureUtils.clearSpaceForStructure`'s blocks: air in the box (stone
    /// below the ground height, which no row of the box is).
    pub(super) fn clear_test_space(&mut self, area: &BoundingBox) {
        let ground = area.min[1] - 1;
        for y in area.min[1]..=area.max[1] {
            for z in area.min[2]..=area.max[2] {
                for x in area.min[0]..=area.max[0] {
                    let name = if y < ground {
                        "minecraft:stone"
                    } else {
                        "minecraft:air"
                    };
                    self.set_test_block_state((x, y, z), name, 818);
                }
            }
        }
        // `getBlockTicks().clearArea`, `clearBlockEvents`.
        self.clear_test_ticks(area);
        // The entities but the players.
        self.discard_entities_in(aabb_of(area));
    }

    /// `getBlockTicks().clearArea(bounds)` and `clearBlockEvents(bounds)`.
    pub(super) fn clear_test_ticks(&mut self, bounds: &BoundingBox) {
        let area = TickArea::new(
            BlockPos::new(bounds.min[0], bounds.min[1], bounds.min[2]),
            BlockPos::new(bounds.max[0], bounds.max[1], bounds.max[2]),
        );
        self.block_ticks.blocks.clear_area(area);
        self.block_ticks.block_events.retain(|e| {
            !(bounds.min[0]..=bounds.max[0]).contains(&e.pos.x)
                || !(bounds.min[1]..=bounds.max[1]).contains(&e.pos.y)
                || !(bounds.min[2]..=bounds.max[2]).contains(&e.pos.z)
        });
    }

    /// `saveTest`: the test's structure id, saved from the world.
    fn save_test(&mut self, pos: Pos) -> Option<String> {
        let id = self
            .test_holder(pos)
            .map(|t| t.structure)
            .or_else(|| self.test_instance_at(pos).and_then(|b| b.data.test.clone()));
        let Some(id) = id else {
            self.system_message(red(
                "test_instance_block.error.unable_to_save",
                &[pos.0, pos.1, pos.2].map(|v| Text::literal(v.to_string())),
            ));
            return None;
        };
        let block = self.test_instance_at(pos)?.clone();
        let padding = self.test_holder(pos).map_or(0, |t| t.padding);
        let corner = TestInstanceBlock::structure_pos_of(
            [0, 1, 2].map(|i| [pos.0, pos.1, pos.2][i] + padding),
        );
        self.save_structure_area(
            &id,
            corner,
            block.data.size,
            block.data.ignore_entities,
            "",
            true,
            true,
        );
        Some(id)
    }

    /// `exportTest`: saves, then writes to the test template folder, which
    /// only a development build has.
    fn export_test(&mut self, pos: Pos) {
        if self.save_test(pos).is_some() {
            self.system_message(
                Text::literal("Test structure exporting is disabled").with_color(text::RED),
            );
        }
    }

    /// `runTest`: the checks, the structure's placement and the test.
    fn run_test(&mut self, pos: Pos) {
        let coordinates = [pos.0, pos.1, pos.2].map(|v| Text::literal(v.to_string()));
        let Some(def) = self.test_holder(pos) else {
            self.system_message(red("test_instance_block.error.no_test", &coordinates));
            return;
        };
        if !self.place_test_structure(pos) {
            self.system_message(red(
                "test_instance_block.error.no_test_structure",
                &coordinates,
            ));
            return;
        }
        self.clear_error_markers(pos);
        // GameTestTicker.SINGLETON.clear(), FailedTestTracker.forgetFailedTests().
        self.clear_game_tests();
        self.game_tests.failed.clear();
        let Some(block) = self.test_instance_at(pos) else {
            return;
        };
        let key = block.data.test.clone().unwrap_or_default();
        let rotation = block.data.rotation;
        let key = if key.contains(':') {
            key
        } else {
            format!("minecraft:{key}")
        };
        self.system_message(Text::translate(
            "test_instance_block.starting",
            &[Text::literal(key.clone())],
        ));
        let id = self.game_tests.next_id();
        let info = Info::new(id, key, def, Some(pos), self.dim, rotation);
        self.track_and_start_runner(vec![info], false, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_print_the_value_in_blue_then_a_line_break() {
        let text = row("test_instance.description.structure", "minecraft:empty");
        assert!(text.plain().ends_with('\n'));
    }
}
