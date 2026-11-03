//! The dedicated server's anticheat (docs/anticheat.md), without the game: the `[anticheat]` section of
//! `server.toml`, each player's violation levels, and the rules of the checks. The server's hub
//! (`horizon_client::lan`) feeds it what players send and carries out what it decides.
//!
//! It is on by default, and so is every check, but only the checks a vanilla client can't trip act on
//! their own (`combat`); the others log only until they have shown no false positive with real
//! players (their `setback`, `alert` and `kick` are 0). `enabled = false` turns the anticheat, or one
//! check, off. A check that fails *flags* the player: its violation level (VL) rises by the flag's
//! weight and falls back by `decay` a second. From `setback` on, the move or action is cancelled (the
//! player is sent back where the server last had them); from `alert` on, online operators and the
//! console are told; from `kick` on, the player is disconnected. A check's kicks are counted per player
//! across sessions ([`Strikes`]), and the kick that reaches its `ban_after` bans the player instead
//! (`banned-players.json`, reason "Horizon anticheat: <check>"). Any threshold at 0 turns that action
//! off.
//!
//! The checks are written from vanilla 26.3's own rules and from what cheats are publicly described to
//! do, never from another anticheat's code.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod combat;
pub mod prediction;
pub mod timer;

/// What one check's violation level leads to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Actions {
    pub setback: f32,
    pub alert: f32,
    pub kick: f32,
    /// Violation level lost each second.
    pub decay: f32,
    /// The kick, counting this check's earlier ones for the player, that bans instead; 0: never.
    pub ban_after: u32,
}

