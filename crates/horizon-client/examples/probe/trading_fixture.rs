//! Trading for tools/test_trading_parity.py: runs each case of INPUT as
//! VanillaTradingOracle does (offer generation, special prices, restocks,
//! the merchant menu, NBT) and prints the same lines.
//!
//! usage: trading_fixture RESOURCES INPUT VANILLA_OUTPUT OUTPUT
use horizon_client::{
    enchant::Enchantments,
    entity_nbt,
    gossip::{GossipType, Gossips},
    items::{ItemStack, Items},
    menu::{Env, Input, Kind, Menu},
    merchant::{self, Event, MerchantOffer, Session, TradeContext, TradeEnv, Trader, Trades},
    random_sequences::RandomSequences,
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::loot::PackOverlay;
use horizon_world::nbt::Tag;
use std::{collections::HashMap, fmt::Write as _, fs, path::Path};

const PLAYER: [i32; 4] = [0, 0x1234, 0, 0x5678];

/// Canonical NBT text, as VanillaTradingOracle prints it.
fn canon(t: &Tag, out: &mut String) {
    match t {
        Tag::Compound(c) => {
            out.push('{');
            let mut keys: Vec<String> = c.keys().map(|k| k.to_string_lossy()).collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                quote(k, out);
                out.push(':');
                canon(t.get(k).expect("key"), out);
            }
            out.push('}');
        }
        Tag::List(l) => {
            out.push('[');
            for (i, v) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon(v, out);
            }
            out.push(']');
        }
        Tag::String(s) => quote(&s.to_string_lossy(), out),
        Tag::Byte(v) => write!(out, "{v}b").unwrap(),
        Tag::Short(v) => write!(out, "{v}s").unwrap(),
        Tag::Int(v) => write!(out, "{v}").unwrap(),
        Tag::Long(v) => write!(out, "{v}L").unwrap(),
        Tag::Float(v) => write!(out, "f:{v:x}").unwrap(),
        Tag::Double(v) => write!(out, "d:{v:x}").unwrap(),
        Tag::ByteArray(v) => {
            out.push_str("[B;");
            let s: Vec<String> = v.iter().map(|b| (*b as i8).to_string()).collect();
            out.push_str(&s.join(","));
            out.push(']');
        }
        Tag::IntArray(v) => {
            out.push_str("[I;");
            let s: Vec<String> = v.iter().map(i32::to_string).collect();
            out.push_str(&s.join(","));
            out.push(']');
        }
        Tag::LongArray(v) => {
            out.push_str("[L;");
            let s: Vec<String> = v.iter().map(i64::to_string).collect();
            out.push_str(&s.join(","));
            out.push(']');
        }
        Tag::End => out.push_str("?End"),
    }
}
fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}
fn canon_str(t: &Tag) -> String {
    let mut s = String::new();
    canon(t, &mut s);
    s
}
fn stack(s: Option<&ItemStack>) -> String {
    match s {
        None => "-".into(),
        Some(s) => canon_str(&horizon_client::save::item_to_nbt(s)),
    }
}

struct World<'a> {
    items: &'a Items,
    enchantments: &'a Enchantments,
    trades: &'a Trades,
    sequences: RandomSequences,
    seed: i64,
}
/// A merchant from PROF:TYPE:LEVEL or "wt", generated level by level.
struct Merchant {
    villager: bool,
    profession: String,
    variant: String,
    level: i32,
    trader: Trader,
}
impl World<'_> {
    fn reset(&mut self, seed: i64) {
        self.sequences = RandomSequences::default();
        self.seed = seed;
    }
    fn add_set(
        &mut self,
        offers: &mut Vec<MerchantOffer>,
        set: &str,
        variant: &str,
        origin: [f64; 3],
    ) {
        let Some(sequence) = self.trades.sequence(set) else {
            return;
        };
        let ctx = TradeContext {
            items: self.items,
            enchantments: self.enchantments,
            variant,
            origin,
            maps: None,
        };
        let random = self.sequences.get(&sequence, self.seed);
        offers.extend(self.trades.offers_from_set(&ctx, set, random));
    }
    fn merchant(&mut self, spec: &str, origin: [f64; 3]) -> Merchant {
        let mut offers = Vec::new();
        if spec == "wt" {
            for set in ["buying", "uncommon", "common"] {
                self.add_set(
                    &mut offers,
                    &format!("minecraft:wandering_trader/{set}"),
                    "minecraft:plains",
                    origin,
                );
            }
            return Merchant {
                villager: false,
                profession: "minecraft:wandering_trader".into(),
                variant: "minecraft:plains".into(),
                level: 1,
                trader: Trader {
                    offers,
                    ..Default::default()
                },
            };
        }
        let p: Vec<&str> = spec.split(':').collect();
        let (profession, variant) = (format!("minecraft:{}", p[0]), format!("minecraft:{}", p[1]));
        let max: i32 = p[2].parse().unwrap_or(1);
        for level in 1..=max {
            let set = Trades::set_for(&profession, level);
            if self.trades.sets.contains_key(&set) {
                self.add_set(&mut offers, &set, &variant, origin);
            }
        }
        Merchant {
            villager: true,
            profession,
            variant,
            level: max,
            trader: Trader {
                offers,
                ..Default::default()
            },
        }
    }
}

