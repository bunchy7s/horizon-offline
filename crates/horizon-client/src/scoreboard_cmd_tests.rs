use super::*;
use crate::command::{Registries, vanilla};

struct Mock {
    sb: Scoreboard,
    out: Vec<String>,
}
impl Env for Mock {
    fn sb(&self) -> &Scoreboard {
        &self.sb
    }
    fn sb_mut(&mut self) -> &mut Scoreboard {
        &mut self.sb
    }
    fn select(&mut self, s: &Selector) -> Vec<Holder> {
        if s.kind == 'e' {
            vec![self.named("Steve"), self.named("Alex")]
        } else {
            vec![self.named("Steve")]
        }
    }
    fn named(&mut self, name: &str) -> Holder {
        Holder {
            name: name.to_owned(),
            text: Text::literal(name),
            display: Some(literal(name)),
        }
    }
    fn source(&mut self) -> Option<(Holder, bool)> {
        Some((Holder::named("Steve"), true))
    }
    fn feedback(&mut self, message: Text) {
        self.out.push(message.plain());
    }
}
fn mock() -> Mock {
    Mock {
        sb: Scoreboard::default(),
        out: Vec::new(),
    }
}
fn run(env: &mut Mock, line: &str) -> CmdResult<i32> {
    let reg = Registries {
        permission: 4,
        ..Registries::default()
    };
    let parse = vanilla::dispatcher().parse(line, &reg);
    let (ctx, id) = parse.executable()?;
    let ctx = ctx.clone();
    execute(id, &ctx, env).expect("a scoreboard command")
}
fn fail(env: &mut Mock, line: &str) -> String {
    run(env, line).expect_err(line).message.plain()
}

#[test]
fn objectives() {
    let mut m = mock();
    assert_eq!(
        run(&mut m, "scoreboard objectives add kills playerKillCount").unwrap(),
        1
    );
    assert_eq!(
        fail(&mut m, "scoreboard objectives add kills dummy"),
        "commands.scoreboard.objectives.add.duplicate"
    );
    run(
        &mut m,
        "scoreboard objectives add hp health {\"text\":\"HP\",\"color\":\"red\"}",
    )
    .unwrap();
    let hp = m.sb.objective("hp").unwrap();
    assert_eq!(hp.render_type, RenderType::Hearts);
    assert_eq!(hp.display_name["color"], "red");
    assert_eq!(run(&mut m, "scoreboard objectives list").unwrap(), 2);
    run(&mut m, "scoreboard objectives setdisplay sidebar kills").unwrap();
    assert_eq!(
        fail(&mut m, "scoreboard objectives setdisplay sidebar kills"),
        "commands.scoreboard.objectives.display.alreadySet"
    );
    run(&mut m, "scoreboard objectives setdisplay sidebar").unwrap();
    assert_eq!(
        fail(&mut m, "scoreboard objectives setdisplay sidebar"),
        "commands.scoreboard.objectives.display.alreadyEmpty"
    );
    run(
        &mut m,
        "scoreboard objectives modify kills rendertype hearts",
    )
    .unwrap();
    assert_eq!(
        m.sb.objective("kills").unwrap().render_type,
        RenderType::Hearts
    );
    run(
        &mut m,
        "scoreboard objectives modify kills displayautoupdate true",
    )
    .unwrap();
    run(
        &mut m,
        "scoreboard objectives modify kills numberformat fixed \"x\"",
    )
    .unwrap();
    assert_eq!(
        m.sb.objective("kills").unwrap().number_format,
        Some(NumberFormat::Fixed(literal("x")))
    );
    run(&mut m, "scoreboard objectives modify kills numberformat").unwrap();
    assert!(m.sb.objective("kills").unwrap().number_format.is_none());
    run(&mut m, "scoreboard objectives remove kills").unwrap();
    assert_eq!(
        fail(&mut m, "scoreboard objectives remove kills"),
        "arguments.objective.notFound"
    );
}

