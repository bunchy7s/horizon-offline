//! `/tellraw` (TellRawCommand) and `/title` (TitleCommand).
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{hud_overlays::Titles, lan::data::component_nbt, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// `EntitySelector.COMPILABLE_CODEC` (`CompilableString.CommandParserHelper.apply`):
/// the `selector` of a selector component and the `entity` of an nbt
/// component are parsed when the component is decoded, and text left after
/// the selector is `command.trailing_data`. Either failure makes the whole
/// component invalid, with vanilla's message. (A score `name` is an `Either`
/// that falls back to a plain name, so it never fails.)
fn invalid_selector(
    v: &serde_json::Value,
    reg: &horizon_client::command::Registries,
) -> Option<String> {
    match v {
        serde_json::Value::Array(a) => a.iter().find_map(|x| invalid_selector(x, reg)),
        serde_json::Value::Object(o) => {
            let source = o.get("selector").and_then(|s| s.as_str()).or_else(|| {
                o.contains_key("nbt")
                    .then(|| o.get("entity").and_then(|s| s.as_str()))
                    .flatten()
            });
            if let Some(source) = source
                && let Some(why) = compile_selector(source, reg)
            {
                return Some(format!("Invalid selector component: {source}: {why}"));
            }
            o.values().find_map(|x| invalid_selector(x, reg))
        }
        _ => None,
    }
}

/// `EntitySelectorParser.parse` on all of `source`; the error's
/// `getMessage` on failure.
fn compile_selector(source: &str, reg: &horizon_client::command::Registries) -> Option<String> {
    let mut reader = horizon_client::command::Reader::new(source);
    match horizon_client::command::selector::parse(&mut reader, reg, true) {
        Err(e) => Some(match e.context() {
            Some(context) => format!("{} at position {}: {context}", e.message.plain(), e.cursor),
            None => e.message.plain(),
        }),
        Ok(_) if reader.can_read() => {
            let parsed = &source[..reader.cursor];
            let leftovers = &source[reader.cursor..];
            Some(
                horizon_client::text::Text::tr(
                    "command.trailing_data",
                    &[&format!("{parsed}[{leftovers}]")],
                )
                .plain(),
            )
        }
        Ok(_) => None,
    }
}

impl Game<'_> {
    /// ComponentArgument with selectors resolved for the source
    /// (ComponentUtils.updateForEntity: names joined by ", ").
    pub(in crate::game) fn component_arg(&mut self, ctx: &Context, name: &str) -> R<Text> {
        let Some(Arg::Str(json)) = ctx.get(name) else {
            return Ok(Text::empty());
        };
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| err("argument.component.invalid", &[&e]))?;
        if let Some(message) = invalid_selector(&value, &self.ui.registries) {
            return Err(err("argument.component.invalid", &[&message]));
        }
        // NbtContents.resolve: the path's values in the entity, block or
        // storage data, rich-styled (or plain, or interpreted), ", "-joined.
        // ScoreContents.resolve: the holder's score in the objective.
        let nbt_values = std::cell::RefCell::new(Vec::new());
        {
            let mut collect = |o: &serde_json::Map<String, serde_json::Value>| {
                if o.contains_key("nbt") || o.contains_key("score") {
                    nbt_values.borrow_mut().push(o.clone());
                }
            };
            fn walk(
                v: &serde_json::Value,
                f: &mut dyn FnMut(&serde_json::Map<String, serde_json::Value>),
            ) {
                match v {
                    serde_json::Value::Object(o) => {
                        f(o);
                        for (_, x) in o {
                            walk(x, f);
                        }
                    }
                    serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, f)),
                    _ => {}
                }
            }
            walk(&value, &mut collect);
        }
        let mut resolved: Vec<(serde_json::Map<String, serde_json::Value>, Text)> = Vec::new();
        for o in nbt_values.into_inner() {
            let t = if o.contains_key("score") {
                self.resolve_score_component(&o)?
            } else {
                self.resolve_nbt_component(&o)
            };
            resolved.push((o, t));
        }
        let mut resolve = |raw: &str| -> Text {
            let mut reader = horizon_client::command::Reader::new(raw);
            let entities = horizon_client::command::args::ArgType::Entity {
                single: false,
                players: false,
            };
            let Ok(Arg::Entity(sel)) = entities.parse(&mut reader, &self.ui.registries) else {
                return Text::literal(raw);
            };
            let found = self.select(&sel);
            let mut out = Text::empty();
            for (i, t) in found.into_iter().enumerate() {
                if i > 0 {
                    out.push(Text::literal(", "));
                }
                out.push(self.target_name(t));
            }
            out
        };
        Ok(horizon_client::component::to_text_with(
            &value,
            &mut resolve,
            &mut |o| {
                resolved
                    .iter()
                    .find(|(k, _)| k == o)
                    .map(|(_, t)| t.clone())
            },
        ))
    }
    /// The text of a `score` component (ScoreContents.resolve): the named
    /// holder's score, `*` being the player the command runs for.
    fn resolve_score_component(
        &mut self,
        o: &serde_json::Map<String, serde_json::Value>,
    ) -> R<Text> {
        let score = o.get("score");
        let field = |k: &str| score.and_then(|s| s.get(k)).and_then(|v| v.as_str());
        let (name, objective) = (
            field("name").unwrap_or(""),
            field("objective").unwrap_or(""),
        );
        // findTargetName: a selector that matches one entity is that entity,
        // one that matches none is just the name.
        let mut reader = horizon_client::command::Reader::new(name);
        let ty = horizon_client::command::args::ArgType::Entity {
            single: false,
            players: false,
        };
        let mut holder = name.to_owned();
        if let Ok(Arg::Entity(sel)) = ty.parse(&mut reader, &self.ui.registries)
            && !reader.can_read()
        {
            let found = self.select(&sel);
            if found.len() > 1 {
                return Err(err("argument.entity.toomany", &[]));
            }
            if let Some(t) = found.first().and_then(|t| self.holder_name(*t)) {
                holder = t;
            }
        }
        if holder == "*" {
            holder = self.acting_player_name().to_owned();
        }
        let board = &self.score.board;
        let (Some(objective), Some(score)) =
            (board.objective(objective), board.score(&holder, objective))
        else {
            return Ok(Text::empty());
        };
        // numberFormatOrDefault(StyledFormat.NO_STYLE).
        let format = (score.number_format.clone())
            .or_else(|| objective.number_format.clone())
            .unwrap_or(horizon_client::scoreboard::NumberFormat::Styled(
                serde_json::json!({}),
            ));
        Ok(format.format(score.value))
    }
    /// The text of an `nbt` component.
    fn resolve_nbt_component(&mut self, o: &serde_json::Map<String, serde_json::Value>) -> Text {
        use horizon_world::nbt::Tag;
        let path = o.get("nbt").and_then(|v| v.as_str()).unwrap_or("");
        let mut sources: Vec<Tag> = Vec::new();
        if let Some(sel) = o.get("entity").and_then(|v| v.as_str()) {
            let mut reader = horizon_client::command::Reader::new(sel);
            let ty = horizon_client::command::args::ArgType::Entity {
                single: false,
                players: false,
            };
            if let Ok(Arg::Entity(s)) = ty.parse(&mut reader, &self.ui.registries) {
                for t in self.select(&s) {
                    if let Ok(tag) = self.data_of(&super::data::DataRef::Entity(t)) {
                        sources.push(tag);
                    }
                }
            }
        } else if let Some(id) = o.get("storage").and_then(|v| v.as_str()) {
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            sources.push(self.score.storage.get(&id));
        } else if let Some(pos) = o.get("block").and_then(|v| v.as_str()) {
            let p: Vec<i32> = pos
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if let [x, y, z] = p[..]
                && let Ok(tag) = self.data_of(&super::data::DataRef::Block([x, y, z]))
            {
                sources.push(tag);
            }
        }
        let Ok(compiled) = horizon_client::nbt_path::NbtPath::parse(path) else {
            return Text::empty();
        };
        let plain = o.get("plain").and_then(|v| v.as_bool()) == Some(true);
        let interpret = o.get("interpret").and_then(|v| v.as_bool()) == Some(true);
        let mut out = Text::empty();
        let mut first = true;
        for data in &sources {
            for tag in compiled.get(data).unwrap_or_default() {
                if !first {
                    out.push(Text::literal(", "));
                }
                first = false;
                let piece = if interpret {
                    horizon_client::scoreboard::comp_text(&horizon_client::scoreboard::tag_to_json(
                        &tag,
                    ))
                } else if plain {
                    Text::literal(horizon_client::snbt::pretty(&tag).plain())
                } else {
                    horizon_client::snbt::pretty(&tag)
                };
                out.push(piece);
            }
        }
        out
    }
    /// The players the command reaches: the window's own and the LAN guests.
    fn player_targets(&mut self, ctx: &Context) -> R<Vec<Target>> {
        let targets = self.entities(ctx, "targets", true)?;
        Ok(targets
            .into_iter()
            .filter(|t| matches!(t, Target::Player | Target::Guest(_)))
            .collect())
    }
    pub(super) fn cmd_tellraw(&mut self, ctx: &Context) -> R<i32> {
        let players = self.player_targets(ctx)?;
        let message = self.component_arg(ctx, "message")?;
        // Each player's chat (a guest's through the connection).
        for &t in &players {
            self.for_player(t, |g| g.system_message(message.clone()));
        }
        Ok(players.len() as i32)
    }
    pub(super) fn cmd_title(&mut self, ctx: &Context) -> R<i32> {
        let players = self.player_targets(ctx)?;
        let n = players.len();
        let name = players
            .first()
            .map_or_else(Text::empty, |t| self.target_name(*t));
        let single = n == 1;
        let key = if ctx.has("clear") {
            self.titles_or_packet(
                &players,
                Titles::clear,
                C::ClearTitles(play::Bool1 { value: false }),
            );
            "cleared"
        } else if ctx.has("reset") {
            self.titles_or_packet(
                &players,
                |t| {
                    t.clear();
                    t.reset_times();
                },
                C::ClearTitles(play::Bool1 { value: true }),
            );
            "reset"
        } else if ctx.has("times") {
            let get = |k: &str| match ctx.get(k) {
                Some(Arg::Int(v)) => *v,
                _ => -1,
            };
            let (fade_in, stay, fade_out) = (get("fadeIn"), get("stay"), get("fadeOut"));
            self.titles_or_packet(
                &players,
                |t| t.set_times(fade_in, stay, fade_out),
                C::SetTitlesAnimation(play::TitlesAnimation {
                    fade_in,
                    stay,
                    fade_out,
                }),
            );
            "times"
        } else {
            let text = self.component_arg(ctx, "title")?;
            let sent = play::Text {
                text: component_nbt(&text.component()),
            };
            if ctx.has("subtitle") {
                self.titles_or_packet(&players, |t| t.set_subtitle(text), C::SetSubtitleText(sent));
                "show.subtitle"
            } else if ctx.has("actionbar") {
                self.titles_or_packet(
                    &players,
                    |t| t.set_overlay(text, false),
                    C::SetActionBarText(sent),
                );
                "show.actionbar"
            } else {
                self.titles_or_packet(&players, |t| t.set_title(text), C::SetTitleText(sent));
                "show.title"
            }
        };
        let message = if single {
            Text::translate(&format!("commands.title.{key}.single"), &[name])
        } else {
            Text::translate(
                &format!("commands.title.{key}.multiple"),
                &[Text::literal(n.to_string())],
            )
        };
        self.feedback(message);
        Ok(n as i32)
    }
    /// TitleCommand's `connection.send` for each player: a guest is sent the
    /// packet, the window's own player shows it.
    fn titles_or_packet(&mut self, players: &[Target], local: impl FnOnce(&mut Titles), packet: C) {
        let mut local = Some(local);
        for &t in players {
            match t {
                Target::Player if self.lan_acting.is_some() => {
                    self.guest_packets.push(packet.clone());
                }
                Target::Player => {
                    if let Some(local) = local.take() {
                        local(&mut self.titles);
                    }
                }
                Target::Guest(conn) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::invalid_selector;
    use horizon_client::command::Registries;

    fn check(json: &str) -> Option<String> {
        invalid_selector(&serde_json::from_str(json).unwrap(), &Registries::default())
    }

    #[test]
    fn a_selector_component_is_compiled_when_decoded() {
        assert_eq!(check(r#"{"selector":"@a"}"#), None);
        let bad = check(r#"{"selector":"@a[x"}"#).unwrap();
        assert!(
            bad.starts_with("Invalid selector component: @a[x: "),
            "{bad}"
        );
    }

    #[test]
    fn text_after_the_selector_is_trailing_data() {
        let bad = check(r#"["x",{"extra":[{"selector":"@s foo"}]}]"#).unwrap();
        assert!(
            bad.starts_with("Invalid selector component: @s foo: "),
            "{bad}"
        );
        assert!(
            bad.contains("command.trailing_data") || bad.contains("Trailing"),
            "{bad}"
        );
    }

    #[test]
    fn an_nbt_entity_is_compiled_and_a_score_name_is_not() {
        assert!(check(r#"{"nbt":"a","entity":"@e[type"}"#).is_some());
        assert_eq!(check(r#"{"nbt":"a","storage":"x:y"}"#), None);
        assert_eq!(
            check(r#"{"score":{"name":"@a foo","objective":"o"}}"#),
            None
        );
    }
}
