//! Launching the program under test with `TM_*` variables, with a timeout.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// How a child run ended.
#[derive(Debug)]
pub enum Exit {
    /// The process exited (code is `None` when killed by a signal).
    Code(Option<i32>),
    /// The process exceeded the timeout and was killed.
    TimedOut,
}

impl Exit {
    pub fn success(&self) -> bool {
        matches!(self, Exit::Code(Some(0)))
    }

    pub fn describe(&self) -> String {
        match self {
            Exit::Code(Some(c)) => format!("exit code {c}"),
            Exit::Code(None) => "killed by a signal".into(),
            Exit::TimedOut => "timed out".into(),
        }
    }
}

pub struct Child<'a> {
    pub cmd: &'a [String],
    pub env: Vec<(&'static str, String)>,
    pub timeout: Duration,
    pub show_output: bool,
}

impl Child<'_> {
    pub fn run(&self) -> Result<Exit> {
        let (program, args) = self
            .cmd
            .split_first()
            .context("no command given after `--`")?;
        let mut command = Command::new(program);
        command.args(args);
        for var in [
            timemachine::envmode::TM_MODE,
            timemachine::envmode::TM_SEED,
            timemachine::envmode::TM_SOURCES,
            timemachine::envmode::TM_TRACE,
            timemachine::envmode::TM_DECISIONS,
            timemachine::envmode::TM_REPORT,
        ] {
            command.env_remove(var);
        }
        command.envs(self.env.iter().map(|(k, v)| (*k, v.as_str())));
        if !self.show_output {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start {program:?}"))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(status) = child.try_wait().context("waiting for child")? {
                return Ok(Exit::Code(code(status)));
            }
            if Instant::now() >= deadline {
                // Best effort: the process may have exited in the meantime.
                let _ = child.kill();
                let _ = child.wait();
                return Ok(Exit::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn code(status: ExitStatus) -> Option<i32> {
    status.code()
}

/// A private scratch directory for one `tm` invocation, removed on drop.
pub struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    pub fn new() -> Result<Scratch> {
        let base = std::env::temp_dir();
        for attempt in 0..100u32 {
            let dir = base.join(format!("tm-{}-{attempt}", std::process::id()));
            if std::fs::create_dir(&dir).is_ok() {
                return Ok(Scratch { dir });
            }
        }
        anyhow::bail!("cannot create a scratch directory in {}", base.display())
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn clear(&self, name: &str) {
        let _ = std::fs::remove_file(self.path(name));
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn display(p: &Path) -> String {
    p.display().to_string()
}