macro_rules! checks {
    ($(
        $(#[$doc:meta])*
        $field:ident: $Check:ident => $Config:ident [$setback:expr, $alert:expr, $kick:expr, $decay:expr, $ban_after:expr] {
            $( $(#[$xdoc:meta])* $extra:ident: $ty:ty = $default:expr, )*
        }
    )*) => {
        /// Every check, in the order of the players' violation levels.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Check { $($Check,)* }

        impl Check {
            pub const ALL: &[Check] = &[$(Check::$Check,)*];
            /// Its name in `server.toml` and in alerts.
            pub fn name(self) -> &'static str {
                match self { $(Check::$Check => stringify!($field),)* }
            }
        }

        $(
            $(#[$doc])*
            #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
            #[serde(default, deny_unknown_fields)]
            pub struct $Config {
                pub enabled: bool,
                pub setback: f32,
                pub alert: f32,
                pub kick: f32,
                pub decay: f32,
                pub ban_after: u32,
                $( $(#[$xdoc])* pub $extra: $ty, )*
            }

            impl Default for $Config {
                fn default() -> Self {
                    Self {
                        enabled: true,
                        setback: $setback,
                        alert: $alert,
                        kick: $kick,
                        decay: $decay,
                        ban_after: $ban_after,
                        $($extra: $default,)*
                    }
                }
            }
        )*

        /// `server.toml`'s `[anticheat]`: on by default.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct Config {
            pub enabled: bool,
            /// Tell online operators of an alert in their chat (the console and the log always are).
            pub alerts: bool,
            /// The permission level a player needs to see alerts.
            pub alert_level: u8,
            /// Every flag, one line each, relative to the server's folder; empty: no file.
            pub log: String,
            /// The players' kicks by check ([`Strikes`]), relative to the server's folder; empty: kept
            /// only while the server runs.
            pub strikes: String,
            $(pub $field: $Config,)*
        }

        impl Default for Config {
            fn default() -> Self {
                Self {
                    enabled: true,
                    alerts: true,
                    alert_level: 2,
                    log: "logs/anticheat.log".into(),
                    strikes: "anticheat-strikes.json".into(),
                    $($field: $Config::default(),)*
                }
            }
        }

        impl Config {
            /// Whether `check` runs: the anticheat and the check both on.
            pub fn runs(&self, check: Check) -> bool {
                self.enabled && match check { $(Check::$Check => self.$field.enabled,)* }
            }

            pub fn actions(&self, check: Check) -> Actions {
                match check {
                    $(Check::$Check => {
                        let c = &self.$field;
                        Actions {
                            setback: c.setback,
                            alert: c.alert,
                            kick: c.kick,
                            decay: c.decay,
                            ban_after: c.ban_after,
                        }
                    })*
                }
            }
        }
    };
}

checks! {
    /// Timer: more client ticks than the server's clock allows (a sped-up game). Vanilla's client ends
    /// every tick with ServerboundClientTickEndPacket, at the server's tick rate; each one spends a tick's
    /// time from a balance that wall-clock time fills, capped so a pause can't be saved up for a burst.
    /// It logs only by default; once trusted: `setback = 2`, `alert = 5`, `kick = 20`.
    timer: Timer => TimerConfig [0.0, 0.0, 0.0, 0.02, 0] {
        /// How far ahead of the clock, in milliseconds, a client may get before it is flagged.
        tolerance_ms: f64 = 300.0,
        /// The most time, in milliseconds, a quiet connection banks for the ticks that follow (a lag
        /// spike's backlog arrives at once).
        max_lag_ms: f64 = 1000.0,
    }

    /// Ground: a move that says it ends on the ground with nothing there to stand on: no-fall (claiming
    /// the ground to skip fall damage) and walking on water. The claim is dropped, so vanilla's
    /// fall damage follows (the player is never sent back: `setback = 0` makes the check log only), and
    /// a player that keeps claiming is kicked, then banned at the second kick (`ban_after = 2`). A claim standing still isn't
    /// judged (no fall to skip). On the official server's first two hours (67 players), every player who
    /// wasn't flying or walking on water peaked at level 7 (4 claims standing still in one second); the
    /// ones who were, at 113 to 3749.
    ground: Ground => GroundConfig [10.0, 15.0, 30.0, 2.0, 2] {}

    /// Prediction: each move played again from the client's own inputs with vanilla's movement code,
    /// and compared (speed, step, fly, jesus...). Each player's client ticks are recorded
    /// (`prediction::Record`), with a ping a tick to place what the server changes, and played again
    /// (horizon-client's `lan::prediction`). A move that lands farther than `tolerance` from the
    /// replay flags, by how far it lands (one `tolerance` off weighs 1, at most 5 a tick), and the
    /// setback puts the player back where its moves last agreed. A tick near what the replay can't
    /// play yet (another entity, a block change or abilities the client hadn't seen, a moving piston,
    /// a chunk the client doesn't have, a riptide launch) is excused, and so are the ticks after it
    /// until the moves agree again. It logs only by default; once trusted: `setback = 5`, `alert = 15`,
    /// `kick = 60`.
    prediction: Prediction => PredictionConfig [0.0, 0.0, 0.0, 2.0, 0] {
        /// How far, in blocks, a move may land from where the replay puts it.
        tolerance: f64 = 0.03,
    }

    /// Combat: attacks vanilla's client can't send (`combat`): one with no swing after it (a kill aura
    /// that hides its swing), or a second entity attacked in the same client tick (vanilla attacks the
    /// one entity under the crosshair, picked once a tick). Each flags once; from `setback` on the
    /// player's attacks are dropped. A vanilla client never trips it, so it acts by default: the first
    /// kick warns, the second bans (`ban_after = 2`).
    combat: Combat => CombatConfig [2.0, 5.0, 20.0, 0.5, 2] {}
}

/// How many checks there are.
pub const CHECKS: usize = Check::ALL.len();

impl Config {
    /// The `[anticheat]` section of a whole `server.toml`; the default (off) when it has none.
    pub fn from_server_toml(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            anticheat: Config,
        }
        // Other sections are the supervisor's to check: only this one is read here.
        let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut only = toml::Table::new();
        if let Some(section) = table.remove("anticheat") {
            only.insert("anticheat".into(), section);
        }
        let file: File = only
            .try_into()
            .map_err(|e: toml::de::Error| format!("[anticheat]: {e}"))?;
        file.anticheat.check()?;
        Ok(file.anticheat)
    }

    /// Thresholds can't be negative.
    pub fn check(&self) -> Result<(), String> {
        for &c in Check::ALL {
            let a = self.actions(c);
            if [a.setback, a.alert, a.kick, a.decay]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
            {
                return Err(format!(
                    "anticheat.{}: setback, alert, kick and decay are numbers from 0 up",
                    c.name()
                ));
            }
        }
        let tolerance = self.prediction.tolerance;
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err("anticheat.prediction: tolerance is a number of blocks above 0".into());
        }
        Ok(())
    }
}

/// How many times each check kicked each player, kept across sessions (`strikes`, a JSON object of
/// players' UUIDs to check names to counts): the kick that reaches a check's `ban_after` is a ban.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Strikes(pub BTreeMap<String, BTreeMap<String, u32>>);

impl Strikes {
    /// Player `uuid` was kicked by `check`: its kicks by it so far, this one included, and whether this
    /// one bans. A ban starts the count again, so a player an operator pardons is kicked before the
    /// next ban.
    pub fn kick(&mut self, uuid: &str, check: Check, ban_after: u32) -> (u32, bool) {
        let count = self
            .0
            .entry(uuid.to_owned())
            .or_default()
            .entry(check.name().to_owned())
            .or_default();
        *count = count.saturating_add(1);
        let kicks = *count;
        let ban = ban_after > 0 && kicks >= ban_after;
        if ban {
            *count = 0;
        }
        (kicks, ban)
    }
}

/// What a flag leads to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Verdict {
    /// The check's violation level after the flag.
    pub vl: f32,
    /// Cancel the move or action.
    pub setback: bool,
    /// Tell operators and the console (at most once a second per check).
    pub alert: bool,
    pub kick: bool,
}

/// One player's anticheat state. Clean players cost nothing: levels only change when a check flags.
#[derive(Clone, Debug, Default)]
pub struct Player {
    vl: [f32; CHECKS],
    /// The server tick of each check's last flag (its decay runs from there).
    flagged: [u64; CHECKS],
    /// The server tick of each check's last alert.
    alerted: [Option<u64>; CHECKS],
    pub timer: timer::Balance,
    /// The server tick and height where the ground last held the player.
    pub ground: Option<(u64, f64)>,
    /// The client ticks the prediction plays again (only while it runs).
    pub record: Option<Box<prediction::Record>>,
    /// Sprinting as the client's own commands left it (ServerboundPlayerCommandPacket's
    /// START_SPRINTING and STOP_SPRINTING: LocalPlayer.wasSprinting), which the record's ticks take.
    /// Not the server's player: a hit with extra knockback stops its sprint
    /// (Player.causeExtraKnockback), the client's own `Player.attack` doesn't (a mob's `hurtClient`
    /// is false), and once the server's flag reaches it LocalPlayer.aiStep starts the sprint again
    /// while the sprint key is held, with nothing to say: wasSprinting never changed.
    pub sprinting: bool,
    /// The attacks of the client tick being heard.
    pub attacks: combat::Attacks,
}

impl Player {
    /// The check's violation level at `tick`, with its decay since the last flag.
    pub fn vl(&self, config: &Config, check: Check, tick: u64) -> f32 {
        let i = check as usize;
        let seconds = tick.saturating_sub(self.flagged[i]) as f32 / 20.0;
        (self.vl[i] - config.actions(check).decay * seconds).max(0.0)
    }

    /// `check` failed with this weight at server tick `tick`.
    pub fn flag(&mut self, config: &Config, check: Check, weight: f32, tick: u64) -> Verdict {
        let i = check as usize;
        let a = config.actions(check);
        let vl = self.vl(config, check, tick) + weight;
        self.vl[i] = vl;
        self.flagged[i] = tick;
        let reached = |threshold: f32| threshold > 0.0 && vl >= threshold;
        let alert = reached(a.alert) && self.alerted[i].is_none_or(|t| tick >= t + 20);
        if alert {
            self.alerted[i] = Some(tick);
        }
        Verdict {
            vl,
            setback: reached(a.setback),
            alert,
            kick: reached(a.kick),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Config {
        Config {
            enabled: true,
            ..Config::default()
        }
    }

    #[test]
    fn on_by_default_but_only_combat_and_ground_act() {
        let c = Config::from_server_toml("[server]\nport = 25565\n").unwrap();
        assert_eq!(c, Config::default());
        for &check in Check::ALL {
            assert!(c.runs(check), "{}", check.name());
            let a = c.actions(check);
            let acts = [a.setback, a.alert, a.kick] != [0.0; 3];
            assert_eq!(
                acts,
                matches!(check, Check::Combat | Check::Ground),
                "{}",
                check.name()
            );
        }
        let off = Config::from_server_toml("[anticheat]\nenabled = false\n").unwrap();
        assert!(Check::ALL.iter().all(|&check| !off.runs(check)));
    }

    #[test]
    fn reads_its_section() {
        let text = "[server]\nport = 1\n[anticheat]\nenabled = true\n[anticheat.timer]\nkick = 0\ntolerance_ms = 500\n";
        let c = Config::from_server_toml(text).unwrap();
        assert!(c.runs(Check::Timer));
        assert_eq!(c.timer.kick, 0.0);
        assert_eq!(c.timer.tolerance_ms, 500.0);
        assert_eq!(c.timer.alert, TimerConfig::default().alert);
        let zero = "[anticheat]\n[anticheat.prediction]\ntolerance = 0\n";
        assert!(Config::from_server_toml(zero).is_err());
        let off = "[anticheat]\nenabled = true\n[anticheat.timer]\nenabled = false\n";
        assert!(!Config::from_server_toml(off).unwrap().runs(Check::Timer));
    }

    #[test]
    fn refuses_unknown_keys_and_negative_levels() {
        assert!(Config::from_server_toml("[anticheat]\nenable = true\n").is_err());
        assert!(Config::from_server_toml("[anticheat.timer]\nkick = -1\n").is_err());
    }

    #[test]
    fn levels_rise_decay_and_act_in_order() {
        let mut c = on();
        c.timer = TimerConfig {
            setback: 2.0,
            alert: 3.0,
            kick: 5.0,
            decay: 1.0,
            ..TimerConfig::default()
        };
        let mut p = Player::default();
        let v = p.flag(&c, Check::Timer, 1.0, 100);
        assert_eq!((v.setback, v.alert, v.kick), (false, false, false));
        let v = p.flag(&c, Check::Timer, 1.0, 100);
        assert_eq!((v.vl, v.setback, v.alert), (2.0, true, false));
        let v = p.flag(&c, Check::Timer, 1.0, 100);
        assert!(v.alert && !v.kick);
        // At most one alert a second.
        assert!(!p.flag(&c, Check::Timer, 0.5, 110).alert);
        // 3 at tick 110 (half a level gone, half added); two seconds later two more are gone.
        assert_eq!(p.vl(&c, Check::Timer, 150), 1.0);
        assert_eq!(p.vl(&c, Check::Timer, 10_000), 0.0);
        let v = p.flag(&c, Check::Timer, 5.0, 10_000);
        assert!(v.kick && v.alert);
    }

    #[test]
    fn the_kick_that_reaches_ban_after_bans() {
        let c = Config::default();
        assert_eq!(c.actions(Check::Combat).ban_after, 2);
        assert_eq!(c.actions(Check::Prediction).ban_after, 0);
        let mut s = Strikes::default();
        assert_eq!(s.kick("a", Check::Combat, 2), (1, false));
        // Another player, another check: their own counts.
        assert_eq!(s.kick("b", Check::Combat, 2), (1, false));
        assert_eq!(s.kick("a", Check::Timer, 2), (1, false));
        assert_eq!(s.kick("a", Check::Combat, 2), (2, true));
        // Pardoned, the player is kicked once more before the next ban.
        assert_eq!(s.kick("a", Check::Combat, 2), (1, false));
        // 0 never bans; 1 bans at the first kick.
        assert_eq!(s.kick("c", Check::Timer, 0), (1, false));
        assert_eq!(s.kick("c", Check::Timer, 0), (2, false));
        assert_eq!(s.kick("d", Check::Ground, 1), (1, true));
    }

    #[test]
    fn zero_turns_an_action_off() {
        let mut c = on();
        c.timer.kick = 0.0;
        let mut p = Player::default();
        assert!(!p.flag(&c, Check::Timer, 1000.0, 1).kick);
    }
}
