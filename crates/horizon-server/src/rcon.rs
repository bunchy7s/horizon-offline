//! Vanilla's RCON (docs/server.md, section 8): `enable-rcon`, `rcon.port` and `rcon.password` in
//! server.properties (`[properties]` in server.toml) turn on a remote console, which listens as vanilla 26.3's
//! `net.minecraft.server.rcon.thread.RconThread` does and speaks `RconClient`'s protocol: little-endian packets
//! of a length, a request id, a type, a payload and two NULs. The supervisor serves it, so it answers while the
//! game restarts, and a command goes to the game as `horizon-server cmd` sends it.
//!
//! Where it differs from vanilla, on purpose: a packet is read from the stream (vanilla takes one `read` for
//! one packet, so a command that TCP splits closes the connection), the password is compared in constant time,
//! and a connection that hasn't logged in within [`LOGIN_WITHIN`] is closed, with at most [`MAX_CLIENTS`] open
//! at once (vanilla has neither limit). A reply is cut in packets of 4096 characters, where vanilla cuts at
//! 4096 UTF-16 units.

use crate::{
    api::{Phase, Shared, same_token},
    engine,
    supervisor::note,
};
use horizon_server_files::properties::{ServerProperties, SystemEntropy};
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// `RconClient.SERVERDATA_AUTH`: the login, with the password as its payload.
const AUTH: i32 = 3;
/// `SERVERDATA_EXECCOMMAND`: a command to run.
const EXEC: i32 = 2;
/// `SERVERDATA_AUTH_RESPONSE`: the answer to a login.
const AUTH_RESPONSE: i32 = 2;
/// `SERVERDATA_RESPONSE_VALUE`: the output of a command.
const RESPONSE: i32 = 0;
/// The request id of a refused login.
const AUTH_FAILURE: i32 = -1;
/// `RconClient.buf`: the most a request takes, the length included.
const MAX_REQUEST: usize = 1460;
/// The characters of a reply in one packet (`sendCmdResponse`).
const CHUNK: usize = 4096;
/// How long a connection may take to log in.
const LOGIN_WITHIN: Duration = Duration::from_secs(30);
/// The connections open at once.
const MAX_CLIENTS: usize = 16;

struct Settings {
    host: String,
    port: u16,
    password: String,
}

/// What server.properties makes of RCON, as `RconThread.create` reads it.
enum Setup {
    /// `enable-rcon=false`.
    Off,
    /// On, but vanilla leaves it off with this warning.
    Refused(String),
    On(Settings),
}

fn setup(dir: &Path) -> Result<Setup, String> {
    let file = crate::config::read_properties(&dir.join(crate::config::PROPERTIES))?;
    let p = ServerProperties::new(file, &mut SystemEntropy::default())?;
    if !p.enable_rcon {
        return Ok(Setup::Off);
    }
    let Some(port) = u16::try_from(p.rcon_port).ok().filter(|port| *port > 0) else {
        return Ok(Setup::Refused(format!(
            "Invalid rcon port {} found in server.properties, rcon disabled!",
            p.rcon_port
        )));
    };
    if p.rcon_password.is_empty() {
        return Ok(Setup::Refused(
            "No rcon password set in server.properties, rcon disabled!".into(),
        ));
    }
    Ok(Setup::On(Settings {
        host: if p.server_ip.is_empty() {
            "0.0.0.0".into()
        } else {
            p.server_ip
        },
        port,
        password: p.rcon_password,
    }))
}

