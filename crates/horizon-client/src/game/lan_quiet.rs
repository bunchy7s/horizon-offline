//! Quiet commands (`horizon_protocol::quiet`): a Horizon client's mod sends a
//! batch of command lines in one payload, and the server runs each exactly as
//! that player typing it would (`guest_command`'s checks, the player's own
//! permission level, at the time it runs) with its output suppressed, as
//! vanilla's functions run (`CommandSourceStack.withSuppressedOutput`): no
//! feedback and no failure in anyone's chat. A batch waits in a queue and runs
//! [`PER_TICK`] lines a player a tick, so a mod can't flood a server; each
//! finished batch is one line in the server's log.
use super::{Game, lan::LanHost, lan_actions::GuestCommand};
use horizon_protocol::quiet;
use std::collections::{HashMap, VecDeque};

/// Lines a player may have waiting at most; a batch past it loses the rest.
pub(super) const PENDING: usize = 4096;
/// Lines run a player a tick at most.
pub(super) const PER_TICK: usize = 64;

/// A batch waiting to run.
pub(super) struct QuietBatch {
    conn: u32,
    mod_id: String,
    lines: VecDeque<String>,
    ran: usize,
    failed: usize,
    /// Lines past [`PENDING`], never run.
    dropped: usize,
}

impl Game<'_> {
    /// A `horizon:quiet_commands` payload from `conn`: queued (or logged as
    /// refused when it breaks the payload's caps).
    pub(super) fn guest_quiet_commands(&mut self, host: &mut LanHost, conn: u32, data: &[u8]) {
        let Some(name) = host.hub.guest(conn).map(|g| g.name.clone()) else {
            return;
        };
        let batch = match quiet::decode(data) {
            Ok(b) => b,
            Err(e) => {
                super::dedicated::log(
                    "WARN",
                    &format!("{name} sent quiet commands that were refused: {e}"),
                );
                return;
            }
        };
        let waiting: usize = host
            .quiet
            .iter()
            .filter(|b| b.conn == conn)
            .map(|b| b.lines.len())
            .sum();
        let room = PENDING.saturating_sub(waiting);
        let dropped = batch.lines.len().saturating_sub(room);
        host.quiet.push_back(QuietBatch {
            conn,
            mod_id: batch.mod_id,
            lines: batch.lines.into_iter().take(room).collect(),
            ran: 0,
            failed: 0,
            dropped,
        });
    }

    /// The waiting batches, [`PER_TICK`] lines a player this tick, in order.
    pub(super) fn run_quiet_commands(&mut self) {
        let mut left: HashMap<u32, usize> = HashMap::new();
        let mut i = 0;
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let Some(batch) = host.quiet.get_mut(i) else {
                return;
            };
            let conn = batch.conn;
            let budget = left.entry(conn).or_insert(PER_TICK);
            let gone = host.hub.guest(conn).is_none();
            if gone || batch.lines.is_empty() {
                let batch = host.quiet.remove(i).expect("got it above");
                if !gone {
                    let name = host
                        .hub
                        .guest(conn)
                        .map_or(String::new(), |g| g.name.clone());
                    super::dedicated::log("INFO", &summary(&name, &batch));
                }
                continue;
            }
            if *budget == 0 {
                i += 1;
                continue;
            }
            *budget -= 1;
            let line = batch.lines.pop_front().expect("not empty");
            let ok = self.run_quiet_line(conn, &line);
            if let Some(batch) = self.lan.as_mut().and_then(|h| h.quiet.get_mut(i)) {
                batch.ran += 1;
                batch.failed += usize::from(!ok);
            }
        }
    }

    /// One line as `guest_command` would run it for `conn`, quietly. False
    /// when it is refused or fails. A command that carries a chat message
    /// (`say`, `msg`...) is refused: it would need the player's signature.
    fn run_quiet_line(&mut self, conn: u32, line: &str) -> bool {
        let allowed = self.options.dedicated.is_some() || self.allow_commands();
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let integrated = host.hub.config.integrated;
        if !integrated && (!host.settings.guest_commands || !allowed) {
            return false;
        }
        let Some((permission, uuid, name)) = host
            .hub
            .guest(conn)
            .map(|g| (g.permission, g.uuid, g.name.clone()))
        else {
            return false;
        };
        let command = line.trim_start_matches('/');
        if !self.signable_arguments(permission, command).is_empty() {
            return false;
        }
        // The mods hear it as they hear a typed command; the batch is quiet,
        // so their replies go nowhere.
        let mut canceled = false;
        self.native_player_command(uuid, &name, command, &mut canceled, &mut Vec::new());
        if canceled {
            return false;
        }
        self.run_guest_command(GuestCommand {
            conn,
            command: command.to_owned(),
            permission,
            integrated,
            quiet: true,
        })
    }
}

/// The batch's line in the server's log.
fn summary(name: &str, b: &QuietBatch) -> String {
    let mut line = format!(
        "{name} ran {} quiet command(s) from mod {}: {} failed",
        b.ran, b.mod_id, b.failed
    );
    if b.dropped > 0 {
        line += &format!(", {} dropped (more than {PENDING} waiting)", b.dropped);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_line_counts_the_batch() {
        let b = QuietBatch {
            conn: 1,
            mod_id: "orbital".into(),
            lines: VecDeque::new(),
            ran: 12,
            failed: 3,
            dropped: 0,
        };
        assert_eq!(
            summary("Alex", &b),
            "Alex ran 12 quiet command(s) from mod orbital: 3 failed"
        );
        let b = QuietBatch { dropped: 5, ..b };
        assert!(summary("Alex", &b).ends_with(", 5 dropped (more than 4096 waiting)"));
    }
}
