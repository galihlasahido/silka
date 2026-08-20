//! Where the numbers come from.
//!
//! There are two implementations of [`Source`] and the reason for the trait is
//! not extensibility, it is testability. Every claim this example makes is
//! about the *consequences* of a number changing — the chart keeps up, the
//! spring settles, the window goes idle. None of those can be tested against a
//! machine whose load nobody controls, so the tests drive [`Synthetic`] and the
//! window drives [`SystemSource`], and both go through the same door.
//!
//! ## The cadence is the source's decision, not the shell's
//!
//! [`Source::interval`] exists because `sysinfo` has a hard floor:
//! `MINIMUM_CPU_UPDATE_INTERVAL` (200 ms on every platform it supports).
//! Refreshing CPU usage faster than that does not produce fresher numbers, it
//! produces *wrong* ones — the figure is a ratio measured between two refreshes,
//! and dividing by an interval that short amplifies scheduling noise into
//! nonsense. So `--hz 60` on the real source is clamped, and `--hz 60` on the
//! synthetic source is honoured. That asymmetry is the truth about the two
//! sources rather than an inconsistency to paper over.

use std::time::Duration;

use crate::sample::{ProcessRow, Sample};

/// How many processes the table shows.
///
/// A monitor listing all 700 processes on a modern desktop is a monitor nobody
/// reads. The table underneath is virtualized and would survive all of them —
/// this is an editorial limit, not a technical one.
pub const PROCESS_LIMIT: usize = 64;

/// Anything that can report the state of a machine.
pub trait Source {
    /// One reading, stamped `at` seconds after the monitor started.
    ///
    /// The timestamp is passed in rather than read from a clock so that a test
    /// can produce a sixty-second history in a millisecond, and so that two
    /// runs of the same test produce the same chart.
    fn sample(&mut self, at: f64) -> Sample;

    /// The shortest gap between readings this source can honestly serve.
    fn interval(&self) -> Duration;

    /// A human-readable name for the status line.
    fn describe(&self) -> String;
}

// ---------------------------------------------------------------------------
// The real machine
// ---------------------------------------------------------------------------

/// The real thing, via `sysinfo`.
pub struct SystemSource {
    system: sysinfo::System,
    interval: Duration,
    /// Processes are refreshed once every this many samples. Walking every
    /// process in the system costs milliseconds; walking it at the CPU
    /// cadence would make the monitor the most expensive process on the list.
    process_every: u32,
    ticks: u32,
    processes: Vec<ProcessRow>,
}

impl SystemSource {
    /// Open a handle on the machine, sampling no faster than `interval`.
    ///
    /// The first CPU figure any process reports is zero — a usage percentage
    /// is measured *between* two refreshes and there has only been one. Rather
    /// than show a machine that is briefly, impossibly idle, the constructor
    /// takes that throwaway reading here.
    pub fn new(interval: Duration) -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        system.refresh_cpu_usage();
        let interval = interval.max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        // Refresh the process list about once a second whatever the CPU
        // cadence is, and at least every sample.
        let process_every = (Duration::from_secs(1).as_secs_f64() / interval.as_secs_f64())
            .ceil()
            .clamp(1.0, 64.0) as u32;
        Self {
            system,
            interval,
            process_every,
            ticks: 0,
            processes: Vec::new(),
        }
    }

    /// Re-read the process table and keep the [`PROCESS_LIMIT`] busiest.
    fn refresh_processes(&mut self) {
        self.system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory(),
        );
        let mut rows: Vec<ProcessRow> = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessRow {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                cpu: process.cpu_usage(),
                memory: process.memory(),
            })
            .collect();
        // Trimmed by CPU and then by memory, so a process that is idle but
        // holding eight gigabytes still makes the list — which is usually the
        // one the monitor was opened to find.
        crate::sample::sort_rows(&mut rows, crate::sample::ProcessSort::Cpu, true);
        if rows.len() > PROCESS_LIMIT {
            let (busy, rest) = rows.split_at_mut(PROCESS_LIMIT / 2);
            crate::sample::sort_rows(rest, crate::sample::ProcessSort::Memory, true);
            let mut kept: Vec<ProcessRow> = busy.to_vec();
            kept.extend_from_slice(&rest[..PROCESS_LIMIT - busy.len()]);
            rows = kept;
        }
        self.processes = rows;
    }
}

