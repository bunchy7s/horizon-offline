use super::*;
use crate::{font::Font, hud::Rect};
use std::{collections::HashMap, path::Path};

/// The Chat option's default: no restriction.
fn full() -> ChatAbilities {
    ChatAbilities::new(ChatVisibility::Full)
}

fn font() -> Font {
    // The missing-glyph metrics let these tests run without game assets.
    Font::load(
        Path::new("/nonexistent-horizon-chat-test-resources"),
        &HashMap::new(),
        &[],
        (1, 1),
        (0, 0),
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        },
    )
}

fn add(chat: &mut Chat, font: &Font, content: Text) {
    for content in font.wrap_components(&content, chat.max_width()) {
        chat.lines.insert(
            0,
            Line {
                added: 0,
                content,
                indicator: None,
            },
        );
    }
}

#[test]
fn clicking_seed_copies_the_raw_value_only_inside_the_link() {
    let font = font();
    for seed in [1, -42, i64::MIN, i64::MAX] {
        let mut chat = Chat::default();
        let seed_text = Text::copyable(seed.to_string()).with_color(text::GOLD);
        assert_eq!(seed_text.plain(), format!("[{seed}]"));
        assert_eq!(seed_text.spans[0].style.color, Some(text::GOLD));
        assert_eq!(seed_text.spans[1].style.color, Some(text::GREEN));
        assert_eq!(seed_text.spans[2].style.color, Some(text::GOLD));
        add(&mut chat, &font, Text::literal("Seed: ").append(seed_text));
        let expected = Some(ClickEvent::CopyToClipboard(seed.to_string()));
        let start = 4.0 + f64::from(font.width("Seed: ["));
        let end = start + f64::from(font.width(&seed.to_string()));
        assert_eq!(chat.click_at(&font, 360, start, 315.0), expected);
        assert_eq!(chat.click_at(&font, 360, end - 0.5, 315.0), expected);
        for (x, y) in [
            (start - 0.5, 315.0),
            (end, 315.0),
            (3.0, 315.0),
            (start, 320.0),
            (start, 310.0),
        ] {
            assert_eq!(chat.click_at(&font, 360, x, y), None, "{x}, {y}");
        }
    }
}

#[test]
fn click_targets_match_rendered_spans_when_scaled_wrapped_and_scrolled() {
    let font = font();
    let screenshot = ClickEvent::OpenFile("/tmp/My screenshots/世界.png".into());
    for scale in [0.5, 0.75, 1.0] {
        for line_spacing in [0.0, 0.5, 1.0] {
            let mut chat = Chat::default();
            chat.options.scale = scale;
            chat.options.width = 0.0;
            chat.options.line_spacing = line_spacing;
            for _ in 0..20 {
                add(
                    &mut chat,
                    &font,
                    Text::literal("Saved screenshot as ").append(Text::styled(
                        "2026-10-14_18.55.14.png",
                        Style {
                            click: Some(screenshot.clone()),
                            bold: Some(true),
                            underlined: Some(true),
                            ..Style::default()
                        },
                    )),
                );
            }
            for scroll in [0, 3, 12] {
                chat.scroll = scroll;
                let mut links = 0;
                for draw in chat.layout(361, 0, true) {
                    if let GuiDraw::Text { text, x, y, .. } = draw {
                        let mut at = x as f64;
                        for (c, style) in text.chars() {
                            let width = f64::from(font.advance(c, style));
                            let cursor_x = (4.0 + at + width / 2.0) * scale;
                            let cursor_y = (y as f64 + 3.0) * scale;
                            assert_eq!(
                                chat.click_at(&font, 361, cursor_x, cursor_y),
                                style.click,
                                "scale={scale}, spacing={line_spacing}, scroll={scroll}, char={c}"
                            );
                            links += usize::from(style.click.is_some());
                            at += width;
                        }
                    }
                }
                assert!(links > 0);
                // The history above the focused viewport is not clickable.
                let top = ((361 - 40) as f64 / scale).floor()
                    - chat.lines_per_page(true) as f64 * chat.options.line_height() as f64;
                assert_eq!(
                    chat.click_at(&font, 361, 10.0 * scale, (top - 1.0) * scale),
                    None
                );
            }
        }
    }
}

