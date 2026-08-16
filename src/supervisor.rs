//! Runtime task supervision (RFC-030).
//!
//! Observes the daemon's critical background tasks (gateway, surfaces,
//! channels) so that an unexpected exit is *never silent*. On a web-surface
//! crash it restarts the surface in-process (bounded backoff); on a
//! critical-task death (gateway, channels) it escalates to a fatal process
//! exit so the OS supervisor (systemd / launchd) restarts to a known-good
//! state. Before this module, `cmd_serve` awaited only `ctrl_c` and treated
//! every task handle as fire-and-forget — a crashed web server left the daemon
//! half-dead and `oxios status` still reporting "Running".
//!
//! # `panic = "abort"` interaction
//!
//! Oxios compiles release builds with `panic = "abort"` (see root `Cargo.toml`
//! `[profile.release]`). A panicking task therefore *aborts the whole process*
//! before its `JoinHandle` can complete — the supervisor never observes the
//! panic. Production panic recovery is consequently "process abort → OS
//! supervisor restart", which already achieves fail-fast for free. The
//! supervisor intercepts the **non-panic** failure path: a task finishing with
//! an error (e.g. `axum::serve` returning `Err`) or completing when it should
//! run forever. That is the realistic, high-value failure mode this module
//! exists to recover from.
//!
//! # Non-blocking supervision (R7)
//!
//! The run loop is purely `select!`-driven. Backoff is modelled as a recorded
//! deadline fired by a dedicated timer branch, never an inline `sleep` — so
//! `ctrl_c` and a critical-task death stay observable throughout every backoff
//! window (a web restart backoff of up to 30s never blinds the loop to a
//! dying gateway).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::stream::{FuturesUnordered, StreamExt};
use parking_lot::RwLock;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use oxios_gateway::{ActiveWebDist, Gateway, Surface, SurfaceContext};

use crate::kernel::Kernel;

/// How often the supervisor checks the Guardian heartbeat (RFC-040 A3).
const GUARDIAN_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Stale threshold: 3 × Guardian cycle (300s). Three consecutive missed
/// cycles before the watchdog fires — generous enough for slow cycles,
/// tight enough to catch real hangs.
const GUARDIAN_STALE_THRESHOLD_SECS: u64 = 900;

/// Current time as Unix epoch seconds.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Outcome of the supervisor's run loop.
pub enum ShutdownOutcome {
    /// `ctrl_c` (or root-token cancel) received — clean shutdown requested.
    Graceful,
    /// A critical task exited unexpectedly — the process should exit non-zero
    /// so the OS supervisor restarts to a known-good state.
    Fatal {
        /// Name of the task that died.
        name: String,
        /// Human-readable reason.
        reason: String,
    },
}

/// Which task a tracked handle belongs to.
#[derive(Clone)]
enum Tag {
    /// Named critical task (gateway, channels, non-web surfaces). Fail-fast.
    Critical(String),
    /// The web surface — scoped-restart policy.
    Web,
}

/// Bounded exponential-backoff configuration for scoped restarts.
pub struct RestartConfig {
    /// Max restart attempts before escalating to fatal.
    pub max_retries: u32,
    /// Reset the retry counter after the surface runs stably this long.
    pub reset_after: Duration,
    /// First backoff duration.
    pub initial_backoff: Duration,
    /// Backoff cap.
    pub max_backoff: Duration,
}

impl Default for RestartConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            reset_after: Duration::from_secs(300),
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// A tagged completion future: yields `(Tag, result)` so the loop knows *which*
/// task completed without a parallel index array.
type TaggedFut = std::pin::Pin<Box<dyn Future<Output = (Tag, Result<(), JoinError>)> + Send>>;

fn tagged(tag: Tag, handle: JoinHandle<()>) -> TaggedFut {
    Box::pin(async move { (tag, handle.await) })
}

/// (Re)starts the web surface. Held by the supervisor to perform scoped
/// restarts after a crash.
pub struct WebSurfaceRestarter {
    gateway: Arc<Gateway>,
    kernel_handle: Arc<oxios_kernel::KernelHandle>,
    config: Arc<RwLock<oxios_kernel::OxiosConfig>>,
    config_path: std::path::PathBuf,
    web_dist: ActiveWebDist,
}

