//! EntitySelectorParser and EntitySelectorOptions: `@e[...]` selectors,
//! player names and UUIDs, with the suggestion state vanilla keeps while
//! parsing (`fillSuggestions`).
use super::args::{
    AdvCondition, GameMode, Registries, Selector, Sort, read_double_bounds, read_float_bounds,
    read_identifier, read_int_bounds, suggest_resources,
};
use super::{CmdResult, CommandError, Reader, SuggestionsBuilder};

/// The option names in vanilla's registration order.
const OPTIONS: [&str; 21] = [
    "name",
    "distance",
    "level",
    "x",
    "y",
    "z",
    "dx",
    "dy",
    "dz",
    "x_rotation",
    "y_rotation",
    "limit",
    "sort",
    "gamemode",
    "team",
    "type",
    "tag",
    "nbt",
    "scores",
    "advancements",
    "predicate",
];
const SELECTOR_KINDS: [(&str, &str); 6] = [
    ("@p", "argument.entity.selector.nearestPlayer"),
    ("@a", "argument.entity.selector.allPlayers"),
    ("@r", "argument.entity.selector.randomPlayer"),
    ("@s", "argument.entity.selector.self"),
    ("@e", "argument.entity.selector.allEntities"),
    ("@n", "argument.entity.selector.nearestEntity"),
];

/// InvertableSetOptionState.
#[derive(Clone, Debug, Default)]
struct InvertableSet {
    /// 0 none, 1 single, 2 multiple.
    state: u8,
    tags: Vec<String>,
}
impl InvertableSet {
    fn can_parse_positive(&self) -> bool {
        self.state == 0
    }
    fn can_parse_negative(&self) -> bool {
        self.state != 1
    }
    fn can_parse_any_tag(&self) -> bool {
        self.can_parse_negative()
    }
    fn can_parse_tag(&self, tag: &str) -> bool {
        self.can_parse_any_tag() && !self.tags.iter().any(|t| t == tag)
    }
    fn can_parse_any(&self) -> bool {
        self.state != 1
    }
    fn can_parse_element(&self, inverted: bool) -> bool {
        if inverted {
            self.can_parse_negative()
        } else {
            self.can_parse_positive()
        }
    }
    fn mark_element(&mut self, inverted: bool) {
        self.state = if inverted { 2 } else { 1 };
    }
    fn mark_tag(&mut self, tag: String) {
        self.state = 2;
        self.tags.push(tag);
    }
}

/// The suggestion function the parser last set.
#[derive(Clone, Debug)]
enum Sugg {
    Nothing,
    NameOrSelector,
    Name,
    Selector,
    OpenOptions,
    OptionsKeyOrClose,
    OptionsKey,
    OptionsNextOrClose,
    Sort,
    GameMode,
    Type,
}

/// EntitySelectorParser over a reader.
pub struct SelectorParser<'r, 'a> {
    pub reader: &'r mut Reader<'a>,
    reg: &'r Registries,
    allow_selectors: bool,
    s: Selector,
    sugg: Sugg,
    start_position: usize,
    current_entity: bool,
    has_distance: bool,
    has_level: bool,
    has_rot_x: bool,
    has_rot_y: bool,
    name_option: InvertableSet,
    limited: bool,
    sorted: bool,
    gamemode_option: InvertableSet,
    team_option: InvertableSet,
    type_option: InvertableSet,
    scores_option: bool,
    advancements_option: bool,
}

/// UUID.fromString.
pub fn java_uuid(name: &str) -> Option<u128> {
    if name.encode_utf16().count() > 36 {
        return None;
    }
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let mut v: [i64; 5] = [0; 5];
    for (i, p) in parts.iter().enumerate() {
        // Long.parseLong(s, 16): an optional sign then hex digits.
        let digits = p.strip_prefix(['+', '-']).unwrap_or(p);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        v[i] = i64::from_str_radix(p, 16).ok()?;
    }
    let msb = (((v[0] as u64) & 0xffff_ffff) << 32)
        | (((v[1] as u64) & 0xffff) << 16)
        | ((v[2] as u64) & 0xffff);
    let lsb = (((v[3] as u64) & 0xffff) << 48) | ((v[4] as u64) & 0xffff_ffff_ffff);
    Some((u128::from(msb) << 64) | u128::from(lsb))
}

