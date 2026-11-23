//! The manager: one runtime generation's sidecars.
//!
//! Parity target: `internal/ext/extension/sidecar/manager.go`. Processes are
//! started four at a time under one generation-wide budget, so a slow optional
//! runtime cannot serialize every later package and N stalled optional runtimes
//! cannot delay boot by N times the per-client handshake budget. Results are
//! consumed in package order, so which failure is fatal never depends on
//! completion order.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::lifecycle::SessionContext;

use super::client::{Client, ClientError, ClientOptions, DEFAULT_HANDSHAKE_TIMEOUT};
use super::installed::{InstalledRuntimePackage, load_runtime_packages};

/// Process-creation bound for one generation.
pub const MAX_CONCURRENT_PACKAGE_STARTS: usize = 4;
/// Shared by every runtime in one generation.
pub const PACKAGE_STARTUP_BUDGET: Duration = DEFAULT_HANDSHAKE_TIMEOUT;

/// Why a batch start failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerError {
    /// No required package failed, but the installed state could not be read.
    State(String),
    /// A required runtime failed to start, which fails the whole build.
    Required {
        plugin: String,
        error: Box<ClientError>,
    },
}

impl std::fmt::Display for ManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(detail) => f.write_str(detail),
            Self::Required { plugin, error } => write!(
                f,
                "required extension runtime {plugin:?} failed to start: {error}"
            ),
        }
    }
}

impl std::error::Error for ManagerError {}

/// The outcome of a batch start: which packages came up and what was skipped.
#[derive(Debug)]
pub struct StartOutcome {
    pub manager: Manager,
    pub warnings: Vec<String>,
}

/// Every sidecar started for one runtime generation.
///
/// Inputs are the installed state alone; there is no constructor that accepts a
/// command, so this cannot launch anything the user did not install.
pub struct Manager {
    clients: BTreeMap<String, Arc<Client>>,
    closed: bool,
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("plugins", &self.clients.keys().collect::<Vec<_>>())
            .field("closed", &self.closed)
            .finish()
    }
}

impl Manager {
    /// An empty manager. Only useful for a generation with no runtimes.
    pub fn empty() -> Self {
        Self {
            clients: BTreeMap::new(),
            closed: false,
        }
    }

    /// Enumerates the installed runtime packages and starts them.
    ///
    /// Optional failures become warnings; the first required failure in package
    /// order fails the call and closes everything already started.
    pub fn start_packages(
        home: &Path,
        session: SessionContext,
        options: &ClientOptions,
    ) -> Result<StartOutcome, ManagerError> {
        let enumerated =
            load_runtime_packages(home).map_err(|err| ManagerError::State(err.to_string()))?;
        let mut warnings = enumerated.warnings;
        let packages = enumerated.packages;
        let mut options = options.clone();
        options.session = session;
        let mut started = start_packages_with_budget(
            packages,
            &options,
            PACKAGE_STARTUP_BUDGET,
            MAX_CONCURRENT_PACKAGE_STARTS,
        );
        warnings.extend(started.warnings);
        match started.required_failure {
            Some(ManagerError::Required { plugin, error }) => {
                started.manager.close();
                Err(ManagerError::Required { plugin, error })
            }
            Some(other) => Err(other),
            None => Ok(StartOutcome {
                manager: started.manager,
                warnings,
            }),
        }
    }

    /// Starts an explicit package list under a shared budget.
    ///
    /// This is the seam the tests drive: the packages still have to come from
    /// the installed state, but a caller that already enumerated them can pass
    /// them in rather than re-reading the disk.
    pub fn start_packages_with(
        packages: Vec<InstalledRuntimePackage>,
        session: SessionContext,
        options: &ClientOptions,
        budget: Duration,
        concurrency: usize,
    ) -> StartOutcome {
        let mut options = options.clone();
        options.session = session;
        let started = start_packages_with_budget(packages, &options, budget, concurrency);
        StartOutcome {
            manager: started.manager,
            warnings: started.warnings,
        }
    }

