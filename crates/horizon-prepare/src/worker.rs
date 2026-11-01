//! A bounded request/reply channel to one initialized exporter JVM.
use crate::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};

pub(crate) struct Worker {
    child: Child,
    input: ChildStdin,
    replies: Receiver<std::io::Result<String>>,
}

impl Worker {
    pub(crate) fn new(run: &Run, name: &str, classes: &Path, cp: &str) -> Result<Self, String> {
        let log = run.log_file(&run.local.join(format!("{name}-worker.log")))?;
        let mut child = java::command(&run.jdk.java)
            .args(java::BATCH_OPTIONS)
            .args([
                "-Xmx2G",
                "-cp",
                &format!(
                    "{}{}{}",
                    classes.display(),
                    if cfg!(windows) { ";" } else { ":" },
                    cp
                ),
                "dev.horizonlab.parity.PrepareWorker",
            ])
            // Vanilla's code logs relative to it (logs/): in the scratch, as `Run::run`.
            .current_dir(&run.local)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
        })
    }

    pub(crate) fn run(&mut self, main: &str, args: &[String], log: &Path) -> Result<(), String> {
        let task = Value::Array(vec![
            Value::Str(arg(log)),
            Value::Str(main.into()),
            Value::Array(args.iter().cloned().map(Value::Str).collect()),
        ]);
        writeln!(self.input, "{}", pyjson::dumps(&task, &pyjson::COMPACT))
            .map_err(|e| e.to_string())?;
        self.input.flush().map_err(|e| e.to_string())?;
        let reply = self
            .replies
            .recv_timeout(std::time::Duration::from_secs(3600))
            .map_err(|e| format!("Exporter JVM stopped or timed out: {e}"))?
            .map_err(|e| e.to_string())?;
        match pyjson::loads(&reply)?.as_str() {
            Some("") => Ok(()),
            _ => Err(format!("Exporter {main} failed: {reply}")),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
