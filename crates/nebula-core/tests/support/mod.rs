//! Test containment, shared by every suite that runs git or `neb`: no test
//! reads or writes the developer's real host state, and none discovers a git
//! repository above its own temporary root (STD-03 §R20, STD-04 §R7).
//!
//! - **This process** is isolated before any test thread starts, by the
//!   constructor below (STD-04 §R6): a fresh `HOME`, none of the variables in
//!   [`CLEARED`] or any `GIT_*`, and a git that reads only [`gitconfig`] and
//!   stops climbing at the temporary directory. That covers the git children
//!   production code starts in-process, which no builder can reach.
//! - **Every child a test spawns** comes from [`command`], which sets the same
//!   isolation on the child command itself (STD-03 §R20), and is owned by a
//!   [`ChildGuard`] that kills and reaps it on drop and waits for it only up to
//!   a deadline (STD-03 §R17, §R18).
//!
//! `crates/nebula-core/tests/core.rs` declares this module;
//! `crates/neb/tests/cli.rs` and `apps/desktop/src-tauri/tests/roundtrip.rs`
//! include it by path, so there is one list of what a test may not inherit.
//! A variable a test needs (`NEBULA_ROOT`, `EDITOR`, `GIT_DIR`) is set on its
//! command after the builder.

// Each suite uses a different part of this module.
#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Removed from this process and from every test child. `neb`'s own roots
/// and editor; the Orbit run context, under which writes refuse; and the
/// terminal settings output depends on. A developer with a real Observatory
/// checkout exported would otherwise resolve records the tests expect to go
/// missing. Tests that exercise one set it explicitly after the builder.
pub const CLEARED: &[&str] = &[
    "NEBULA_ROOT",
    "OBSERVATORY_ROOT",
    "VISUAL",
    "EDITOR",
    "XDG_CONFIG_HOME",
    "ORBIT_RUN_ID",
    "ORBIT_TASK_ID",
    "NEBULA_READ_ONLY",
    "NO_COLOR",
    "CLICOLOR_FORCE",
    "TERM",
    "COLUMNS",
];

/// What `git rev-parse --local-env-vars` prints: the variables that point git
/// at one particular repository, as a git hook or `git rebase --exec`
/// exports them. Listed rather than asked for, so building a command runs no
/// process; `cli.rs` checks the list against the installed git. Every other
/// inherited `GIT_*` is removed as well.
pub const GIT_LOCAL_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// How long a test waits for any child it started. Generous, because a
/// loaded CI machine is slow; the point is that the wait ends.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// The global git configuration every test git reads instead of the host's:
/// an identity, and no signing, so a commit never depends on the machine.
const GITCONFIG: &str = "\
[user]
\tname = neb-test
\temail = neb-test@example.invalid
[commit]
\tgpgsign = false
[tag]
\tgpgsign = false
[init]
\tdefaultBranch = main
";

/// This process's own home, made by [`isolate_this_process`].
static HOME: OnceLock<PathBuf> = OnceLock::new();

/// Isolate this process before `main`, and so before the test harness starts
/// any thread (STD-04 §R6): nothing else can be reading the environment.
#[ctor::ctor]
unsafe fn isolate_this_process() {
    let home = tempfile::Builder::new()
        .prefix("nebula-test-home.")
        .tempdir()
        .expect("creating the test home")
        .keep();
    std::fs::write(home.join("gitconfig"), GITCONFIG).expect("writing the test gitconfig");
    let ceilings = ceilings(&home);
    for name in removed() {
        // SAFETY: this runs before `main`, so no other thread exists to read
        // or write the environment concurrently.
        unsafe { std::env::remove_var(name) };
    }
    // SAFETY: as above.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_CONFIG_GLOBAL", home.join("gitconfig"));
        std::env::set_var("GIT_CEILING_DIRECTORIES", ceilings);
    }
    HOME.set(home).expect("isolated once");
}

/// Remove this process's home when it exits.
#[ctor::dtor]
unsafe fn remove_this_process_home() {
    if let Some(home) = HOME.get() {
        let _ = std::fs::remove_dir_all(home);
    }
}

/// This process's isolated home: for children that belong to no fixture,
/// such as a test's own git.
pub fn home() -> &'static Path {
    HOME.get()
        .expect("the test process is isolated before main")
}

/// The file `GIT_CONFIG_GLOBAL` names, for this process and every child.
pub fn gitconfig() -> PathBuf {
    home().join("gitconfig")
}

/// Every name to remove: [`CLEARED`], [`GIT_LOCAL_ENV`], and every `GIT_*`
/// this process inherited.
fn removed() -> Vec<OsString> {
    let inherited = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| name.as_encoded_bytes().starts_with(b"GIT_"));
    CLEARED
        .iter()
        .chain(GIT_LOCAL_ENV)
        .map(OsString::from)
        .chain(inherited)
        .collect()
}

/// Where git discovery stops: the directory holding the fixture's home, and
/// the system temporary directory, so a repository around either one is
/// never found from inside a fixture (STD-04 §R7).
fn ceilings(home: &Path) -> OsString {
    let dirs = home
        .parent()
        .into_iter()
        .map(Path::to_path_buf)
        .chain([std::env::temp_dir()])
        .filter(|dir| dir.is_absolute());
    std::env::join_paths(dirs).expect("temporary paths hold no separator")
}