impl Source for SystemSource {
    fn sample(&mut self, at: f64) -> Sample {
        self.system.refresh_memory();
        self.system.refresh_cpu_usage();
        if self.ticks.is_multiple_of(self.process_every) {
            self.refresh_processes();
        }
        self.ticks = self.ticks.wrapping_add(1);
        Sample {
            at,
            cpu: self.system.global_cpu_usage(),
            cores: self
                .system
                .cpus()
                .iter()
                .map(sysinfo::Cpu::cpu_usage)
                .collect(),
            memory_used: self.system.used_memory(),
            memory_total: self.system.total_memory(),
            processes: self.processes.clone(),
        }
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn describe(&self) -> String {
        format!(
            "{} cores · every {} ms",
            self.system.cpus().len(),
            self.interval.as_millis()
        )
    }
}

// ---------------------------------------------------------------------------
// A machine that does what it is told
// ---------------------------------------------------------------------------

/// A deterministic stand-in for a machine under load.
///
/// Two jobs. In the tests it is the only way to say "here are six hundred
/// samples, sixty a second, and now nothing" and get the same answer twice. In
/// the window (`--source synthetic`) it is what makes the 60 Hz claim
/// *visible*: the real CPU counter cannot be read that fast, so without this
/// the chart-under-continuous-update claim would only ever be an assertion in a
/// test file.
///
/// The generator is a plain linear congruential sequence — not because it is a
/// good random number generator but because it is a reproducible one that needs
/// no dependency.
pub struct Synthetic {
    seed: u64,
    cores: usize,
    memory_total: u64,
    memory_used: f64,
    interval: Duration,
    processes: Vec<ProcessRow>,
    /// When set, every sample is identical to the last. The lever the "data
    /// stopped, so the window must go idle" test pulls.
    frozen: bool,
}

impl Synthetic {
    /// A machine with `cores` cores, sampling every `interval`.
    pub fn new(cores: usize, interval: Duration) -> Self {
        let cores = cores.clamp(1, 256);
        let memory_total = 17_179_869_184;
        Self {
            seed: 0x0005_DEEC_E66D_u64,
            cores,
            memory_total,
            memory_used: memory_total as f64 * 0.42,
            interval,
            processes: (0..24)
                .map(|i| ProcessRow {
                    pid: 100 + i as u32,
                    name: PROCESS_NAMES[i % PROCESS_NAMES.len()].to_string(),
                    cpu: 0.0,
                    memory: 32_000_000 * (i as u64 + 1),
                })
                .collect(),
            frozen: false,
        }
    }