/// Starts RCON if server.properties turns it on. As in vanilla, what keeps it off is a warning in the log,
/// not a server that doesn't start.
pub fn start(shared: Arc<Shared>) -> Result<Option<SocketAddr>, String> {
    // The file the game reads, as server.toml makes it.
    crate::config::Config::load(&shared.dir)?
        .write_properties(&shared.dir)
        .map_err(|e| format!("server.properties: {e}"))?;
    let settings = match setup(&shared.dir)? {
        Setup::Off => return Ok(None),
        Setup::Refused(why) => {
            note(&shared, "WARN", &why);
            return Ok(None);
        }
        Setup::On(settings) => settings,
    };
    let (host, port) = (settings.host.as_str(), settings.port);
    let listener = match TcpListener::bind((host, port)) {
        Ok(listener) => listener,
        Err(e) => {
            note(
                &shared,
                "WARN",
                &format!("Unable to initialise RCON on {host}:{port}: {e}"),
            );
            return Ok(None);
        }
    };
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (game, log) = (shared.clone(), shared.clone());
    serve(
        listener,
        settings.password,
        Limits {
            clients: MAX_CLIENTS,
            login: LOGIN_WITHIN,
        },
        move |line| run_on_game(&game, line),
        move |message| note(&log, "WARN", message),
    )
    .map_err(|e| format!("RCON: {e}"))?;
    note(&shared, "INFO", &format!("RCON running on {host}:{port}"));
    Ok(Some(address))
}

/// The control request that runs `line` as an RCON command: the console's, with the `Rcon` source
/// (`RconConsoleSource`), so the operators hear of it when `broadcast-rcon-to-ops` is on.
fn control_request(line: &str) -> String {
    format!("rcon {}", line.trim_start_matches('/'))
}

