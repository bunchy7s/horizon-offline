//! `TestEnvironmentDefinition`: what a game test batch changes in the world
//! while it runs (game rules, weather, the day time, difficulty, functions,
//! all of them), set up when the batch starts and torn down after it. The
//! definitions are the data pack's `test_environment` entries, or inline.
use super::Game;
use horizon_client::gamerules::{self, Value};
use serde_json::Value as Json;

/// `Weather.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WeatherType {
    Clear,
    Rain,
    Thunder,
}

impl WeatherType {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "clear" => Some(Self::Clear),
            "rain" => Some(Self::Rain),
            "thunder" => Some(Self::Thunder),
            _ => None,
        }
    }

    /// (clearTime, rainTime, raining, thundering).
    fn parameters(self) -> (i32, i32, bool, bool) {
        match self {
            Self::Clear => (100_000, 0, false, false),
            Self::Rain => (0, 100_000, true, false),
            Self::Thunder => (0, 100_000, true, true),
        }
    }
}

/// A `TestEnvironmentDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Environment {
    AllOf(Vec<Environment>),
    ClockTime {
        clock: String,
        time: i64,
    },
    Difficulty(u8),
    Functions {
        setup: Option<String>,
        teardown: Option<String>,
    },
    GameRules(Vec<(String, Value)>),
    /// `timeline_attributes`: Horizon has no environment attribute system to
    /// layer timelines on, so it changes nothing.
    Timelines,
    Weather(WeatherType),
}

/// The `SavedDataType` of an activated definition.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Saved {
    AllOf(Vec<(Environment, Saved)>),
    Clock(i64),
    Difficulty(u8),
    Unit,
    GameRules(Vec<(String, Value)>),
    Weather(WeatherType),
}

/// `TestEnvironmentDefinition.Activation`.
#[derive(Clone, Debug)]
pub(super) struct Activation {
    environment: Environment,
    saved: Saved,
}

/// Reads a definition from its JSON (`DIRECT_CODEC`); a string is the id of
/// a registry entry, read by `load`.
pub(super) fn parse(
    json: &Json,
    load: &dyn Fn(&str) -> Option<Json>,
    depth: u32,
) -> Option<Environment> {
    if depth > 16 {
        return None;
    }
    if let Some(id) = json.as_str() {
        return parse(&load(id)?, load, depth + 1);
    }
    let kind = json.get("type")?.as_str()?;
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    Some(match kind {
        "all_of" => Environment::AllOf(
            json.get("definitions")?
                .as_array()?
                .iter()
                .map(|d| parse(d, load, depth + 1))
                .collect::<Option<_>>()?,
        ),
        "clock_time" => Environment::ClockTime {
            clock: json.get("clock")?.as_str()?.to_owned(),
            time: json.get("time")?.as_i64().filter(|t| *t >= 0)?,
        },
        "difficulty" => Environment::Difficulty(
            ["peaceful", "easy", "normal", "hard"]
                .iter()
                .position(|d| Some(*d) == json.get("difficulty").and_then(Json::as_str))?
                as u8,
        ),
        "function" => Environment::Functions {
            setup: json.get("setup").and_then(Json::as_str).map(str::to_owned),
            teardown: json
                .get("teardown")
                .and_then(Json::as_str)
                .map(str::to_owned),
        },
        "game_rules" => Environment::GameRules(
            json.get("rules")?
                .as_object()?
                .iter()
                .map(|(name, v)| {
                    let rule = gamerules::rule(name)?;
                    let value = match (v.as_bool(), v.as_i64()) {
                        (Some(b), _) => Value::Bool(b),
                        (_, Some(i)) => Value::Int(i32::try_from(i).ok()?),
                        _ => return None,
                    };
                    Some((rule.name.to_owned(), value))
                })
                .collect::<Option<_>>()?,
        ),
        "timeline_attributes" => Environment::Timelines,
        "weather" => Environment::Weather(WeatherType::from_name(json.get("weather")?.as_str()?)?),
        _ => return None,
    })
}

