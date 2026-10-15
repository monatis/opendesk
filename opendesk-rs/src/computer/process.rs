//! Running shell commands with a timeout that is safe for remote use.
//!
//! - The whole process tree is killed on timeout (a Job Object on Windows, a
//!   process group on Unix), not just the shell.
//! - Output is collected incrementally, so a timeout still returns what was
//!   printed, and a leftover grandchild holding the pipe open cannot hang the
//!   call.
//! - Output that is not valid UTF-8 is decoded with the console code page on
//!   Windows (lossy elsewhere) instead of being dropped.

use anyhow::{Result, anyhow};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::local::ExecResult;

/// Per-stream cap on captured output.
const MAX_CAPTURE: usize = 8 * 1024 * 1024;
/// How long to keep reading after the process exits, for output still in
/// flight. A detached grandchild that keeps the pipe open is abandoned after
/// this.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Build the PowerShell invocation for `command`.
///
/// `-EncodedCommand` (UTF-16LE base64) passes the script through untouched;
/// `-Command` would let powershell.exe's own argument parsing strip embedded
/// double quotes. Scripts too long for a command line go through a temp file.
#[cfg(target_os = "windows")]
pub fn powershell_command(command: &str) -> Result<(Command, Option<std::path::PathBuf>)> {
    const COMMON: [&str; 4] = [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
    ];
    // With stderr redirected, progress records ("Preparing modules for first
    // use") arrive as CLIXML noise on stderr. Scripts can still re-enable it.
    let command = format!("$ProgressPreference = 'SilentlyContinue'\n{command}");
    let utf16: Vec<u8> = command
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let encoded = data_encoding::BASE64.encode(&utf16);
    let mut c = Command::new("powershell.exe");
    c.args(COMMON);
    // Windows caps a command line at 32767 UTF-16 units.
    if encoded.len() < 30_000 {
        c.args(["-EncodedCommand", &encoded]);
        return Ok((c, None));
    }
    let path = std::env::temp_dir().join(format!(
        "opendesk-exec-{}-{:016x}.ps1",
        std::process::id(),
        rand::random::<u64>()
    ));
    // A BOM makes Windows PowerShell read the file as UTF-8.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(command.as_bytes());
    std::fs::write(&path, bytes).map_err(|e| anyhow!("failed to write temp script: {e}"))?;
    c.arg("-File").arg(&path);
    Ok((c, Some(path)))
}

/// Run `cmd`, killing its whole process tree if it outlives `timeout`.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<ExecResult> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("failed to spawn process: {e}"))?;
    let tree = ProcessTree::attach(&child);

    let (stdout_buf, stdout_reader) = spawn_reader(child.stdout.take());
    let (stderr_buf, stderr_reader) = spawn_reader(child.stderr.take());

    let start = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (Some(status), false);
        }
        if start.elapsed() >= timeout {
            tree.kill(&mut child);
            break (child.wait().ok(), true);
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let drain_deadline = Instant::now() + DRAIN_GRACE;
    for reader in [stdout_reader, stderr_reader].into_iter().flatten() {
        while !reader.is_finished() && Instant::now() < drain_deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let stdout = decode_output(&take(&stdout_buf));
    let mut stderr = decode_output(&take(&stderr_buf));
    if timed_out {
        if !stderr.is_empty() && !stderr.ends_with('\n') {
            stderr.push('\n');
        }
        stderr.push_str(&format!(
            "[opendesk] command timed out after {} seconds; its process tree was killed\n",
            timeout.as_secs()
        ));
    }
    let exit_code = if timed_out {
        -1
    } else {
        status.and_then(|s| s.code()).unwrap_or(-1)
    };
    Ok(ExecResult {
        stdout,
        stderr,
        exit_code,
        success: !timed_out && status.is_some_and(|s| s.success()),
        timed_out,
    })
}

/// A command started in the background: poll it for new output and its exit
/// status, or kill its whole process tree.
pub struct BackgroundProcess {
    pub command: String,
    child: std::process::Child,
    tree: ProcessTree,
    stdout: SharedBuf,
    stderr: SharedBuf,
    started: Instant,
    deadline: Option<Instant>,
    exit: Option<std::process::ExitStatus>,
    timed_out: bool,
    killed: bool,
    temp_script: Option<std::path::PathBuf>,
}

/// One poll's view of a background job.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct JobPoll {
    pub job_id: u64,
    pub command: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub killed: bool,
    pub elapsed_secs: f64,
    /// Output produced since the previous poll.
    pub stdout: String,
    pub stderr: String,
}