    /// Stop the machine changing: from here on every sample equals the last.
    ///
    /// Only the tests pull this lever — a window has no button for "pretend
    /// the machine froze" and should not grow one.
    ///
    /// Not a pause — a paused monitor takes no samples at all. This is the
    /// harder case, where samples keep arriving and carry no news, and where an
    /// application that redraws on arrival rather than on *change* would never
    /// go idle.
    #[cfg(test)]
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// A number in 0..1.
    fn next(&mut self) -> f64 {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.seed >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

/// Names for the synthetic process list — recognisable, and none of them a
/// real person's.
const PROCESS_NAMES: &[&str] = &[
    "kernel_task",
    "WindowServer",
    "rustc",
    "cargo",
    "silka-monitor",
    "zsh",
    "node",
    "postgres",
];

impl Source for Synthetic {
    fn sample(&mut self, at: f64) -> Sample {
        if self.frozen {
            return Sample {
                at,
                cpu: self.processes.iter().map(|p| p.cpu).sum::<f32>() / self.cores.max(1) as f32,
                cores: (0..self.cores).map(|i| (i as f32 * 7.0) % 100.0).collect(),
                memory_used: self.memory_used as u64,
                memory_total: self.memory_total,
                processes: self.processes.clone(),
            };
        }

        // A slow sine for the shape plus a little noise for the texture: a
        // chart of pure noise proves the renderer works and tells the eye
        // nothing about whether the *scrolling* is right.
        let base = 35.0 + 30.0 * (at * 0.7).sin();
        let cores: Vec<f32> = (0..self.cores)
            .map(|i| {
                let phase = i as f64 * 0.6;
                let v = base + 22.0 * (at * 1.3 + phase).sin() + (self.next() - 0.5) * 18.0;
                v.clamp(0.0, 100.0) as f32
            })
            .collect();
        let cpu = cores.iter().sum::<f32>() / cores.len() as f32;

        // Memory drifts rather than jumping, and is held inside a believable
        // band: a monitor whose memory line touches zero is a monitor with a
        // bug, not a machine with free RAM.
        let drift = (self.next() - 0.48) * self.memory_total as f64 * 0.004;
        self.memory_used = (self.memory_used + drift).clamp(
            self.memory_total as f64 * 0.25,
            self.memory_total as f64 * 0.92,
        );

        for (i, row) in self.processes.iter_mut().enumerate() {
            let phase = i as f64 * 0.9;
            row.cpu = (12.0 * (at * 0.5 + phase).sin() + 14.0).max(0.0) as f32;
        }

        Sample {
            at,
            cpu,
            cores,
            memory_used: self.memory_used as u64,
            memory_total: self.memory_total,
            processes: self.processes.clone(),
        }
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn describe(&self) -> String {
        format!(
            "synthetic · {} cores · every {} ms",
            self.cores,
            self.interval.as_millis()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_dua_kali_jalan_memberi_angka_yang_sama() {
        // Reproducibility is not a nicety here: without it "the chart keeps up
        // at 60 Hz" would be a different test on every run.
        let mut a = Synthetic::new(8, Duration::from_millis(16));
        let mut b = Synthetic::new(8, Duration::from_millis(16));
        for i in 0..50 {
            let at = i as f64 * 0.016;
            assert_eq!(a.sample(at), b.sample(at));
        }
    }

    #[test]
    fn nilai_synthetic_tetap_masuk_akal() {
        let mut s = Synthetic::new(10, Duration::from_millis(16));
        for i in 0..400 {
            let sample = s.sample(i as f64 * 0.016);
            assert_eq!(sample.cores.len(), 10);
            assert!(sample.cpu.is_finite());
            assert!((0.0..=100.0).contains(&sample.cpu), "cpu {}", sample.cpu);
            for c in &sample.cores {
                assert!((0.0..=100.0).contains(c), "inti {c}");
            }
            assert!(sample.memory_used < sample.memory_total);
            assert!(sample.memory_fraction() > 0.2);
        }
    }

    #[test]
    fn dibekukan_berarti_sampel_berikutnya_persis_sama() {
        // The precondition of the idle proof: "data stopped" has to mean
        // bit-identical samples, or the equality checks downstream would be
        // testing the generator's smoothness instead of the framework.
        let mut s = Synthetic::new(4, Duration::from_millis(16));
        for i in 0..30 {
            s.sample(i as f64 * 0.016);
        }
        s.freeze();
        let first = s.sample(1.0);
        for i in 0..30 {
            let next = s.sample(1.0 + i as f64 * 0.016);
            assert_eq!(next.cpu, first.cpu);
            assert_eq!(next.cores, first.cores);
            assert_eq!(next.memory_used, first.memory_used);
            assert_eq!(next.processes, first.processes);
        }
    }

    #[test]
    fn jumlah_inti_selalu_setidaknya_satu() {
        let mut s = Synthetic::new(0, Duration::from_millis(16));
        assert_eq!(s.sample(0.0).cores.len(), 1);
    }

    #[test]
    fn sumber_asli_tidak_pernah_menyampel_lebih_cepat_dari_yang_jujur() {
        // Asking for 60 Hz from `sysinfo` does not give fresher numbers, it
        // gives wrong ones — the percentage is a ratio measured between two
        // refreshes.
        let s = SystemSource::new(Duration::from_millis(16));
        assert!(s.interval() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let slow = SystemSource::new(Duration::from_secs(2));
        assert_eq!(slow.interval(), Duration::from_secs(2));
    }

    #[test]
    fn sumber_asli_melaporkan_mesin_yang_masuk_akal() {
        // The one test that touches the real machine. It asserts shape rather
        // than values: a CI runner's load is nobody's business.
        let mut s = SystemSource::new(Duration::from_millis(200));
        let sample = s.sample(0.0);
        assert!(sample.cpu.is_finite());
        assert!(sample.memory_total > 0, "mesin tanpa RAM tidak ada");
        assert!(sample.memory_used <= sample.memory_total);
        assert!(sample.memory_fraction().is_finite());
        assert!(!sample.cores.is_empty(), "mesin tanpa inti tidak ada");
        assert!(sample.processes.len() <= PROCESS_LIMIT);
    }
}
