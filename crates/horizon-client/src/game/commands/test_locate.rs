//! `TestCommand`'s `locate`, `pos` and `create`.
use super::{Arg, Context, Game, R, err};
use crate::game::test_instances::TestDef;
use horizon_client::{
    containers::BlockEntity,
    test_instance_block::{Status, TestInstanceBlock},
    text::{self, ClickEvent, Style, Text},
};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::worldgen::structure::Rotation;

type Pos = (i32, i32, i32);

/// `Rotation.rotate(Direction.NORTH)` as (dx, dz) and the `toYRot` of the
/// opposite direction.
fn north_turned(rotation: Rotation) -> ((i32, i32), i32) {
    match rotation {
        Rotation::None => ((0, -1), 0),
        Rotation::Clockwise90 => ((1, 0), 90),
        Rotation::Clockwise180 => ((0, 1), 180),
        Rotation::Counterclockwise90 => ((-1, 0), 270),
    }
}

/// `String.format("/execute in %s run tp @s %d %d %d %d 0", ..)` of `locate`.
fn locate_command(dimension: &str, at: Pos, y_rot: i32) -> String {
    format!(
        "/execute in {dimension} run tp @s {} {} {} {y_rot} 0",
        at.0, at.1, at.2
    )
}

/// `Mth.floor(Mth.sqrt(dx * dx + dz * dz))` over ints.
fn horizontal_distance(dx: i32, dz: i32) -> i32 {
    ((dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz))) as f32)
        .sqrt()
        .floor() as i32
}

/// The text of a clickable bracketed position.
fn link(inner: Text, color: u32, command: String) -> Text {
    Text::literal("[")
        .append(inner)
        .append("]")
        .with_style(&Style {
            click: Some(ClickEvent::SuggestCommand(command)),
            hover: Some(Box::new(Text::translate("chat.coordinates.tooltip", &[]))),
            ..Style::color(color)
        })
}

fn path_of(dimension: &str) -> &str {
    dimension.split_once(':').map_or(dimension, |(_, p)| p)
}