impl BackgroundProcess {
    pub fn spawn(
        cmd: &mut Command,
        command: &str,
        timeout: Option<Duration>,
        temp_script: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn process: {e}"))?;
        let tree = ProcessTree::attach(&child);
        let (stdout, _) = spawn_reader(child.stdout.take());
        let (stderr, _) = spawn_reader(child.stderr.take());
        let started = Instant::now();
        Ok(Self {
            command: command.to_string(),
            child,
            tree,
            stdout,
            stderr,
            started,
            deadline: timeout.map(|t| started + t),
            exit: None,
            timed_out: false,
            killed: false,
            temp_script,
        })
    }

    fn refresh(&mut self) {
        if self.exit.is_some() {
            return;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.finish(status);
        } else if self.deadline.is_some_and(|d| Instant::now() >= d) {
            self.timed_out = true;
            self.kill();
        }
    }

    fn finish(&mut self, status: std::process::ExitStatus) {
        self.exit = Some(status);
        if let Some(p) = self.temp_script.take() {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.refresh();
        self.exit.is_none()
    }

    /// Kill the whole process tree.
    pub fn kill(&mut self) {
        if self.exit.is_some() {
            return;
        }
        self.killed = true;
        self.tree.kill(&mut self.child);
        if let Ok(status) = self.child.wait() {
            self.finish(status);
        }
    }

    /// Status plus the output produced since the previous poll.
    pub fn poll(&mut self, job_id: u64) -> JobPoll {
        self.refresh();
        let running = self.exit.is_none();
        if !running {
            // Let output still in flight arrive before the final read.
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut stderr = decode_output(&take_complete(&self.stderr, !running));
        if self.timed_out && !running {
            stderr.push_str("[opendesk] job timed out; its process tree was killed\n");
        }
        JobPoll {
            job_id,
            command: self.command.clone(),
            running,
            exit_code: self.exit.and_then(|s| s.code()),
            timed_out: self.timed_out,
            killed: self.killed,
            elapsed_secs: self.started.elapsed().as_secs_f64(),
            stdout: decode_output(&take_complete(&self.stdout, !running)),
            stderr,
        }
    }
}

/// Drain `buf`, but keep a trailing incomplete UTF-8 sequence for the next
/// read so a character split across polls is not garbled.
fn take_complete(buf: &SharedBuf, everything: bool) -> Vec<u8> {
    let Ok(mut b) = buf.lock() else {
        return Vec::new();
    };
    let keep_from = match std::str::from_utf8(&b) {
        Err(e) if !everything && e.error_len().is_none() => e.valid_up_to(),
        _ => b.len(),
    };
    b.drain(..keep_from).collect()
}

type SharedBuf = Arc<Mutex<Vec<u8>>>;

fn spawn_reader<R: Read + Send + 'static>(pipe: Option<R>) -> (SharedBuf, Option<JoinHandle<()>>) {
    let buf: SharedBuf = Arc::new(Mutex::new(Vec::new()));
    let Some(mut pipe) = pipe else {
        return (buf, None);
    };
    let sink = buf.clone();
    let handle = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let Ok(mut b) = sink.lock() else { break };
                    let room = MAX_CAPTURE.saturating_sub(b.len());
                    b.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
        }
    });
    (buf, Some(handle))
}

fn take(buf: &SharedBuf) -> Vec<u8> {
    buf.lock()
        .map(|mut b| std::mem::take(&mut *b))
        .unwrap_or_default()
}