impl WebSurfaceRestarter {
    /// Build from the running kernel plus the paths/dist resolved at boot.
    pub fn new(
        kernel: &Kernel,
        config: Arc<RwLock<oxios_kernel::OxiosConfig>>,
        config_path: std::path::PathBuf,
        web_dist: ActiveWebDist,
    ) -> Self {
        Self {
            gateway: kernel.gateway(),
            kernel_handle: kernel.handle(),
            config,
            config_path,
            web_dist,
        }
    }

    /// (Re)start the web surface with a fresh shutdown child-token.
    ///
    /// Unregisters any stale `"web"` channel first (the previous instance is
    /// already dead, so there is nothing to drain — RFC-030 A5), then re-invokes
    /// `WebSurface::start` and registers the new channel. Returns the spawned
    /// server task handle.
    async fn start(&self, shutdown: CancellationToken) -> Result<JoinHandle<()>, anyhow::Error> {
        // Stale channel from the crashed instance — remove it and stop its
        // gateway receive task. Ignore "not registered" errors.
        let _ = self.gateway.unregister("web").await;

        let surface = crate::api::WebSurface::new();
        let ctx = SurfaceContext {
            kernel: self.kernel_handle.clone(),
            gateway: self.gateway.clone(),
            config: self.config.clone(),
            config_path: self.config_path.clone(),
            web_dist: self.web_dist.clone(),
            shutdown,
        };
        let handle = surface.start(ctx).await?;
        if let Some(channel) = handle.channel {
            self.gateway.register(channel).await?;
        }
        // WebSurface::start always spawns exactly one server task.
        // Audit F-4: surface.start() contract is to spawn exactly one task;
        // return an explicit error instead of panicking if violated.
        handle
            .tasks
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("WebSurface::start returned no server task"))
    }
}

struct WebState {
    restarter: Arc<WebSurfaceRestarter>,
    retries: u32,
    last_start: Instant,
}

/// A scheduled restart awaiting its backoff deadline.
struct PendingRestart {
    deadline: Instant,
}

/// Runtime supervisor: observes critical tasks, restarts the web surface on
/// crash, escalates to fatal exit for critical-task deaths.
pub struct TaskSupervisor {
    root: CancellationToken,
    restart: RestartConfig,
    tasks: FuturesUnordered<TaggedFut>,
    web: Option<WebState>,
    pending: Option<PendingRestart>,
    /// Callback to stop the gateway event loop during shutdown drain.
    stop_gateway: Option<Box<dyn FnOnce() + Send>>,
    /// Guardian heartbeat (Unix epoch seconds). `None` when no Guardian
    /// is running (e.g. CLI subcommands that don't start the daemon).
    /// The supervisor checks staleness every 60s via the select! timer
    /// branch (RFC-040 A3).
    guardian_heartbeat: Option<Arc<AtomicU64>>,
}

impl TaskSupervisor {
    /// Construct with the root shutdown token (owned by the supervisor) and a
    /// restart policy for the web surface.
    pub fn new(root: CancellationToken, restart: RestartConfig) -> Self {
        Self {
            root,
            restart,
            tasks: FuturesUnordered::new(),
            web: None,
            pending: None,
            stop_gateway: None,
            guardian_heartbeat: None,
        }
    }

    /// Track a critical (fail-fast) task by name. Any exit → fatal.
    pub fn track_critical(&mut self, name: impl Into<String>, handle: JoinHandle<()>) {
        self.tasks.push(tagged(Tag::Critical(name.into()), handle));
    }

    /// Track the web surface as scoped-restart, together with its restarter.
    pub fn track_web(&mut self, handle: JoinHandle<()>, restarter: Arc<WebSurfaceRestarter>) {
        self.web = Some(WebState {
            restarter,
            retries: 0,
            last_start: Instant::now(),
        });
        self.tasks.push(tagged(Tag::Web, handle));
    }

    /// Set the callback used to stop the gateway during shutdown drain. The
    /// gateway task is tracked separately (fail-fast); this stops its own
    /// event loop so it drains in-flight dispatches before the process exits.
    pub fn with_gateway_stop(&mut self, stop: impl FnOnce() + Send + 'static) {
        self.stop_gateway = Some(Box::new(stop));
    }

