//! CPU pinning and the platform probes BEN-011 asks for.
//!
//! # Why this exists
//!
//! BEN-011 says "pinned runs". The harness reported warmup, repetitions and
//! dispersion — and then pinned nothing. `artifact.rs` even shipped
//! `governor: "unknown (platform probe lands at T4.3)"` while T4.3 was the
//! task that closed without landing it.
//!
//! The cost was not theoretical. Between full matrix runs, lanes nobody had
//! touched moved +95% (resp3), +75% (bolt) and +45% (http) on the same cell,
//! and four cells swung up to 43 points — against a gate that asks whether
//! Thunder leads by 10%. The scheduler was free to migrate the driver and the
//! listener across cores (and across P/E clusters on a hybrid CPU) between
//! repetitions, so consecutive repetitions were not measuring the same machine.
//!
//! # What pinning does and does not fix
//!
//! Pinning removes migration and cross-core cache effects from the comparison.
//! It does **not** make a busy machine quiet: another process on the pinned
//! cores still perturbs the run. That is what the noise floor
//! ([`crate::stats::noise_check`]) is for — pinning narrows the distribution,
//! the floor refuses the run when it is still too wide to judge.
//!
//! # Fairness (BEN-001)
//!
//! Every lane runs in the same process on the same runtime, so pinning applies
//! identically to Thunder and to every peer. It cannot advantage one lane; it
//! removes a source of variance from all of them at once.

use std::fmt;

/// What a run actually pinned, recorded in the artifact header so a reader can
/// tell a pinned run from an unpinned one instead of trusting the flag.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PinReport {
    /// Whether pinning was requested for this run.
    pub requested: bool,
    /// Core ids the runtime threads were pinned to, in assignment order.
    pub cores: Vec<usize>,
    /// Cores visible to the process.
    pub available: usize,
    /// Present when pinning was requested and could not be honored — the run
    /// is still valid, but it is not a pinned run and must not claim to be.
    pub failure: Option<String>,
}

impl PinReport {
    /// A run that never asked to pin.
    pub fn unpinned(available: usize) -> Self {
        Self {
            requested: false,
            cores: Vec::new(),
            available,
            failure: None,
        }
    }

    /// Whether this run is genuinely pinned.
    pub fn is_pinned(&self) -> bool {
        self.requested && self.failure.is_none() && !self.cores.is_empty()
    }
}

impl fmt::Display for PinReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.requested {
            return write!(f, "not pinned ({} cores available)", self.available);
        }
        match &self.failure {
            Some(why) => write!(f, "pinning REQUESTED BUT FAILED: {why}"),
            None => write!(f, "pinned to cores {:?} of {}", self.cores, self.available),
        }
    }
}

/// The cores this process may run on, in a stable order.
pub fn available_cores() -> Vec<usize> {
    core_affinity::get_core_ids()
        .map(|ids| {
            let mut cores: Vec<usize> = ids.into_iter().map(|c| c.id).collect();
            cores.sort_unstable();
            cores
        })
        .unwrap_or_default()
}

/// Pin the calling thread to `core`. Returns whether it took.
pub fn pin_current_thread(core: usize) -> bool {
    core_affinity::set_for_current(core_affinity::CoreId { id: core })
}

/// Choose the cores a run should pin to.
///
/// Takes the **lowest-numbered** cores deliberately: on hybrid CPUs (Intel
/// P/E, ARM big.LITTLE) the performance cores are conventionally numbered
/// first, and a run that lands half on P-cores and half on E-cores measures the
/// topology rather than the protocol. Leaves the rest of the machine free so
/// the OS has somewhere else to run.
pub fn choose_cores(available: &[usize], want: usize) -> Vec<usize> {
    available.iter().copied().take(want).collect()
}

/// Kernel/OS version — the BEN-011 header field that shipped as
/// `"unknown (platform probe lands at T4.3)"`.
pub fn kernel_version() -> String {
    #[cfg(windows)]
    {
        // Not `cmd /C ver`: that prints a *localized* string in the OEM
        // codepage ("Microsoft Windows [versão 10.0.19045]"), which arrives
        // here as mojibake and differs per machine language. OSVersion is
        // ASCII and locale-independent, so artifacts from two machines stay
        // comparable.
        probe(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "[System.Environment]::OSVersion.VersionString",
            ],
        )
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
    }
    #[cfg(unix)]
    {
        probe("uname", &["-sr"])
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|| "unknown".to_owned())
    }
    #[cfg(not(any(windows, unix)))]
    {
        "unknown (unsupported platform)".to_owned()
    }
}