fn rollback(
    reader: &mut Reader,
    start: usize,
    key: &str,
    args: &[&dyn std::fmt::Display],
) -> CommandError {
    reader.cursor = start;
    CommandError::tr_at(key, args, reader)
}

impl<'r, 'a> SelectorParser<'r, 'a> {
    pub fn new(reader: &'r mut Reader<'a>, reg: &'r Registries, allow_selectors: bool) -> Self {
        Self {
            reader,
            reg,
            allow_selectors,
            s: Selector::default(),
            sugg: Sugg::Nothing,
            start_position: 0,
            current_entity: false,
            has_distance: false,
            has_level: false,
            has_rot_x: false,
            has_rot_y: false,
            name_option: InvertableSet::default(),
            limited: false,
            sorted: false,
            gamemode_option: InvertableSet::default(),
            team_option: InvertableSet::default(),
            type_option: InvertableSet::default(),
            scores_option: false,
            advancements_option: false,
        }
    }

    /// EntitySelectorParser.parse.
    pub fn parse(&mut self) -> CmdResult<Selector> {
        self.start_position = self.reader.cursor;
        self.sugg = Sugg::NameOrSelector;
        if self.reader.can_read() && self.reader.peek() == '@' {
            if !self.allow_selectors {
                return Err(CommandError::tr_at(
                    "argument.entity.selector.not_allowed",
                    &[],
                    self.reader,
                ));
            }
            self.reader.skip();
            self.parse_selector()?;
        } else {
            self.parse_name_or_uuid()?;
        }
        Ok(self.s.clone())
    }

    fn parse_selector(&mut self) -> CmdResult<()> {
        self.sugg = Sugg::Selector;
        if !self.reader.can_read() {
            return Err(CommandError::tr_at(
                "argument.entity.selector.missing",
                &[],
                self.reader,
            ));
        }
        let start = self.reader.cursor;
        let kind = self.reader.read();
        let s = &mut self.s;
        s.kind = kind;
        match kind {
            'a' => {
                s.limit = usize::MAX;
                s.include_entities = false;
                s.types.push(("minecraft:player".into(), false));
            }
            'e' => {
                s.limit = usize::MAX;
                s.include_entities = true;
            }
            'n' => {
                s.limit = 1;
                s.include_entities = true;
                s.sort = Sort::Nearest;
            }
            'p' => {
                s.limit = 1;
                s.include_entities = false;
                s.sort = Sort::Nearest;
                s.types.push(("minecraft:player".into(), false));
            }
            'r' => {
                s.limit = 1;
                s.include_entities = false;
                s.sort = Sort::Random;
                s.types.push(("minecraft:player".into(), false));
            }
            's' => {
                s.limit = 1;
                s.include_entities = true;
                self.current_entity = true;
            }
            other => {
                self.reader.cursor = start;
                return Err(CommandError::tr_at(
                    "argument.entity.selector.unknown",
                    &[&format!("@{other}")],
                    self.reader,
                ));
            }
        }
        self.sugg = Sugg::OpenOptions;
        if self.reader.can_read() && self.reader.peek() == '[' {
            self.reader.skip();
            self.sugg = Sugg::OptionsKeyOrClose;
            self.parse_options()?;
        }
        Ok(())
    }

    fn parse_name_or_uuid(&mut self) -> CmdResult<()> {
        if self.reader.can_read() {
            self.sugg = Sugg::Name;
        }
        let start = self.reader.cursor;
        let name = self.reader.read_string()?;
        // A UUID selects by entityUUID alone: findEntities looks a
        // playerName up first, among the players only.
        if let Some(uuid) = java_uuid(&name) {
            self.s.uuid = Some(uuid);
            self.s.include_entities = true;
        } else {
            if name.is_empty() || name.encode_utf16().count() > 16 {
                self.reader.cursor = start;
                return Err(CommandError::tr_at(
                    "argument.entity.invalid",
                    &[],
                    self.reader,
                ));
            }
            self.s.include_entities = false;
            self.s.player_name = Some(name);
        }
        self.s.kind = '\0';
        self.s.limit = 1;
        Ok(())
    }

