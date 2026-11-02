//! Server commands and players for native mods (docs/native-mods.md,
//! "Server commands and players", API 8): the commands mods add to the
//! server's command tree (`native_register_commands`), a player's command
//! reaching them (`native_perform_command`), and the helpers a server mod
//! answers with: the players online, a line to one of them or to all, the
//! command's feedback and the world's folder; and (API 9) the acting
//! player's plain items: count, take, give. They act on the server's own
//! players (the hub's: a dedicated server's, a LAN host's, single player's
//! server thread's), never on a client's.
use super::{Dim, Game, commands::execute, lan::LanHost};
// Their old home, for mixin code that names them here.
pub(crate) use horizon_client::mod_api::{NativeCommand, NativeServerPlayer};
use horizon_client::{items::ItemStack, lan::data::component_nbt, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::PathBuf};

/// A sidebar a server mod shows one player (`native_sidebar`): its title and
/// up to 15 lines, top first, each a text on the left and, where the score's
/// number would be, an optional text on the right. API 49.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeSidebar {
    pub(crate) title: Text,
    pub(crate) lines: Vec<(Text, Option<Text>)>,
}

/// The objective a mod's sidebar is sent as: the player's client only; the
/// server's scoreboard never holds it.
const SIDEBAR_OBJECTIVE: &str = "horizon:native_sidebar";

/// The brigadier parser of an argument kind, and its properties.
fn parser(kind: &str) -> Option<(&'static str, Value)> {
    Some(match kind {
        "word" => ("brigadier:string", json!({"type": "word"})),
        "text" => ("brigadier:string", json!({"type": "greedy"})),
        "integer" => ("brigadier:integer", Value::Null),
        "number" => ("brigadier:double", Value::Null),
        "boolean" => ("brigadier:bool", Value::Null),
        "player" => (
            "minecraft:entity",
            json!({"amount": "single", "type": "players"}),
        ),
        "item" => ("minecraft:item_stack", Value::Null),
        "block_pos" => ("minecraft:block_pos", Value::Null),
        _ => return None,
    })
}

/// The report's name of an operator level (`commands.json`'s
/// `permissions`).
const LEVELS: [&str; 5] = ["", "moderators", "gamemasters", "admins", "owners"];

/// One parsed word of a usage: its key in the tree and its node, without
/// children.
fn usage_nodes(usage: &str) -> Result<Vec<(String, Value, bool)>, String> {
    let mut out = Vec::new();
    let mut optional_seen = false;
    let mut greedy_seen = false;
    for word in usage.split_whitespace() {
        if greedy_seen {
            return Err(format!("`{usage}`: a text argument must be last"));
        }
        let (inner, optional) = if let Some(w) = word.strip_prefix('<') {
            (w.strip_suffix('>'), false)
        } else if let Some(w) = word.strip_prefix('[') {
            (w.strip_suffix(']'), true)
        } else {
            if optional_seen {
                return Err(format!("`{usage}`: `{word}` follows an optional argument"));
            }
            let literal = !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c));
            if !literal {
                return Err(format!("`{usage}`: `{word}` is not a command word"));
            }
            out.push((word.to_owned(), json!({"type": "literal"}), false));
            continue;
        };
        let Some((name, kind)) = inner.and_then(|w| w.split_once(':')) else {
            return Err(format!("`{usage}`: `{word}` is not `<name:kind>`"));
        };
        if optional_seen && !optional {
            return Err(format!("`{usage}`: `{word}` follows an optional argument"));
        }
        let Some((parser, properties)) = parser(kind) else {
            return Err(format!("`{usage}`: unknown kind `{kind}`"));
        };
        optional_seen |= optional;
        greedy_seen |= kind == "text";
        let mut node = json!({"type": "argument", "parser": parser});
        if !properties.is_null() {
            node["properties"] = properties;
        }
        out.push((name.to_owned(), node, optional));
    }
    if out.first().is_none_or(|n| n.1["type"] != "literal") {
        return Err(format!("`{usage}`: a command starts with its name"));
    }
    Ok(out)
}