#[test]
fn locate_coordinates_fill_chat_with_teleport_without_submitting() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries::default();
    let commands = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    // Structures and POIs use relative Y; biome results include absolute Y.
    for (y, expected) in [
        (None, "/tp @s -1136 ~ 848"),
        (Some(73), "/tp @s -1136 73 848"),
    ] {
        let link = Text::teleport_link(-1136, y, 848);
        assert_eq!(link.component()["click_event"]["action"], "suggest_command");
        assert_eq!(link.component()["click_event"]["command"], expected);
        let mut chat = Chat::default();
        add(&mut chat, &font, Text::literal("Found at ").append(link));
        let mut screen = ChatScreen::new(&m, &commands, "draft", 0, 640, 360, full());
        let x = 4.0 + f64::from(font.width("Found at "));
        assert!(!screen.click(&m, &commands, x, 315.0));
        let Some(ClickEvent::SuggestCommand(command)) = chat.click_at(&font, 360, x, 315.0) else {
            panic!("locate coordinates must suggest a command");
        };
        screen.suggest_command(&m, &commands, &command);
        assert_eq!(screen.value(), expected);
        assert_eq!(screen.input.cursor, expected.chars().count());
        assert_eq!(screen.input.highlight, screen.input.cursor);
        // Submission is a separate, explicit Enter press.
        assert_eq!(
            screen.key(
                &m,
                &commands,
                &[],
                winit::keyboard::KeyCode::Enter,
                false,
                false
            ),
            ChatAction::Submit(expected.to_owned())
        );
    }
}

#[test]
fn history_arrows_recall_messages_and_restore_the_draft() {
    use winit::keyboard::KeyCode::{ArrowDown, ArrowUp};

    let font = font();
    let m = Measure(&font);
    let registries = Registries::default();
    let commands = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let history = vec!["first message".to_owned(), "latest message".to_owned()];
    let mut screen = ChatScreen::new(
        &m,
        &commands,
        "unfinished draft",
        history.len(),
        640,
        360,
        full(),
    );
    for (key, expected) in [
        (ArrowUp, "latest message"),
        (ArrowUp, "first message"),
        (ArrowUp, "first message"),
        (ArrowDown, "latest message"),
        (ArrowDown, "unfinished draft"),
        (ArrowDown, "unfinished draft"),
    ] {
        assert_eq!(
            screen.key(&m, &commands, &history, key, false, false),
            ChatAction::None
        );
        assert_eq!(screen.value(), expected);
    }
}

#[test]
fn mod_commands_are_coloured_and_complete_like_vanilla_ones() {
    let font = font();
    let m = Measure(&font);
    // Allow Commands off: a mod's command needs no level.
    let registries = Registries::default();
    let vanilla = crate::command::vanilla::game_dispatcher();
    let modded = vanilla.with_mod_commands(["where", "coords"]);
    let red = |screen: &ChatScreen| {
        screen
            .format(screen.value(), 0)
            .spans
            .iter()
            .any(|s| s.style.color == Some(text::RED))
    };
    let typed = |dispatcher: &crate::command::Dispatcher, line: &str| {
        let c = Commands {
            dispatcher,
            registries: &registries,
        };
        let mut screen = ChatScreen::new(&m, &c, "", 0, 640, 360, full());
        screen.type_text(&m, &c, line);
        screen
    };
    // Unknown without the mod's tree (#456).
    assert!(red(&typed(vanilla, "/coords")));
    for line in ["/coords", "/where", "/where me and you"] {
        assert!(!red(&typed(&modded, line)), "{line}");
    }
    let c = Commands {
        dispatcher: &modded,
        registries: &registries,
    };
    let mut screen = typed(&modded, "/coo");
    screen.key(&m, &c, &[], winit::keyboard::KeyCode::Tab, false, false);
    assert_eq!(screen.value(), "/coords");
}

