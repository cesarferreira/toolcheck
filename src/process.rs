use crate::{policy::Tool, version};
use semver::Version;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

pub const OUTPUT_LIMIT: usize = 65_536;

#[derive(Debug)]
pub struct Failure {
    pub issue: &'static str,
    pub message: String,
}

impl Failure {
    fn new(issue: &'static str, message: impl ToString) -> Self {
        Self {
            issue,
            message: message.to_string(),
        }
    }
}

pub fn resolve(command: &str) -> Vec<PathBuf> {
    let path = Path::new(command);
    let candidates: Vec<_> = if path.components().count() > 1 || path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join(command))
                    .collect()
            })
            .unwrap_or_default()
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|path| {
            #[cfg(windows)]
            let path = if path.extension().is_none() {
                path.with_extension("exe")
            } else {
                path
            };
            let path = if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            };
            let metadata = fs::metadata(&path).ok()?;
            if !metadata.is_file() || !executable(&path) {
                return None;
            }
            #[cfg(unix)]
            let key = {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            };
            #[cfg(not(unix))]
            let key = fs::canonicalize(&path).ok()?;
            if seen.insert(key) { Some(path) } else { None }
        })
        .collect()
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: path is a valid NUL-terminated string, and access retains no pointer.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: the child was spawned in its own process group; a negative
            // PID targets that group and never the checker's process group.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

enum Event {
    Data(bool, Vec<u8>),
    Done,
    Error(io::Error),
}

fn reader(mut pipe: impl Read + Send + 'static, stderr: bool, tx: SyncSender<Event>) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            let event = match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => Event::Data(stderr, buffer[..count].to_vec()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = tx.send(Event::Error(error));
                    break;
                }
            };
            if tx.send(event).is_err() {
                return;
            }
        }
        let _ = tx.send(Event::Done);
    });
}

pub fn probe(path: &OsStr, tool: &Tool, deadline: Instant) -> Result<Version, Failure> {
    if Instant::now() >= deadline {
        return Err(Failure::new(
            "timeout",
            "tool deadline exhausted before probe",
        ));
    }
    let mut command = Command::new(path);
    command
        .args(&tool.version_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|e| Failure::new("command_failed", format!("cannot run version command: {e}")))?;
    let mut running = Running(child);
    let (tx, rx) = mpsc::sync_channel(8);
    reader(
        running.0.stdout.take().expect("piped stdout"),
        false,
        tx.clone(),
    );
    reader(running.0.stderr.take().expect("piped stderr"), true, tx);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut done = 0;
    loop {
        if Instant::now() >= deadline {
            return Err(Failure::new(
                "timeout",
                format!(
                    "version command exceeded {} ms tool budget",
                    tool.timeout.as_millis()
                ),
            ));
        }
        let status = running
            .0
            .try_wait()
            .map_err(|e| Failure::new("command_failed", e))?;
        if let Some(status) = status {
            if !status.success() {
                return Err(Failure::new(
                    "command_failed",
                    format!("version command exited with {status}"),
                ));
            }
            if done == 2 {
                break;
            }
        }
        match rx.recv_timeout(
            Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
        ) {
            Ok(Event::Data(is_stderr, data)) => {
                if stdout.len() + stderr.len() + data.len() > OUTPUT_LIMIT {
                    return Err(Failure::new(
                        "output_limit",
                        format!(
                            "version output exceeded {OUTPUT_LIMIT} bytes across stdout and stderr"
                        ),
                    ));
                }
                if is_stderr {
                    stderr.extend(data);
                } else {
                    stdout.extend(data);
                }
            }
            Ok(Event::Done) => done += 1,
            Ok(Event::Error(error)) => {
                return Err(Failure::new(
                    "command_failed",
                    format!("cannot capture output: {error}"),
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(1)),
        }
    }
    let stdout = String::from_utf8(stdout)
        .map_err(|e| Failure::new("unknown_version", format!("stdout is not UTF-8: {e}")))?;
    let stderr = String::from_utf8(stderr)
        .map_err(|e| Failure::new("unknown_version", format!("stderr is not UTF-8: {e}")))?;
    version::extract(&stdout, &stderr, tool.version_regex.as_ref())
        .map_err(|e| Failure::new("unknown_version", format!("{e:#}")))
}