/// Native mods' commands as a root in the commands report's format
/// (`lan::commands`, which sends each player what their level allows), and
/// each command's level: the lowest of its usages. A node is runnable where
/// only optional arguments follow it. A command vanilla has (`vanilla`
/// answers) or a usage that doesn't parse is left out, with why.
pub(super) fn command_tree(
    commands: &[NativeCommand],
    vanilla: impl Fn(&str) -> bool,
) -> (Value, HashMap<String, u8>, Vec<String>) {
    let mut root = Map::new();
    let mut levels: HashMap<String, u8> = HashMap::new();
    let mut problems = Vec::new();
    for command in commands {
        let nodes = match usage_nodes(&command.usage) {
            Ok(n) => n,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        if vanilla(&nodes[0].0) {
            problems.push(format!("`{}`: vanilla has /{}", command.usage, nodes[0].0));
            continue;
        }
        let mut children = &mut root;
        for (i, (name, node, _)) in nodes.iter().enumerate() {
            let runnable = nodes.get(i + 1).is_none_or(|n| n.2);
            let entry = children.entry(name.clone()).or_insert_with(|| {
                let mut n = node.clone();
                n["executable"] = false.into();
                n["level"] = 4.into();
                n
            });
            if entry["type"] != node["type"] || entry["parser"] != node["parser"] {
                problems.push(format!(
                    "`{}`: `{name}` is another kind in an earlier usage",
                    command.usage
                ));
                break;
            }
            if runnable {
                entry["executable"] = true.into();
            }
            let level = entry["level"]
                .as_u64()
                .unwrap_or(4)
                .min(command.level.into());
            entry["level"] = level.into();
            if !entry["children"].is_object() {
                entry["children"] = Value::Object(Map::new());
            }
            children = entry["children"].as_object_mut().expect("made above");
        }
        let level = levels.entry(nodes[0].0.clone()).or_insert(4);
        *level = (*level).min(command.level);
    }
    let mut root = Value::Object(root);
    finish(&mut root);
    let tree = if levels.is_empty() {
        Value::Null
    } else {
        json!({"type": "root", "children": root})
    };
    (tree, levels, problems)
}

/// Turns the working `level` into the report's `permissions`, and drops
/// empty child lists.
fn finish(children: &mut Value) {
    let Some(map) = children.as_object_mut() else {
        return;
    };
    for node in map.values_mut() {
        let level = node["level"].as_u64().unwrap_or(0) as usize;
        let object = node.as_object_mut().expect("a node");
        object.remove("level");
        if level > 0 {
            object.insert(
                "permissions".into(),
                json!({"type": "require", "permission": {"level": LEVELS[level.min(4)]}}),
            );
        }
        if object
            .get("children")
            .is_some_and(|c| c.as_object().is_some_and(Map::is_empty))
        {
            object.remove("children");
        } else if let Some(c) = object.get_mut("children") {
            finish(c);
        }
    }
}

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The players on this server, in the order they joined. Empty on a
    /// client and in a world nobody else can join.
    pub(super) fn native_server_players(&self) -> Vec<NativeServerPlayer> {
        let Some(host) = &self.lan else {
            return Vec::new();
        };
        host.hub
            .guests
            .iter()
            .filter(|g| g.placed)
            .map(|g| NativeServerPlayer {
                uuid: g.uuid,
                name: g.name.clone(),
                permission: g.permission,
                dimension: g.dimension.clone(),
                position: g.position,
                game_mode: g.game_mode,
                health: g.health,
                yaw: g.yaw,
                pitch: g.pitch,
            })
            .collect()
    }
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    pub(super) fn native_tell(&mut self, uuid: u128, line: Text) -> bool {
        self.native_send(uuid, system_chat(&line))
    }
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on. API 21.
    pub(super) fn native_action_bar(&mut self, uuid: u128, line: Text) -> bool {
        let packet = C::SystemChat(play::SystemChat {
            content: component_nbt(&line.component()),
            overlay: true,
        });
        self.native_send(uuid, packet)
    }
    /// Sends `packet` to that player, also when they are the acting player
    /// (their packets then go after the hook); false when they aren't on.
    pub(super) fn native_send(&mut self, uuid: u128, packet: C) -> bool {
        self.native_send_all(uuid, vec![packet])
    }
    fn native_send_all(&mut self, uuid: u128, packets: Vec<C>) -> bool {
        if self.lan_acting_uuid == Some(uuid) {
            self.guest_packets.extend(packets);
            return true;
        }
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.placed && g.uuid == uuid) else {
            return false;
        };
        for packet in &packets {
            host.hub.send_to(g.conn, packet);
        }
        true
    }
    /// Shows `sidebar` on that player's screen, in place of the one shown
    /// before, or takes it away (None). Only their client holds it: the
    /// server's scoreboard, its `/scoreboard` and the other players don't
    /// see it, and it is gone when they leave (a mod sends it again when
    /// they come back). It takes the sidebar's place from an objective the
    /// world shows there, for this player, until they rejoin. False when
    /// they aren't on. API 49.
    pub(super) fn native_sidebar(&mut self, uuid: u128, sidebar: Option<&NativeSidebar>) -> bool {
        let objective = SIDEBAR_OBJECTIVE.to_owned();
        // Removed first, so the new one's lines replace all the old ones.
        let mut packets = vec![C::SetObjective(play::SetObjective {
            name: objective.clone(),
            method: 1,
            display: None,
        })];
        if let Some(sidebar) = sidebar {
            let title = component_nbt(&sidebar.title.component());
            packets.push(C::SetObjective(play::SetObjective {
                name: objective.clone(),
                method: 0,
                display: Some((title, 0, Some(play::NumberFormat::Blank))),
            }));
            // DisplaySlot.SIDEBAR.
            packets.push(C::SetDisplayObjective(play::SetDisplayObjective {
                slot: 1,
                objective: objective.clone(),
            }));
            let lines = &sidebar.lines[..sidebar.lines.len().min(15)];
            for (i, (left, right)) in lines.iter().enumerate() {
                packets.push(C::SetScore(play::SetScore {
                    // Highest first. The display name shows, not the owner;
                    // an owner starting with '#' is hidden from the sidebar
                    // (ScoreHolder's isHidden), and ':' is in no player's name.
                    owner: format!("horizon:line{i:02}"),
                    objective: objective.clone(),
                    score: (lines.len() - i) as i32,
                    display: Some(component_nbt(&left.component())),
                    number_format: Some(match right {
                        Some(t) => play::NumberFormat::Fixed(component_nbt(&t.component())),
                        None => play::NumberFormat::Blank,
                    }),
                }));
            }
        }
        self.native_send_all(uuid, packets)
    }
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on. API 49.
    pub(super) fn native_tab_list(&mut self, uuid: u128, header: &Text, footer: &Text) -> bool {
        let packet = C::TabList(play::TabList {
            header: component_nbt(&header.component()),
            footer: component_nbt(&footer.component()),
        });
        self.native_send(uuid, packet)
    }
    /// The server's tick rate (20 unless `/tick rate` changed it) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower. API 49.
    pub(super) fn native_tick_stats(&self) -> (f32, f64) {
        (
            self.clock.rate(),
            self.clock.average_nanos() as f64 / 1_000_000.0,
        )
    }
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. Only where no player's state is swapped in, as
    /// `native_run_command`; false there and when they aren't on. API 49.
    pub(super) fn native_sleep_ignored(&mut self, uuid: u128, ignored: bool) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        if !host.hub.guests.iter().any(|g| g.placed && g.uuid == uuid) {
            return false;
        }
        if ignored {
            host.sleep_ignored.insert(uuid);
        } else {
            host.sleep_ignored.remove(&uuid);
        }
        true
    }
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (`native_text_allowed`, its length,
    /// other players' names). Only where no player's state is swapped in,
    /// as `native_run_command`; false there and when they aren't on. API
    /// 62.
    pub(super) fn native_set_nickname(&mut self, uuid: u128, nickname: Option<&str>) -> bool {
        let Some(host) = &mut self.lan else {
            return false;
        };
        if !host.hub.guests.iter().any(|g| g.placed && g.uuid == uuid) {
            return false;
        }
        host.hub.set_nickname(uuid, nickname.map(str::to_owned));
        true
    }
    /// Whether the server's chat filter (`server.toml` `[chat_filter]`)
    /// lets `text` through: true without a filter, None inside a player's
    /// hook (where the server is out of reach). API 62.
    pub(super) fn native_text_allowed(&self, text: &str) -> Option<bool> {
        let host = self.lan.as_ref()?;
        Some(
            host.hub
                .chat_filter
                .as_ref()
                .is_none_or(|g| !g.filter.matches(text)),
        )
    }
    /// A system line in every player's chat and the server's log.
    pub(super) fn native_broadcast(&self, line: Text) {
        let plain = line.plain();
        if self.options.dedicated.is_some() {
            super::dedicated::log("INFO", &plain);
        } else {
            log::info!("{plain}");
        }
        let Some(host) = &self.lan else {
            return;
        };
        let packet = system_chat(&line);
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            host.hub.send_to(g.conn, &packet);
        }
    }
    /// Posts a line to the dedicated server's event stream (`GET /api/v1/events`)
    /// as an `announce` event, which horizon-server's Discord bridge relays when
    /// `[discord] events` lists `announce`: `name` and `uuid` say who it is
    /// about (empty and 0 for no one), `message` is the line (`§` codes colour it).
    /// Only on a dedicated server; false elsewhere and for an empty message.
    pub(super) fn native_server_event(&self, name: &str, uuid: u128, message: &str) -> bool {
        if self.options.dedicated.is_none() || message.trim().is_empty() {
            return false;
        }
        super::dedicated::events::record("announce", name, uuid, json!({ "message": message }));
        true
    }
    /// A line of the command's answer, inside `native_perform_command`:
    /// sent to the player whose command it is after the call, as vanilla
    /// commands' feedback is.
    pub(super) fn native_feedback(&mut self, line: Text) {
        match &mut self.chat_capture {
            Some(lines) => lines.push(line),
            None => self.system_message(line),
        }
    }
    /// The player whose action or command a hook hears, as their state is
    /// swapped in (`native_perform_command`, `native_right_click_block`,
    /// `native_use_item_on`, `native_destroy_block`...): None outside such a
    /// hook. API 21.
    pub(super) fn native_acting_player(&self) -> Option<NativeServerPlayer> {
        let p = self.player.as_ref()?;
        Some(NativeServerPlayer {
            uuid: self.lan_acting_uuid?,
            name: self.lan_acting_name.clone()?,
            permission: self.lan_acting_permission?,
            dimension: self.dim.id().to_owned(),
            position: [p.position.x, p.position.y, p.position.z],
            game_mode: self.mode_id(),
            health: p.health.health(),
            yaw: p.yaw,
            pitch: p.pitch,
        })
    }
    /// Runs `line` (no slash) as the server's console does: permission
    /// level 4, from the overworld's spawn, with no output. A teleport to
    /// another level (`execute in minecraft:the_nether run tp Alex 0 70 0`)
    /// happens at the next tick's start, as the console's. Whether it ran
    /// without failing. Only where no player's state is swapped in
    /// (`native_tick`, `native_menu_clicked`): false inside a player's hook,
    /// where the other players are out of reach. API 21.
    pub(super) fn native_run_command(&mut self, line: &str) -> bool {
        if self.lan_acting.is_some() || self.lan.is_none() {
            return false;
        }
        let server = execute::Source {
            dimension: Dim::Overworld,
            ..self.server_source()
        };
        let outer = self.command_source.replace(server);
        let ok = self.as_console(|g| g.run_command(line));
        self.command_source = outer;
        ok
    }
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    pub(super) fn native_world_dir(&self) -> Option<PathBuf> {
        (!self.world_root.as_os_str().is_empty()).then(|| self.world_root.clone())
    }
}