impl Game<'_> {
    /// `TestCommand.locate`.
    pub(super) fn test_locate(&mut self, ctx: &Context) -> R<i32> {
        let selected: Vec<String> = match ctx.get("tests") {
            Some(Arg::Str(pattern)) => horizon_client::command::args::select_resources(
                self.ui.registries.ids("minecraft:test_instance"),
                pattern,
            )
            .into_iter()
            .cloned()
            .collect(),
            _ => Vec::new(),
        };
        // CommandSourceStack.sendSystemMessage: no feedback rule.
        if !self.command_source.is_some_and(|s| s.silent) {
            self.system_message(Text::translate("commands.test.locate.started", &[]));
        }
        let center = self.test_source_block();
        let source_dim = self.dim;
        // findSelectedTestPositionsAcrossDimensions: every level's test
        // blocks within 250 blocks whose test was selected.
        let mut found: Vec<(Pos, String, Rotation)> = Vec::new();
        if !selected.is_empty() {
            for dim in self.loaded_dims() {
                let back = self.enter_command_level(dim);
                for pos in self.find_test_instance_blocks(center, 250) {
                    let key = self.test_instance_at(pos).and_then(|b| b.data.test.clone());
                    let key = key.map(|k| {
                        if k.contains(':') {
                            k
                        } else {
                            format!("minecraft:{k}")
                        }
                    });
                    if key.is_some_and(|k| selected.contains(&k)) {
                        found.push((pos, dim.id().to_owned(), self.test_rotation(pos)));
                    }
                }
                if let Some(back) = back {
                    self.enter_command_level(back);
                }
            }
        }
        let mut dimensions: Vec<String> = Vec::new();
        for (pos, dimension, rotation) in &found {
            if !dimensions.contains(dimension) {
                dimensions.push(dimension.clone());
            }
            let ((dx, dz), y_rot) = north_turned(*rotation);
            let tp = (pos.0 + dx * 3, pos.1, pos.2 + dz * 3);
            let command = locate_command(dimension, tp, y_rot);
            let coordinates = link(
                Text::tr("chat.coordinates", &[&pos.0, &pos.1, &pos.2])
                    .append(format!(" ->[{}]", path_of(dimension))),
                text::GREEN,
                command,
            );
            let distance = if *dimension == source_dim.id() {
                horizontal_distance(center.0 - pos.0, center.2 - pos.2).to_string()
            } else {
                "N/A".to_owned()
            };
            self.feedback_unbroadcast(Text::translate(
                "commands.test.locate.found",
                &[coordinates, Text::literal(distance)],
            ));
        }
        let count = found.len() as i32;
        if count == 0 {
            return Err(err("commands.test.error.no_test_instances", &[]));
        }
        self.output_player_coordinates(&dimensions);
        self.feedback(Text::translate(
            "commands.test.locate.done",
            &[Text::literal(count.to_string())],
        ));
        Ok(count)
    }

    /// `outputPlayerCoordinates`.
    fn output_player_coordinates(&mut self, test_dimensions: &[String]) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let dimension = self.dim.id().to_owned();
        // isPlayerInCurrentTestDimension
        if test_dimensions.len() <= 1
            && test_dimensions.contains(&dimension)
            && test_dimensions.iter().any(|d| d == "minecraft:overworld")
        {
            return;
        }
        let at = [
            player.position.x.floor() as i32,
            player.position.y.floor() as i32,
            player.position.z.floor() as i32,
        ];
        let command = format!(
            "/execute in {dimension} run tp @s {} {} {} {} {}",
            at[0],
            at[1],
            at[2],
            self.yaw.floor() as i32,
            self.pitch.floor() as i32
        );
        let coordinates = link(
            Text::translate(
                "test.player.coordinates",
                &[
                    Text::literal(at[0].to_string()),
                    Text::literal(at[1].to_string()),
                    Text::literal(at[2].to_string()),
                    Text::literal(path_of(&dimension).to_owned()),
                ],
            ),
            text::GOLD,
            command,
        );
        self.feedback_unbroadcast(coordinates);
    }

    /// `TestCommand.showPos`, but for the highlight the client draws
    /// (`ClientboundGameTestHighlightPosPacket`).
    pub(super) fn test_show_pos(&mut self, ctx: &Context) -> R<i32> {
        self.source_player()?;
        let var = match ctx.get("var") {
            Some(Arg::Str(v)) => v.clone(),
            _ => "pos".to_owned(),
        };
        let target = self.test_pick_block();
        let target = (target[0], target[1], target[2]);
        let containing = |game: &Self, radius: i32| {
            game.find_test_instance_blocks(target, radius)
                .into_iter()
                .find(|&p| {
                    game.test_structure_box(p).is_some_and(|b| {
                        (0..3).all(|i| {
                            let t = [target.0, target.1, target.2][i];
                            t >= b.min[i] && t <= b.max[i]
                        })
                    })
                })
        };
        let found = containing(self, 15).or_else(|| containing(self, 250));
        let Some(block) = found else {
            return Err(err(
                "commands.test.error.no_test_containing_pos",
                &[&target.0, &target.1, &target.2],
            ));
        };
        let Some(origin) = self.test_structure_box(block).map(|b| b.min) else {
            return Err(err("commands.test.error.test_instance_not_found", &[]));
        };
        let relative = [
            target.0 - origin[0],
            target.1 - origin[1],
            target.2 - origin[2],
        ];
        let description = format!("{}, {}, {}", relative[0], relative[1], relative[2]);
        let coords = Text::translate(
            "commands.test.coordinates",
            &relative.map(|v| Text::literal(v.to_string())),
        )
        .with_style(&Style {
            bold: Some(true),
            click: Some(ClickEvent::CopyToClipboard(format!(
                "final BlockPos {var} = new BlockPos({description});"
            ))),
            hover: Some(Box::new(Text::translate(
                "commands.test.coordinates.copy",
                &[],
            ))),
            ..Style::color(text::GREEN)
        });
        let name = self.test_name(block).plain();
        self.feedback_unbroadcast(Text::translate(
            "commands.test.relative_position",
            &[Text::literal(name), coords],
        ));
        // `player.connection.send(new ClientboundGameTestHighlightPosPacket(..))`.
        if self.lan_acting.is_some() {
            self.guest_packets
                .push(C::GameTestHighlightPos(play::GameTestHighlightPos {
                    absolute: [target.0, target.1, target.2],
                    relative,
                }));
        } else {
            self.highlight_test_pos([target.0, target.1, target.2], relative);
        }
        Ok(1)
    }

    /// `TestCommand.createNewStructure`.
    pub(super) fn test_create(&mut self, ctx: &Context) -> R<i32> {
        let int = |name: &str, default: i32| match ctx.get(name) {
            Some(Arg::Int(v)) => *v,
            _ => default,
        };
        let Some(Arg::Resource(id)) = ctx.get("id") else {
            return Ok(0);
        };
        let key = if id.contains(':') {
            id.clone()
        } else {
            format!("minecraft:{id}")
        };
        // `create <id> <width>` is a cube.
        let width = int("width", 5);
        let size = if ctx.get("height").is_some() {
            [width, int("height", 5), int("depth", 5)]
        } else {
            [width; 3]
        };
        if size.iter().any(|&v| v > 48) {
            return Err(err("commands.test.error.too_large", &[&48]));
        }
        let at = self.test_position_around();
        self.create_empty_test(&key, at, size);
        let padding = self.test_holder_padding(at);
        let low =
            TestInstanceBlock::structure_pos_of([at.0 + padding, at.1 + padding, at.2 + padding]);
        let bedrock = self.pack.assets.registry.default_state("minecraft:bedrock");
        if bedrock.is_some() {
            for x in low[0]..low[0] + size[0] {
                for z in low[2]..low[2] + size[2] {
                    self.set_test_block_state(
                        (x, low[1], z),
                        "minecraft:bedrock",
                        horizon_world::level::flags::UPDATE_ALL,
                    );
                }
            }
        }
        let name = self.test_name(at);
        self.feedback(Text::translate("commands.test.create.success", &[name]));
        Ok(1)
    }
}