fn kv(fields: &[&str]) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    for f in &fields[2..] {
        if let Some((k, v)) = f.split_once('=') {
            m.entry(k.to_owned())
                .and_modify(|e| {
                    e.push(' ');
                    e.push_str(v);
                })
                .or_insert_with(|| v.to_owned());
        }
    }
    m
}

/// Uses, demand, gossip, xp and restock state, as the oracle's `prepare`.
fn prepare(m: &mut Merchant, gossips: &mut Gossips, kv: &HashMap<String, String>) {
    for key in ["uses", "demand"] {
        let Some(list) = kv.get(key) else { continue };
        for s in list.split(' ') {
            let mut f = s.split(':');
            let i: usize = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let n: i32 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let Some(o) = m.trader.offers.get_mut(i) else {
                continue;
            };
            if key == "uses" {
                o.uses += n;
            } else {
                o.demand = n;
            }
        }
    }
    if let Some(list) = kv.get("gossip") {
        for s in list.split(' ') {
            let (kind, n) = s.split_once(':').unwrap_or((s, "0"));
            if let Some(kind) = GossipType::from_id(kind) {
                gossips.add(PLAYER, kind, n.parse().unwrap_or(0));
            }
        }
    }
    if let Some(xp) = kv.get("xp") {
        m.trader.xp = xp.parse().unwrap_or(0);
    }
    if let Some(v) = kv.get("lastrestock") {
        m.trader.last_restock = v.parse().unwrap_or(0);
    }
    if let Some(v) = kv.get("restocks") {
        m.trader.restocks_today = v.parse().unwrap_or(0);
    }
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: trading_fixture RESOURCES INPUT VANILLA_OUTPUT OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let input = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let vanilla = fs::read_to_string(args.get(3).ok_or(usage)?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let enchantments = Enchantments::load(root);
    let names = horizon_client::creative::Names::default();
    let trades = Trades::load(root);
    let recipes = horizon_client::crafting::Recipes::load(root, &items, &PackOverlay::NONE)?;
    let brewing = horizon_client::brewing::Brewing::load(root, &items, &PackOverlay::NONE);
    let banners = horizon_client::banner::Banners::load(root);
    let station_recipes = horizon_client::stations::StationRecipes::default();
    let types: HashMap<_, _> = horizon_client::mobs::MobTypes::load(root)?
        .types
        .into_iter()
        .map(|(k, v)| (k, std::sync::Arc::new(v)))
        .collect();
    // The vanilla villagers the nbt cases read.
    let snbt: HashMap<&str, &str> = vanilla
        .lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(' ')?;
            Some((name, rest.strip_prefix("snbt ")?))
        })
        .collect();
    let mut world = World {
        items: &items,
        enchantments: &enchantments,
        trades: &trades,
        sequences: RandomSequences::default(),
        seed: 0,
    };
    let mut out = String::new();
    for line in input.lines().filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split(' ').collect();
        let (kind, name) = (fields[0], fields[1]);
        let kv = kv(&fields);
        let seed: i64 = kv.get("seed").and_then(|s| s.parse().ok()).unwrap_or(0);
        world.reset(seed);
        let origin = [0.5, -60.0, 0.5];
        match kind {
            "gen" => {
                let specs: Vec<String> = kv["v"].split(' ').map(str::to_owned).collect();
                for (k, spec) in specs.iter().enumerate() {
                    let m = world.merchant(spec, origin);
                    writeln!(
                        out,
                        "{name} v{k} {}",
                        canon_str(&entity_nbt::offers_to_nbt(&m.trader.offers))
                    )?;
                }
                let mut line = format!("{name} draws");
                for id in trades.sequences() {
                    write!(line, " {}", world.sequences.get(&id, seed).next_int())?;
                }
                writeln!(out, "{line}")?;
            }
            "price" => {
                let mut m = world.merchant(&kv["v"], origin);
                let mut gossips = Gossips::default();
                prepare(&mut m, &mut gossips, &kv);
                let hero = kv.get("hero").and_then(|h| h.parse().ok());
                let rep = gossips.reputation(PLAYER);
                merchant::update_special_prices(&mut m.trader.offers, rep, hero);
                let mut line = format!("{name} rep {rep}");
                for o in &m.trader.offers {
                    write!(line, " {}/{}", o.cost_a_count(&items), o.special_price_diff)?;
                }
                writeln!(out, "{line}")?;
            }
            "restock" => {
                let mut m = world.merchant(&kv["v"], origin);
                let mut gossips = Gossips::default();
                prepare(&mut m, &mut gossips, &kv);
                let steps = kv.get("steps").cloned().unwrap_or_default();
                for (k, s) in steps.split(',').filter(|s| !s.is_empty()).enumerate() {
                    let f: Vec<&str> = s.split(':').collect();
                    let mut line = format!("{name} {k} {s}");
                    match f[0] {
                        "use" => {
                            let i: usize = f[1].parse().unwrap_or(0);
                            if let Some(o) = m.trader.offers.get_mut(i) {
                                o.uses += f[2].parse::<i32>().unwrap_or(0);
                            }
                        }
                        "check" => {
                            let time: i64 = f[1].parse().unwrap_or(0);
                            let day: i64 = f[2].parse().unwrap_or(0);
                            let should = m.trader.should_restock(time, day);
                            if should {
                                m.trader.restock(time);
                            }
                            write!(line, " {should}")?;
                        }
                        _ => {}
                    }
                    write!(
                        line,
                        " lr {} rt {} cd {}",
                        m.trader.last_restock, m.trader.restocks_today, m.trader.check_day
                    )?;
                    for o in &m.trader.offers {
                        write!(line, " {}/{}", o.uses, o.demand)?;
                    }
                    writeln!(out, "{line}")?;
                }
            }
            "nbt" => {
                let text = snbt.get(name).ok_or("missing vanilla snbt")?;
                let (tag, _) = horizon_world::nbt::parse_snbt(text)?;
                let id = if kv["v"] == "wt" {
                    "minecraft:wandering_trader"
                } else {
                    "minecraft:villager"
                };
                let mut tag = tag;
                if let Tag::Compound(c) = &mut tag {
                    c.insert("id".into(), Tag::String(id.into()));
                }
                let mob = entity_nbt::mob_from_nbt(&tag, &types, 1).ok_or("unreadable villager")?;
                let back = entity_nbt::mob_to_nbt(&mob, [0; 4], None);
                writeln!(out, "{name} snbt {text}")?;
                let mut kept = horizon_world::nbt::Compound::new();
                for key in [
                    "Offers",
                    "Xp",
                    "LastRestock",
                    "RestocksToday",
                    "VillagerData",
                ] {
                    if let Some(v) = back.get(key) {
                        kept.insert(key.into(), v.clone());
                    }
                }
                writeln!(out, "{name} canon {}", canon_str(&Tag::Compound(kept)))?;
            }
            "trade" => {
                let env_trade = TradeEnv {
                    trades: &trades,
                    enchantments: &enchantments,
                    maps: None,
                };
                let env = Env {
                    items: &items,
                    recipes: &recipes,
                    brewing: &brewing,
                    banners: &banners,
                    station_recipes: &station_recipes,
                    enchantments: &enchantments,
                    names: &names,
                    trade: Some(&env_trade),
                    maps: None,
                };
                trade(&mut out, name, &kv, &mut world, &env, origin)?;
            }
            _ => return Err(format!("unknown case {kind}").into()),
        }
    }
    fs::write(args.get(4).ok_or(usage)?, out)?;
    Ok(())
}