/// Plain stacks: no components beyond the count (no damage, name,
/// enchantment...), the only ones the item helpers count, take or give, so
/// a mod never prices a worn or enchanted item as a new one.
fn plain(stack: &ItemStack) -> bool {
    stack.damage == 0 && stack.extra.is_none()
}

/// The slots the item helpers look at, in `Inventory.add`'s order: the
/// selected slot, the off hand, then the hotbar and the main inventory.
fn order(selected: usize) -> impl Iterator<Item = usize> {
    std::iter::once(selected)
        .chain(std::iter::once(OFFHAND))
        .chain((0..36).filter(move |&i| i != selected))
}
const OFFHAND: usize = 36;

/// The 36 inventory slots and the off hand as one list ([`OFFHAND`] last).
fn slots<'a>(
    inventory: &'a mut [Option<ItemStack>],
    offhand: &'a mut Option<ItemStack>,
) -> Vec<&'a mut Option<ItemStack>> {
    inventory
        .iter_mut()
        .take(36)
        .chain(std::iter::once(offhand))
        .collect()
}

/// How many of plain `item` the slots hold.
fn count_in(slots: &[&mut Option<ItemStack>], item: &str) -> u32 {
    slots
        .iter()
        .filter_map(|s| s.as_ref())
        .filter(|s| &*s.item == item && plain(s))
        .map(|s| s.count)
        .sum()
}

