//! The `compile` tool's sandboxed compiler process (B4, #40).
//!
//! The operator configures the weave-language binary (`serve --compiler
//! PATH`). Each call runs `weave sdk --untrusted [--read-only]` as a child
//! process that gets exactly one `weave-compiler-request/1` on stdin and
//! writes one response on stdout. The command-line flags are a policy floor
//! the request cannot lower. The child gets:
//!
//! - an empty environment and a fresh, empty working directory that is
//!   removed afterwards; no store, no credential and no network address;
//! - no stderr (discarded) and a bounded stdout;
//! - a wall-clock timeout, after which it is killed;
//! - on Unix, resource limits set before `exec`: CPU seconds, no file
//!   writes (`RLIMIT_FSIZE` 0), no core dumps, and on Linux an address-space
//!   cap.
//!
//! Compiling grants nothing: the response is returned as data, and a plan is
//! authorized again when the caller runs it through `POST /v1/execute`.
use crate::error::{codes, ApiError};
use crate::ids::{hex, random};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The configured compiler and the bounds of one run.
#[derive(Debug, Clone)]
pub struct Compiler {
    /// The weave-language `weave` binary.
    pub binary: PathBuf,
    /// Wall-clock limit of one run.
    pub timeout: Duration,
    /// Largest response the service reads.
    pub max_output_bytes: usize,
    /// Address-space limit of the child on Linux.
    pub max_memory_bytes: u64,
}

/// Default wall-clock limit of one compilation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default largest response: 16 MiB.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
/// Default address-space limit: 2 GiB.
pub const DEFAULT_MAX_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

impl Compiler {
    pub fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        }
    }

    /// Compile one request. Exit 0 (ok) and 1 (a diagnostic) return the
    /// compiler's response; anything else is `E_COMPILER`.
    pub fn compile(&self, request: &[u8], read_only: bool) -> Result<Value, ApiError> {
        let sandbox = std::env::temp_dir().join(format!("weave-compile-{}", hex(&random::<12>())));
        std::fs::create_dir(&sandbox).map_err(|_| failed("cannot create the sandbox"))?;
        let outcome = self.run(&sandbox, request, read_only);
        let _ = std::fs::remove_dir_all(&sandbox);
        outcome
    }

    fn run(
        &self,
        sandbox: &std::path::Path,
        request: &[u8],
        read_only: bool,
    ) -> Result<Value, ApiError> {
        let mut command = Command::new(&self.binary);
        command.arg("sdk").arg("--untrusted");
        if read_only {
            command.arg("--read-only");
        }
        command
            .env_clear()
            .current_dir(sandbox)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        limit(&mut command, self);
        let mut child = command
            .spawn()
            .map_err(|_| failed("the compiler could not be started"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| failed("no compiler stdin"))?;
        let input = request.to_vec();
        let writer = std::thread::spawn(move || {
            // A child that exits early closes the pipe; that is its answer.
            let _ = stdin.write_all(&input);
        });
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failed("no compiler stdout"))?;
        let cap = self.max_output_bytes as u64 + 1;
        let (sender, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.take(cap).read_to_end(&mut out);
            let _ = sender.send(out);
        });
        let deadline = Instant::now() + self.timeout;
        let timed_out = || failed("the compiler did not finish within its time limit");
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    kill(&mut child);
                    return Err(timed_out());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => {
                    kill(&mut child);
                    return Err(failed("the compiler could not be awaited"));
                }
            }
        };
        // Anything the child left running holds its stdout open: the output
        // must still arrive within the deadline.
        let remaining = deadline.saturating_duration_since(Instant::now());
        let out = match received.recv_timeout(remaining) {
            Ok(out) => out,
            Err(_) => {
                kill(&mut child);
                return Err(timed_out());
            }
        };
        let _ = writer.join();
        if out.len() > self.max_output_bytes {
            return Err(failed("the compiler response exceeds the output limit"));
        }
        let ok = match status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(failed("the compiler failed")),
        };
        let response: Value = serde_json::from_slice(&out)
            .map_err(|_| failed("the compiler response is not JSON"))?;
        if response.get("ok") != Some(&Value::Bool(ok)) {
            return Err(failed(
                "the compiler response does not match its exit status",
            ));
        }
        Ok(response)
    }
}

/// Kill the child and, on Unix, its whole process group.
fn kill(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: `kill` takes plain integers; the group is the child's own,
        // created before `exec`.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn failed(message: &str) -> ApiError {
    ApiError::new(codes::COMPILER, message)
}

#[cfg(unix)]
fn limit(command: &mut Command, compiler: &Compiler) {
    use std::os::unix::process::CommandExt;
    let cpu = compiler.timeout.as_secs() + 1;
    #[cfg(target_os = "linux")]
    let memory = compiler.max_memory_bytes;
    // SAFETY: the closure runs in the forked child before `exec` and only
    // calls `setpgid` and `setrlimit`, which are async-signal-safe; it
    // allocates nothing.
    unsafe {
        command.pre_exec(move || {
            let set = |resource, value: u64| {
                let limit = libc::rlimit {
                    rlim_cur: value as libc::rlim_t,
                    rlim_max: value as libc::rlim_t,
                };
                if libc::setrlimit(resource, &limit) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            };
            // Its own process group, so a timeout kills everything it starts.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            set(libc::RLIMIT_CPU, cpu)?;
            set(libc::RLIMIT_FSIZE, 0)?;
            set(libc::RLIMIT_CORE, 0)?;
            #[cfg(target_os = "linux")]
            set(libc::RLIMIT_AS, memory)?;
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn limit(_: &mut Command, _: &Compiler) {
    // Windows: the timeout and the output cap apply; no resource limits.
}
