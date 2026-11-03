//! The command tree a server sends (ClientboundCommandsPacket) as a
//! dispatcher the chat parses, colours and completes with
//! (ClientPacketListener.handleCommands).
//!
//! The packet holds each argument's parser and its network properties. A
//! node the client's own vanilla tree has at the same path takes that node's
//! argument type and suggestion provider (the parsers' suggestion lists and
//! the custom providers are the same code on both sides); any other node
//! (another server's or a mod's) gets the type its parser and properties
//! describe.
use super::{ArgType, Dispatcher, Node, NodeKind, Suggests};
use horizon_protocol::packets::play::{ARGUMENT_TYPES, ArgumentProperties, Commands};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Mutex, OnceLock},
};

/// The command id of every executable node of a server's tree: the chat
/// sends the command, the server runs it.
pub const SERVER_COMMAND: &str = "server";

/// The dispatcher for a server's tree. `local` is the client's own tree
/// (vanilla's), whose matching nodes lend their types and providers.
pub fn from_packet(packet: &Commands, local: &Dispatcher) -> Dispatcher {
    let count = packet.nodes.len();
    let root = usize::try_from(packet.root)
        .ok()
        .filter(|&r| r < count)
        .unwrap_or(0);
    // The packet's root becomes node 0, the others follow in packet order.
    let index = |i: usize| -> usize {
        if i == root {
            0
        } else if i < root {
            i + 1
        } else {
            i
        }
    };
    let valid = |c: &i32| usize::try_from(*c).ok().filter(|&c| c < count);
    // The local node at the same path, found breadth first from the roots
    // along children (redirects are not paths of their own).
    let mut matched: Vec<Option<usize>> = vec![None; count];
    matched[root] = Some(0);
    let mut queue = VecDeque::from([root]);
    let mut seen = HashSet::from([root]);
    while let Some(n) = queue.pop_front() {
        for c in packet.nodes[n].children.iter().filter_map(valid) {
            if !seen.insert(c) {
                continue;
            }
            let node = &packet.nodes[c];
            matched[c] = matched[n].and_then(|l| {
                local.nodes[l].children.iter().copied().find(|&lc| {
                    let ln = &local.nodes[lc];
                    ln.name() == node.name
                        && matches!(
                            (&ln.kind, node.kind),
                            (NodeKind::Literal(_), 1) | (NodeKind::Argument { .. }, 2)
                        )
                })
            });
            queue.push_back(c);
        }
    }
    let mut nodes = vec![None; count];
    for (i, node) in packet.nodes.iter().enumerate() {
        let local_node = matched[i].map(|l| &local.nodes[l]);
        let kind = match node.kind {
            1 => NodeKind::Literal(node.name.clone()),
            2 => {
                let ty = match local_node.map(|l| &l.kind) {
                    Some(NodeKind::Argument { ty, .. }) => ty.clone(),
                    _ => node
                        .parser
                        .as_ref()
                        .map_or(ArgType::Word, |(id, props)| argument_type(*id, props)),
                };
                NodeKind::Argument {
                    name: node.name.clone(),
                    ty,
                }
            }
            _ => NodeKind::Root,
        };
        // The server's providers, as the client knows them
        // (ArgumentNodeStub.build): its own custom ones arrive as `ask_server`.
        let suggests = node.suggestions.as_deref().and_then(provider);
        nodes[index(i)] = Some(Node {
            kind: if i == root { NodeKind::Root } else { kind },
            children: node.children.iter().filter_map(valid).map(index).collect(),
            command: node.executable.then_some(SERVER_COMMAND),
            redirect: node.redirect.as_ref().and_then(valid).map(index),
            // The server sent only what the player may use.
            permission: 0,
            suggests,
            restricted: node.restricted,
        });
    }
    let mut d = Dispatcher::new();
    if count > 0 {
        d.nodes = nodes.into_iter().flatten().collect();
    }
    d
}

/// SuggestionProviders by id (`SuggestionProviders.getProvider`).
fn provider(id: &str) -> Option<Suggests> {
    match id {
        "minecraft:ask_server" => Some(Suggests::AskServer),
        "minecraft:summonable_entities" => Some(Suggests::SummonableEntities),
        _ => None,
    }
}

