//! What the monitor knows about the machine, and how it remembers it.
//!
//! Everything in this module is **pure data plus pure functions**. Nothing here
//! reads a clock, opens a file, or asks the operating system anything — that is
//! [`crate::source`]'s job. The split is not tidiness for its own sake: the
//! three claims this example exists to prove are all claims about what happens
//! *after* a number changes, and a test can only make a number change on
//! command if producing it is somebody else's problem.
//!
//! The shapes are deliberately boring. A [`Sample`] is one reading. A
//! [`History`] is the last few hundred of them in a ring. A [`Snapshot`] is the
//! pair, and it is what one signal carries.

use std::collections::VecDeque;

/// How many readings the scrolling charts remember.
///
/// At the default one-second cadence that is four minutes of history; at the
/// 60 Hz cadence the `--hz` flag can ask for, it is four seconds. Both are
/// intentional: the same buffer has to look right whether it fills in a minute
/// or in a heartbeat, because the chart underneath it does not get to know
/// which one is happening.
pub const HISTORY: usize = 240;

/// One process, as the table shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    /// The process id.
    pub pid: u32,
    /// The executable's name.
    pub name: String,
    /// Share of one core, in percent. Above 100 on a busy multi-threaded
    /// process, which is what the platform reports and not something to hide.
    pub cpu: f32,
    /// Resident memory, in bytes.
    pub memory: u64,
}

/// One reading of the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Seconds since the monitor started. This is the charts' x axis, and it is
    /// **not** wall-clock time: a monitor that plots `Instant::now()` produces
    /// a different picture on every machine and an untestable one everywhere.
    pub at: f64,
    /// Total CPU load across every core, 0..=100.
    pub cpu: f32,
    /// Per-core load, 0..=100, in the platform's core order.
    pub cores: Vec<f32>,
    /// Physical memory in use, in bytes. Billions of them — which is the whole
    /// reason [`crate::smooth`] exists.
    pub memory_used: u64,
    /// Physical memory installed, in bytes.
    pub memory_total: u64,
    /// The processes worth showing, unsorted.
    pub processes: Vec<ProcessRow>,
}

impl Sample {
    /// The fraction of memory in use, 0..=1.
    ///
    /// Guarded against a total of zero, which is what a sandboxed or
    /// unsupported platform reports and which would otherwise put a `NaN`
    /// straight into a progress bar's width.
    pub fn memory_fraction(&self) -> f32 {
        if self.memory_total == 0 {
            0.0
        } else {
            (self.memory_used as f64 / self.memory_total as f64).clamp(0.0, 1.0) as f32
        }
    }

    /// Memory still available, in bytes.
    pub fn memory_free(&self) -> u64 {
        self.memory_total.saturating_sub(self.memory_used)
    }

    /// The busiest core's load, or zero on a machine that reports none.
    pub fn busiest_core(&self) -> f32 {
        self.cores.iter().copied().fold(0.0f32, f32::max)
    }
}

/// One point on the scrolling charts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Seconds since the monitor started.
    pub at: f64,
    /// Total CPU load, 0..=100.
    pub cpu: f64,
    /// Memory in use, in bytes.
    pub memory: f64,
}

/// The rolling window the charts draw.
///
/// A ring rather than a growing `Vec`: a monitor left open overnight at 60 Hz
/// would otherwise accumulate two million points, and the honest failure mode
/// of that is not a slow chart but a machine the monitor itself is the biggest
/// consumer on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    capacity: usize,
    points: VecDeque<Point>,
    cores: Vec<VecDeque<f64>>,
}