/// Takes up to `count` plain `item`, selected slot first; what it took.
fn take_from(
    mut slots: Vec<&mut Option<ItemStack>>,
    selected: usize,
    item: &str,
    count: u32,
) -> u32 {
    let mut taken = 0;
    for i in order(selected) {
        let slot = &mut *slots[i];
        if let Some(s) = slot
            && &*s.item == item
            && plain(s)
        {
            let n = s.count.min(count - taken);
            s.count -= n;
            taken += n;
            if s.count == 0 {
                *slot = None;
            }
        }
        if taken == count {
            break;
        }
    }
    taken
}

/// How many plain `item` (stacking to `max`) still fit.
fn room_in(slots: &[&mut Option<ItemStack>], item: &str, max: u32) -> u32 {
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| match s.as_ref() {
            // getFreeSlot: never into an empty off hand.
            None if i == OFFHAND => 0,
            None => max,
            Some(s) if &*s.item == item && plain(s) => max.saturating_sub(s.count),
            Some(_) => 0,
        })
        .sum()
}

/// Adds up to `count` plain `item`, onto its stacks first
/// (`getSlotWithRemainingSpace`: the off hand's too), then into empty slots
/// of the inventory (`getFreeSlot`), in [`order`]; what it added. Nothing
/// is dropped.
fn add_to(
    mut slots: Vec<&mut Option<ItemStack>>,
    selected: usize,
    item: &str,
    max: u32,
    count: u32,
) -> u32 {
    let mut added = 0;
    for pass in 0..2 {
        for i in order(selected) {
            let slot = &mut *slots[i];
            let n = match slot {
                Some(s) if pass == 0 && &*s.item == item && plain(s) => {
                    let n = max.saturating_sub(s.count).min(count - added);
                    s.count += n;
                    n
                }
                None if pass == 1 && i != OFFHAND => {
                    let n = max.min(count - added);
                    *slot = Some(ItemStack {
                        item: item.into(),
                        count: n,
                        damage: 0,
                        extra: None,
                    });
                    n
                }
                _ => 0,
            };
            added += n;
            if added == count {
                return added;
            }
        }
    }
    added
}

