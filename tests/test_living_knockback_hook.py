"""Run the real knockback bridge and validation without prepared game assets."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import function, method

ROOT = Path(__file__).resolve().parents[1]


class LivingKnockbackHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_one_hook_attacker_held_items_cancellation_and_validation(self):
        hooks = (ROOT / "crates/horizon-client/src/game/native_hooks.rs").read_text(encoding="utf-8")
        health = (ROOT / "crates/horizon-client/src/health.rs").read_text(encoding="utf-8")
        context_start = health.index("thread_local! {")
        context = health[context_start:health.index("/// Whether [", context_start)]
        functions = hooks.replace("pub(super) fn", "fn").replace("\nfn ", "\n    fn ")
        bridge = method(functions, "knockback")
        arguments = method(functions, "hurt_server_args")
        validation = function(health, "knockback_canceled")
        fixture = r"""
use std::cell::{Cell, RefCell};
mod items { #[derive(Clone, Debug, PartialEq)] pub struct ItemStack(pub &'static str); }
use items::ItemStack;
#[derive(Clone, Copy)]
enum Who { Player, Mob(u32) }
struct Damage { attacker: Option<Who> }
struct Ai { last_hurt_by: Option<Who> }
struct Kind { id: String }
struct Mob { id: u32, kind: Kind, ai: Ai }
#[derive(Clone, Copy)]
struct HurtSource<'a> { damage_type: &'a str, causing: i32, direct: i32 }
enum Hurting<'a> {
    Player(i32, HurtSource<'a>), Mob(&'a Mob, &'a Damage),
    Dragon(i32, HurtSource<'a>), ArmorStand(i32, HurtSource<'a>),
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Knock { strength: f64, x: f64, z: f64 }
mod horizon_client { pub mod health {
    use super::super::*;
    use std::sync::OnceLock;
    static KNOCKBACK: OnceLock<fn(Hurting<'_>, &mut Knock) -> bool> = OnceLock::new();
    CONTEXT
    VALIDATION
    pub fn install() { KNOCKBACK.set(super::super::knockback).unwrap(); }
} }
thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static CANCEL_V1: Cell<bool> = const { Cell::new(false) };
    static CANCEL_V2: Cell<bool> = const { Cell::new(false) };
    static NONFINITE: Cell<bool> = const { Cell::new(false) };
    static CALLS: RefCell<Vec<(u8, i32, f64)>> = const { RefCell::new(Vec::new()) };
    static HELD: RefCell<Vec<(Option<ItemStack>, Option<ItemStack>)>> = const { RefCell::new(Vec::new()) };
}
mod mod_api {
    pub fn mods() -> &'static [()] { if super::ENABLED.get() { &[()] } else { &[] } }
}
struct Game;
impl Game {
    fn native_living_knockback(_: i32, _: &str, _: &str, causing: i32, _: i32, attacker: i32,
                              main: Option<&ItemStack>, off: Option<&ItemStack>,
                              strength: &mut f64, x: &mut f64, _: &mut f64, canceled: &mut bool) {
        CALLS.with_borrow_mut(|calls| calls.push((1, causing.max(attacker), *strength)));
        HELD.with_borrow_mut(|held| held.push((main.cloned(), off.cloned())));
        *x = -*x;
        if NONFINITE.get() { *strength = f64::NAN; }
        *canceled = CANCEL_V1.get() || CANCEL_V2.get();
    }
}
fn mob_source(damage: &Damage) -> HurtSource<'static> {
    let causing = match damage.attacker { Some(Who::Mob(id)) => id as i32, _ => 0 };
    HurtSource { damage_type: "minecraft:mob_attack", causing, direct: causing }
}
ARGUMENTS
BRIDGE
#[test]
fn the_one_hook_names_player_mob_and_stand_attackers_and_what_they_hold() {
    use horizon_client::health::{knockback_from, knockback_from_holding, knockback_canceled, install};
    install();
    let mob = Mob { id: 12, kind: Kind { id: "minecraft:zombie".into() },
                    ai: Ai { last_hurt_by: Some(Who::Mob(99)) } };
    let player_hit = Damage { attacker: Some(Who::Player) };
    let source = HurtSource { damage_type: "minecraft:player_attack", causing: 0, direct: 0 };
    let original = Knock { strength: 0.4, x: 1.0, z: 0.0 };
    let _scope = knockback_from(7);
    for pushed in [Hurting::Mob(&mob, &player_hit), Hurting::Player(12, source)] {
        let mut knock = original;
        assert!(!knockback_canceled(pushed, &mut knock));
        assert_eq!(knock, Knock { strength: 0.4, x: -1.0, z: 0.0 });
        assert_eq!(CALLS.take(), [(1, 7, 0.4)]);
    }
    HELD.take();
    {
        let _held = knockback_from_holding(7, [Some(ItemStack("sword")), None]);
        let mut knock = original;
        knockback_canceled(Hurting::Player(12, source), &mut knock);
        CALLS.take();
        assert_eq!(HELD.take(), [(Some(ItemStack("sword")), None)]);
    }
    let mut knock = original;
    knockback_canceled(Hurting::Player(12, source), &mut knock);
    CALLS.take();
    assert_eq!(HELD.take(), [(None, None)]);
    for attacker in [Some(Who::Mob(3)), None] {
        let damage = Damage { attacker };
        let mut knock = original;
        knockback_canceled(Hurting::Mob(&mob, &damage), &mut knock);
        assert_eq!(CALLS.take()[0].1, if attacker.is_some() { 3 } else { 99 });
    }
    let explicit = HurtSource { causing: 5, ..source };
    let mut knock = original;
    knockback_canceled(Hurting::Dragon(12, explicit), &mut knock);
    assert_eq!(CALLS.take()[0].1, 5);
    // An armor stand's sweep fires the same hook.
    let mut knock = original;
    knockback_canceled(Hurting::ArmorStand(12, source), &mut knock);
    assert_eq!(CALLS.take(), [(1, 7, 0.4)]);
    CANCEL_V1.set(true);
    let mut knock = original;
    assert!(knockback_canceled(Hurting::Player(12, source), &mut knock));
    assert_eq!(CALLS.take().len(), 1);
    CANCEL_V1.set(false);
    NONFINITE.set(true);
    let mut knock = original;
    knockback_canceled(Hurting::ArmorStand(12, source), &mut knock);
    assert_eq!(knock.strength, original.strength);
    CALLS.take();
    ENABLED.set(false);
    let mut knock = original;
    assert!(!knockback_canceled(Hurting::Player(12, source), &mut knock));
    assert_eq!(knock, original);
    assert!(CALLS.take().is_empty());
}
""".replace("CONTEXT", context).replace("VALIDATION", validation).replace("ARGUMENTS", arguments).replace("BRIDGE", bridge)
        with tempfile.TemporaryDirectory() as folder:
            source, exe = Path(folder) / "knockback.rs", Path(folder) / "knockback.exe"
            source.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(source), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
