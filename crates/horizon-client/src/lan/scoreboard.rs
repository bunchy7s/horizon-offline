//! `ServerScoreboard`: what a player's client knows of the server's
//! scoreboard. Every team goes to every player; an objective goes once it
//! sits in a display slot (`startTrackingObjective`: its objective, its
//! slots, then its scores) and leaves when it sits in none
//! (`stopTrackingObjective`). A joining player gets it all
//! (`PlayerList.updateEntireScoreboard`).
//!
//! A server's commands change the board in place; each tick a player is
//! sent the packets that turn what it was last sent into what it should
//! have now, which are the ones `ServerScoreboard`'s callbacks send.
use crate::scoreboard::{NumberFormat, RenderType, Scoreboard, component_tag, display_slots};
use crate::text::COLORS;
use horizon_protocol::packets::play::{
    self, Clientbound as C, ResetScore, SetDisplayObjective, SetObjective, SetPlayerTeam, SetScore,
    TeamParameters,
};
use horizon_world::nbt::Tag;
use std::collections::{BTreeMap, BTreeSet};

type Format = Option<play::NumberFormat>;

/// The scoreboard as one player's client has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// The tracked objectives: display name, render type (1 hearts), format.
    objectives: BTreeMap<String, (Tag, i32, Format)>,
    /// `DisplaySlot` id to objective.
    display: BTreeMap<i32, String>,
    /// The tracked objectives' scores, by holder and objective.
    scores: BTreeMap<(String, String), (i32, Option<Tag>, Format)>,
    /// Every team: its parameters and players, in the team's order.
    teams: BTreeMap<String, (TeamParameters, Vec<String>)>,
}

fn format(f: Option<&NumberFormat>) -> Format {
    f.map(|f| match f {
        NumberFormat::Blank => play::NumberFormat::Blank,
        NumberFormat::Styled(s) => play::NumberFormat::Styled(crate::scoreboard::json_to_tag(s)),
        NumberFormat::Fixed(c) => play::NumberFormat::Fixed(component_tag(c)),
    })
}

impl View {
    /// What the board's state gives a client.
    pub fn of(board: &Scoreboard) -> Self {
        let slots = display_slots();
        let display: BTreeMap<i32, String> = board
            .display
            .iter()
            .filter_map(|(slot, o)| Some((slots.iter().position(|s| s == slot)? as i32, o.clone())))
            .collect();
        let tracked: BTreeSet<&String> = display.values().collect();
        let objectives = board
            .objectives
            .iter()
            .filter(|o| tracked.contains(&o.name))
            .map(|o| {
                let render = i32::from(o.render_type == RenderType::Hearts);
                (
                    o.name.clone(),
                    (
                        component_tag(&o.display_name),
                        render,
                        format(o.number_format.as_ref()),
                    ),
                )
            })
            .collect();
        let scores = board
            .scores
            .iter()
            .flat_map(|(holder, scores)| {
                scores
                    .iter()
                    .filter(|(o, _)| tracked.contains(o))
                    .map(move |(o, s)| {
                        (
                            (holder.clone(), o.clone()),
                            (
                                s.value,
                                s.display.as_ref().map(component_tag),
                                format(s.number_format.as_ref()),
                            ),
                        )
                    })
            })
            .collect();
        let teams = board
            .teams
            .iter()
            .map(|t| {
                let params = TeamParameters {
                    display_name: component_tag(&t.display_name),
                    prefix: component_tag(&t.prefix),
                    suffix: component_tag(&t.suffix),
                    name_tag_visibility: t.nametag_visibility as i32,
                    collision_rule: t.collision_rule as i32,
                    color: t
                        .color
                        .as_deref()
                        .and_then(|c| COLORS.iter().position(|(_, n, _)| *n == c))
                        .map(|i| i as i32),
                    options: i8::from(t.allow_friendly_fire)
                        | (i8::from(t.see_friendly_invisibles) << 1),
                };
                (
                    t.name.clone(),
                    (params, t.players.iter().cloned().collect()),
                )
            })
            .collect();
        Self {
            objectives,
            display,
            scores,
            teams,
        }
    }