#[test]
fn players() {
    let mut m = mock();
    run(&mut m, "scoreboard objectives add pts dummy").unwrap();
    run(&mut m, "scoreboard objectives add hp health").unwrap();
    assert_eq!(
        run(&mut m, "scoreboard players set Steve pts 5").unwrap(),
        5
    );
    assert_eq!(
        run(&mut m, "scoreboard players add Steve pts 3").unwrap(),
        8
    );
    assert_eq!(
        run(&mut m, "scoreboard players remove Steve pts 10").unwrap(),
        -2
    );
    assert_eq!(run(&mut m, "scoreboard players get Steve pts").unwrap(), -2);
    assert_eq!(
        fail(&mut m, "scoreboard players get Alex pts"),
        "commands.scoreboard.players.get.null"
    );
    assert_eq!(
        fail(&mut m, "scoreboard players set Steve hp 5"),
        "arguments.objective.readonly"
    );
    assert_eq!(run(&mut m, "scoreboard players set @e pts 4").unwrap(), 8);
    assert_eq!(m.sb.score("Alex", "pts").unwrap().value, 4);
    // Entity holders get their display name stored when auto-update is on.
    run(
        &mut m,
        "scoreboard objectives modify pts displayautoupdate true",
    )
    .unwrap();
    run(&mut m, "scoreboard players set @s pts 1").unwrap();
    assert_eq!(
        m.sb.score("Steve", "pts").unwrap().display,
        Some(literal("Steve"))
    );
    assert_eq!(run(&mut m, "scoreboard players list").unwrap(), 2);
    assert_eq!(run(&mut m, "scoreboard players list Steve").unwrap(), 1);
    // operation
    run(&mut m, "scoreboard players set Steve pts 17").unwrap();
    run(&mut m, "scoreboard players set Alex pts 5").unwrap();
    run(&mut m, "scoreboard players operation Steve pts %= Alex pts").unwrap();
    assert_eq!(m.sb.score("Steve", "pts").unwrap().value, 2);
    run(&mut m, "scoreboard players operation Steve pts >< Alex pts").unwrap();
    assert_eq!(m.sb.score("Steve", "pts").unwrap().value, 5);
    assert_eq!(m.sb.score("Alex", "pts").unwrap().value, 2);
    run(&mut m, "scoreboard players set Alex pts 0").unwrap();
    assert_eq!(
        fail(&mut m, "scoreboard players operation Steve pts /= Alex pts"),
        "arguments.operation.div0"
    );
    // wildcard
    assert_eq!(run(&mut m, "scoreboard players reset * pts").unwrap(), 2);
    assert!(m.sb.tracked().is_empty());
    assert_eq!(
        fail(&mut m, "scoreboard players reset * pts"),
        "argument.scoreHolder.empty"
    );
}

#[test]
fn operations_use_floor_semantics() {
    assert_eq!(apply_operation("/=", -7, 2).unwrap().0, -4);
    assert_eq!(apply_operation("/=", 7, -2).unwrap().0, -4);
    assert_eq!(apply_operation("/=", 7, 2).unwrap().0, 3);
    assert_eq!(apply_operation("%=", -7, 3).unwrap().0, 2);
    assert_eq!(apply_operation("<", 3, 9).unwrap().0, 3);
    assert_eq!(apply_operation(">", 3, 9).unwrap().0, 9);
    assert_eq!(apply_operation("*=", i32::MAX, 2).unwrap().0, -2);
}

#[test]
fn division_wraps_minimum_integer_by_negative_one() {
    let mut m = mock();
    run(&mut m, "scoreboard objectives add poc dummy").unwrap();
    run(&mut m, "scoreboard players set target poc -2147483648").unwrap();
    run(&mut m, "scoreboard players set divisor poc -1").unwrap();
    run(
        &mut m,
        "scoreboard players operation target poc /= divisor poc",
    )
    .unwrap();
    assert_eq!(
        run(&mut m, "scoreboard players get target poc").unwrap(),
        i32::MIN
    );
    assert_eq!(
        run(&mut m, "scoreboard players get divisor poc").unwrap(),
        -1
    );
}

#[test]
fn remainder_handles_integer_boundaries() {
    for (dividend, divisor, expected) in [
        (i32::MAX - 1, i32::MAX, i32::MAX - 1),
        (7, -3, -2),
        (-7, 3, 2),
        (i32::MIN, -1, 0),
        (-1, i32::MIN, -1),
        (1, i32::MIN, i32::MIN + 1),
        (0, i32::MAX, 0),
    ] {
        let mut m = mock();
        run(&mut m, "scoreboard objectives add boundary dummy").unwrap();
        run(
            &mut m,
            &format!("scoreboard players set left boundary {dividend}"),
        )
        .unwrap();
        run(
            &mut m,
            &format!("scoreboard players set right boundary {divisor}"),
        )
        .unwrap();
        run(
            &mut m,
            "scoreboard players operation left boundary %= right boundary",
        )
        .unwrap();
        assert_eq!(
            run(&mut m, "scoreboard players get left boundary").unwrap(),
            expected,
            "{dividend} %= {divisor}"
        );
        assert_eq!(
            run(&mut m, "scoreboard players get right boundary").unwrap(),
            divisor
        );
    }
    assert!(apply_operation("%=", 1, 0).is_err());
}

