//! Daemon lifecycle management — PID file, start/stop, system service install.
//!
//! On macOS: launchd (`~/Library/LaunchAgents/com.a7garden.oxios.plist`)
//! On Linux: systemd (`/etc/systemd/system/oxiosd.service`)

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Maximum time `stop` waits after SIGTERM before escalating to SIGKILL.
/// Generous enough for a graceful agent/MCP drain; not so long that a hung
/// daemon makes `stop` feel stuck. SIGKILL cannot be caught — it is the
/// absolute last resort.
const SIGTERM_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Daemon status.
#[derive(Debug, Clone)]
pub enum DaemonStatus {
    /// Daemon is running.
    Running {
        /// Process ID.
        pid: u32,
    },
    /// PID file exists but process is dead (stale).
    Stale {
        /// Process ID of the dead process.
        pid: u32,
    },
    /// Daemon is not running.
    Stopped,
    /// Port is held by an oxios-shaped process that left no pidfile
    /// (e.g. a `--foreground` debug instance launched directly).
    /// Caller doesn't have a PID to report because there is no
    /// pidfile/lockfile, just a port.
    Orphaned {
        /// Port that appears to be held by an oxios instance.
        port: u16,
    },
}

impl std::fmt::Display for DaemonStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonStatus::Running { pid } => write!(f, "running (PID {pid})"),
            DaemonStatus::Stale { pid } => write!(f, "stale (PID {pid} dead)"),
            DaemonStatus::Stopped => write!(f, "stopped"),
            DaemonStatus::Orphaned { port } => {
                write!(f, "orphaned (no pidfile, port {port} in use)")
            }
        }
    }
}

/// Manages the oxios background daemon.
pub struct DaemonManager {
    pid_file: PathBuf,
    log_dir: PathBuf,
    /// Port to consult when both pidfile and lockfile fail to identify a
    /// live daemon. Set by the CLI (`main.rs`) from the gateway config so
    /// `status`/`stop` can detect `--foreground` orphans that bypassed the
    /// pidfile. `None` disables the orphan-detection path.
    probe_port: Option<u16>,
    /// Optional override for the binary path used when spawning/restarting
    /// the daemon. When `Some`, both `start` and `install_service` use this
    /// path instead of `std::env::current_exe()`. The launcher
    /// (`~/.oxios/bin/oxios`) is a symlink into `versions/<v>/oxios`; the
    /// resolved path flips on every version update, so the daemon must
    /// respawn itself via the launcher — not the resolved target — or it
    /// resurrects the previous binary on restart. `None` preserves the
    /// historical `current_exe()` behavior for every existing caller.
    exe_override: Option<PathBuf>,
}

impl DaemonManager {
    /// Create a daemon manager from config paths.
    pub fn new(pid_file: &str, log_dir: &str) -> Self {
        Self {
            pid_file: crate::config::expand_home(pid_file),
            log_dir: crate::config::expand_home(log_dir),
            probe_port: None,
            exe_override: None,
        }
    }

    /// Set the port used by orphan-detection probes. Called by the CLI
    /// after assembling a manager; without it `status`/`stop` cannot catch
    /// `--foreground` debug instances that escape the pidfile.
    pub fn set_probe_port(&mut self, port: u16) {
        self.probe_port = Some(port);
    }

    /// Consuming builder for the probe port. Lets CLI sites write
    /// `DaemonManager::new(..).with_probe_port(port)` without binding to
    /// `mut`, which is awkward in expression position.
    pub fn with_probe_port(mut self, port: u16) -> Self {
        self.probe_port = Some(port);
        self
    }

    /// Consuming builder for the daemon binary path. When set, `start` and
    /// `install_service` spawn this path instead of `std::env::current_exe()`.
    /// Used by managed installs so the launcher symlink
    /// (`~/.oxios/bin/oxios`) — not the resolved `versions/<v>/oxios` —
    /// is what restarts the daemon. A version flip that rewrites the
    /// symlink target would otherwise respawn the previous binary
    /// (`current_exe()` resolves symlinks), which was the root cause of the
    /// 2026-08-26 stale-binary incident.
    pub fn with_exe(mut self, exe: PathBuf) -> Self {
        self.exe_override = Some(exe);
        self
    }