/// The acting player's items, for server mods (API 9): the player whose
/// action or command a hook hears (`native_perform_command`), as their
/// state is swapped in. Only plain stacks count; the inventory's 36 slots
/// and the off hand, never the armour. Each change reaches the player's
/// screen as any change to their inventory does.
#[allow(dead_code)]
impl Game<'_> {
    /// The main hand's stack: its item, count, and whether it is plain. On a
    /// client, the player's own.
    pub(super) fn native_held(&self) -> Option<(String, u32, bool)> {
        let s = self.inventory.get(self.selected)?.as_ref()?;
        Some((s.item.to_string(), s.count, plain(s)))
    }
    /// How many of plain `item` (`minecraft:diamond`) the player carries.
    pub(super) fn native_count_items(&mut self, item: &str) -> u32 {
        count_in(&slots(&mut self.inventory, &mut self.offhand), item)
    }
    /// Every plain item the player carries, with how many, by id.
    pub(super) fn native_carried_items(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = Vec::new();
        let held = self
            .inventory
            .iter()
            .take(36)
            .chain(std::iter::once(&self.offhand));
        for s in held.flatten().filter(|s| plain(s)) {
            match out.iter_mut().find(|(i, _)| *i == *s.item) {
                Some((_, n)) => *n += s.count,
                None => out.push((s.item.to_string(), s.count)),
            }
        }
        out
    }
    /// Takes up to `count` plain `item`, the main hand first; what it took.
    pub(super) fn native_take_items(&mut self, item: &str, count: u32) -> u32 {
        if self.player.is_none() {
            return 0;
        }
        let selected = self.selected;
        take_from(
            slots(&mut self.inventory, &mut self.offhand),
            selected,
            item,
            count,
        )
    }
    /// How many plain `item` still fit; 0 for an unknown item.
    pub(super) fn native_room_for(&mut self, item: &str) -> u32 {
        let Some(max) = self.res.items().get(item).map(|i| i.max_stack) else {
            return 0;
        };
        room_in(
            &slots(&mut self.inventory, &mut self.offhand),
            item,
            max.max(1),
        )
    }
    /// Gives up to `count` plain `item` where they fit, as `Inventory.add`
    /// places them; what it gave (nothing is dropped). 0 for an unknown item.
    pub(super) fn native_give_items(&mut self, item: &str, count: u32) -> u32 {
        let Some(max) = self.res.items().get(item).map(|i| i.max_stack) else {
            return 0;
        };
        if self.player.is_none() {
            return 0;
        }
        let selected = self.selected;
        add_to(
            slots(&mut self.inventory, &mut self.offhand),
            selected,
            item,
            max.max(1),
            count,
        )
    }
    /// Gives `stack` whole (its damage, name, enchantments... kept) to the
    /// player `uuid`, as `Inventory.add` places it: onto equal stacks first,
    /// then into empty inventory slots, never the armour or an empty off
    /// hand. What did not fit comes back (nothing is dropped): all of it for
    /// a player who isn't here, is dead, or whose state is swapped out by
    /// another player's (inside that player's hook, only they are reached).
    /// The player's screen follows as for any change to their inventory.
    /// What an auto-pickup or a machine's output to a player needs, from any
    /// server hook. API 98.
    pub(super) fn native_give_stack(
        &mut self,
        uuid: u128,
        mut stack: ItemStack,
    ) -> Option<ItemStack> {
        if stack.count == 0 {
            return None;
        }
        if uuid == self.native_acting_id() {
            // Their state is in place: the host's, or a guest's swapped in.
            if self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0)
            {
                return Some(stack);
            }
            return self.give(stack);
        }
        // Inside a player's hook their state is in place and the records of
        // the others may hold swapped state (turns nest): only they are
        // reached, as the doc says.
        if self.lan_acting.is_some() {
            return Some(stack);
        }
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack);
        let Some(host) = self.lan.as_mut() else {
            return Some(stack);
        };
        let Some(g) = host
            .hub
            .guests
            .iter_mut()
            .find(|g| g.placed && !g.dead && g.uuid == uuid)
        else {
            return Some(stack);
        };
        // inventory_add indexes the 36 main slots.
        if g.inventory.len() < 36 {
            g.inventory.resize(36, None);
        }
        let left = super::inventory_add(
            &mut g.inventory,
            &mut g.offhand,
            g.selected,
            max,
            &mut stack,
        );
        left.then_some(stack)
    }
}

/// An experience orb as a server mod reads it (`native_experience_orbs`).
/// Read by mods only.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct NativeExperienceOrb {
    /// The entity's ID, as `native_entity_join_level` names it.
    pub id: u32,
    pub position: [f64; 3],
    /// `ExperienceOrb.value`: the points one pickup gives.
    pub value: u32,
    /// `ExperienceOrb.count`: orbs merged into this one, each worth `value`.
    pub count: u32,
}