    /// The packets that take a client from `self` to `next`.
    pub fn changes(&self, next: &View) -> Vec<C> {
        let mut out = Vec::new();
        let team = |name: &str, method, parameters, players| {
            C::SetPlayerTeam(SetPlayerTeam {
                name: name.to_owned(),
                method,
                parameters,
                players,
            })
        };
        for name in self.teams.keys().filter(|n| !next.teams.contains_key(*n)) {
            out.push(team(name, 1, None, Vec::new()));
        }
        // Every leave before any join: `addPlayerToTeam` sends the old
        // team's leave first, and a client's `removePlayerFromTeam` throws
        // when the player already sits in another team.
        for (name, (_, had)) in &self.teams {
            let Some((_, players)) = next.teams.get(name) else {
                continue;
            };
            let left: Vec<_> = had
                .iter()
                .filter(|p| !players.contains(p))
                .cloned()
                .collect();
            if !left.is_empty() {
                out.push(team(name, 4, None, left));
            }
        }
        for (name, (params, players)) in &next.teams {
            let Some((was, had)) = self.teams.get(name) else {
                out.push(team(name, 0, Some(params.clone()), players.clone()));
                continue;
            };
            if was != params {
                out.push(team(name, 2, Some(params.clone()), Vec::new()));
            }
            let joined: Vec<_> = players
                .iter()
                .filter(|p| !had.contains(p))
                .cloned()
                .collect();
            if !joined.is_empty() {
                out.push(team(name, 3, None, joined));
            }
        }
        for (name, display) in &next.objectives {
            let method = match self.objectives.get(name) {
                None => 0,
                Some(was) if was != display => 2,
                Some(_) => continue,
            };
            out.push(C::SetObjective(SetObjective {
                name: name.clone(),
                method,
                display: Some(display.clone()),
            }));
        }
        let slots: BTreeSet<i32> = self
            .display
            .keys()
            .chain(next.display.keys())
            .copied()
            .collect();
        for slot in slots {
            let objective = next.display.get(&slot);
            if self.display.get(&slot) != objective {
                out.push(C::SetDisplayObjective(SetDisplayObjective {
                    slot,
                    objective: objective.cloned().unwrap_or_default(),
                }));
            }
        }
        for ((holder, objective), (value, display, number_format)) in &next.scores {
            let key = (holder.clone(), objective.clone());
            if self.scores.get(&key) != Some(&(*value, display.clone(), number_format.clone())) {
                out.push(C::SetScore(SetScore {
                    owner: holder.clone(),
                    objective: objective.clone(),
                    score: *value,
                    display: display.clone(),
                    number_format: number_format.clone(),
                }));
            }
        }
        for (holder, objective) in self.scores.keys() {
            let key = (holder.clone(), objective.clone());
            if !next.scores.contains_key(&key) && next.objectives.contains_key(objective) {
                out.push(C::ResetScore(ResetScore {
                    owner: holder.clone(),
                    objective: Some(objective.clone()),
                }));
            }
        }
        for name in self
            .objectives
            .keys()
            .filter(|n| !next.objectives.contains_key(*n))
        {
            out.push(C::SetObjective(SetObjective {
                name: name.clone(),
                method: 1,
                display: None,
            }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn methods(packets: &[C]) -> Vec<String> {
        packets
            .iter()
            .map(|p| match p {
                C::SetObjective(o) => format!("objective {} {}", o.name, o.method),
                C::SetDisplayObjective(d) => format!("display {} {}", d.slot, d.objective),
                C::SetScore(s) => format!("score {} {} {}", s.owner, s.objective, s.score),
                C::ResetScore(r) => format!("reset {} {:?}", r.owner, r.objective),
                C::SetPlayerTeam(t) => format!("team {} {} {:?}", t.name, t.method, t.players),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn a_shown_objective_goes_with_its_scores_and_leaves_when_hidden() {
        let mut board = Scoreboard::default();
        board.add_objective(
            "audit",
            "dummy",
            json!("Audit"),
            RenderType::Integer,
            false,
            None,
        );
        board.score_mut("Alpha", "audit").value = 7;
        let empty = View::default();
        // Not displayed: not tracked, nothing sent.
        let hidden = View::of(&board);
        assert!(empty.changes(&hidden).is_empty());
        board.set_display_objective("sidebar", Some("audit"));
        let shown = View::of(&board);
        assert_eq!(
            methods(&hidden.changes(&shown)),
            [
                "objective audit 0",
                "display 1 audit",
                "score Alpha audit 7"
            ]
        );
        board.score_mut("Alpha", "audit").value = 12;
        board.score_mut("Beta", "audit").value = 3;
        board.reset_one("Beta", "audit");
        let changed = View::of(&board);
        assert_eq!(methods(&shown.changes(&changed)), ["score Alpha audit 12"]);
        board.reset_one("Alpha", "audit");
        let reset = View::of(&board);
        assert_eq!(
            methods(&changed.changes(&reset)),
            ["reset Alpha Some(\"audit\")"]
        );
        board.set_display_objective("sidebar", None);
        assert_eq!(
            methods(&reset.changes(&View::of(&board))),
            ["display 1 ", "objective audit 1"]
        );
    }

    #[test]
    fn teams_go_to_everyone_with_their_players() {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_to_team("Alpha", "red");
        let first = View::of(&board);
        assert_eq!(
            methods(&View::default().changes(&first)),
            ["team red 0 [\"Alpha\"]"]
        );
        board.add_to_team("Beta", "red");
        board.remove_from_team("Alpha");
        let second = View::of(&board);
        assert_eq!(
            methods(&first.changes(&second)),
            ["team red 4 [\"Alpha\"]", "team red 3 [\"Beta\"]"]
        );
        board.remove_team("red");
        assert_eq!(
            methods(&second.changes(&View::of(&board))),
            ["team red 1 []"]
        );
    }

    #[test]
    fn a_player_moving_teams_leaves_the_old_one_first() {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_team("blue");
        board.add_to_team("Alpha", "red");
        let first = View::of(&board);
        // `blue` sorts before `red`; the leave still goes first.
        board.add_to_team("Alpha", "blue");
        board.add_team("amber");
        board.add_to_team("Beta", "amber");
        assert_eq!(
            methods(&first.changes(&View::of(&board))),
            [
                "team red 4 [\"Alpha\"]",
                "team amber 0 [\"Beta\"]",
                "team blue 3 [\"Alpha\"]"
            ]
        );
    }
}