    /// Check daemon status by reading every liveness source we have.
    ///
    /// Resolution order — every source that can prove an oxios daemon is
    /// alive is consulted in turn, because each one alone is unreliable:
    /// the pidfile can be stale or absent (a direct `--foreground` run),
    /// the lockfile-based daemon can be invisible to a future daemon's
    /// own write, and the launchd-managed daemon can be invisible to
    /// both. We surface the strongest signal.
    pub fn status(&self) -> DaemonStatus {
        // 1. Instance-lock holder: cross-binary, cross-launcher truth.
        if let Some(pid) = self.read_lock_pid().filter(|&p| self.is_alive(p)) {
            return DaemonStatus::Running { pid };
        }
        // 2. Legacy pidfile (kept for backwards compat with daemons that
        //    weren't updated to take the instance lock).
        if let Some(pid) = self.read_pid() {
            if self.is_alive(pid) {
                return DaemonStatus::Running { pid };
            }
            return DaemonStatus::Stale { pid };
        }
        // 3. Port probe: catches an orphan. This is the path that would
        //    have caught the user's debug-instance scenario (--foreground
        //    without a pidfile). Note: we don't surface a PID because
        //    there isn't one to trust — port-based attribution is best-
        //    effort, and `stop` will re-derive it via `lsof` at kill time.
        if let Some(port) = self.probe_port
            && self.port_in_use(port)
        {
            return DaemonStatus::Orphaned { port };
        }
        DaemonStatus::Stopped
    }

    /// Start the daemon in the background and wait for it to begin accepting
    /// connections on `port` (RFC-024 SP4: verifies the listener came up so
    /// a port-bind failure is reported immediately instead of masked by a
    /// `started` message that never resolves).
    pub fn start(&self, config_path: &Path, port: u16) -> Result<()> {
        match self.status() {
            DaemonStatus::Running { pid } => {
                anyhow::bail!("oxios is already running (PID {pid})");
            }
            DaemonStatus::Stale { .. } => {
                self.cleanup()?;
            }
            DaemonStatus::Stopped | DaemonStatus::Orphaned { .. } => {
                // Orphaned means "something on the port is in the way" —
                // fall through to the port-in-use guard, which gives the
                // user an actionable error.
            }
        }

        // Pre-spawn port guard: catches an orphaned oxios process that still
        // holds the port even though the pidfile is stale or missing (e.g. a
        // prior `oxios stop` removed the pidfile but the process refused to
        // die). Without this the spawned daemon's bind fails silently while
        // the post-spawn readiness probe connects to the *old* listener and
        // reports success — leaving the broken daemon running undetected.
        if self.port_in_use(port) {
            anyhow::bail!(
                "port {port} is already in use — another oxios instance is \
                 likely still running. Run `oxios stop`, or find and kill the \
                 process with `lsof -i :{port}` then retry."
            );
        }

        // Ensure log directory exists
        std::fs::create_dir_all(&self.log_dir).context("failed to create log directory")?;

        let log_file = self.log_dir.join("oxios.log");
        let exe = self.exe_override.clone().map_or_else(
            || std::env::current_exe().context("failed to locate oxios binary"),
            Ok,
        )?;

        // Append-mode shared handle for stdout+stderr: O_APPEND writes land
        // atomically at EOF and never truncate previous runs, so a panic
        // message (written synchronously to stderr by the panic hook before
        // abort) survives a restart and stays diagnosable. Two separate
        // `File::create` handles (O_TRUNC, offset 0) used to clobber each
        // other and wipe the evidence on every restart.
        let log_handle = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .with_context(|| format!("failed to open log file {}", log_file.display()))?;
        let stderr_handle = log_handle
            .try_clone()
            .context("failed to duplicate log handle for stderr")?;
        let child = std::process::Command::new(&exe)
            .arg("--foreground")
            .arg("--config")
            .arg(config_path)
            .stdout(log_handle)
            .stderr(stderr_handle)
            .spawn()
            .context("failed to spawn oxios daemon")?;

        let pid = child.id();
        self.write_pid(pid)?;

        println!("⬡ oxios started (PID {pid})");
        println!("  Logs: {}", log_file.display());
        println!("  Dashboard: http://127.0.0.1:{port}");

        // RFC-024 SP4: verify the daemon is actually accepting connections.
        // A misconfigured bind (TIME_WAIT, port in use) used to be invisible
        // here — the user saw `started` but `curl` got connection refused.
        match self.wait_until_listening(port, std::time::Duration::from_secs(15)) {
            Ok(()) => println!("  Status:   ready (listening on :{port})"),
            Err(_) => {
                // The spawned daemon never accepted a connection — almost
                // always a fatal startup error (web UI unavailable, config
                // problem) or a bind failure we failed to anticipate.
                // Surface the log tail so the user sees *why* instead of a
                // misleading "started", and fail the start.
                println!("  Status:   FAILED to start (no listener on :{port} within 15s)");
                let log_path = self.log_dir.join("oxios.log");
                if let Ok(content) = std::fs::read_to_string(&log_path) {
                    let lines: Vec<&str> = content.lines().collect();
                    let start = lines.len().saturating_sub(30);
                    if start < lines.len() {
                        println!("  ── recent log (last {} lines) ──", lines.len() - start);
                        for line in &lines[start..] {
                            println!("  {line}");
                        }
                    }
                }
                println!("  Full log: {}", log_path.display());
                anyhow::bail!(
                    "daemon failed to start listening on :{port} \
                     (see the log above and {})",
                    log_path.display()
                );
            }
        }
        Ok(())
    }