    /// Register a Guardian heartbeat for watchdog monitoring (RFC-040 A3).
    /// The supervisor checks staleness every 60s via the select! timer
    /// branch. The heartbeat must be updated by the Guardian loop every
    /// cycle; three missed cycles (900s) → `process::abort()`.
    pub fn watch_guardian(&mut self, heartbeat: Arc<AtomicU64>) {
        self.guardian_heartbeat = Some(heartbeat);
    }

    /// Run the supervision loop until graceful or fatal exit, then drain all
    /// tracked tasks. For BOTH outcomes the live tasks are drained (RFC-030
    /// A6): a fatal exit still cancels the root token and waits so the process
    /// doesn't leave in-flight requests half-served — only the exit code
    /// differs (non-zero on fatal, so the OS supervisor restarts).
    pub async fn run(mut self) -> ShutdownOutcome {
        let outcome = self.watch().await;
        let drain_timeout = match &outcome {
            ShutdownOutcome::Graceful => Duration::from_secs(10),
            ShutdownOutcome::Fatal { .. } => Duration::from_secs(3),
        };
        self.drain(drain_timeout).await;
        outcome
    }

    /// The non-blocking supervision loop (R7): always inside `select!`, never
    /// blocks inline. ctrl_c and critical-task deaths stay observable
    /// throughout every backoff window.
    async fn watch(&mut self) -> ShutdownOutcome {
        loop {
            // Snapshot the pending deadline into a local so the timer branch
            // owns no borrow of `self` (lets the arm body borrow `self` mutably).
            let mut timer: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
                match self.pending.as_ref() {
                    Some(p) => Box::pin(tokio::time::sleep_until(p.deadline.into())),
                    None => Box::pin(std::future::pending::<()>()),
                };

            // Heartbeat timer: sleep when Guardian is registered, pending
            // forever when not — same select!-safety pattern as timer.
            let hb_timer: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
                if self.guardian_heartbeat.is_some() {
                    Box::pin(tokio::time::sleep(GUARDIAN_CHECK_INTERVAL))
                } else {
                    Box::pin(std::future::pending::<()>())
                };

            tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => return ShutdownOutcome::Graceful,
                _ = self.root.cancelled() => return ShutdownOutcome::Graceful,
                // Any tracked task completed. FuturesUnordered auto-removes
                // the finished future; we push a fresh one on restart.
                Some((tag, result)) = self.tasks.next() => {
                    if let Some(outcome) = self.on_completion(tag, result).await {
                        return outcome;
                    }
                }
                _ = async {
                    timer.as_mut().await;
                } => {
                    if let Some(outcome) = self.fire_web_restart().await {

                        return outcome;
                    }
                }
                // RFC-040 A3: Guardian heartbeat watchdog. Fires every 60s
                // when a heartbeat is registered. Only reads an atomic and
                // compares integers — zero kernel calls, survives a wedged
                // kernel because it makes no blocking/syscall demands.
                _ = hb_timer => {
                    if let Some(hb) = &self.guardian_heartbeat {
                        let last = hb.load(Ordering::Relaxed);
                        let now = now_secs();
                        let stale = now.saturating_sub(last);
                        if stale > GUARDIAN_STALE_THRESHOLD_SECS {
                            tracing::error!(
                                last_seen = last,
                                now = now,
                                stale_secs = stale,
                                threshold = GUARDIAN_STALE_THRESHOLD_SECS,
                                "Guardian heartbeat stale — aborting process"
                            );
                            // tracing's non-blocking appender does not flush
                            // on abort — write directly to stderr (redirected
                            // to the append-mode log file by the daemon launcher)
                            // so the diagnostic survives.
                            eprintln!(
                                "Guardian heartbeat stale ({stale}s > {GUARDIAN_STALE_THRESHOLD_SECS}s) — aborting"
                            );
                            std::process::abort();
                        }
                    }
                }
            }
        }
    }

    /// Drain all tracked tasks on shutdown: cancel the root token (web
    /// surfaces drain in-flight requests), stop the gateway, then await every
    /// remaining task up to `timeout`. Tasks that don't finish in time are
    /// detached — the process is exiting regardless.
    async fn drain(&mut self, timeout: Duration) {
        tracing::info!(
            timeout_secs = timeout.as_secs(),
            "Supervisor draining tracked tasks"
        );
        self.root.cancel();
        if let Some(stop) = self.stop_gateway.take() {
            stop();
        }
        let _ = tokio::time::timeout(timeout, async {
            while self.tasks.next().await.is_some() {}
        })
        .await;
    }

    /// Process one completed task. Returns `Some(outcome)` to exit the loop.
    async fn on_completion(
        &mut self,
        tag: Tag,
        result: Result<(), JoinError>,
    ) -> Option<ShutdownOutcome> {
        match tag {
            Tag::Critical(name) => {
                let reason = describe_exit(&name, result);
                tracing::error!(task = %name, %reason, "Critical task exited unexpectedly");
                Some(ShutdownOutcome::Fatal { name, reason })
            }
            Tag::Web => {
                let reason = describe_exit("web", result);
                // Audit F-4: the web state must be present while a web task
                // is tracked. If it's not, this is a supervisor invariant
                // violation — escalate to fatal rather than panic.
                let ws = match self.web.as_mut() {
                    Some(ws) => ws,
                    None => {
                        tracing::error!(
                            task = "web",
                            "Web state missing while web task is tracked — supervisor invariant broken"
                        );
                        return Some(ShutdownOutcome::Fatal {
                            name: "web".to_string(),
                            reason: format!("{reason} (missing web state)"),
                        });
                    }
                };
                // Reset the retry budget if the surface ran stably long enough.
                if ws.last_start.elapsed() >= self.restart.reset_after {
                    ws.retries = 0;
                }
                ws.retries += 1;
                if ws.retries > self.restart.max_retries {
                    tracing::error!(
                        task = "web",
                        retries = ws.retries,
                        %reason,
                        "Web surface restart budget exhausted; escalating to fatal"
                    );
                    return Some(ShutdownOutcome::Fatal {
                        name: "web".to_string(),
                        reason: format!("{reason} (retries exhausted: {})", ws.retries),
                    });
                }
                let backoff = compute_backoff(&self.restart, ws.retries);
                tracing::warn!(
                    task = "web",
                    retry = ws.retries,
                    max = self.restart.max_retries,
                    backoff_ms = backoff.as_millis() as u64,
                    %reason,
                    "Web surface exited; scheduling restart"
                );
                self.pending = Some(PendingRestart {
                    deadline: Instant::now() + backoff,
                });
                None
            }
        }
    }

    async fn fire_web_restart(&mut self) -> Option<ShutdownOutcome> {
        let _pending = self.pending.take()?;
        // Audit F-4: invariant — web state must exist while a web restart
        // is pending. If missing, escalate to fatal rather than panic.
        let ws = match self.web.as_mut() {
            Some(ws) => ws,
            None => {
                tracing::error!(
                    task = "web",
                    "Web state missing during fire_web_restart — supervisor invariant broken"
                );
                return Some(ShutdownOutcome::Fatal {
                    name: "web".to_string(),
                    reason: "missing web state during restart".into(),
                });
            }
        };
        // Fresh child token for the new instance (root cancel still cascades).
        let child = self.root.child_token();
        match ws.restarter.start(child).await {
            Ok(handle) => {
                ws.last_start = Instant::now();
                self.tasks.push(tagged(Tag::Web, handle));
                tracing::info!(task = "web", "Web surface restarted");
                oxios_kernel::metrics::get_metrics().inc_supervisor_restart();
                None
            }
            Err(e) => {
                // Re-bind / start failure — reschedule with the next backoff,
                // or escalate if the budget is spent.
                ws.retries += 1;
                if ws.retries > self.restart.max_retries {
                    tracing::error!(task = "web", error = %e, "Web restart failed; budget exhausted");
                    return Some(ShutdownOutcome::Fatal {
                        name: "web".to_string(),
                        reason: format!("restart failed: {e}"),
                    });
                }
                let backoff = compute_backoff(&self.restart, ws.retries);
                tracing::warn!(
                    task = "web",
                    retry = ws.retries,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %e,
                    "Web restart failed; rescheduling"
                );
                self.pending = Some(PendingRestart {
                    deadline: Instant::now() + backoff,
                });
                None
            }
        }
    }
}