#[test]
fn ask_server_nodes_wait_for_the_servers_completions() {
    // ClientSuggestionProvider.customSuggestion: an `ask_server` node at the
    // cursor sends the command so far, and the list is done with the reply.
    let font = font();
    let m = Measure(&font);
    let registries = Registries {
        permission: 4,
        ..Registries::default()
    };
    let mut d = crate::command::vanilla::game_dispatcher().clone();
    let child = |d: &crate::command::Dispatcher, n: usize, name: &str| {
        d.nodes[n]
            .children
            .iter()
            .copied()
            .find(|&c| d.nodes[c].name() == name)
            .expect(name)
    };
    let id = child(&d, child(&d, child(&d, 0, "bossbar"), "get"), "id");
    d.nodes[id].suggests = Some(crate::command::Suggests::AskServer);
    let c = Commands {
        dispatcher: &d,
        registries: &registries,
    };
    let tab = |screen: &mut ChatScreen| {
        screen.key(&m, &c, &[], winit::keyboard::KeyCode::Tab, false, false);
    };
    let mut screen = ChatScreen::new(&m, &c, "", 0, 640, 360, full());
    screen.type_text(&m, &c, "/bossbar get pt");
    let (request, sent) = screen.take_request().expect("the server is asked");
    assert_eq!(sent, "/bossbar get pt");
    assert_eq!(screen.take_request(), None);
    // Nothing to complete until the reply.
    tab(&mut screen);
    assert_eq!(screen.value(), "/bossbar get pt");
    // A reply to an older request is dropped; the range is in UTF-16 units.
    let reply = |text: &str| vec![(text.to_owned(), None)];
    screen.server_suggestions(&m, &c, request - 1, (13, 2), reply("pt6:old"));
    tab(&mut screen);
    assert_eq!(screen.value(), "/bossbar get pt");
    screen.server_suggestions(&m, &c, request, (13, 2), reply("pt6:check"));
    tab(&mut screen);
    assert_eq!(screen.value(), "/bossbar get pt6:check");
    // Without the server's provider the local list is done at once.
    let local = crate::command::vanilla::game_dispatcher();
    let c = Commands {
        dispatcher: local,
        registries: &registries,
    };
    let mut screen = ChatScreen::new(&m, &c, "", 0, 640, 360, full());
    screen.type_text(&m, &c, "/bossbar get pt");
    assert_eq!(screen.take_request(), None);
}

#[test]
fn legacy_codes_colour_the_line_or_vanish_without_chat_colours() {
    let mut chat = Chat::default();
    let red = Text::literal("\u{a7}cRED");
    let shown = chat.displayed(&red);
    assert_eq!(shown.plain(), "RED");
    assert!(shown.chars().all(|(_, s)| s.color == Some(0xFF5555)));
    chat.options.colors = false;
    let shown = chat.displayed(&red);
    assert_eq!(shown.plain(), "RED");
    assert!(shown.chars().all(|(_, s)| s.color.is_none()));
    // A lone section sign or an unknown code stays as written.
    assert_eq!(
        chat.displayed(&Text::literal("a\u{a7}zb")).plain(),
        "a\u{a7}zb"
    );
}

#[test]
fn a_server_component_keeps_its_click_and_hover_events() {
    let font = font();
    let json = serde_json::json!({
        "text": "CLICK", "color": "green", "bold": true,
        "click_event": {"action": "suggest_command", "command": "/say clicked"},
        "hover_event": {"action": "show_text", "value": {"text": "Tip", "color": "aqua"}},
    });
    let line = crate::component::to_text(&json, &mut |s| Text::literal(s));
    let mut chat = Chat::default();
    add(&mut chat, &font, line);
    let x = 4.0 + f64::from(font.width("CL"));
    let Some(ClickEvent::SuggestCommand(command)) = chat.click_at(&font, 360, x, 315.0) else {
        panic!("the line must suggest its command");
    };
    assert_eq!(command, "/say clicked");
    let hover = chat.hover_at(&font, 360, x, 315.0).expect("a hover text");
    assert_eq!(hover.plain(), "Tip");
    assert_eq!(hover.spans[0].style.color, Some(text::AQUA));
    // Past the (bold, so wider) text, nothing.
    let past = 4.0 + f64::from(font.width("CLICK")) + 20.0;
    assert!(chat.hover_at(&font, 360, past, 315.0).is_none());
}