impl Game<'_> {
    /// `StructureUtils.createNewEmptyTest`.
    fn create_empty_test(&mut self, key: &str, at: Pos, size: [i32; 3]) {
        let padding = self.test_def(key).map_or(0, |d: TestDef| d.padding);
        let corner =
            TestInstanceBlock::structure_pos_of([at.0 + padding, at.1 + padding, at.2 + padding]);
        // getStructureBoundingBox(getStructurePos(testPos), size, NONE)
        let area = horizon_world::worldgen::structure::BoundingBox::new(
            corner,
            [0, 1, 2].map(|i| corner[i] + size[i] - 1),
        );
        self.clear_test_space(&area);
        self.set_test_block_state(
            at,
            "minecraft:test_instance_block",
            horizon_world::level::flags::UPDATE_ALL,
        );
        if self.test_instance_at(at).is_none() {
            self.world.block_entities.insert(
                at,
                BlockEntity::TestInstanceBlock(Box::<TestInstanceBlock>::default()),
            );
        }
        if let Some(block) = self.test_instance_mut(at) {
            block.data.test = Some(key.to_owned());
            block.data.size = size;
            block.data.rotation = Rotation::None;
            block.data.ignore_entities = false;
            block.data.status = Status::Cleared;
            block.data.error_message = None;
        }
        self.mark_block_entity_updated(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_teleports_three_blocks_in_front_of_the_rotated_north_face() {
        // Rotation.rotate(Direction.NORTH) and the toYRot of its opposite.
        assert_eq!(north_turned(Rotation::None), ((0, -1), 0));
        assert_eq!(north_turned(Rotation::Clockwise90), ((1, 0), 90));
        assert_eq!(north_turned(Rotation::Clockwise180), ((0, 1), 180));
        assert_eq!(north_turned(Rotation::Counterclockwise90), ((-1, 0), 270));
        assert_eq!(
            locate_command("minecraft:the_nether", (1, 64, -3), 90),
            "/execute in minecraft:the_nether run tp @s 1 64 -3 90 0"
        );
    }

    #[test]
    fn the_distance_is_the_floored_horizontal_one() {
        assert_eq!(horizontal_distance(3, 4), 5);
        assert_eq!(horizontal_distance(-1, 1), 1);
        assert_eq!(path_of("minecraft:overworld"), "overworld");
    }
}