#[test]
fn triggers() {
    let mut m = mock();
    run(&mut m, "scoreboard objectives add t trigger").unwrap();
    run(&mut m, "scoreboard objectives add d dummy").unwrap();
    assert_eq!(
        fail(&mut m, "trigger t"),
        "commands.trigger.failed.unprimed"
    );
    assert_eq!(fail(&mut m, "trigger d"), "commands.trigger.failed.invalid");
    run(&mut m, "scoreboard players enable Steve t").unwrap();
    assert_eq!(
        fail(&mut m, "scoreboard players enable Steve t"),
        "commands.scoreboard.players.enable.failed"
    );
    assert_eq!(run(&mut m, "trigger t").unwrap(), 1);
    // Triggering locks the objective again.
    assert_eq!(
        fail(&mut m, "trigger t"),
        "commands.trigger.failed.unprimed"
    );
    run(&mut m, "scoreboard players enable Steve t").unwrap();
    assert_eq!(run(&mut m, "trigger t add 4").unwrap(), 5);
    run(&mut m, "scoreboard players enable Steve t").unwrap();
    assert_eq!(run(&mut m, "trigger t set 9").unwrap(), 9);
    assert_eq!(
        fail(&mut m, "scoreboard players enable Steve d"),
        "commands.scoreboard.players.enable.invalid"
    );
}

#[test]
fn teams() {
    let mut m = mock();
    assert_eq!(run(&mut m, "team add red").unwrap(), 1);
    assert_eq!(fail(&mut m, "team add red"), "commands.team.add.duplicate");
    assert_eq!(fail(&mut m, "team remove nope"), "team.notFound");
    run(&mut m, "team modify red color red").unwrap();
    assert_eq!(
        fail(&mut m, "team modify red color red"),
        "commands.team.option.color.unchanged"
    );
    assert_eq!(m.sb.team("red").unwrap().color.as_deref(), Some("red"));
    run(&mut m, "team modify red color reset").unwrap();
    assert!(m.sb.team("red").unwrap().color.is_none());
    run(&mut m, "team modify red friendlyFire false").unwrap();
    assert_eq!(
        fail(&mut m, "team modify red friendlyFire false"),
        "commands.team.option.friendlyfire.alreadyDisabled"
    );
    run(
        &mut m,
        "team modify red nametagVisibility hideForOtherTeams",
    )
    .unwrap();
    run(&mut m, "team modify red collisionRule pushOwnTeam").unwrap();
    run(&mut m, "team modify red prefix {\"text\":\"[R] \"}").unwrap();
    run(&mut m, "team modify red seeFriendlyInvisibles false").unwrap();
    let t = m.sb.team("red").unwrap();
    assert_eq!(t.nametag_visibility, Visibility::HideForOtherTeams);
    assert_eq!(t.collision_rule, sb::CollisionRule::PushOwnTeam);
    assert!(!t.see_friendly_invisibles);
    // join defaults to the executor
    assert_eq!(run(&mut m, "team join red").unwrap(), 1);
    // addPlayerToTeam leaves and rejoins: the player counts again.
    assert_eq!(run(&mut m, "team join red @e").unwrap(), 2);
    assert_eq!(m.sb.team("red").unwrap().players.len(), 2);
    assert_eq!(run(&mut m, "team list red").unwrap(), 2);
    assert_eq!(run(&mut m, "team leave Alex").unwrap(), 1);
    assert_eq!(run(&mut m, "team empty red").unwrap(), 1);
    assert_eq!(
        fail(&mut m, "team empty red"),
        "commands.team.empty.unchanged"
    );
    assert_eq!(run(&mut m, "team remove red").unwrap(), 0);
    assert_eq!(run(&mut m, "team list").unwrap(), 0);
}

#[test]
fn selector_options_parse() {
    let reg = Registries {
        permission: 4,
        ..Registries::default()
    };
    let parse = vanilla::dispatcher().parse(
        "scoreboard players get @e[scores={kills=1..,deaths=..3},team=!red,nbt={a:1b},tag=x,limit=1] kills",
        &reg,
    );
    let (ctx, _) = parse.executable().unwrap();
    let Some(Arg::ScoreHolder(ScoreHolderArg::Selector(s))) = ctx.get("target") else {
        panic!("selector");
    };
    assert_eq!(
        s.scores,
        vec![
            ("kills".to_owned(), (Some(1), None)),
            ("deaths".to_owned(), (None, Some(3)))
        ]
    );
    assert_eq!(s.teams, vec![("red".to_owned(), true)]);
    assert_eq!(s.nbt.len(), 1);
    // Single-target arguments refuse multi-entity selectors.
    let parse = vanilla::dispatcher().parse("scoreboard players get @e kills", &reg);
    assert!(parse.executable().is_err());
}