/// Whether an orb at `position` (its box: 0.5 wide and high, standing on
/// it) meets the box `min`..`max`, touching faces excluded (`AABB.intersects`).
fn orb_meets(position: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    position[0] - 0.25 < max[0]
        && position[0] + 0.25 > min[0]
        && position[1] < max[1]
        && position[1] + 0.5 > min[1]
        && position[2] - 0.25 < max[2]
        && position[2] + 0.25 > min[2]
}

/// Experience for server mods. API 101.
#[allow(dead_code)]
impl Game<'_> {
    /// The experience orbs of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: what a mod reads before it
    /// cancels an orb in `native_entity_join_level`, to give its points with
    /// `native_give_experience`. Empty for a level not in place and on a
    /// client of a server.
    pub(super) fn native_experience_orbs(
        &self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Vec<NativeExperienceOrb> {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return Vec::new();
        }
        self.orbs
            .iter()
            .filter(|o| o.count > 0)
            .map(|o| NativeExperienceOrb {
                id: o.id,
                position: [o.position.x, o.position.y, o.position.z],
                value: o.value,
                count: o.count,
            })
            .filter(|o| orb_meets(o.position, min, max))
            .collect()
    }
    /// Writes the experience orb `id`'s `value` (the points a pickup gives), `count`
    /// (orbs merged into it, each worth `value`) and `age` (ticks lived; an orb
    /// despawns at 6000); a `None` is left as it is. API 128.
    /// False, and nothing written, for an orb not in the server level `dimension`, a
    /// `value` or `count` of 0, and on a client of a server.
    pub(super) fn native_set_experience_orb(
        &mut self,
        dimension: &str,
        id: u32,
        value: Option<u32>,
        count: Option<u32>,
        age: Option<u32>,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || value == Some(0)
            || count == Some(0)
        {
            return false;
        }
        let Some(orb) = self.orbs.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        orb.value = value.unwrap_or(orb.value);
        orb.count = count.unwrap_or(orb.count);
        orb.age = age.unwrap_or(orb.age);
        true
    }
    /// Gives `points` of experience to the player `uuid` as an orb they touch
    /// does (`ExperienceOrb.playerTouch`): Mending repairs their damaged items
    /// with it first (`repairPlayerItems`), then the rest goes to their
    /// experience (`Player.giveExperiencePoints`), with the level-up sound
    /// every fifth level. No orb pickup sound. False, and nothing given, for
    /// a player who isn't here or is dead, and for another player than the one
    /// whose state is swapped in (inside that player's hook, only they are
    /// reached), and while the server's player list is busy elsewhere in the
    /// tick: give first, and cancel an orb only when this returned true.
    /// Their bar follows as for any change to their experience.
    pub(super) fn native_give_experience(&mut self, uuid: u128, points: u32) -> bool {
        self.native_as_player(uuid, |game| {
            let rest = if points > 0 { game.mending(points) } else { 0 };
            if rest > 0 {
                game.give_orb_points(rest);
            }
        })
    }
    /// Adds `levels` to the player `uuid`'s level, or takes them off when
    /// negative (`Player.giveExperienceLevels`): what the anvil and the
    /// enchanting table charge with, and a mod can charge or reward with.
    /// Below level 0 the level and the points are 0. The progress toward the
    /// next level is kept. False, and nothing changed, as for
    /// `native_give_experience`.
    pub(super) fn native_give_levels(&mut self, uuid: u128, levels: i32) -> bool {
        self.native_as_player(uuid, |game| game.give_levels(levels))
    }
    /// Runs `f` with the player `uuid`'s state in place: theirs already when
    /// they are the acting player (the host, or a guest swapped in), else a
    /// guest's through `as_guest`, only where no other player is swapped in.
    /// False when it didn't run: no such player here, or dead.
    fn native_as_player(&mut self, uuid: u128, f: impl FnOnce(&mut Self)) -> bool {
        if uuid == self.native_acting_id() {
            if self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0)
            {
                return false;
            }
            f(self);
            return true;
        }
        if self.lan_acting.is_some() {
            return false;
        }
        let Some(mut host) = self.lan.take() else {
            return false;
        };
        let conn = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && !g.dead && g.uuid == uuid)
            .map(|g| g.conn);
        let ran = conn
            .and_then(|conn| self.as_guest(&mut host, conn, f))
            .is_some();
        self.lan = Some(host);
        ran
    }
}

fn system_chat(line: &Text) -> C {
    C::SystemChat(play::SystemChat {
        content: component_nbt(&line.component()),
        overlay: false,
    })
}

