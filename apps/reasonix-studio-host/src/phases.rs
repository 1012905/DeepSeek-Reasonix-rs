//! Startup phase log lines (docs/contracts/host.md §1.4): one stderr line per
//! phase transition, `reasonix-studio-host: startup phase=<name> elapsed_ms=<n>`,
//! in the frozen 12-phase order.

use std::io::Write;
use std::time::Instant;

// The names the assembly emits; kept as the binary's frozen vocabulary.
#[allow(dead_code)]
pub const PAGE: &str = "page";
#[allow(dead_code)]
pub const CONFIG_UPGRADE: &str = "config-upgrade";
#[allow(dead_code)]
pub const CONFIG: &str = "config";
#[allow(dead_code)]
pub const SINKS: &str = "sinks";
#[allow(dead_code)]
pub const WORKSPACE: &str = "workspace";
#[allow(dead_code)]
pub const RUNTIME: &str = "runtime";
#[allow(dead_code)]
pub const HUB: &str = "hub";
#[allow(dead_code)]
pub const ADOPT: &str = "adopt";
#[allow(dead_code)]
pub const BACKGROUND: &str = "background";
#[allow(dead_code)]
pub const BIND: &str = "bind";
#[allow(dead_code)]
pub const PROVIDER_SETUP: &str = "provider-setup";
pub const ANNOUNCE: &str = "announce";

pub struct StartupPhases<W: Write> {
    logs: W,
    last: Instant,
}

impl<W: Write> StartupPhases<W> {
    pub fn new(logs: W) -> Self {
        Self {
            logs,
            last: Instant::now(),
        }
    }

    pub fn mark(&mut self, phase: &str) {
        let now = Instant::now();
        self.write(phase, now.duration_since(self.last).as_millis() as u64);
        self.last = now;
    }

    /// A kernel boot phase reported by the assembly: logged with the `boot.`
    /// prefix, its own duration. Consumed when the runtime slice reports its
    /// boot phases.
    #[allow(dead_code)]
    pub fn boot(&mut self, name: &str, elapsed_ms: u64) {
        self.write(&format!("boot.{name}"), elapsed_ms);
    }

    fn write(&mut self, phase: &str, elapsed_ms: u64) {
        let _ = writeln!(
            self.logs,
            "reasonix-studio-host: startup phase={phase} elapsed_ms={elapsed_ms}"
        );
        let _ = self.logs.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_lines_use_the_exact_format() {
        let mut buf = Vec::new();
        {
            let mut p = StartupPhases::new(&mut buf);
            p.mark(PAGE);
            p.mark(CONFIG_UPGRADE);
            p.boot("sessions", 42);
        }
        let out = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("reasonix-studio-host: startup phase=page elapsed_ms="));
        assert!(lines[1].contains("phase=config-upgrade elapsed_ms="));
        assert!(lines[2].contains("phase=boot.sessions elapsed_ms=42"));
    }
}