/// CPU frequency governor / power policy — the other unfulfilled header field.
///
/// This one matters for a benchmark: a governor that ramps frequency on demand
/// makes the first repetition slower than the last for reasons that have
/// nothing to do with the code under test.
pub fn governor() -> String {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|_| "unknown (no cpufreq sysfs — VM or unsupported driver)".to_owned())
    }
    #[cfg(windows)]
    {
        // The active power scheme is Windows' equivalent knob. `powercfg`
        // prints "Power Scheme GUID: <guid>  (<name>)" — the name is
        // localized, so the GUID is what makes two artifacts comparable.
        // Report both, GUID first, and translate the well-known ones.
        probe("powercfg", &["/getactivescheme"])
            .and_then(|s| {
                let guid = s
                    .split_whitespace()
                    .find(|w| w.len() == 36 && w.contains('-'))?;
                Some(match guid {
                    "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c" => {
                        format!("{guid} (High performance)")
                    }
                    "381b4222-f694-41f0-9685-ff5bb260df2e" => {
                        format!(
                            "{guid} (Balanced — ramps frequency on demand; prefer High \
                                 performance for benchmark runs)"
                        )
                    }
                    "a1841308-3541-4fab-bc81-f71556f20b4a" => {
                        format!("{guid} (Power saver — caps frequency; not a benchmark setting)")
                    }
                    other => format!("{other} (unrecognized scheme)"),
                })
            })
            .unwrap_or_else(|| "unknown (powercfg unavailable)".to_owned())
    }
    #[cfg(all(not(target_os = "linux"), not(windows)))]
    {
        "unknown (no governor probe for this platform)".to_owned()
    }
}

/// Run a probe command, returning its stdout when it succeeds.
///
/// Probes are best-effort by design: a missing tool or a locked-down runner
/// must degrade the header to `"unknown"`, never fail the benchmark.
#[cfg(any(windows, unix))]
fn probe(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn choose_takes_the_lowest_cores_for_hybrid_topologies() {
        let available = vec![0, 1, 2, 3, 4, 5, 6, 7];
        assert_eq!(choose_cores(&available, 4), vec![0, 1, 2, 3]);
    }

    #[test]
    fn choose_never_asks_for_more_than_exists() {
        let available = vec![0, 1];
        assert_eq!(choose_cores(&available, 8), vec![0, 1]);
    }

    #[test]
    fn choose_on_no_cores_is_empty_rather_than_panicking() {
        assert!(choose_cores(&[], 4).is_empty());
    }

    #[test]
    fn an_unpinned_report_says_so_and_is_not_pinned() {
        let r = PinReport::unpinned(8);
        assert!(!r.is_pinned());
        assert!(r.to_string().contains("not pinned"));
    }

    #[test]
    fn a_failed_pin_is_not_reported_as_pinned() {
        // The trap this guards: a run that asked to pin, failed, and would
        // otherwise be read as pinned because the flag was set.
        let r = PinReport {
            requested: true,
            cores: vec![0, 1],
            available: 8,
            failure: Some("set_for_current returned false".to_owned()),
        };
        assert!(!r.is_pinned());
        assert!(r.to_string().contains("FAILED"));
    }

    #[test]
    fn a_successful_pin_reports_its_cores() {
        let r = PinReport {
            requested: true,
            cores: vec![0, 1],
            available: 8,
            failure: None,
        };
        assert!(r.is_pinned());
        assert!(r.to_string().contains("pinned to cores [0, 1]"));
    }

    #[test]
    fn the_machine_reports_at_least_one_core() {
        // Sanity: if this returns empty, pinning is unavailable here and the
        // runner must degrade rather than claim a pinned run.
        let cores = available_cores();
        assert!(!cores.is_empty(), "no cores visible: {cores:?}");
    }

    #[test]
    fn probes_return_something_or_a_stated_unknown() {
        // Never empty, never a panic — the header must always say something.
        assert!(!kernel_version().is_empty());
        assert!(!governor().is_empty());
    }
}