impl History {
    /// An empty history that will remember `capacity` readings.
    ///
    /// A capacity of zero is promoted to one: a ring that cannot hold anything
    /// is not a useful degenerate case, it is a division by zero waiting for a
    /// chart to find it.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            points: VecDeque::with_capacity(capacity),
            cores: Vec::new(),
        }
    }

    /// Record one reading, dropping the oldest when the ring is full.
    ///
    /// A change in core count — a virtual machine gaining a CPU, or simply the
    /// first sample arriving — resets the per-core rings rather than trying to
    /// line the old ones up with the new ones. Guessing which core used to be
    /// which is the kind of cleverness that produces a chart nobody can trust.
    pub fn push(&mut self, sample: &Sample) {
        if self.cores.len() != sample.cores.len() {
            self.cores = vec![VecDeque::with_capacity(self.capacity); sample.cores.len()];
        }
        for (ring, value) in self.cores.iter_mut().zip(&sample.cores) {
            if ring.len() == self.capacity {
                ring.pop_front();
            }
            ring.push_back(*value as f64);
        }
        if self.points.len() == self.capacity {
            self.points.pop_front();
        }
        self.points.push_back(Point {
            at: sample.at,
            cpu: sample.cpu as f64,
            memory: sample.memory_used as f64,
        });
    }

    /// The readings, oldest first.
    pub fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.points.iter().copied()
    }

    /// How many readings are remembered.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// True before the first reading arrives.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The largest number of readings this history will hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many cores this history is tracking.
    pub fn core_count(&self) -> usize {
        self.cores.len()
    }

    /// One core's readings, oldest first. Empty for a core that does not exist.
    pub fn core(&self, index: usize) -> Vec<f64> {
        self.cores
            .get(index)
            .map(|ring| ring.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The most recent reading.
    pub fn latest(&self) -> Option<Point> {
        self.points.back().copied()
    }

    /// The highest memory figure in the window, in bytes.
    ///
    /// Shown beside the current figure, because a monitor is usually opened
    /// after the interesting moment has passed: "it peaked at 15 GB" is the
    /// answer, and the current reading is not.
    pub fn peak_memory(&self) -> f64 {
        self.points.iter().map(|p| p.memory).fold(0.0, f64::max)
    }
}

/// Everything one signal carries: the ring plus the reading that just landed.
///
/// The two travel together because they change together. Splitting them would
/// mean two signal writes per sample, two dirty scopes, and a rebuild count
/// that no longer says anything useful about how much work a sample costs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// The rolling window.
    pub history: History,
    /// The reading that produced the last point, if any has arrived.
    pub latest: Option<Sample>,
    /// How many readings have been recorded since the monitor started.
    ///
    /// Monotonic, and never reset by a pause: it is what a test uses to say
    /// "the data really did stop" without having to compare two histories.
    pub sequence: u64,
}

impl Snapshot {
    /// An empty snapshot with the default window size.
    pub fn new() -> Self {
        Self {
            history: History::new(HISTORY),
            latest: None,
            sequence: 0,
        }
    }

    /// This snapshot with one more reading in it.
    ///
    /// Returns a new value rather than mutating, because what it feeds is a
    /// signal: the whole point of the reactive model is that a *new value*
    /// arrives and the components that read it rebuild (§2.5).
    pub fn pushed(&self, sample: Sample) -> Snapshot {
        let mut next = self.clone();
        next.history.push(&sample);
        next.latest = Some(sample);
        next.sequence += 1;
        next
    }
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

/// Which column the process table is ordered by.
///
/// Its numeric values are the table's column indices, so the widget's
/// `SortBy { column, .. }` maps onto it without a lookup table that could drift
/// out of step with [`crate::processes::columns`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessSort {
    /// By process id.
    Pid = 0,
    /// By executable name, case-insensitively.
    Name = 1,
    /// By CPU share — the default, because that is the question a monitor is
    /// opened to answer.
    #[default]
    Cpu = 2,
    /// By resident memory.
    Memory = 3,
}

impl ProcessSort {
    /// The sort a table column index selects, if that column sorts at all.
    pub fn from_column(column: usize) -> Option<Self> {
        match column {
            0 => Some(ProcessSort::Pid),
            1 => Some(ProcessSort::Name),
            2 => Some(ProcessSort::Cpu),
            3 => Some(ProcessSort::Memory),
            _ => None,
        }
    }
}