/// Where the game calls the server hook points.
impl Game<'_> {
    /// Asks the mods for their commands once per hub, and puts them in every
    /// player's command tree.
    pub(super) fn register_native_commands(&mut self, host: &mut LanHost) {
        if host.native_commands.is_some() {
            return;
        }
        let mut commands = Vec::new();
        self.native_register_commands(&mut commands);
        let (tree, levels, problems) =
            command_tree(&commands, |name| host.hub.vanilla_command(name));
        for p in problems {
            log::warn!("Horizon: a native mod's command was left out: {p}");
        }
        host.hub.set_mod_commands(0, tree);
        host.native_commands = Some(levels);
    }

    /// The level a guest needs for a mod's command (`line`, without its
    /// slash), when a mod registered it.
    pub(super) fn native_command_level(host: &LanHost, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        host.native_commands.as_ref()?.get(name).copied()
    }

    /// A guest's command for the mods, inside their swap
    /// (`run_guest_command`): true when one ran it.
    pub(super) fn perform_native_command(&mut self, conn: u32, permission: u8, line: &str) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        if Self::native_command_level(host, line).is_none_or(|level| level > permission) {
            return false;
        }
        let Some(g) = host.hub.guest(conn) else {
            return false;
        };
        let player = NativeServerPlayer {
            uuid: g.uuid,
            name: g.name.clone(),
            permission,
            dimension: g.dimension.clone(),
            position: g.position,
            game_mode: g.game_mode,
            health: g.health,
            yaw: g.yaw,
            pitch: g.pitch,
        };
        let mut ran = false;
        self.native_perform_command(&player, line, &mut ran);
        if !ran {
            // Registered, but no mod took it: vanilla's unknown command.
            self.native_feedback(
                Text::translate("command.unknown.command", &[])
                    .with_color(horizon_client::text::RED),
            );
        }
        true
    }
}