/// Decode process output: UTF-8 if valid, else the console code page on
/// Windows, else lossy UTF-8.
pub fn decode_output(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    #[cfg(target_os = "windows")]
    if let Some(s) = win::decode_codepage(bytes) {
        return s;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(target_os = "windows")]
struct ProcessTree {
    job: Option<win::Handle>,
}

#[cfg(target_os = "windows")]
impl ProcessTree {
    fn attach(child: &std::process::Child) -> Self {
        Self {
            job: win::job_for(child),
        }
    }

    fn kill(&self, child: &mut std::process::Child) {
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = child.kill();
    }
}

#[cfg(unix)]
struct ProcessTree;

#[cfg(unix)]
impl ProcessTree {
    fn attach(_child: &std::process::Child) -> Self {
        Self
    }

    fn kill(&self, child: &mut std::process::Child) {
        // The child leads its own process group (process_group(0) above).
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = child.kill();
    }
}

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
mod win {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    unsafe extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> *mut c_void;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn GetConsoleOutputCP() -> u32;
        fn GetOEMCP() -> u32;
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            src: *const u8,
            src_len: i32,
            dst: *mut u16,
            dst_len: i32,
        ) -> i32;
    }

    pub struct Handle(*mut c_void);

    // The job handle is only used from the thread that owns the ProcessTree.
    unsafe impl Send for Handle {}

    impl Handle {
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // No KILL_ON_JOB_CLOSE: processes the command deliberately
            // detached (e.g. Start-Process) survive a normal exit.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Put `child` in a fresh job so its descendants can be killed together.
    /// Processes it spawned before this point are not covered; the shell has
    /// not run any user code that early.
    pub fn job_for(child: &std::process::Child) -> Option<Handle> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let handle = Handle(job);
            if AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                return None;
            }
            Some(handle)
        }
    }

    pub fn decode_codepage(bytes: &[u8]) -> Option<String> {
        let len = i32::try_from(bytes.len()).ok()?;
        let cp = match unsafe { GetConsoleOutputCP() } {
            0 => unsafe { GetOEMCP() },
            cp => cp,
        };
        unsafe {
            let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
            if n <= 0 {
                return None;
            }
            let mut wide = vec![0u16; n as usize];
            let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n);
            if n <= 0 {
                return None;
            }
            wide.truncate(n as usize);
            Some(String::from_utf16_lossy(&wide))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_keeps_valid_utf8() {
        assert_eq!(decode_output("çğış".as_bytes()), "çğış");
    }

    #[test]
    fn decode_never_drops_invalid_utf8() {
        let s = decode_output(&[b'A', 0xFE, b'B']);
        assert!(s.starts_with('A') && s.ends_with('B'), "{s:?}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn timeout_kills_tree_and_keeps_partial_output() {
        // `ping` is a grandchild that holds the pipe open; killing only cmd.exe
        // would leave the call blocked for ~30 s.
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/C", "echo partial-output & ping -n 30 127.0.0.1 >nul"]);
        let start = Instant::now();
        let res = run_with_timeout(&mut cmd, Duration::from_secs(2)).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
        assert!(res.timed_out);
        assert!(!res.success);
        assert!(res.stdout.contains("partial-output"), "{res:?}");
        assert!(res.stderr.contains("timed out"), "{res:?}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_keeps_embedded_double_quotes() {
        let (mut cmd, tmp) = powershell_command(r#"Write-Output "say ""hi"" 'x'""#).unwrap();
        assert!(tmp.is_none());
        let res = run_with_timeout(&mut cmd, Duration::from_secs(30)).unwrap();
        assert_eq!(res.stdout.trim(), r#"say "hi" 'x'"#, "{res:?}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_long_script_uses_temp_file() {
        let script = format!("# {}\nWrite-Output long-ok", "x".repeat(20_000));
        let (mut cmd, tmp) = powershell_command(&script).unwrap();
        let tmp = tmp.unwrap();
        let res = run_with_timeout(&mut cmd, Duration::from_secs(30)).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(res.stdout.trim(), "long-ok", "{res:?}");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_tree_and_keeps_partial_output() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo partial-output; sleep 30 & wait"]);
        let start = Instant::now();
        let res = run_with_timeout(&mut cmd, Duration::from_secs(2)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(res.timed_out);
        assert!(res.stdout.contains("partial-output"), "{res:?}");
    }
}
