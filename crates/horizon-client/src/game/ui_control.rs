//! The widget tree of the open screen for agents (`ui`, `click ID`, `set ID
//! VALUE`; docs/agent-control.md): what the last frame drew, with ids, and
//! clicks that go through the same window events as the mouse.
use super::{Game, Screen, agent::Handled};
use horizon_client::{
    Result,
    control::Request,
    creative::Creative,
    widgets::{self, Kind, Node},
};
use serde_json::{Value, json};
use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use winit::{event::MouseButton, event_loop::ActiveEventLoop};

pub(super) const HELP: &[&str] = &[
    "ui                             every widget of the open screen: id, kind, label, bounds, value",
    "click ID [right|middle] [shift] click a widget of ui by its id (slot-N: as click-slot N)",
    "set ID VALUE                   move a slider to VALUE (0..1), as a click on its track",
];

/// After a click, `ui`, `click` and `set` wait for a frame drawn since, so
/// they never read the screen the click left.
#[derive(Default)]
pub(super) struct UiWait {
    /// The widget generation at the last click, and when it happened.
    after: Option<(u64, Instant)>,
    waiting: Vec<Request>,
}

impl UiWait {
    fn stale(&self) -> bool {
        self.after.is_some_and(|(generation, at)| {
            generation == widgets::generation() && at.elapsed() < Duration::from_secs(1)
        })
    }
    /// The commands that can run now.
    pub(super) fn ready(&mut self) -> Vec<Request> {
        if self.stale() {
            return Vec::new();
        }
        self.after = None;
        std::mem::take(&mut self.waiting)
    }
    /// The screen changed: widget commands wait for it to be drawn again.
    pub(super) fn clicked(&mut self) {
        self.after = Some((widgets::generation(), Instant::now()));
    }
}

impl Game<'_> {
    /// The open screen's widgets: those the last frame drew, then the slots
    /// of an open inventory or menu (`slot-ID`, the ids of `click-slot`).
    pub(super) fn ui_nodes(&self) -> Vec<Node> {
        let size = self.gui_scale().map(|b| [b.width, b.height]);
        let mut nodes = widgets::tree(&widgets::last_labelled(), &widgets::last_targets(), size);
        if matches!(self.screen, Screen::Inventory | Screen::Creative)
            && self.title.is_none()
            && let Ok((ox, oy)) = self.gui_origin()
        {
            nodes.extend(self.slots().into_iter().map(|i| {
                let (x, y) = self.slot_position(i);
                Node {
                    id: format!("slot-{i}"),
                    kind: Kind::Slot,
                    label: None,
                    rect: [ox + x, oy + y, 16.0, 16.0],
                    enabled: true,
                }
            }));
        }
        nodes
    }

    fn node_value(&mut self, node: &Node) -> Value {
        let mut v = json!({
            "id": node.id,
            "kind": node.kind.name(),
            "bounds": node.rect,
            "enabled": node.enabled,
        });
        if let Some(label) = &node.label {
            v["label"] = json!(label);
        }
        match node.kind {
            Kind::Slider { value, .. } => v["value"] = json!(value),
            Kind::Tab(selected) => v["selected"] = json!(selected),
            Kind::Slot => {
                if let Some(slot) = node.id.strip_prefix("slot-").and_then(|s| s.parse().ok()) {
                    let stack = if slot == 200 {
                        self.craft_result()
                    } else if slot >= Creative::DISPLAY {
                        // An item tab's display slot shows the tab's list.
                        let first = self.creative_screen.first_row(&self.pack.names);
                        self.creative_screen
                            .shown(&self.pack.names, first * 9 + slot - Creative::DISPLAY)
                    } else {
                        self.slot_mut(slot).clone()
                    };
                    v["stack"] = stack.map_or(Value::Null, |s| self.stack_value(&s));
                }
            }
            _ => {}
        }
        v
    }

    fn find_node(&self, id: &str) -> Result<Node> {
        let nodes = self.ui_nodes();
        nodes.iter().find(|n| n.id == id).cloned().ok_or_else(|| {
            let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
            format!(
                "no widget {id:?} on the {} screen; widgets: {ids:?}",
                self.screen_label()
            )
            .into()
        })
    }

    /// `ui`, `click` and `set`; `Handled::No` for other commands.
    pub(super) fn ui_command(
        &mut self,
        words: &[&str],
        line: &str,
        reply: &Sender<String>,
        event_loop: &ActiveEventLoop,
    ) -> Result<Handled> {
        if !matches!(words[0], "ui" | "click" | "set") {
            return Ok(Handled::No);
        }
        if self.ui_wait.stale() {
            self.ui_wait.waiting.push(Request {
                line: line.to_owned(),
                reply: reply.clone(),
            });
            return Ok(Handled::Deferred);
        }
        let id = || words.get(1).copied().ok_or("missing the widget id; see ui");
        match words[0] {
            "ui" => {
                let nodes = self.ui_nodes();
                let widgets: Vec<Value> = nodes.iter().map(|n| self.node_value(n)).collect();
                let size = self
                    .gui_scale()
                    .map(|b| [b.width.round(), b.height.round()]);
                Ok(Handled::Reply(json!({
                    "screen": self.screen_label(),
                    "gui_size": size,
                    "frame": widgets::generation(),
                    "widgets": widgets,
                })))
            }
            "click" => {
                let node = self.find_node(id()?)?;
                if !node.enabled {
                    return Err(format!("widget {:?} is disabled", node.id).into());
                }
                let right = words.contains(&"right");
                let middle = words.contains(&"middle");
                if node.kind == Kind::Slot && !middle {
                    let slot = node.id.trim_start_matches("slot-");
                    let mut click = vec!["click-slot", slot];
                    click.extend(words.iter().filter(|w| matches!(**w, "right" | "shift")));
                    self.station_command(&click)?;
                } else {
                    let (x, y) = (
                        node.rect[0] + node.rect[2] / 2.0,
                        node.rect[1] + node.rect[3] / 2.0,
                    );
                    // The middle button goes through the window's own
                    // events at the widget's centre, slots included.
                    let button = if middle {
                        MouseButton::Middle
                    } else if right {
                        MouseButton::Right
                    } else {
                        MouseButton::Left
                    };
                    self.synth_click(event_loop, x, y, button)?;
                }
                self.ui_wait.clicked();
                Ok(Handled::Reply(
                    json!({ "ok": true, "clicked": node.id, "screen": self.screen_label() }),
                ))
            }
            "set" => {
                let node = self.find_node(id()?)?;
                let value: f32 = words
                    .get(2)
                    .ok_or("set ID VALUE: missing the value")?
                    .parse()
                    .map_err(|_| "set ID VALUE: the value is a number from 0 to 1")?;
                let Kind::Slider { track, .. } = node.kind else {
                    return Err(format!(
                        "widget {:?} is a {}; set moves sliders (click a button, type into a field)",
                        node.id,
                        node.kind.name()
                    )
                    .into());
                };
                if !node.enabled {
                    return Err(format!("widget {:?} is disabled", node.id).into());
                }
                // AbstractSliderButton.setValueFromMouse: (x - track start) / track width.
                let x = track[0] + value.clamp(0.0, 1.0) * track[1];
                let y = node.rect[1] + node.rect[3] / 2.0;
                self.synth_click(event_loop, x, y, MouseButton::Left)?;
                self.ui_wait.clicked();
                Ok(Handled::Reply(
                    json!({ "ok": true, "set": node.id, "screen": self.screen_label() }),
                ))
            }
            _ => Ok(Handled::No),
        }
    }
}