/// Isolate `cmd` from the host: `HOME` is `home`; nothing in [`CLEARED`] and
/// no `GIT_*` is inherited; git reads [`gitconfig`] and no system file, and
/// does not climb into the parent of `home` or the temporary directory.
pub fn isolate<'a>(cmd: &'a mut Command, home: &Path) -> &'a mut Command {
    for name in removed() {
        cmd.env_remove(name);
    }
    cmd.env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", gitconfig())
        .env("GIT_CEILING_DIRECTORIES", ceilings(home))
}

/// The one place a test creates a child command: `program`, isolated with
/// `home` as its home.
pub fn command(program: impl AsRef<OsStr>, home: &Path) -> Command {
    let mut cmd = Command::new(program);
    isolate(&mut cmd, home);
    cmd
}

/// `git -C dir`, isolated with `home` as its home.
pub fn git_command(dir: &Path, home: &Path) -> Command {
    let mut cmd = command("git", home);
    cmd.arg("-C").arg(dir);
    cmd
}

/// `cmd` as a person would type it, for messages: the program's file name
/// and its arguments.
pub fn describe(cmd: &Command) -> String {
    let program = Path::new(cmd.get_program());
    let program = program.file_name().unwrap_or(program.as_os_str());
    std::iter::once(program)
        .chain(cmd.get_args())
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run `cmd` to completion the way [`Command::output`] does (no stdin,
/// stdout and stderr captured), under a guard and within `deadline`.
pub fn output(cmd: &mut Command, deadline: Duration) -> Result<Output, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ChildGuard::spawn(cmd)?.wait_with_output(deadline)
}

/// A spawned child that is killed and reaped when dropped, so a failed
/// assertion between spawn and wait leaves no process behind (STD-03 §R18).
/// Its waits end at a deadline, checking the child's liveness in-process
/// (STD-03 §R17).
pub struct ChildGuard {
    what: String,
    child: Child,
    reaped: bool,
}

impl ChildGuard {
    /// Spawn `cmd` with the stdio it was given.
    pub fn spawn(cmd: &mut Command) -> Result<Self, String> {
        let what = describe(cmd);
        let child = cmd.spawn().map_err(|e| format!("spawning `{what}`: {e}"))?;
        Ok(Self {
            what,
            child,
            reaped: false,
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The child's piped stdin, taken; dropping it closes the pipe.
    pub fn take_stdin(&mut self) -> ChildStdin {
        self.child.stdin.take().expect("stdin was piped")
    }

    /// The child's piped stdout, taken.
    pub fn take_stdout(&mut self) -> ChildStdout {
        self.child.stdout.take().expect("stdout was piped")
    }

    /// Wait up to `deadline` for the child to exit. Past it, the child is
    /// killed and reaped, and the error names the command.
    pub fn wait(&mut self, deadline: Duration) -> Result<ExitStatus, String> {
        let until = Instant::now() + deadline;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.reaped = true;
                    return Ok(status);
                }
                Ok(None) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    self.kill();
                    return Err(format!(
                        "`{}` was still running after {deadline:?}; killed it",
                        self.what
                    ));
                }
                Err(e) => {
                    self.kill();
                    return Err(format!("waiting for `{}`: {e}", self.what));
                }
            }
        }
    }

    /// [`Self::wait`], collecting whatever of stdout and stderr was piped.
    /// The pipes are drained on threads while the child runs, so a child
    /// that writes more than a pipe holds cannot stall, and those reads end
    /// at the same deadline.
    pub fn wait_with_output(mut self, deadline: Duration) -> Result<Output, String> {
        let until = Instant::now() + deadline;
        let stdout = self.child.stdout.take().map(drain);
        let stderr = self.child.stderr.take().map(drain);
        let status = self.wait(deadline)?;
        let collect = |reader: Option<mpsc::Receiver<Vec<u8>>>, stream: &str| {
            let Some(reader) = reader else {
                return Ok(Vec::new());
            };
            // Past the deadline, a moment more: the child has exited, so only
            // a descendant still holding the pipe keeps the read open.
            let left = until
                .saturating_duration_since(Instant::now())
                .max(Duration::from_secs(1));
            reader
                .recv_timeout(left)
                .map_err(|_| format!("`{}` exited but its {stream} stayed open", self.what))
        };
        Ok(Output {
            status,
            stdout: collect(stdout, "stdout")?,
            stderr: collect(stderr, "stderr")?,
        })
    }

    fn kill(&mut self) {
        // Either may fail only because the child already exited, which is
        // the outcome wanted.
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reaped = true;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.reaped {
            self.kill();
        }
    }
}

/// Read `pipe` to its end on a thread; the bytes arrive on the channel.
fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        let _ = send.send(bytes);
    });
    receive
}

/// Every line of `source` that creates a `Command` outside the `allowed`
/// functions, as `<line>: in `<fn>`: <text>`: a child that skipped the
/// isolating builder. A line belongs to the nearest `fn` item above it, so a
/// closure inside a test is the test's.
pub fn commands_outside(source: &str, allowed: &[&str]) -> Vec<String> {
    // Spelled in two halves so this function's own text is not a match.
    let needle = concat!("Command", "::new(");
    let mut within = "";
    let mut strays = Vec::new();
    for (number, line) in source.lines().enumerate() {
        if let Some(name) = fn_name(line) {
            within = name;
        }
        if line.contains(needle) && !allowed.contains(&within) {
            strays.push(format!("{}: in `{within}`: {}", number + 1, line.trim()));
        }
    }
    strays
}

/// The name of the function `line` declares, if it declares one.
fn fn_name(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = rest.strip_prefix("pub ").unwrap_or(rest);
    let rest = rest.strip_prefix("unsafe ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}