    fn can_use(&self, option: &str) -> bool {
        match option {
            "name" => self.name_option.can_parse_any(),
            "distance" => !self.has_distance,
            "level" => !self.has_level,
            "x" => self.s.pos[0].is_none(),
            "y" => self.s.pos[1].is_none(),
            "z" => self.s.pos[2].is_none(),
            "dx" => self.s.delta[0].is_none(),
            "dy" => self.s.delta[1].is_none(),
            "dz" => self.s.delta[2].is_none(),
            "x_rotation" => !self.has_rot_x,
            "y_rotation" => !self.has_rot_y,
            "limit" => !self.current_entity && !self.limited,
            "sort" => !self.current_entity && !self.sorted,
            "gamemode" => self.gamemode_option.can_parse_any(),
            "team" => self.team_option.can_parse_any(),
            "type" => self.type_option.can_parse_any(),
            "scores" => !self.scores_option,
            "advancements" => !self.advancements_option,
            _ => true,
        }
    }

    fn parse_options(&mut self) -> CmdResult<()> {
        self.sugg = Sugg::OptionsKey;
        self.reader.skip_whitespace();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            let start = self.reader.cursor;
            let key = self.reader.read_string()?;
            if !OPTIONS.contains(&key.as_str()) {
                return Err(rollback(
                    self.reader,
                    start,
                    "argument.entity.options.unknown",
                    &[&key],
                ));
            }
            if !self.can_use(&key) {
                return Err(rollback(
                    self.reader,
                    start,
                    "argument.entity.options.inapplicable",
                    &[&key],
                ));
            }
            self.reader.skip_whitespace();
            if !self.reader.can_read() || self.reader.peek() != '=' {
                return Err(rollback(
                    self.reader,
                    start,
                    "argument.entity.options.valueless",
                    &[&key],
                ));
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            self.sugg = Sugg::Nothing;
            self.handle(&key)?;
            self.reader.skip_whitespace();
            self.sugg = Sugg::OptionsNextOrClose;
            if self.reader.can_read() {
                if self.reader.peek() != ',' {
                    if self.reader.peek() != ']' {
                        return Err(CommandError::tr_at(
                            "argument.entity.options.unterminated",
                            &[],
                            self.reader,
                        ));
                    }
                    break;
                }
                self.reader.skip();
                self.sugg = Sugg::OptionsKey;
            }
        }
        if self.reader.can_read() {
            self.reader.skip();
            self.sugg = Sugg::Nothing;
            Ok(())
        } else {
            Err(CommandError::tr_at(
                "argument.entity.options.unterminated",
                &[],
                self.reader,
            ))
        }
    }

    fn should_invert(&mut self) -> bool {
        self.reader.skip_whitespace();
        if self.reader.can_read() && self.reader.peek() == '!' {
            self.reader.skip();
            self.reader.skip_whitespace();
            true
        } else {
            false
        }
    }
    fn is_tag(&mut self) -> bool {
        self.reader.skip_whitespace();
        if self.reader.can_read() && self.reader.peek() == '#' {
            self.reader.skip();
            self.reader.skip_whitespace();
            true
        } else {
            false
        }
    }