/// A registry name kept for the life of the game (argument types hold
/// `&'static str`): each distinct name is stored once.
fn intern(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(&n) = names.get(name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(n);
    n
}

/// The argument type a parser and its properties describe
/// (ArgumentTypeInfo.Template.instantiate).
pub fn argument_type(id: i32, props: &[u8]) -> ArgType {
    let Some(name) = usize::try_from(id).ok().and_then(|i| ARGUMENT_TYPES.get(i)) else {
        return ArgType::Word;
    };
    let p = ArgumentProperties::decode(id, props).unwrap_or(ArgumentProperties::None);
    let registry = || match &p {
        ArgumentProperties::Registry(r) => intern(r),
        _ => "",
    };
    match *name {
        "brigadier:bool" => ArgType::Bool,
        "brigadier:integer" => match p {
            ArgumentProperties::Integer { min, max } => ArgType::Integer {
                min: min.unwrap_or(i32::MIN),
                max: max.unwrap_or(i32::MAX),
            },
            _ => ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        },
        // No long type of its own: a double over the same bounds reads
        // the same digits.
        "brigadier:long" => match p {
            ArgumentProperties::Long { min, max } => ArgType::Double {
                min: min.map_or(f64::MIN, |v| v as f64),
                max: max.map_or(f64::MAX, |v| v as f64),
            },
            _ => ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        },
        "brigadier:float" => match p {
            ArgumentProperties::Float { min, max } => ArgType::Float {
                min: min.unwrap_or(f32::MIN),
                max: max.unwrap_or(f32::MAX),
            },
            _ => ArgType::Float {
                min: f32::MIN,
                max: f32::MAX,
            },
        },
        "brigadier:double" => match p {
            ArgumentProperties::Double { min, max } => ArgType::Double {
                min: min.unwrap_or(f64::MIN),
                max: max.unwrap_or(f64::MAX),
            },
            _ => ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        },
        "brigadier:string" => match p {
            ArgumentProperties::String(1) => ArgType::Str,
            ArgumentProperties::String(2) => ArgType::Greedy,
            _ => ArgType::Word,
        },
        "minecraft:entity" => match p {
            ArgumentProperties::Entity {
                single,
                players_only,
            } => ArgType::Entity {
                single,
                players: players_only,
            },
            _ => ArgType::Entity {
                single: false,
                players: false,
            },
        },
        "minecraft:game_profile" => ArgType::GameProfile,
        "minecraft:block_pos" => ArgType::BlockPos,
        "minecraft:column_pos" => ArgType::ColumnPos,
        "minecraft:vec3" => ArgType::Vec3 { center: true },
        "minecraft:vec2" => ArgType::Vec2,
        "minecraft:block_state" => ArgType::BlockState,
        "minecraft:block_predicate" => ArgType::BlockPredicate,
        "minecraft:item_stack" => ArgType::Item,
        "minecraft:item_predicate" => ArgType::ItemPredicate,
        "minecraft:team_color" => ArgType::TeamColor,
        "minecraft:hex_color" => ArgType::HexColor,
        "minecraft:component" => ArgType::Component,
        "minecraft:style" => ArgType::Style,
        "minecraft:message" => ArgType::Message,
        "minecraft:nbt_compound_tag" => ArgType::Nbt,
        "minecraft:nbt_tag" => ArgType::NbtTag,
        "minecraft:nbt_path" => ArgType::NbtPath,
        "minecraft:objective" => ArgType::Objective,
        "minecraft:objective_criteria" => ArgType::Criteria,
        "minecraft:operation" => ArgType::Operation,
        "minecraft:particle" => ArgType::Particle,
        "minecraft:angle" => ArgType::Angle,
        "minecraft:rotation" => ArgType::Rotation,
        "minecraft:scoreboard_slot" => ArgType::ScoreboardSlot,
        "minecraft:score_holder" => ArgType::ScoreHolder {
            multiple: matches!(p, ArgumentProperties::ScoreHolder { multiple: true }),
        },
        "minecraft:swizzle" => ArgType::Swizzle,
        "minecraft:team" => ArgType::Team,
        "minecraft:item_slot" => ArgType::Slot,
        "minecraft:item_slots" => ArgType::Slots,
        "minecraft:resource_location" => ArgType::Id("none"),
        "minecraft:function" => ArgType::Function,
        "minecraft:entity_anchor" => ArgType::Anchor,
        "minecraft:int_range" => ArgType::IntRange,
        "minecraft:float_range" => ArgType::FloatRange,
        "minecraft:dimension" => ArgType::Dimension,
        "minecraft:gamemode" => ArgType::GameMode,
        "minecraft:time" => ArgType::Time {
            min: match p {
                ArgumentProperties::Time { min } => min,
                _ => 0,
            },
        },
        "minecraft:resource_or_tag" => ArgType::ResOrTag(registry()),
        "minecraft:resource_or_tag_key" => ArgType::ResOrTagKey(registry()),
        "minecraft:resource" => ArgType::Res(registry()),
        "minecraft:resource_key" => ArgType::ResKey(registry()),
        "minecraft:resource_selector" => ArgType::ResourceSelector(registry()),
        "minecraft:template_mirror" => ArgType::Enum(&["none", "front_back", "left_right"]),
        "minecraft:template_rotation" => {
            ArgType::Enum(&["none", "clockwise_90", "180", "counterclockwise_90"])
        }
        "minecraft:heightmap" => ArgType::Enum(&[
            "world_surface",
            "motion_blocking",
            "motion_blocking_no_leaves",
            "ocean_floor",
        ]),
        "minecraft:swing_animation" => ArgType::Enum(&["none", "whack", "stab"]),
        "minecraft:loot_table" => ArgType::ResourceOrId("minecraft:loot_table"),
        "minecraft:loot_predicate" => ArgType::ResourceOrId("minecraft:predicate"),
        "minecraft:loot_modifier" => ArgType::ResourceOrId("minecraft:item_modifier"),
        "minecraft:context_float_provider" => {
            ArgType::ResourceOrId("minecraft:context_float_provider")
        }
        "minecraft:context_int_provider" => ArgType::ResourceOrId("minecraft:context_int_provider"),
        "minecraft:slot_source" => ArgType::SlotSource,
        "minecraft:dialog" => ArgType::ResourceOrId("minecraft:dialog"),
        "minecraft:feature" => ArgType::ResourceOrId("minecraft:worldgen/feature"),
        "minecraft:uuid" => ArgType::Uuid,
        _ => ArgType::Word,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::{
        buf::{Codec, Reader, Writer},
        packets::play::CommandNode,
    };

    fn literal(name: &str, children: Vec<i32>, executable: bool) -> CommandNode {
        CommandNode {
            kind: 1,
            executable,
            restricted: false,
            children,
            redirect: None,
            name: name.into(),
            parser: None,
            suggestions: None,
        }
    }

    fn argument(name: &str, parser: &str, props: Vec<u8>, executable: bool) -> CommandNode {
        let id = ARGUMENT_TYPES.iter().position(|p| *p == parser).unwrap() as i32;
        CommandNode {
            kind: 2,
            executable,
            restricted: false,
            children: Vec::new(),
            redirect: None,
            name: name.into(),
            parser: Some((id, props)),
            suggestions: Some("minecraft:ask_server".into()),
        }
    }

    /// A tree with the root last (as vanilla's numbering need not put it
    /// first), a vanilla command and one the client does not know.
    fn packet() -> Commands {
        let mut range = Writer::new();
        range.u8(3);
        range.i32(1);
        range.i32(64);
        Commands {
            nodes: vec![
                literal("give", vec![2], false),
                literal("warp", vec![3], false),
                argument("targets", "minecraft:entity", vec![2], false),
                argument("count", "brigadier:integer", range.data, true),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0, 1],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
            ],
            root: 4,
        }
    }

    /// `stop` runs but is restricted, `say <message>` signs, `list` is plain.
    fn tree_for_checks() -> Dispatcher {
        let mut stop = literal("stop", Vec::new(), true);
        stop.restricted = true;
        let tree = Commands {
            nodes: vec![
                stop,
                literal("say", vec![2], false),
                argument("message", "minecraft:message", Vec::new(), true),
                literal("list", Vec::new(), true),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0, 1, 3],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
            ],
            root: 4,
        };
        from_packet(&tree, &Dispatcher::new())
    }

    #[test]
    fn is_valid_needs_a_whole_command_that_runs() {
        let tree = tree_for_checks();
        let reg = crate::Registries::default();
        assert!(tree.parse("list", &reg).is_valid());
        assert!(tree.parse("say hello", &reg).is_valid());
        // Unknown, stopping short of a command, and with something left over.
        assert!(!tree.parse("nosuchcommand", &reg).is_valid());
        assert!(!tree.parse("say", &reg).is_valid());
        assert!(!tree.parse("list extra", &reg).is_valid());
    }

    #[test]
    fn without_restricted_drops_only_restricted_nodes() {
        let tree = tree_for_checks();
        let reg = crate::Registries::default();
        assert!(tree.parse("stop", &reg).is_valid());
        let restricted = tree.without_restricted();
        assert!(!restricted.parse("stop", &reg).is_valid());
        assert!(restricted.parse("list", &reg).is_valid());
        assert!(restricted.parse("say hello", &reg).is_valid());
    }

    #[test]
    fn argument_nodes_round_trip() {
        let p = packet();
        let mut w = Writer::new();
        Commands::write(&p, &mut w);
        let back = Commands::read(&mut Reader::new(&w.data)).unwrap();
        assert_eq!(back, p);
    }

    /// What a client signs (SignableCommand.of over the server's tree): the
    /// message arguments of vanilla's /msg and of a command the client does
    /// not know, and nothing for a command without one.
    #[test]
    fn signable_arguments_of_a_server_tree() {
        let mut shout = argument("text", "minecraft:message", vec![], true);
        shout.suggestions = None;
        let mut targets = argument("targets", "minecraft:entity", vec![0], false);
        targets.children = vec![2];
        let packet = Commands {
            nodes: vec![
                literal("msg", vec![1], false),
                targets,
                argument("message", "minecraft:message", vec![], true),
                literal("shout", vec![4], false),
                shout,
                literal("spawn", vec![], true),
                CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: vec![0, 3, 5],
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                },
            ],
            root: 6,
        };
        let d = from_packet(&packet, super::super::vanilla::dispatcher());
        let reg = crate::Registries::default();
        let signable = |command: &str| d.parse(command, &reg).signable_arguments(&d);
        assert_eq!(
            signable("msg Steve hi there @a"),
            vec![("message".to_owned(), "hi there @a".to_owned())]
        );
        assert_eq!(
            signable("shout hello"),
            vec![("text".to_owned(), "hello".to_owned())]
        );
        assert!(signable("spawn").is_empty());
        assert!(signable("unknown words").is_empty());
    }

    #[test]
    fn server_tree_becomes_a_dispatcher() {
        let local = super::super::vanilla::dispatcher();
        let d = from_packet(&packet(), local);
        assert!(matches!(d.nodes[0].kind, NodeKind::Root));
        let warp = d.find("warp").expect("warp");
        let count = d.nodes[warp].children[0];
        match &d.nodes[count].kind {
            NodeKind::Argument {
                ty: ArgType::Integer { min, max },
                ..
            } => assert_eq!((*min, *max), (1, 64)),
            other => panic!("count is {other:?}"),
        }
        assert_eq!(d.nodes[count].command, Some(SERVER_COMMAND));
        // `give <targets>` is vanilla's: it keeps the client's type.
        let give = d.find("give").expect("give");
        let targets = d.nodes[give].children[0];
        assert!(matches!(
            d.nodes[targets].kind,
            NodeKind::Argument {
                ty: ArgType::Entity {
                    single: false,
                    players: true
                },
                ..
            }
        ));
    }

    #[test]
    fn every_argument_type_decodes() {
        for (id, name) in ARGUMENT_TYPES.iter().enumerate() {
            let mut w = Writer::new();
            match *name {
                "brigadier:integer" | "brigadier:long" | "brigadier:float" | "brigadier:double" => {
                    w.u8(0)
                }
                "brigadier:string" => w.varint(2),
                "minecraft:entity" | "minecraft:score_holder" => w.u8(1),
                "minecraft:time" => w.i32(0),
                n if n.starts_with("minecraft:resource") && n != "minecraft:resource_location" => {
                    w.string("minecraft:item")
                }
                _ => {}
            }
            let mut r = Reader::new(&w.data);
            ArgumentProperties::read(&mut r, id as i32).unwrap();
            assert_eq!(r.remaining(), 0, "{name}");
            let _ = argument_type(id as i32, &w.data);
        }
    }
}