impl Game<'_> {
    /// The data pack entry `data/<ns>/test_environment/<path>.json`.
    fn read_test_environment_json(&self, id: &str) -> Option<Json> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let file = self
            .options
            .resources
            .join("data")
            .join(ns)
            .join("test_environment")
            .join(format!("{path}.json"));
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    }

    /// The definition of a batch's environment (`batch()` of its tests).
    pub(super) fn test_environment(&self, id: &str) -> Option<Environment> {
        parse(
            &Json::String(id.to_owned()),
            &|i| self.read_test_environment_json(i),
            0,
        )
    }

    /// `TestEnvironmentDefinition.activate`.
    pub(super) fn activate_test_environment(&mut self, environment: Environment) -> Activation {
        let saved = self.set_up_environment(&environment);
        Activation { environment, saved }
    }

    /// `Activation.teardown`.
    pub(super) fn tear_down_test_environment(&mut self, activation: Activation) {
        self.tear_down_environment(&activation.environment, activation.saved);
    }

    fn set_up_environment(&mut self, environment: &Environment) -> Saved {
        match environment {
            Environment::AllOf(all) => Saved::AllOf(
                all.iter()
                    .map(|e| (e.clone(), self.set_up_environment(e)))
                    .collect(),
            ),
            Environment::ClockTime { clock, time } => {
                // Only the overworld clock runs in Horizon.
                let previous = self.world.day_time;
                if clock == "minecraft:overworld" {
                    self.world.day_time = *time;
                    self.ui.clock.partial = 0.0;
                }
                Saved::Clock(previous)
            }
            Environment::Difficulty(d) => {
                let previous = self.ui.difficulty;
                self.set_difficulty_forced(*d);
                Saved::Difficulty(previous)
            }
            Environment::Functions { setup, .. } => {
                if let Some(id) = setup {
                    self.run_environment_function(id);
                }
                Saved::Unit
            }
            Environment::GameRules(rules) => {
                let original = rules
                    .iter()
                    .filter_map(|(name, _)| {
                        let rule = gamerules::rule(name)?;
                        Some((
                            rule.name.to_owned(),
                            self.ui.rules.get(rule.name).unwrap_or(rule.default),
                        ))
                    })
                    .collect();
                self.set_environment_rules(rules);
                Saved::GameRules(original)
            }
            Environment::Timelines => Saved::Unit,
            Environment::Weather(kind) => {
                let w = &self.world.weather;
                let previous = if w.thundering {
                    WeatherType::Thunder
                } else if w.raining {
                    WeatherType::Rain
                } else {
                    WeatherType::Clear
                };
                self.set_weather_type(*kind);
                Saved::Weather(previous)
            }
        }
    }

    fn tear_down_environment(&mut self, environment: &Environment, saved: Saved) {
        match (environment, saved) {
            (Environment::AllOf(_), Saved::AllOf(all)) => {
                for (e, s) in all.into_iter().rev() {
                    self.tear_down_environment(&e, s);
                }
            }
            (Environment::ClockTime { clock, .. }, Saved::Clock(previous)) => {
                if clock == "minecraft:overworld" {
                    self.world.day_time = previous;
                    self.ui.clock.partial = 0.0;
                }
            }
            (_, Saved::Difficulty(d)) => self.set_difficulty_forced(d),
            (
                Environment::Functions {
                    teardown: Some(id), ..
                },
                Saved::Unit,
            ) => self.run_environment_function(id),
            (_, Saved::GameRules(original)) => self.set_environment_rules(&original),
            (_, Saved::Weather(previous)) => {
                // resetWeatherCycle, then the saved type.
                let w = &mut self.world.weather;
                w.rain_time = 0;
                w.raining = false;
                w.thunder_time = 0;
                w.thundering = false;
                self.set_weather_type(previous);
            }
            _ => {}
        }
    }

    /// `GameRules.setAll`.
    fn set_environment_rules(&mut self, rules: &[(String, Value)]) {
        for (name, value) in rules {
            if let Some(rule) = gamerules::rule(name) {
                self.ui.rules.set(rule.name, *value);
            }
        }
        self.sync_rules();
    }

    /// `Weather.Type.apply`: `ServerLevel.setWeatherParameters`.
    fn set_weather_type(&mut self, kind: WeatherType) {
        let (clear, rain, raining, thundering) = kind.parameters();
        let w = &mut self.world.weather;
        w.clear_time = clear;
        w.rain_time = rain;
        w.thunder_time = rain;
        w.raining = raining;
        w.thundering = thundering;
    }

    /// `Functions.run`: the function with game master permission and
    /// suppressed output; a missing one is logged.
    fn run_environment_function(&mut self, id: &str) {
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let found = self
            .functions_named(&id)
            .ok()
            .and_then(|f| f.into_iter().next());
        match found {
            Some(function) => {
                let source = crate::game::commands::execute::Source {
                    silent: true,
                    ..self.server_source()
                };
                self.exec.commands = 0;
                let _ = self.run_function(&function, None, source);
            }
            None => log::error!("Test Batch failed for non-existent function {id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn load(id: &str) -> Option<Json> {
        match id {
            "minecraft:night" => Some(
                json!({"type": "minecraft:clock_time", "clock": "minecraft:overworld", "time": 18000}),
            ),
            _ => None,
        }
    }

    #[test]
    fn definitions_parse_with_references_and_nesting() {
        let json = json!({
            "type": "minecraft:all_of",
            "definitions": [
                "minecraft:night",
                {"type": "minecraft:weather", "weather": "thunder"},
                {"type": "minecraft:game_rules", "rules": {"minecraft:advance_time": false}},
                {"type": "minecraft:difficulty", "difficulty": "hard"},
                {"type": "minecraft:function", "setup": "test:on"}
            ]
        });
        let Some(Environment::AllOf(all)) = parse(&json, &load, 0) else {
            panic!("not all_of");
        };
        assert_eq!(
            all[0],
            Environment::ClockTime {
                clock: "minecraft:overworld".into(),
                time: 18000
            }
        );
        assert_eq!(all[1], Environment::Weather(WeatherType::Thunder));
        assert!(matches!(&all[2], Environment::GameRules(r) if r.len() == 1));
        assert_eq!(all[3], Environment::Difficulty(3));
        assert_eq!(
            all[4],
            Environment::Functions {
                setup: Some("test:on".into()),
                teardown: None
            }
        );
    }

    #[test]
    fn unknown_or_invalid_definitions_do_not_parse() {
        assert_eq!(parse(&json!("minecraft:missing"), &load, 0), None);
        assert_eq!(parse(&json!({"type": "minecraft:nothing"}), &load, 0), None);
        assert_eq!(
            parse(
                &json!({"type": "minecraft:clock_time", "clock": "c", "time": -1}),
                &load,
                0
            ),
            None
        );
    }

    #[test]
    fn weather_types_set_vanillas_timers() {
        assert_eq!(WeatherType::Clear.parameters(), (100_000, 0, false, false));
        assert_eq!(WeatherType::Thunder.parameters(), (0, 100_000, true, true));
    }
}