/// `ChatVisiblity`: Commands Only keeps the chat out, Hidden takes the
/// commands too (`ChatAbilities`), and Enter sends nothing the option hides.
#[test]
fn the_chat_option_decides_what_may_be_sent() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries::default();
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let enter = |screen: &mut ChatScreen| {
        screen.key(&m, &c, &[], winit::keyboard::KeyCode::Enter, false, false)
    };
    for (visibility, chat, command) in [
        (ChatVisibility::Full, true, true),
        (ChatVisibility::System, false, true),
        (ChatVisibility::Hidden, false, false),
    ] {
        let abilities = ChatAbilities::new(visibility);
        assert_eq!(abilities.allows("hi"), chat, "{visibility:?}");
        assert_eq!(abilities.allows("/time set day"), command, "{visibility:?}");
        for (line, allowed) in [("hi", chat), ("/time set day", command)] {
            let mut screen = ChatScreen::new(&m, &c, "", 0, 640, 360, abilities);
            screen.type_text(&m, &c, line);
            match enter(&mut screen) {
                ChatAction::Submit(sent) => {
                    assert!(allowed, "{visibility:?} sent {sent}");
                    assert_eq!(sent, line);
                }
                ChatAction::None => assert!(!allowed, "{visibility:?} kept {line}"),
                ChatAction::Close => panic!("Enter does not close the chat"),
            }
        }
    }
}

/// The restriction the option sets is a usage line: red, under the input, for
/// a command the Chat option takes and for a message it takes.
#[test]
fn restricted_input_says_why_it_will_not_send() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries::default();
    let c = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    let usage = |visibility, line: &str| {
        let mut screen = ChatScreen::new(&m, &c, "", 0, 640, 360, ChatAbilities::new(visibility));
        screen.type_text(&m, &c, line);
        screen.usage.iter().map(|t| t.plain()).collect::<Vec<_>>()
    };
    assert_eq!(usage(ChatVisibility::Full, "hi"), Vec::<String>::new());
    // The font has no language files here, so the line is its key.
    assert_eq!(
        usage(ChatVisibility::System, "hi"),
        ["chat_screen.messages_not_allowed"]
    );
    // A command the option allows says nothing about the restriction.
    assert!(
        !usage(ChatVisibility::System, "/time set day")
            .iter()
            .any(|line| line == "chat_screen.commands_not_allowed")
    );
    assert!(
        usage(ChatVisibility::Hidden, "/time set day")
            .iter()
            .any(|line| line == "chat_screen.commands_not_allowed")
    );
}

/// `ChatComponent.setVisibleMessageFilter`: a message the Chat option hides
/// on arrival is dropped; one already in the chat leaves it while the option
/// hides its kind and comes back when it shows it again.
#[test]
fn the_chat_option_filters_the_lines_both_ways() {
    let font = font();
    let mut chat = Chat::default();
    let says = |chat: &mut Chat, text: &str, source| {
        chat.add_message(&font, Text::literal(text), None, source, 0, false);
    };
    let shown =
        |chat: &Chat| -> Vec<String> { chat.lines.iter().map(|l| l.content.plain()).collect() };
    says(&mut chat, "player", ChatSource::Player);
    says(&mut chat, "server", ChatSource::Server);
    chat.set_abilities(&font, ChatAbilities::new(ChatVisibility::System));
    assert_eq!(shown(&chat), ["server"]);
    says(&mut chat, "dropped", ChatSource::Player);
    chat.set_abilities(&font, ChatAbilities::new(ChatVisibility::Hidden));
    says(&mut chat, "client", ChatSource::Client);
    assert_eq!(shown(&chat), ["client"]);
    chat.set_abilities(&font, ChatAbilities::new(ChatVisibility::Full));
    assert_eq!(shown(&chat), ["client", "server", "player"]);
}

/// The in-frame LAN host's chat: a guest's line is a player message, so the
/// Chat option's player-message permission hides it at Commands Only and
/// Hidden; command feedback (`/list`, `/tp`) is a system message and stays
/// at Commands Only.
#[test]
fn a_guests_line_follows_the_player_message_permission() {
    let font = font();
    let shown = |visibility, source| {
        let mut chat = Chat::default();
        chat.set_abilities(&font, ChatAbilities::new(visibility));
        chat.add_message(&font, Text::literal("line"), None, source, 0, false);
        chat.messages.len()
    };
    for (visibility, player, system) in [
        (ChatVisibility::Full, 1, 1),
        (ChatVisibility::System, 0, 1),
        (ChatVisibility::Hidden, 0, 0),
    ] {
        assert_eq!(
            shown(visibility, ChatSource::Player),
            player,
            "{visibility:?}"
        );
        assert_eq!(
            shown(visibility, ChatSource::Server),
            system,
            "{visibility:?}"
        );
    }
}