impl Game<'_> {
    /// A console line for the mods: true when a mod registered its command
    /// (`native_register_commands`). The console is level 4, so a registered
    /// usage is always allowed; a command no mod takes answers as vanilla's
    /// unknown command.
    pub(super) fn perform_native_console_command(&mut self, line: &str) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        if Self::native_command_level(host, line).is_none() {
            return false;
        }
        let mut ran = false;
        self.native_perform_console_command(line, &mut ran);
        if !ran {
            self.native_feedback(
                Text::translate("command.unknown.command", &[])
                    .with_color(horizon_client::text::RED),
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(commands: &[NativeCommand]) -> (Value, HashMap<String, u8>, Vec<String>) {
        command_tree(commands, |name| name == "give")
    }

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        })
    }

    #[test]
    fn items_are_taken_from_the_hand_first_and_only_plain() {
        let mut inv = vec![None; 36];
        inv[0] = stack("minecraft:diamond", 10);
        inv[4] = stack("minecraft:diamond", 5);
        inv[8] = stack("minecraft:diamond", 7);
        inv[8].as_mut().unwrap().damage = 3;
        let mut off = stack("minecraft:diamond", 2);
        assert_eq!(
            count_in(&slots(&mut inv, &mut off), "minecraft:diamond"),
            17
        );
        assert_eq!(
            take_from(slots(&mut inv, &mut off), 4, "minecraft:diamond", 6),
            6
        );
        assert!(inv[4].is_none(), "the selected slot first");
        assert_eq!(off.as_ref().unwrap().count, 1, "then the off hand");
        assert_eq!(
            take_from(slots(&mut inv, &mut off), 4, "minecraft:diamond", 100),
            11
        );
        assert_eq!(inv[8].as_ref().unwrap().count, 7, "a damaged stack stays");
        assert!(inv[0].is_none() && off.is_none());
    }

    #[test]
    fn items_are_given_only_where_they_fit() {
        let mut inv = vec![stack("minecraft:dirt", 64); 36];
        inv[3] = stack("minecraft:diamond", 60);
        inv[9] = None;
        let mut off = None;
        assert_eq!(
            room_in(&slots(&mut inv, &mut off), "minecraft:diamond", 64),
            4 + 64
        );
        assert_eq!(
            add_to(slots(&mut inv, &mut off), 0, "minecraft:diamond", 64, 200),
            68
        );
        assert_eq!(inv[3].as_ref().unwrap().count, 64, "onto its stack first");
        assert!(off.is_none(), "never into an empty off hand (getFreeSlot)");
        assert_eq!(inv[9].as_ref().unwrap().count, 64);
        assert_eq!(
            room_in(&slots(&mut inv, &mut off), "minecraft:diamond", 64),
            0
        );
        assert_eq!(
            add_to(slots(&mut inv, &mut off), 0, "minecraft:diamond", 64, 1),
            0
        );
    }

    #[test]
    fn orbs_are_found_by_their_half_block_box() {
        // A block's cell, as a mod asks for a broken block's drops.
        let (min, max) = ([10.0, 64.0, -3.0], [11.0, 65.0, -2.0]);
        assert!(orb_meets([10.5, 64.2, -2.5], min, max), "inside");
        assert!(
            orb_meets([11.2, 64.0, -2.5], min, max),
            "its box reaches in"
        );
        assert!(
            orb_meets([10.5, 63.6, -2.5], min, max),
            "standing just below"
        );
        assert!(!orb_meets([11.25, 64.0, -2.5], min, max), "a face touching");
        assert!(!orb_meets([10.5, 63.5, -2.5], min, max), "its top touching");
        assert!(!orb_meets([10.5, 65.0, -2.5], min, max), "standing on top");
    }

    #[test]
    fn whole_stacks_are_given_onto_equal_stacks_only() {
        use super::super::inventory_add;
        let mut inv = vec![stack("minecraft:dirt", 64); 36];
        inv[2] = stack("minecraft:iron_sword", 1);
        inv[2].as_mut().unwrap().damage = 5;
        inv[5] = stack("minecraft:arrow", 60);
        inv[7] = None;
        inv[20] = None;
        let mut off = stack("minecraft:arrow", 62);

        // A damaged sword never joins another stack: the first free slot.
        let mut sword = stack("minecraft:iron_sword", 1).unwrap();
        sword.damage = 40;
        assert!(!inventory_add(&mut inv, &mut off, 5, 1, &mut sword));
        assert_eq!(inv[7].as_ref().unwrap().damage, 40, "its damage is kept");
        assert_eq!(inv[2].as_ref().unwrap().damage, 5);

        // Arrows: the selected slot, then the off hand, then a free slot,
        // and what is left over comes back.
        let mut arrows = stack("minecraft:arrow", 80).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 5, 64, &mut arrows));
        assert_eq!(inv[5].as_ref().unwrap().count, 64);
        assert_eq!(off.as_ref().unwrap().count, 64);
        assert_eq!(inv[20].as_ref().unwrap().count, 64);
        assert_eq!(arrows.count, 80 - 4 - 2 - 64);

        // A full inventory with an empty off hand takes nothing.
        let mut off = None;
        let mut apple = stack("minecraft:apple", 3).unwrap();
        assert!(inventory_add(&mut inv, &mut off, 0, 64, &mut apple));
        assert!(off.is_none(), "never into an empty off hand (getFreeSlot)");
        assert_eq!(apple.count, 3);
    }

    #[test]
    fn usages_merge_into_one_tree_with_runnable_nodes() {
        let (t, levels, problems) = tree(&[
            NativeCommand::new("balance [player:player]"),
            NativeCommand::new("pay <player:player> <amount:integer>"),
            NativeCommand::op("eco give <player:player> <amount:integer>", 3),
            NativeCommand::op("eco take <player:player> <amount:integer>", 3),
        ]);
        assert!(problems.is_empty(), "{problems:?}");
        let c = &t["children"];
        assert_eq!(c["balance"]["executable"], true);
        assert_eq!(c["balance"]["children"]["player"]["executable"], true);
        assert_eq!(
            c["balance"]["children"]["player"]["parser"],
            "minecraft:entity"
        );
        assert_eq!(c["pay"]["executable"], false);
        assert_eq!(c["pay"]["children"]["player"]["executable"], false);
        let amount = &c["pay"]["children"]["player"]["children"]["amount"];
        assert_eq!(amount["executable"], true);
        assert_eq!(amount["parser"], "brigadier:integer");
        assert!(amount.get("children").is_none());
        assert_eq!(c["eco"]["permissions"]["permission"]["level"], "admins");
        assert!(c["eco"]["children"]["give"].is_object());
        assert!(c["eco"]["children"]["take"].is_object());
        assert!(c["pay"].get("permissions").is_none());
        assert_eq!(levels["eco"], 3);
        assert_eq!(levels["pay"], 0);
    }

    #[test]
    fn a_command_takes_its_lowest_level() {
        let (t, levels, _) = tree(&[
            NativeCommand::new("warp [name:word]"),
            NativeCommand::op("warp set <name:word>", 2),
        ]);
        assert_eq!(levels["warp"], 0);
        assert!(t["children"]["warp"].get("permissions").is_none());
        let set = &t["children"]["warp"]["children"]["set"];
        assert_eq!(set["permissions"]["permission"]["level"], "gamemasters");
    }

    #[test]
    fn bad_usages_and_vanilla_names_are_left_out() {
        let (t, levels, problems) = tree(&[
            NativeCommand::new("give <x:integer>"),
            NativeCommand::new("a [x:integer] <y:integer>"),
            NativeCommand::new("b <x:text> <y:word>"),
            NativeCommand::new("c <x:colour>"),
            NativeCommand::new("<x:integer>"),
            NativeCommand::new("Bad"),
            NativeCommand::new("d <x:integer>"),
            NativeCommand::new("d <x:word>"),
        ]);
        assert_eq!(problems.len(), 7, "{problems:?}");
        assert_eq!(levels.keys().collect::<Vec<_>>(), ["d"]);
        assert_eq!(
            t["children"]["d"]["children"]["x"]["parser"],
            "brigadier:integer"
        );
        let (none, levels, _) = tree(&[]);
        assert!(none.is_null() && levels.is_empty());
    }
}