    /// Poll `127.0.0.1:port` until a TCP connect succeeds or `timeout` elapses.
    fn wait_until_listening(&self, port: u16, timeout: std::time::Duration) -> Result<()> {
        use std::net::ToSocketAddrs;
        let addr = format!("127.0.0.1:{port}")
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid bind address 127.0.0.1:{port}"))?;
        let start = std::time::Instant::now();
        let interval = std::time::Duration::from_millis(200);
        while start.elapsed() < timeout {
            if std::net::TcpStream::connect_timeout(&addr, interval).is_ok() {
                return Ok(());
            }
            std::thread::sleep(interval);
        }
        anyhow::bail!("daemon did not start listening on :{port} within {timeout:?}")
    }

    /// Whether anything is currently accepting connections on `127.0.0.1:port`.
    ///
    /// Pre-spawn guard used by [`start`](Self::start) and the orphan-
    /// detection path in [`status`](Self::status) to detect a stray daemon
    /// that escaped the pidfile.
    fn port_in_use(&self, port: u16) -> bool {
        use std::net::{TcpStream, ToSocketAddrs};
        let Some(addr) = format!("127.0.0.1:{port}")
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
        else {
            return false;
        };
        TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(200)).is_ok()
    }

    /// Stop the daemon.
    ///
    /// Resolution order — every source that can prove an oxios daemon is
    /// alive is consulted, because each one alone is unreliable:
    ///
    /// 1. **Instance lock** (`<pid_file>.lock`): held by `flock` by every
    ///    daemon (`cmd_serve`/`--foreground` AND `start()`-spawned). The PID
    ///    recorded in the file is the authoritative owner.
    /// 2. **PID file** (`<pid_file>`): the legacy spawn-and-fork channel.
    ///    Kept for backwards compat with prior daemons that wrote the
    ///    pidfile but never took the instance lock.
    /// 3. **Orphan port probe**: when neither (1) nor (2) identifies a
    ///    daemon, `lsof -ti tcp:PORT -sTCP:LISTEN` is consulted. This path
    ///    catches a `--foreground` debug instance launched without writing
    ///    a pidfile — the exact symptom the user reported.
    /// 4. **System service**: launchd plist or systemd unit. If loaded,
    ///    unload it for this session; otherwise KeepAlive (macOS) or
    ///    Restart=on-failure (Linux) will silently undo our kill within
    ///    milliseconds. We do NOT remove the plist/unit here; that's
    ///    `uninstall_service`'s job.
    ///
    /// User contract: an explicit `daemon install` is the only way to opt
    /// into the supervisor; in that mode `stop` must reverse it for this
    /// session, otherwise the daemon respawns within seconds and `stop`
    /// lies. A user who never ran `daemon install` and never spawned a
    /// daemon in another shell simply gets "not running".
    pub fn stop(&self) -> Result<()> {
        let service_loaded = self.is_service_loaded();

        // 1. Resolve a live PID from any source we can find it.
        let lock_pid = self.read_lock_pid().filter(|&p| self.is_alive(p));
        let file_pid = self.read_pid().filter(|&p| self.is_alive(p));
        let orphan_pid = self.orphan_pid_from_port();

        match (lock_pid, file_pid, orphan_pid) {
            (Some(pid), _, _) | (None, Some(pid), _) | (None, None, Some(pid)) => {
                self.kill_pid(pid)?;
            }
            (None, None, None) => {
                if service_loaded {
                    self.unload_service()?;
                    std::thread::sleep(std::time::Duration::from_millis(300));
                } else {
                    println!("⬡ oxios is not running");
                    return Ok(());
                }
            }
        }

        // Always unload if registered. Otherwise KeepAlive (macOS) /
        // Restart=on-failure (Linux) will silently undo our kill.
        if service_loaded {
            self.unload_service()?;
        }

        self.cleanup()?;
        println!("⬡ oxios stopped");
        Ok(())
    }

    /// Send SIGTERM to `pid`, escalating to SIGKILL after `SIGTERM_GRACE`
    /// if the process hasn't exited. SIGKILL cannot be caught — last resort.
    ///
    /// The actual SIGTERM delivery is handled by `cmd_serve`'s supervisor
    /// (it sees the signal and runs its graceful-drain path). From this
    /// side we just wait up to `SIGTERM_GRACE` for the process to vanish;
    /// if it doesn't, SIGKILL finishes the job.
    fn kill_pid(&self, pid: u32) -> Result<()> {
        #[cfg(unix)]
        unsafe {
            let send_ret = libc::kill(pid as i32, libc::SIGTERM);
            if send_ret != 0 {
                let e = std::io::Error::last_os_error();
                // ESRCH: process died between our liveness check and now.
                if e.raw_os_error() != Some(libc::ESRCH) {
                    anyhow::bail!("failed to send SIGTERM to PID {pid}: {e}");
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }

        let poll_start = std::time::Instant::now();
        let interval = std::time::Duration::from_millis(200);
        while poll_start.elapsed() < SIGTERM_GRACE {
            std::thread::sleep(interval);
            if !self.is_alive(pid) {
                return Ok(());
            }
        }

        // Process ignored SIGTERM within `SIGTERM_GRACE` — escalate.
        #[cfg(unix)]
        unsafe {
            let send_ret = libc::kill(pid as i32, libc::SIGKILL);
            if send_ret != 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ESRCH) {
                    anyhow::bail!("failed to send SIGKILL to PID {pid}: {e}");
                }
            }
        }
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if !self.is_alive(pid) {
                return Ok(());
            }
        }
        anyhow::bail!(
            "PID {pid} ignored SIGTERM and SIGKILL; cannot stop. Kill it manually: `kill -9 {pid}`"
        )
    }

    /// Restart the daemon.
    pub fn restart(&self, config_path: &Path, port: u16) -> Result<()> {
        if matches!(self.status(), DaemonStatus::Running { .. }) {
            self.stop()?;
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        self.start(config_path, port)
    }

    /// Install as a system service (launchd on macOS, systemd on Linux).
    ///
    /// After this returns successfully, `stop()` MUST be used to halt the
    /// daemon in this session — a plain `kill -TERM` will be undone by
    /// `KeepAlive` (macOS) / `Restart=on-failure` (Linux) within
    /// milliseconds.
    pub fn install_service(&self) -> Result<()> {
        let exe = self.exe_override.clone().map_or_else(
            || std::env::current_exe().context("failed to locate oxios binary"),
            Ok,
        )?;
        #[cfg(target_os = "macos")]
        {
            let plist_dir = dirs::home_dir()
                .map(|h| h.join("Library/LaunchAgents"))
                .context("failed to locate LaunchAgents directory")?;
            std::fs::create_dir_all(&plist_dir)?;
            let plist_path = plist_dir.join("com.a7garden.oxios.plist");

            let home = dirs::home_dir().context("failed to get HOME")?;
            let log_path = self.log_dir.join("oxiosd.log");

            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.a7garden.oxios</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>WorkingDirectory</key>
    <string>{home}</string>
</dict>
</plist>
"#,
                exe = escape_xml(&exe.display().to_string()),
                log = escape_xml(&log_path.display().to_string()),
                home = escape_xml(&home.display().to_string()),
            );

            std::fs::write(&plist_path, &plist)?;
            println!("✓ Installed launchd service");
            println!("  {}", plist_path.display());
            println!();
            println!("  Loaded at boot by macOS launchd (KeepAlive=true).");
            println!("  Stop with `oxios stop` (it will unload launchd), or:");
            println!("    launchctl bootout gui/$UID/com.a7garden.oxios");
            println!("  Disable auto-start on next boot:");
            println!("    oxios daemon uninstall");
        }

        #[cfg(target_os = "linux")]
        {
            let unit_dir = PathBuf::from("/etc/systemd/system");
            let unit_path = unit_dir.join("oxiosd.service");

            // Validate the binary path before embedding it in ExecStart. systemd
            // ExecStart parsing has its own quoting rules; rather than implement
            // full escaping, refuse paths containing shell/systemd metacharacters.
            let exe_str = exe.display().to_string();
            if exe_str.chars().any(|c| {
                matches!(
                    c,
                    '"' | '\''
                        | '\\'
                        | '$'
                        | '`'
                        | ';'
                        | '&'
                        | '|'
                        | '*'
                        | '?'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                )
            }) {
                anyhow::bail!(
                    "Refusing to install systemd unit: binary path '{exe_str}' contains shell/systemd metacharacters"
                );
            }

            let unit = format!(
                r#"[Unit]
Description=Oxios Agent Operating System
After=network.target
StartLimitBurst=5
StartLimitIntervalSec=60

[Service]
Type=simple
ExecStart={exe} --foreground
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#,
                exe = exe_str,
            );

            // Try to write — may fail without sudo
            if let Err(e) = std::fs::write(&unit_path, &unit) {
                anyhow::bail!(
                    "Failed to write {} — run with sudo: {}",
                    unit_path.display(),
                    e
                );
            }

            println!("✓ Installed systemd service");
            println!("  {}", unit_path.display());
            println!();
            println!("  Reload:  sudo systemctl daemon-reload");
            println!("  Start:   sudo systemctl start oxiosd");
            println!("  Enable:  sudo systemctl enable oxiosd");
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            anyhow::bail!("daemon install only supported on macOS and Linux");
        }

        Ok(())
    }

    /// Uninstall the system service.
    ///
    /// Tears down any live supervisor registration BEFORE removing the
    /// file. On macOS, removing a loaded plist has no effect on the
    /// running job — bootout must run first.
    pub fn uninstall_service(&self) -> Result<()> {
        let _ = self.unload_service();

        #[cfg(target_os = "macos")]
        {
            let plist_path = dirs::home_dir()
                .map(|h| h.join("Library/LaunchAgents/com.a7garden.oxios.plist"))
                .context("failed to locate plist")?;

            if plist_path.exists() {
                std::fs::remove_file(&plist_path)?;
                println!("✓ Removed launchd service (will not auto-start on next boot)");
            } else {
                println!("  Service not installed");
            }
        }

        #[cfg(target_os = "linux")]
        {
            let unit_path = PathBuf::from("/etc/systemd/system/oxiosd.service");
            if unit_path.exists() {
                if let Err(e) = std::fs::remove_file(&unit_path) {
                    anyhow::bail!(
                        "Failed to remove {} — run with sudo: {}",
                        unit_path.display(),
                        e
                    );
                }
                println!("✓ Removed systemd service");
            } else {
                println!("  Service not installed");
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            anyhow::bail!("daemon uninstall only supported on macOS and Linux");
        }

        Ok(())
    }

    // ── Internal helpers ─────────────────────────────────────────────

    fn read_pid(&self) -> Option<u32> {
        let content = std::fs::read_to_string(&self.pid_file).ok()?;
        content.trim().parse().ok()
    }

    fn write_pid(&self, pid: u32) -> Result<()> {
        if let Some(parent) = self.pid_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.pid_file, pid.to_string())?;
        Ok(())
    }

    pub fn cleanup(&self) -> Result<()> {
        if self.pid_file.exists() {
            std::fs::remove_file(&self.pid_file)?;
        }
        Ok(())
    }

    fn is_alive(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            // Signal 0 = check if process exists.
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
        #[cfg(not(unix))]
        {
            // On non-Unix, always return false (conservative)
            let _ = pid;
            false
        }
    }

    // ── Service + lock + orphan introspection ────────────────────────

    /// Path of the `flock`-based single-instance lock file.
    fn lock_path(&self) -> PathBuf {
        self.pid_file.with_extension("lock")
    }

    /// Read the PID stored in the instance-lock file. Diagnostic only —
    /// the flock is the real truth. Callers MUST verify `is_alive(pid)`.
    fn read_lock_pid(&self) -> Option<u32> {
        let path = self.lock_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    }

    /// Probe `self.probe_port` for an orphaned oxios-shaped PID.
    ///
    /// Returns `None` when:
    ///   - no probe port was configured
    ///   - the port is free
    ///   - `lsof` is missing or returns nothing parseable
    ///
    /// Returns `Some(pid)` only for processes that BOTH listen on
    /// `tcp:PORT` AND have a `comm` matching `oxios`. Without the
    /// comm check, port 4200 (the daemon's default) is also Angular's
    /// dev-server default and other tools'; SIGKILL of a PID surfaced
    /// from lsof alone could murder an unrelated listener — a contract
    /// `oxios stop` must not break. `ps -o comm= -p PID` reads the
    /// kernel-resident process name; a substring match on `oxios`
    /// covers both release binaries and `target/debug/oxios` builds.
    fn orphan_pid_from_port(&self) -> Option<u32> {
        let port = self.probe_port?;
        if !self.port_in_use(port) {
            return None;
        }
        // `lsof -ti tcp:PORT -sTCP:LISTEN` returns listening PIDs.
        let out = std::process::Command::new("lsof")
            .args(["-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?;
        let pid: u32 = s
            .lines()
            .find_map(|line| line.split_whitespace().find_map(|t| t.parse().ok()))?;

        // Identity check: refuse to kill anything that doesn't look like
        // oxios. Cheaper than walking /proc, just calls `ps` for the
        // short process name. Match is substring — `oxios` covers both
        // `./target/debug/oxios` (comm truncation preserves the basename
        // tail) and `oxiosd` if anyone renames it.
        let comm = std::process::Command::new("ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !comm.contains("oxios") {
            eprintln!(
                "  ⚠ port {port} held by PID {pid} ({comm}), not oxios — \
                 not killing; resolve manually (`lsof -i :{port}`)."
            );
            return None;
        }
        Some(pid)
    }

    /// Whether the OS-managed supervisor (launchd LaunchAgent or systemd
    /// unit) currently considers oxios loaded.
    fn is_service_loaded(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            let uid = current_uid_str();
            if uid.is_empty() {
                return false;
            }
            let target = format!("gui/{uid}/com.a7garden.oxios");
            std::process::Command::new("launchctl")
                .args(["print", &target])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(target_os = "linux")]
        {
            // `systemctl is-active` exits 0 only for `active` state.
            if std::process::Command::new("systemctl")
                .args(["--user", "is-active", "oxiosd.service"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return true;
            }
            std::process::Command::new("systemctl")
                .args(["is-active", "oxiosd.service"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = self;
            false
        }
    }

    /// Unload the OS-level supervisor registration WITHOUT deleting the
    /// plist/unit file. Reverses KeepAlive for the current session; the
    /// file remains so `start`/`launchctl load` can re-arm it.
    fn unload_service(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let uid = current_uid_str();
            if uid.is_empty() {
                return Ok(());
            }
            let target = format!("gui/{uid}/com.a7garden.oxios");
            // bootout (10.11+) is preferred; fall back to legacy `unload`.
            let bootout_ok = std::process::Command::new("launchctl")
                .args(["bootout", &target])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !bootout_ok {
                let plist = dirs::home_dir()
                    .map(|h| h.join("Library/LaunchAgents/com.a7garden.oxios.plist"));
                if let Some(p) = plist.filter(|p| p.exists()) {
                    let _ = std::process::Command::new("launchctl")
                        .args(["unload", &p.to_string_lossy()])
                        .output();
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "stop", "oxiosd.service"])
                .output();
            let _ = std::process::Command::new("systemctl")
                .args(["stop", "oxiosd.service"])
                .output();
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = self;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
/// Return the current user's numeric UID as a string.
///
/// Used to construct the launchd target path (`gui/$UID/<label>`), which
/// is the only stable handle we have for a job loaded into the user
/// domain. Returns an empty string on probe failure so callers can fall
/// back gracefully.
fn current_uid_str() -> String {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
/// Escape a string for safe inclusion in an XML plist text node.
///
/// Replaces the five XML-predefined entities (`&`, `<`, `>`, `"`, `'`). Paths
/// inserted into the launchd plist are usually trusted system paths, but a
/// HOME or install path containing `<`, `&`, etc. would produce malformed XML
/// that launchd refuses to load — and would be a defense-in-depth gap.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_in_use_detects_a_live_listener() {
        // Bind an ephemeral port and confirm port_in_use reports it in use.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dm = DaemonManager::new("/tmp/oxios-test.pid", "/tmp");
        assert!(
            dm.port_in_use(port),
            "port should be reported in use while a listener is bound"
        );
    }

    #[test]
    fn port_in_use_false_for_unused_port() {
        let dm = DaemonManager::new("/tmp/oxios-test.pid", "/tmp");
        // Obtain a port that was just free by binding and dropping, then
        // confirm port_in_use no longer sees a listener.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(
            !dm.port_in_use(port),
            "port should be reported free once the listener is dropped"
        );
    }

    #[test]
    fn status_reports_orphaned_when_only_port_responds() {
        // Bind a listener and verify that with no pidfile/lockfile but a
        // configured probe port, status classifies the system as
        // `Orphaned` rather than `Stopped` — the exact case that bit
        // the user when `--foreground` was launched directly.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut dm = DaemonManager::new("/tmp/oxios-orphan-test.pid", "/tmp");
        dm.set_probe_port(port);
        // Hold the listener — don't drop until after status() returns.
        let status = dm.status();
        drop(listener);
        match status {
            DaemonStatus::Orphaned { port: p } => assert_eq!(p, port),
            other => panic!("expected Orphaned, got {other:?}"),
        }
    }

    #[test]
    fn with_exe_overrides_spawn_target() {
        let dm = DaemonManager::new("/tmp/oxios-with-exe-test.pid", "/tmp")
            .with_exe(std::path::PathBuf::from("/nonexistent/launcher"));
        assert_eq!(
            dm.exe_override.as_deref(),
            Some(std::path::Path::new("/nonexistent/launcher"))
        );
        // spawn would fail on nonexistent path; the unit pins the plumbing only
    }

    #[test]
    fn status_stopped_with_no_signal() {
        let mut dm = DaemonManager::new("/tmp/oxios-stopped-test.pid", "/tmp");
        dm.set_probe_port(1); // privileged port, never bound in tests.
        assert!(matches!(dm.status(), DaemonStatus::Stopped));
    }
}