fn trade(
    out: &mut String,
    name: &str,
    kv: &HashMap<String, String>,
    world: &mut World,
    env: &Env,
    origin: [f64; 3],
) -> horizon_client::Result<()> {
    let vseed: i64 = kv.get("vseed").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut m = world.merchant(&kv["v"], origin);
    let mut gossips = Gossips::default();
    if m.villager {
        prepare(&mut m, &mut gossips, kv);
    }
    let hero: Option<i32> = kv.get("hero").and_then(|h| h.parse().ok());
    let mut menu = Menu::new(Kind::Merchant);
    if let Some(inv) = kv.get("inv") {
        for s in inv.split(',') {
            let (slot, rest) = s.split_once(':').ok_or("inv")?;
            let (item, count) = rest.rsplit_once('*').ok_or("inv")?;
            let item = if item.contains(':') {
                item.to_owned()
            } else {
                format!("minecraft:{item}")
            };
            menu.set_player_item(slot.parse()?, Some(ItemStack::new(&item, count.parse()?)));
        }
    }
    menu.selected = kv.get("hand").and_then(|h| h.parse().ok()).unwrap_or(8);
    // Villager.mobInteract / WanderingTrader.mobInteract.
    let open = !m.trader.offers.is_empty();
    let unhappy = if m.villager && !open { 40 } else { 0 };
    let mut voice = LegacyRandom::new(vseed);
    if unhappy > 0 {
        // setUnhappy: makeSound(VILLAGER_NO) draws the voice pitch.
        voice.next_float();
        voice.next_float();
    }
    writeln!(out, "{name} open {open} unhappy {unhappy}")?;
    let mut session = Session::new(m.trader.offers.clone(), m.villager);
    session.xp = m.trader.xp;
    session.level = m.level;
    session.profession = m.profession.clone();
    session.variant = m.variant.clone();
    session.random = voice.clone();
    session.origin = origin;
    session.reputation = gossips.reputation(PLAYER);
    session.hero = hero;
    for id in Session::sequences_needed(world.trades, &m.profession, m.level) {
        let r = world.sequences.lend(&id, world.seed);
        session.sequences.insert(id, r);
    }
    session.update_special_prices();
    let mut regen = false;
    let mut open_menu = open;
    if open_menu {
        menu.merchant = Some(Box::new(session));
    }
    let steps = kv.get("steps").cloned().unwrap_or_default();
    let mut session_after: Option<Box<Session>> = None;
    for (k, s) in steps.split(',').filter(|s| !s.is_empty()).enumerate() {
        let f: Vec<&str> = s.split(':').collect();
        match f[0] {
            "select" if open_menu => menu.merchant_select(env, f[1].parse().unwrap_or(0)),
            "click" if open_menu => {
                let input = Input::parse(&f[3].to_uppercase()).ok_or("input")?;
                let _ = menu.click(env, f[1].parse()?, f[2].parse()?, input);
            }
            "tick" => {
                let sess = menu.merchant.as_mut().or(session_after.as_mut());
                if let Some(sess) = sess
                    && sess.villager
                    && sess.last_traded
                {
                    sess.last_traded = false;
                    gossips.add(PLAYER, GossipType::Trading, 2);
                    sess.reputation = gossips.reputation(PLAYER);
                    if open_menu {
                        sess.update_special_prices();
                        menu.merchant_update(env);
                    }
                }
            }
            "close" if open_menu => {
                menu.merchant_close(env);
                open_menu = false;
                session_after = Some(menu.merchant.take().ok_or("session")?);
            }
            _ => {}
        }
        let mut line = format!("{name} {k} {s} menu {open_menu}");
        if open_menu {
            for i in 0..3 {
                write!(line, " {}", stack(menu.slot(i)))?;
            }
            write!(
                line,
                " fxp {}",
                menu.merchant.as_ref().map_or(0, |s| s.future_xp)
            )?;
        }
        write!(line, " carried {}", stack(menu.carried.as_ref()))?;
        line.push_str(" inv");
        for i in 0..36 {
            if let Some(st) = menu.player_item(i) {
                write!(line, " {i}={}", stack(Some(st)))?;
            }
        }
        let sess = menu
            .merchant
            .as_deref_mut()
            .or(session_after.as_deref_mut());
        let (xp, level, offers, orbs) = match sess {
            Some(sess) => {
                let mut orbs = Vec::new();
                for e in sess.events.drain(..) {
                    match e {
                        Event::Orb(v) => orbs.push(v),
                        Event::LevelUp(_) => regen = true,
                        _ => {}
                    }
                }
                (sess.xp, sess.level, sess.offers.clone(), orbs)
            }
            None => (m.trader.xp, m.level, m.trader.offers.clone(), Vec::new()),
        };
        write!(line, " xp {xp}")?;
        if m.villager {
            write!(
                line,
                " lvl {level} rep {} regen {regen}",
                gossips.reputation(PLAYER)
            )?;
        }
        line.push_str(" offers");
        for o in &offers {
            write!(
                line,
                " {}/{}/{}",
                o.uses,
                o.special_price_diff,
                o.cost_a_count(env.items)
            )?;
        }
        line.push_str(" orbs");
        for v in orbs {
            write!(line, " {v}")?;
        }
        writeln!(out, "{line}")?;
    }
    let random = match menu.merchant.as_mut().or(session_after.as_mut()) {
        Some(s) => s.random.next_int(),
        None => voice.next_int(),
    };
    writeln!(out, "{name} next {random}")?;
    Ok(())
}