/// Exponential backoff: `initial * 2^(attempt-1)`, capped at `max_backoff`.
fn compute_backoff(cfg: &RestartConfig, attempt: u32) -> Duration {
    let exp = cfg
        .initial_backoff
        .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
    exp.min(cfg.max_backoff)
}

/// Render a `JoinHandle` result as a human-readable exit reason.
fn describe_exit(name: &str, result: Result<(), JoinError>) -> String {
    match result {
        Ok(()) => format!("'{name}' task completed unexpectedly"),
        Err(e) if e.is_cancelled() => format!("'{name}' task was cancelled"),
        Err(e) if e.is_panic() => format!("'{name}' task panicked"),
        Err(e) => format!("'{name}' task error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_and_capped() {
        let cfg = RestartConfig {
            max_retries: 5,
            reset_after: Duration::from_secs(300),
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        };
        assert_eq!(compute_backoff(&cfg, 1), Duration::from_millis(500));
        assert_eq!(compute_backoff(&cfg, 2), Duration::from_secs(1));
        assert_eq!(compute_backoff(&cfg, 3), Duration::from_secs(2));
        assert_eq!(compute_backoff(&cfg, 4), Duration::from_secs(4));
        // Capped at max_backoff (30s) for large attempts.
        assert_eq!(compute_backoff(&cfg, 100), Duration::from_secs(30));
    }

    #[test]
    fn backoff_attempt_zero_is_initial() {
        let cfg = RestartConfig::default();
        // saturating_sub clamps the exponent base to 2^0 = 1 → initial_backoff.
        assert_eq!(compute_backoff(&cfg, 0), cfg.initial_backoff);
    }

    #[tokio::test]
    async fn failfast_task_exit_is_fatal() {
        let root = CancellationToken::new();
        let mut sup = TaskSupervisor::new(root.clone(), RestartConfig::default());
        // A task that immediately finishes (unexpected for a critical task).
        sup.track_critical("gateway", tokio::spawn(async {}));
        let outcome = sup.run().await;
        match outcome {
            ShutdownOutcome::Fatal { name, .. } => assert_eq!(name, "gateway"),
            _ => panic!("expected Fatal for critical task exit"),
        }
    }

    #[tokio::test]
    async fn ctrl_c_is_graceful() {
        // We can't easily send a real ctrl_c in a test, but root cancellation
        // is an equivalent graceful signal the supervisor honours.
        let root = CancellationToken::new();
        let mut sup = TaskSupervisor::new(root.clone(), RestartConfig::default());
        // A task that stays alive until cancelled, so the post-watch drain()
        // completes instantly when the root token is cancelled instead of
        // waiting out the full drain timeout.
        let task_token = root.clone();
        sup.track_critical(
            "gateway",
            tokio::spawn(async move {
                task_token.cancelled().await;
            }),
        );
        root.cancel();
        let outcome = sup.run().await;
        assert!(matches!(outcome, ShutdownOutcome::Graceful));
    }

    #[test]
    fn heartbeat_stale_past_threshold() {
        let hb = Arc::new(AtomicU64::new(0)); // epoch = very stale
        let now = now_secs();
        let stale = now.saturating_sub(hb.load(Ordering::Relaxed));
        assert!(
            stale > GUARDIAN_STALE_THRESHOLD_SECS,
            "epoch 0 should be stale past threshold"
        );
    }

    #[test]
    fn heartbeat_fresh_under_threshold() {
        let hb = Arc::new(AtomicU64::new(now_secs()));
        let now = now_secs();
        let stale = now.saturating_sub(hb.load(Ordering::Relaxed));
        assert!(
            stale <= GUARDIAN_STALE_THRESHOLD_SECS,
            "current time should be fresh"
        );
    }

    #[test]
    fn watch_guardian_registers_heartbeat() {
        let root = CancellationToken::new();
        let mut sup = TaskSupervisor::new(root.clone(), RestartConfig::default());
        assert!(sup.guardian_heartbeat.is_none());
        let hb = Arc::new(AtomicU64::new(now_secs()));
        sup.watch_guardian(hb);
        assert!(sup.guardian_heartbeat.is_some());
    }

    #[tokio::test]
    async fn drain_does_not_hang_on_unfinished_task() {
        let root = CancellationToken::new();
        let mut sup = TaskSupervisor::new(root.clone(), RestartConfig::default());
        // Task that never completes.
        sup.track_critical(
            "stuck",
            tokio::spawn(async {
                std::future::pending::<()>().await;
            }),
        );
        root.cancel();
        // run() will drain with timeout — must not hang.
        let start = Instant::now();
        let _ = sup.run().await;
        // Graceful drain timeout is 10s; fatal is 3s. Either way
        // should complete well under 15s.
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "drain should timeout, not hang"
        );
    }
}