/// What the game's console printed for `line`, or why it couldn't run it. The lines are joined as they are in
/// vanilla (`RconConsoleSource` appends each message to one buffer).
fn run_on_game(shared: &Shared, line: &str) -> Result<String, String> {
    if line.contains(['\n', '\r']) {
        return Err("a command is one line".into());
    }
    if line.trim().is_empty() {
        return Ok(String::new());
    }
    if shared.state.lock().unwrap().phase != Phase::Running {
        return Err("the game isn't running right now".into());
    }
    let reply = engine::Control::connect(&shared.dir)
        .and_then(|mut game| game.ask(&control_request(line)))
        .map_err(|e| format!("the game didn't answer: {e}"))?;
    if let Some(error) = reply["error"].as_str() {
        return Err(error.to_owned());
    }
    Ok(reply["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect())
}

struct Limits {
    /// The connections open at once; one more is closed at once.
    clients: usize,
    /// How long a connection may take to log in.
    login: Duration,
}

/// Frees a connection's place when its thread ends.
struct Place(Arc<AtomicUsize>);

impl Drop for Place {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serves `listener` until the process ends, a thread per connection. `run` runs a command and `warn` logs.
fn serve(
    listener: TcpListener,
    password: String,
    limits: Limits,
    run: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static,
    warn: impl Fn(&str) + Send + Sync + 'static,
) -> io::Result<()> {
    let (password, run, warn) = (Arc::new(password), Arc::new(run), Arc::new(warn));
    let open = Arc::new(AtomicUsize::new(0));
    thread::Builder::new().name("rcon".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            if open.fetch_add(1, Ordering::SeqCst) >= limits.clients {
                open.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let place = Place(open.clone());
            let (password, run, warn) = (password.clone(), run.clone(), warn.clone());
            let login = limits.login;
            let _ = thread::Builder::new()
                .name("rcon-client".into())
                .spawn(move || {
                    // The place is free again before the client sees the connection close.
                    let mut stream = stream;
                    let _place = place;
                    let from = stream
                        .peer_addr()
                        .map_or_else(|_| "?".into(), |a| a.ip().to_string());
                    // The client going away, or sending what isn't RCON, ends its connection.
                    let _ = client(&mut stream, &password, login, &*run, &|why| {
                        warn(&format!("RCON {why} from {from}"));
                    });
                });
        }
    })?;
    Ok(())
}

/// One connection: login and commands until the client leaves, or sends what vanilla would close on.
fn client(
    stream: &mut TcpStream,
    password: &str,
    login: Duration,
    run: &dyn Fn(&str) -> Result<String, String>,
    warn: &dyn Fn(&str),
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let deadline = Instant::now() + login;
    let mut authed = false;
    loop {
        let Some(request) = read_request(stream, (!authed).then_some(deadline))? else {
            return Ok(());
        };
        match request.kind {
            AUTH => {
                // An empty password never matches, as in vanilla.
                authed = !request.text.is_empty() && same_token(&request.text, password);
                if authed {
                    send(stream, request.id, AUTH_RESPONSE, "")?;
                } else {
                    warn("login refused");
                    send(stream, AUTH_FAILURE, AUTH_RESPONSE, "")?;
                }
            }
            EXEC if authed => {
                let output = run(&request.text)
                    .unwrap_or_else(|why| format!("Error executing: {} ({why})", request.text));
                send_output(stream, request.id, &output)?;
            }
            EXEC => send(stream, AUTH_FAILURE, AUTH_RESPONSE, "")?,
            other => send_output(stream, request.id, &format!("Unknown request {other:x}"))?,
        }
    }
}

struct Request {
    id: i32,
    kind: i32,
    /// The payload up to its first NUL.
    text: String,
}

/// Reads exactly `buf.len()` bytes, within `deadline` if there is one. `false` when the client had closed
/// before the first.
fn fill(stream: &mut TcpStream, buf: &mut [u8], deadline: Option<Instant>) -> io::Result<bool> {
    let mut have = 0;
    while have < buf.len() {
        let left = deadline
            .map(|d| {
                d.checked_duration_since(Instant::now())
                    .filter(|left| !left.is_zero())
                    .ok_or(io::ErrorKind::TimedOut)
            })
            .transpose()?;
        stream.set_read_timeout(left)?;
        match stream.read(&mut buf[have..]) {
            Ok(0) if have == 0 => return Ok(false),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => have += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// The next request, or `None` when the client has closed. A length vanilla closes on (it takes a request of
/// 10 to 1460 bytes, the length's four included) is an error.
fn read_request(stream: &mut TcpStream, deadline: Option<Instant>) -> io::Result<Option<Request>> {
    let mut length = [0; 4];
    if !fill(stream, &mut length, deadline)? {
        return Ok(None);
    }
    let length = i32::from_le_bytes(length);
    let length = usize::try_from(length)
        .ok()
        .filter(|length| (10..=MAX_REQUEST - 4).contains(length))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an RCON request"))?;
    let mut body = vec![0; length];
    if !fill(stream, &mut body, deadline)? {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let int = |at: usize| i32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    // `PktUtils.stringFromByteArray`: the payload ends at the first NUL.
    let payload = &body[8..];
    let end = payload
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(payload.len());
    Ok(Some(Request {
        id: int(0),
        kind: int(4),
        text: String::from_utf8_lossy(&payload[..end]).into_owned(),
    }))
}

/// One packet, as `RconClient.send` writes it.
fn encode(id: i32, kind: i32, text: &str) -> Vec<u8> {
    let mut packet = Vec::with_capacity(text.len() + 14);
    packet.extend_from_slice(&(text.len() as i32 + 10).to_le_bytes());
    packet.extend_from_slice(&id.to_le_bytes());
    packet.extend_from_slice(&kind.to_le_bytes());
    packet.extend_from_slice(text.as_bytes());
    packet.extend_from_slice(&[0, 0]);
    packet
}

fn send(stream: &mut TcpStream, id: i32, kind: i32, text: &str) -> io::Result<()> {
    stream.write_all(&encode(id, kind, text))
}

/// A command's output in packets of [`CHUNK`] characters: `sendCmdResponse` sends one, even when it is empty.
fn send_output(stream: &mut TcpStream, id: i32, output: &str) -> io::Result<()> {
    let mut rest = output;
    loop {
        let end = rest
            .char_indices()
            .nth(CHUNK)
            .map_or(rest.len(), |(i, _)| i);
        send(stream, id, RESPONSE, &rest[..end])?;
        rest = &rest[end..];
        if rest.is_empty() {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_goes_to_the_game_as_an_rcon_one_without_its_slash() {
        assert_eq!(control_request("/say hi"), "rcon say hi");
        assert_eq!(control_request("time set day"), "rcon time set day");
    }

    /// A reply: its request id, type and payload.
    fn reply(stream: &mut TcpStream) -> (i32, i32, String) {
        let mut length = [0; 4];
        stream.read_exact(&mut length).unwrap();
        let mut body = vec![0; i32::from_le_bytes(length) as usize];
        stream.read_exact(&mut body).unwrap();
        let int = |at: usize| i32::from_le_bytes(body[at..at + 4].try_into().unwrap());
        let text = String::from_utf8(body[8..body.len() - 2].to_vec()).unwrap();
        assert_eq!(body[body.len() - 2..], [0, 0], "two NULs end the payload");
        (int(0), int(4), text)
    }

    /// RCON on a port of its own, whose commands answer `echo:` and the command, or fail on `boom`.
    fn server(limits: Limits) -> (SocketAddr, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let warned = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = warned.clone();
        serve(
            listener,
            "hunter2".into(),
            limits,
            |line| match line {
                "boom" => Err("it blew up".into()),
                "long" => Ok("é".repeat(CHUNK + 5)),
                "silent" => Ok(String::new()),
                line => Ok(format!("echo:{line}")),
            },
            move |message| log.lock().unwrap().push(message.to_owned()),
        )
        .unwrap();
        (address, warned)
    }

    fn limits() -> Limits {
        Limits {
            clients: MAX_CLIENTS,
            login: LOGIN_WITHIN,
        }
    }

    fn connect(address: SocketAddr) -> TcpStream {
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    #[test]
    fn a_packet_is_vanillas() {
        assert_eq!(
            encode(7, AUTH_RESPONSE, ""),
            [10, 0, 0, 0, 7, 0, 0, 0, 2, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            encode(-1, RESPONSE, "hi"),
            [
                12, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, b'h', b'i', 0, 0
            ]
        );
    }

    #[test]
    fn a_session_logs_in_and_runs_commands() {
        let (address, warned) = server(limits());
        let mut rcon = connect(address);
        // A command before the login, and a wrong password: refused, and the connection stays open.
        rcon.write_all(&encode(1, EXEC, "list")).unwrap();
        assert_eq!(
            reply(&mut rcon),
            (AUTH_FAILURE, AUTH_RESPONSE, String::new())
        );
        rcon.write_all(&encode(2, AUTH, "hunter3")).unwrap();
        assert_eq!(
            reply(&mut rcon),
            (AUTH_FAILURE, AUTH_RESPONSE, String::new())
        );
        rcon.write_all(&encode(3, AUTH, "")).unwrap();
        assert_eq!(
            reply(&mut rcon),
            (AUTH_FAILURE, AUTH_RESPONSE, String::new())
        );
        rcon.write_all(&encode(4, AUTH, "hunter2")).unwrap();
        assert_eq!(reply(&mut rcon), (4, AUTH_RESPONSE, String::new()));
        rcon.write_all(&encode(5, EXEC, "say hi")).unwrap();
        assert_eq!(reply(&mut rcon), (5, RESPONSE, "echo:say hi".into()));
        // A command that fails, one with no output, and a type vanilla doesn't know.
        rcon.write_all(&encode(6, EXEC, "boom")).unwrap();
        assert_eq!(
            reply(&mut rcon),
            (6, RESPONSE, "Error executing: boom (it blew up)".into())
        );
        rcon.write_all(&encode(7, EXEC, "silent")).unwrap();
        assert_eq!(reply(&mut rcon), (7, RESPONSE, String::new()));
        rcon.write_all(&encode(8, -2, "")).unwrap();
        assert_eq!(
            reply(&mut rcon),
            (8, RESPONSE, "Unknown request fffffffe".into())
        );
        // A login that fails takes the login back.
        rcon.write_all(&encode(9, AUTH, "nope")).unwrap();
        assert_eq!(reply(&mut rcon).0, AUTH_FAILURE);
        rcon.write_all(&encode(10, EXEC, "list")).unwrap();
        assert_eq!(reply(&mut rcon).0, AUTH_FAILURE);
        assert_eq!(
            warned.lock().unwrap().len(),
            3,
            "each refused login is logged"
        );
    }

    #[test]
    fn a_long_output_comes_in_packets_of_4096_characters() {
        let (address, _) = server(limits());
        let mut rcon = connect(address);
        rcon.write_all(&encode(1, AUTH, "hunter2")).unwrap();
        reply(&mut rcon);
        rcon.write_all(&encode(2, EXEC, "long")).unwrap();
        let first = reply(&mut rcon);
        let second = reply(&mut rcon);
        assert_eq!(
            (first.0, first.1, first.2.chars().count()),
            (2, RESPONSE, CHUNK)
        );
        assert_eq!((second.0, second.2.chars().count()), (2, 5));
    }

    #[test]
    fn a_packet_split_by_tcp_is_one_request() {
        let (address, _) = server(limits());
        let mut rcon = connect(address);
        let login = encode(1, AUTH, "hunter2");
        let (head, tail) = login.split_at(6);
        rcon.write_all(head).unwrap();
        rcon.flush().unwrap();
        thread::sleep(Duration::from_millis(50));
        rcon.write_all(tail).unwrap();
        assert_eq!(reply(&mut rcon), (1, AUTH_RESPONSE, String::new()));
        // Two requests in one write are two requests.
        let mut both = encode(2, EXEC, "a");
        both.extend(encode(3, EXEC, "b"));
        rcon.write_all(&both).unwrap();
        assert_eq!(reply(&mut rcon), (2, RESPONSE, "echo:a".into()));
        assert_eq!(reply(&mut rcon), (3, RESPONSE, "echo:b".into()));
    }

    #[test]
    fn what_vanilla_closes_on_closes() {
        let (address, _) = server(limits());
        // A length under 10, over what vanilla's buffer takes, or negative.
        for length in [9, MAX_REQUEST as i32 - 3, -1, i32::MAX] {
            let mut rcon = connect(address);
            // The server may close before the rest is written.
            let _ = rcon.write_all(&length.to_le_bytes());
            let _ = rcon.write_all(&[0; 16]);
            let mut byte = [0];
            assert!(
                matches!(rcon.read(&mut byte), Ok(0) | Err(_)),
                "length {length}"
            );
        }
        // The largest request takes.
        let mut rcon = connect(address);
        rcon.write_all(&encode(1, AUTH, "hunter2")).unwrap();
        reply(&mut rcon);
        let command = "x".repeat(MAX_REQUEST - 14);
        rcon.write_all(&encode(2, EXEC, &command)).unwrap();
        assert_eq!(reply(&mut rcon).2, format!("echo:{command}"));
    }

    #[test]
    fn a_connection_that_does_not_log_in_is_closed() {
        let (address, _) = server(Limits {
            clients: 1,
            login: Duration::from_secs(1),
        });
        let mut idle = connect(address);
        // The only place is taken: the next connection is closed at once.
        let mut second = connect(address);
        let mut byte = [0];
        assert!(matches!(second.read(&mut byte), Ok(0) | Err(_)));
        assert!(matches!(idle.read(&mut byte), Ok(0) | Err(_)));
        // Its place is free again.
        let mut again = connect(address);
        again.write_all(&encode(1, AUTH, "hunter2")).unwrap();
        assert_eq!(reply(&mut again).0, 1);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-rcon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".horizon")).unwrap();
        dir
    }

    fn properties(dir: &Path, text: &str) {
        std::fs::write(dir.join(crate::config::PROPERTIES), text).unwrap();
    }

    #[test]
    fn server_properties_turn_it_on_as_vanilla_reads_them() {
        let dir = scratch("setup");
        properties(&dir, "motd=hi\n");
        assert!(matches!(setup(&dir), Ok(Setup::Off)));
        properties(&dir, "enable-rcon=true\nrcon.password=pw\n");
        let Ok(Setup::On(on)) = setup(&dir) else {
            panic!("on")
        };
        assert_eq!(
            (on.host.as_str(), on.port, on.password.as_str()),
            ("0.0.0.0", 25575, "pw")
        );
        properties(
            &dir,
            "enable-rcon=true\nrcon.password=pw\nrcon.port=25600\nserver-ip=127.0.0.1\n",
        );
        let Ok(Setup::On(on)) = setup(&dir) else {
            panic!("on")
        };
        assert_eq!((on.host.as_str(), on.port), ("127.0.0.1", 25600));
        for (text, warning) in [
            (
                "enable-rcon=true\nrcon.password=pw\nrcon.port=0\n",
                "Invalid rcon port 0 found in server.properties, rcon disabled!",
            ),
            (
                "enable-rcon=true\nrcon.password=pw\nrcon.port=65536\n",
                "Invalid rcon port 65536 found in server.properties, rcon disabled!",
            ),
            (
                "enable-rcon=true\n",
                "No rcon password set in server.properties, rcon disabled!",
            ),
        ] {
            properties(&dir, text);
            let Ok(Setup::Refused(why)) = setup(&dir) else {
                panic!("refused: {text}")
            };
            assert_eq!(why, warning);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The game's control endpoint, as far as `rcon` goes: it answers `say hi` and fails on `fail`.
    fn game(dir: &Path) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            engine::control_path(dir),
            format!("{{\"port\":{port},\"token\":\"t\"}}"),
        )
        .unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut lines = io::BufRead::lines(io::BufReader::new(stream.try_clone().unwrap()));
                let mut stream = stream;
                while let Some(Ok(line)) = lines.next() {
                    let answer = match line.as_str() {
                        "auth t" => "{\"ok\":true}",
                        "rcon say hi" => "{\"output\":[\"[Server] \",\"hi\"]}",
                        _ => "{\"error\":\"unknown request\"}",
                    };
                    let _ = writeln!(stream, "{answer}");
                }
            }
        });
    }

    #[test]
    fn a_command_goes_to_the_game_as_an_rcon_request_that_says_what_the_console_says() {
        let dir = scratch("game");
        game(&dir);
        let shared = Shared::with_install(&dir, None);
        shared.set_phase(Phase::Restarting);
        assert_eq!(
            run_on_game(&shared, "say hi"),
            Err("the game isn't running right now".into())
        );
        shared.set_phase(Phase::Running);
        // The output of every message, one after the other, and a slash that vanilla's RCON takes too.
        assert_eq!(run_on_game(&shared, "say hi").as_deref(), Ok("[Server] hi"));
        assert_eq!(
            run_on_game(&shared, "/say hi").as_deref(),
            Ok("[Server] hi")
        );
        assert_eq!(run_on_game(&shared, "   ").as_deref(), Ok(""));
        assert_eq!(
            run_on_game(&shared, "say hi\nstop"),
            Err("a command is one line".into())
        );
        assert_eq!(
            run_on_game(&shared, "nonsense"),
            Err("unknown request".into())
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn server_toml_starts_it() {
        let dir = scratch("start");
        game(&dir);
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        // The dotted keys are quoted in TOML.
        // `[server] bind` is `server-ip`, the address RCON listens on too.
        let toml = crate::config::TEMPLATE
            .replacen("bind = \"\"", "bind = \"127.0.0.1\"", 1)
            .replace(
                "[properties]
",
                &format!(
                    "[properties]
enable-rcon = true
\"rcon.port\" = {port}
\"rcon.password\" = \"hunter2\"
"
                ),
            );
        std::fs::write(dir.join(crate::config::FILE), toml).unwrap();
        let shared = Shared::with_install(&dir, None);
        shared.set_phase(Phase::Running);
        let address = start(shared.clone()).unwrap().expect("RCON is on");
        assert_eq!(address, SocketAddr::from(([127, 0, 0, 1], port)));
        let mut rcon = connect(address);
        rcon.write_all(&encode(1, AUTH, "hunter2")).unwrap();
        assert_eq!(reply(&mut rcon), (1, AUTH_RESPONSE, String::new()));
        rcon.write_all(&encode(2, EXEC, "say hi")).unwrap();
        assert_eq!(reply(&mut rcon), (2, RESPONSE, "[Server] hi".into()));
        let (_, lines) = shared.log.lock().unwrap().since(0);
        let said = format!("RCON running on 127.0.0.1:{port}");
        assert!(lines.iter().any(|l| l.contains(&said)), "{lines:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