    /// The client for one plugin ID.
    pub fn client(&self, plugin_id: &str) -> Option<Arc<Client>> {
        self.clients.get(plugin_id).cloned()
    }

    /// Every live client, ordered by plugin ID.
    pub fn clients(&self) -> Vec<Arc<Client>> {
        self.clients.values().cloned().collect()
    }

    /// Live plugin IDs, ordered.
    pub fn plugin_ids(&self) -> Vec<&str> {
        self.clients.keys().map(String::as_str).collect()
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Shuts every sidecar down in parallel; each client's own budgets bound
    /// the total. Idempotent.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let clients: Vec<Arc<Client>> = std::mem::take(&mut self.clients).into_values().collect();
        let handles: Vec<_> = clients
            .into_iter()
            .map(|client| std::thread::spawn(move || client.close()))
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.close();
    }
}

struct Started {
    manager: Manager,
    warnings: Vec<String>,
    required_failure: Option<ManagerError>,
}

/// One package's slot in the batch, so results keep package order.
struct Slot {
    package: InstalledRuntimePackage,
    outcome: Mutex<Option<Result<Arc<Client>, ClientError>>>,
}

/// A counting semaphore bounding concurrent process creation.
struct Gate {
    state: Mutex<GateState>,
    cv: Condvar,
}

struct GateState {
    free: usize,
    closed: bool,
}

impl Gate {
    fn new(permits: usize) -> Self {
        Self {
            state: Mutex::new(GateState {
                free: permits.max(1),
                closed: false,
            }),
            cv: Condvar::new(),
        }
    }