    fn handle(&mut self, key: &str) -> CmdResult<()> {
        match key {
            "name" => {
                let start = self.reader.cursor;
                let inverted = self.should_invert();
                let name = self.reader.read_string()?;
                if !self.name_option.can_parse_element(inverted) {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.inapplicable",
                        &[&"name"],
                    ));
                }
                self.name_option.mark_element(inverted);
                self.s.names.push((name, inverted));
            }
            "distance" => {
                let start = self.reader.cursor;
                let value = read_double_bounds(self.reader)?;
                if value.0.is_some_and(|v| v < 0.0) || value.1.is_some_and(|v| v < 0.0) {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.distance.negative",
                        &[],
                    ));
                }
                self.s.distance = Some(value);
                self.has_distance = true;
            }
            "level" => {
                let start = self.reader.cursor;
                let value = read_int_bounds(self.reader)?;
                if value.0.is_some_and(|v| v < 0) || value.1.is_some_and(|v| v < 0) {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.level.negative",
                        &[],
                    ));
                }
                self.s.level = Some(value);
                self.has_level = true;
                self.s.include_entities = false;
            }
            "x" | "y" | "z" => {
                let i = usize::from(key.as_bytes()[0] - b'x');
                self.s.pos[i] = Some(self.reader.read_double()?);
            }
            "dx" | "dy" | "dz" => {
                let i = usize::from(key.as_bytes()[1] - b'x');
                self.s.delta[i] = Some(self.reader.read_double()?);
            }
            "x_rotation" | "y_rotation" => {
                let (lo, hi) = read_float_bounds(self.reader)?;
                let range = (lo.map(f64::from), hi.map(f64::from));
                if key == "x_rotation" {
                    self.s.x_rotation = Some(range);
                    self.has_rot_x = true;
                } else {
                    self.s.y_rotation = Some(range);
                    self.has_rot_y = true;
                }
            }
            "limit" => {
                let start = self.reader.cursor;
                let count = self.reader.read_int()?;
                if count < 1 {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.limit.toosmall",
                        &[],
                    ));
                }
                self.s.limit = count as usize;
                self.limited = true;
            }
            "sort" => {
                let start = self.reader.cursor;
                let name = self.reader.read_unquoted_string();
                self.sugg = Sugg::Sort;
                self.s.sort = match name {
                    "nearest" => Sort::Nearest,
                    "furthest" => Sort::Furthest,
                    "random" => Sort::Random,
                    "arbitrary" => Sort::Arbitrary,
                    _ => {
                        return Err(rollback(
                            self.reader,
                            start,
                            "argument.entity.options.sort.irreversible",
                            &[&name],
                        ));
                    }
                };
                self.sorted = true;
            }
            "gamemode" => {
                self.sugg = Sugg::GameMode;
                let start = self.reader.cursor;
                let inverted = self.should_invert();
                if !self.gamemode_option.can_parse_element(inverted) {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.inapplicable",
                        &[&"gamemode"],
                    ));
                }
                let name = self.reader.read_unquoted_string();
                if GameMode::from_name(name).is_none() {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.mode.invalid",
                        &[&name],
                    ));
                }
                self.s.include_entities = false;
                self.s.gamemodes.push((name.to_owned(), inverted));
                self.gamemode_option.mark_element(inverted);
            }
            "team" => {
                let start = self.reader.cursor;
                let inverted = self.should_invert();
                let name = self.reader.read_unquoted_string().to_owned();
                if !self.team_option.can_parse_element(inverted) {
                    return Err(rollback(
                        self.reader,
                        start,
                        "argument.entity.options.inapplicable",
                        &[&"team"],
                    ));
                }
                self.s.teams.push((name, inverted));
                self.team_option.mark_element(inverted);
            }
            "type" => {
                self.sugg = Sugg::Type;
                let start = self.reader.cursor;
                let inverted = self.should_invert();
                if self.is_tag() {
                    if !self.type_option.can_parse_any_tag() {
                        return Err(rollback(
                            self.reader,
                            start,
                            "argument.entity.options.inapplicable",
                            &[&"type"],
                        ));
                    }
                    let id = read_identifier(self.reader)?;
                    if !self.type_option.can_parse_tag(&id) {
                        return Err(rollback(
                            self.reader,
                            start,
                            "argument.entity.options.inapplicable",
                            &[&"type"],
                        ));
                    }
                    self.s.types.push((format!("#{id}"), inverted));
                    self.type_option.mark_tag(id);
                } else {
                    if !self.type_option.can_parse_element(inverted) {
                        return Err(rollback(
                            self.reader,
                            start,
                            "argument.entity.options.inapplicable",
                            &[&"type"],
                        ));
                    }
                    let id = read_identifier(self.reader)?;
                    let types = self.reg.ids("minecraft:entity_type");
                    if !types.is_empty() && !types.contains(&id) {
                        return Err(rollback(
                            self.reader,
                            start,
                            "argument.entity.options.type.invalid",
                            &[&id],
                        ));
                    }
                    if id == "minecraft:player" && !inverted {
                        self.s.include_entities = false;
                    }
                    if !inverted {
                        // limitToType replaces the player type of @a/@p/@r.
                        self.s.types.retain(|(t, n)| *n || t.starts_with('#'));
                    }
                    self.s.types.push((id, inverted));
                    self.type_option.mark_element(inverted);
                }
            }
            "tag" => {
                let inverted = self.should_invert();
                let tag = self.reader.read_unquoted_string().to_owned();
                self.s.tags.push((tag, inverted));
            }
            "nbt" => {
                let inverted = self.should_invert();
                let start = self.reader.cursor;
                super::snbt::parse_compound_argument(self.reader)?;
                let text = self.reader.string[start..self.reader.cursor].to_owned();
                self.s.nbt.push((text, inverted));
            }
            "scores" => {
                let r = &mut *self.reader;
                r.expect('{')?;
                r.skip_whitespace();
                let mut expected = Vec::new();
                while r.can_read() && r.peek() != '}' {
                    r.skip_whitespace();
                    let name = r.read_unquoted_string().to_owned();
                    r.skip_whitespace();
                    r.expect('=')?;
                    r.skip_whitespace();
                    let value = read_int_bounds(r)?;
                    expected.push((name, value));
                    r.skip_whitespace();
                    if r.can_read() && r.peek() == ',' {
                        r.skip();
                    }
                }
                r.expect('}')?;
                self.s.scores.extend(expected);
                self.scores_option = true;
            }
            "advancements" => {
                let r = &mut *self.reader;
                r.expect('{')?;
                r.skip_whitespace();
                let mut expected = Vec::new();
                while r.can_read() && r.peek() != '}' {
                    r.skip_whitespace();
                    let name = read_identifier(r)?;
                    r.skip_whitespace();
                    r.expect('=')?;
                    r.skip_whitespace();
                    if r.can_read() && r.peek() == '{' {
                        let mut progress = Vec::new();
                        r.skip_whitespace();
                        r.expect('{')?;
                        r.skip_whitespace();
                        while r.can_read() && r.peek() != '}' {
                            r.skip_whitespace();
                            let criterion = r.read_unquoted_string().to_owned();
                            r.skip_whitespace();
                            r.expect('=')?;
                            r.skip_whitespace();
                            let value = r.read_boolean()?;
                            progress.push((criterion, value));
                            r.skip_whitespace();
                            if r.can_read() && r.peek() == ',' {
                                r.skip();
                            }
                        }
                        r.skip_whitespace();
                        r.expect('}')?;
                        r.skip_whitespace();
                        expected.push((name, AdvCondition::Criteria(progress)));
                    } else {
                        let value = r.read_boolean()?;
                        expected.push((name, AdvCondition::Done(value)));
                    }
                    r.skip_whitespace();
                    if r.can_read() && r.peek() == ',' {
                        r.skip();
                    }
                }
                r.expect('}')?;
                if !expected.is_empty() {
                    self.s.include_entities = false;
                }
                self.s.advancements.extend(expected);
                self.advancements_option = true;
            }
            "predicate" => {
                let inverted = self.should_invert();
                let id = read_identifier(self.reader)?;
                self.s.predicates.push((id, inverted));
            }
            _ => {}
        }
        Ok(())
    }

    fn fill_selectors(builder: &mut SuggestionsBuilder) {
        for (s, tip) in SELECTOR_KINDS {
            builder.suggest_tooltip(s, horizon_client_core::lang::get(tip));
        }
    }

    /// EntitySelectorParser.fillSuggestions: `names` adds the name
    /// candidates (online players for most arguments).
    pub fn fill_suggestions(
        &self,
        builder: &mut SuggestionsBuilder,
        names: &dyn Fn(&mut SuggestionsBuilder),
    ) {
        let mut b = builder.offset(self.reader.cursor);
        match self.sugg {
            Sugg::Nothing => {}
            Sugg::NameOrSelector => {
                names(&mut b);
                if self.allow_selectors {
                    Self::fill_selectors(&mut b);
                }
            }
            Sugg::Name => {
                let mut sub = b.offset(self.start_position);
                names(&mut sub);
                b.add(sub);
            }
            Sugg::Selector => {
                let mut sub = b.offset(b.start - 1);
                Self::fill_selectors(&mut sub);
                b.add(sub);
            }
            Sugg::OpenOptions => b.suggest("["),
            Sugg::OptionsKeyOrClose => {
                b.suggest("]");
                self.suggest_names(&mut b);
            }
            Sugg::OptionsKey => self.suggest_names(&mut b),
            Sugg::OptionsNextOrClose => {
                b.suggest(",");
                b.suggest("]");
            }
            Sugg::Sort => b.suggest_matching(["nearest", "furthest", "random", "arbitrary"]),
            Sugg::GameMode => {
                let state = &self.gamemode_option;
                let mut prefix = b.remaining_lower();
                let mut add_normal = state.can_parse_positive();
                let mut add_inverted = state.can_parse_negative();
                if !prefix.is_empty() {
                    if prefix.starts_with('!') {
                        add_normal = false;
                        prefix.remove(0);
                    } else {
                        add_inverted = false;
                    }
                }
                for mode in GameMode::ALL {
                    if mode.name().starts_with(&prefix) {
                        if add_inverted {
                            b.suggest(format!("!{}", mode.name()));
                        }
                        if add_normal {
                            b.suggest(mode.name());
                        }
                    }
                }
            }
            Sugg::Type => {
                let state = &self.type_option;
                let types = self.reg.ids("minecraft:entity_type");
                if state.can_parse_negative() {
                    suggest_resources(&mut b, types, "!");
                }
                if state.can_parse_positive() {
                    suggest_resources(&mut b, types, "");
                }
                if state.can_parse_any_tag() {
                    let tags: Vec<String> = self
                        .reg
                        .tags("minecraft:entity_type")
                        .iter()
                        .filter(|t| state.can_parse_tag(t))
                        .cloned()
                        .collect();
                    if !tags.is_empty() {
                        suggest_resources(&mut b, &tags, "#");
                        suggest_resources(&mut b, &tags, "!#");
                    }
                }
            }
        }
        builder.add(b);
    }

    fn suggest_names(&self, b: &mut SuggestionsBuilder) {
        let lower = b.remaining_lower();
        for o in OPTIONS {
            if self.can_use(o) && o.starts_with(&lower) {
                b.suggest_tooltip(
                    format!("{o}="),
                    horizon_client_core::lang::get(&format!(
                        "argument.entity.options.{o}.description"
                    )),
                );
            }
        }
    }
}

/// Parse a selector from `reader` (EntitySelectorParser.parse).
pub fn parse(reader: &mut Reader, reg: &Registries, allow_selectors: bool) -> CmdResult<Selector> {
    SelectorParser::new(reader, reg, allow_selectors).parse()
}

/// Re-parse the text from `builder.start` and fill the selector suggestions.
pub fn suggest(
    builder: &mut SuggestionsBuilder,
    reg: &Registries,
    names: &dyn Fn(&mut SuggestionsBuilder),
) {
    let mut reader = Reader {
        string: builder.input,
        cursor: builder.start,
    };
    let mut parser = SelectorParser::new(&mut reader, reg, reg.permission >= 2);
    let _ = parser.parse();
    parser.fill_suggestions(builder, names);
}