/// Order `rows` in place.
///
/// Descending is what a monitor wants for the two usage columns and wrong for
/// the two identity columns, so the caller passes the direction rather than
/// this deciding — but every ordering ends with `pid` as the tie-break, which
/// is what stops rows from swapping places on their own every time two
/// processes happen to report the same figure. Without it a table sorted by
/// CPU, on an idle machine where forty processes all report 0.0, would shuffle
/// on every sample: sixty visible reorderings a second of data that never
/// changed.
pub fn sort_rows(rows: &mut [ProcessRow], sort: ProcessSort, descending: bool) {
    rows.sort_by(|a, b| {
        let ord = match sort {
            ProcessSort::Pid => a.pid.cmp(&b.pid),
            ProcessSort::Name => a
                .name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.pid.cmp(&b.pid)),
            ProcessSort::Cpu => a.cpu.total_cmp(&b.cpu).then(a.pid.cmp(&b.pid)),
            ProcessSort::Memory => a.memory.cmp(&b.memory).then(a.pid.cmp(&b.pid)),
        };
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at: f64, cpu: f32, cores: &[f32], used: u64) -> Sample {
        Sample {
            at,
            cpu,
            cores: cores.to_vec(),
            memory_used: used,
            memory_total: 17_179_869_184,
            processes: Vec::new(),
        }
    }

    #[test]
    fn ring_membuang_yang_terlama_bukan_yang_terbaru() {
        let mut h = History::new(3);
        for i in 0..5 {
            h.push(&sample(i as f64, i as f32, &[i as f32], 1_000 + i as u64));
        }
        assert_eq!(h.len(), 3);
        let at: Vec<f64> = h.points().map(|p| p.at).collect();
        assert_eq!(at, vec![2.0, 3.0, 4.0], "yang tersisa harus yang terbaru");
        assert_eq!(h.core(0), vec![2.0, 3.0, 4.0]);
        assert_eq!(h.latest().unwrap().at, 4.0);
    }

    #[test]
    fn kapasitas_nol_tidak_pernah_terjadi() {
        // A capacity of zero would make `pop_front` on an empty ring the
        // steady state, and every chart downstream would draw nothing forever.
        let mut h = History::new(0);
        assert_eq!(h.capacity(), 1);
        h.push(&sample(1.0, 5.0, &[5.0], 10));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn jumlah_inti_berubah_mengatur_ulang_cincin_per_inti() {
        let mut h = History::new(4);
        h.push(&sample(0.0, 10.0, &[10.0, 20.0], 1));
        assert_eq!(h.core_count(), 2);
        h.push(&sample(1.0, 30.0, &[10.0, 20.0, 30.0, 40.0], 1));
        assert_eq!(h.core_count(), 4);
        // Reset rather than realigned: only the new reading survives, and no
        // core inherits a history that was never its own.
        assert_eq!(h.core(0), vec![10.0]);
        assert_eq!(h.core(3), vec![40.0]);
        // …and a core that does not exist answers with nothing rather than
        // panicking, because a chart asking for core 8 on a 4-core machine is
        // a layout question, not a bug.
        assert!(h.core(8).is_empty());
    }

    #[test]
    fn memori_nol_total_tidak_menghasilkan_nan() {
        let mut s = sample(0.0, 0.0, &[], 0);
        s.memory_total = 0;
        assert_eq!(s.memory_fraction(), 0.0);
        assert!(s.memory_fraction().is_finite());
        assert_eq!(s.memory_free(), 0);
    }

    #[test]
    fn pecahan_memori_terkunci_di_nol_sampai_satu() {
        // A platform that reports more used than installed exists — a Linux
        // container reading the host's totals is the usual way — and a
        // progress bar wider than its track is not an acceptable answer.
        let mut s = sample(0.0, 0.0, &[], 20_000_000_000);
        s.memory_total = 17_179_869_184;
        assert_eq!(s.memory_fraction(), 1.0);
    }

    #[test]
    fn snapshot_menambah_urutan_dan_tidak_mengubah_yang_lama() {
        let a = Snapshot::new();
        let b = a.pushed(sample(0.0, 1.0, &[1.0], 100));
        let c = b.pushed(sample(1.0, 2.0, &[2.0], 200));
        assert_eq!(a.sequence, 0);
        assert!(
            a.history.is_empty(),
            "snapshot lama tidak boleh ikut berubah"
        );
        assert_eq!(b.sequence, 1);
        assert_eq!(c.sequence, 2);
        assert_eq!(c.history.len(), 2);
        assert_eq!(c.latest.as_ref().unwrap().cpu, 2.0);
    }

    fn rows() -> Vec<ProcessRow> {
        vec![
            ProcessRow {
                pid: 7,
                name: "zsh".into(),
                cpu: 0.0,
                memory: 4_000_000,
            },
            ProcessRow {
                pid: 2,
                name: "Xcode".into(),
                cpu: 0.0,
                memory: 9_000_000_000,
            },
            ProcessRow {
                pid: 9,
                name: "rustc".into(),
                cpu: 312.5,
                memory: 1_200_000_000,
            },
        ]
    }

    #[test]
    fn urutan_bawaan_adalah_pemakaian_cpu_terbesar_dulu() {
        let mut r = rows();
        sort_rows(&mut r, ProcessSort::Cpu, true);
        assert_eq!(r[0].name, "rustc");
        // The two zeroes tie, so pid decides — and descending reverses that
        // too, which is fine: what matters is that it is *decided*.
        assert_eq!(r[1].pid, 7);
        assert_eq!(r[2].pid, 2);
    }

    #[test]
    fn nilai_seri_selalu_dipatahkan_oleh_pid_supaya_tabel_tidak_bergoyang() {
        // The failure this guards: forty idle processes all reporting 0.0, and
        // an unstable comparator reshuffling them sixty times a second.
        let mut a = rows();
        let mut b = rows();
        b.reverse();
        sort_rows(&mut a, ProcessSort::Cpu, true);
        sort_rows(&mut b, ProcessSort::Cpu, true);
        let pids_a: Vec<u32> = a.iter().map(|r| r.pid).collect();
        let pids_b: Vec<u32> = b.iter().map(|r| r.pid).collect();
        assert_eq!(pids_a, pids_b, "urutan bergantung pada urutan masukan");
    }

    #[test]
    fn urut_nama_mengabaikan_besar_kecil_huruf() {
        let mut r = rows();
        sort_rows(&mut r, ProcessSort::Name, false);
        let names: Vec<&str> = r.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["rustc", "Xcode", "zsh"]);
    }

    #[test]
    fn urut_memori_menaik_dan_menurun_saling_membalik() {
        let mut naik = rows();
        let mut turun = rows();
        sort_rows(&mut naik, ProcessSort::Memory, false);
        sort_rows(&mut turun, ProcessSort::Memory, true);
        naik.reverse();
        assert_eq!(naik, turun);
    }

    #[test]
    fn kolom_tabel_dan_kunci_urut_tidak_bisa_bergeser() {
        // The discriminants **are** the table's column indices, and that is
        // the whole mapping: if the two ever drift apart, clicking a heading
        // silently sorts by something else.
        for (column, sort) in [
            (0, ProcessSort::Pid),
            (1, ProcessSort::Name),
            (2, ProcessSort::Cpu),
            (3, ProcessSort::Memory),
        ] {
            assert_eq!(ProcessSort::from_column(column), Some(sort));
            assert_eq!(sort as usize, column);
        }
        assert_eq!(ProcessSort::from_column(99), None);
        assert_eq!(ProcessSort::default(), ProcessSort::Cpu);
    }
}