    fn acquire(&self, deadline: Instant) -> bool {
        let mut state = self.state.lock().expect("gate mutex");
        loop {
            if state.closed {
                return false;
            }
            if state.free > 0 {
                state.free -= 1;
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (next, timeout) = self
                .cv
                .wait_timeout(state, deadline - now)
                .expect("gate mutex");
            state = next;
            if timeout.timed_out() && state.free == 0 {
                return false;
            }
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("gate mutex");
        state.free += 1;
        self.cv.notify_one();
    }

    fn close(&self) {
        let mut state = self.state.lock().expect("gate mutex");
        state.closed = true;
        self.cv.notify_all();
    }
}

fn start_packages_with_budget(
    packages: Vec<InstalledRuntimePackage>,
    options: &ClientOptions,
    budget: Duration,
    concurrency: usize,
) -> Started {
    let deadline = Instant::now() + budget;
    let slots: Arc<Vec<Slot>> = Arc::new(
        packages
            .into_iter()
            .map(|package| Slot {
                package,
                outcome: Mutex::new(None),
            })
            .collect(),
    );
    if slots.is_empty() {
        return Started {
            manager: Manager::empty(),
            warnings: Vec::new(),
            required_failure: None,
        };
    }
    let workers = concurrency
        .clamp(1, MAX_CONCURRENT_PACKAGE_STARTS)
        .min(slots.len());
    let gate = Arc::new(Gate::new(workers));
    let next = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let slots = Arc::clone(&slots);
            let gate = Arc::clone(&gate);
            let next = Arc::clone(&next);
            let options = options.clone();
            std::thread::spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(slot) = slots.get(index) else {
                        return;
                    };
                    if Instant::now() >= deadline || !gate.acquire(deadline) {
                        *slot.outcome.lock().expect("slot mutex") =
                            Some(Err(ClientError::InvalidOptions(format!(
                                "extension generation startup stopped before launch of {}",
                                slot.package.plugin_id()
                            ))));
                        continue;
                    }
                    let outcome = Client::start(&slot.package, &options).map(Arc::new);
                    gate.release();
                    *slot.outcome.lock().expect("slot mutex") = Some(outcome);
                }
            })
        })
        .collect();
    for handle in handles {
        let _ = handle.join();
    }
    gate.close();

    let mut manager = Manager::empty();
    let mut warnings = Vec::new();
    let mut required_failure = None;
    for slot in slots.iter() {
        let outcome = slot.outcome.lock().expect("slot mutex").take();
        let package = &slot.package;
        match outcome {
            Some(Ok(client)) => {
                manager
                    .clients
                    .insert(package.plugin_id().to_string(), client);
            }
            Some(Err(error)) => {
                if package.runtime().required {
                    if required_failure.is_none() {
                        required_failure = Some(ManagerError::Required {
                            plugin: package.plugin_id().to_string(),
                            error: Box::new(error),
                        });
                    }
                } else {
                    warnings.push(format!(
                        "{}: optional extension runtime failed to start: {error}",
                        package.plugin_id()
                    ));
                }
            }
            None => warnings.push(format!(
                "{}: optional extension runtime was not started",
                package.plugin_id()
            )),
        }
    }
    if required_failure.is_some() {
        manager.close();
    }
    Started {
        manager,
        warnings,
        required_failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::installed::{InstalledPlugin, RuntimeManifest, RuntimeSpec};

    fn package(name: &str, required: bool, command: &str) -> InstalledRuntimePackage {
        InstalledRuntimePackage {
            installed: InstalledPlugin {
                name: name.into(),
                version: "1.0.0".into(),
                enabled: true,
                ..InstalledPlugin::default()
            },
            manifest: RuntimeManifest {
                name: name.into(),
                version: "1.0.0".into(),
                runtime: Some(RuntimeSpec {
                    command: command.into(),
                    required,
                    ..RuntimeSpec::default()
                }),
                ..RuntimeManifest::default()
            },
            root: std::env::temp_dir(),
        }
    }

    fn session() -> SessionContext {
        SessionContext {
            session_id: "s-1".into(),
            workspace_root: std::env::temp_dir().display().to_string(),
            generation: 1,
            epoch: String::new(),
        }
    }

    #[test]
    fn required_failure_fails_the_batch_and_closes_optional_clients() {
        let started = Manager::start_packages_with(
            vec![
                package("bad-required", true, "/nonexistent/binary"),
                package("good-optional", false, "relative-name"),
            ],
            session(),
            &ClientOptions::default(),
            Duration::from_secs(5),
            2,
        );
        // Both fail (one is required, one violates exec form), but the optional
        // one must surface as a warning, not a fatal error.
        assert_eq!(started.warnings.len(), 1);
        assert!(
            started.warnings[0].starts_with("good-optional:"),
            "{:?}",
            started.warnings
        );
        assert!(started.manager.is_empty());
        assert!(started.manager.is_closed());
    }

    #[test]
    fn concurrency_bound_is_the_smaller_of_workers_and_packages() {
        let packages: Vec<_> = (0..6)
            .map(|index| package(&format!("p{index}"), false, "relative-name"))
            .collect();
        let started = Manager::start_packages_with(
            packages,
            session(),
            &ClientOptions::default(),
            Duration::from_secs(5),
            50,
        );
        // All six fail on the exec contract, in package order.
        assert_eq!(started.warnings.len(), 6);
        for (index, warning) in started.warnings.iter().enumerate() {
            assert!(
                warning.starts_with(&format!("p{index}:")),
                "warning {index} = {warning}"
            );
        }
    }

    #[test]
    fn an_expired_budget_starts_nothing() {
        let started = Manager::start_packages_with(
            vec![package("slow", false, "/nonexistent/binary")],
            session(),
            &ClientOptions::default(),
            Duration::ZERO,
            1,
        );
        assert!(started.manager.is_empty());
        assert_eq!(started.warnings.len(), 1);
        assert!(
            started.warnings[0].contains("stopped before launch"),
            "{:?}",
            started.warnings
        );
    }
}
