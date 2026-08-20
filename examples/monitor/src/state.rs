//! The monitor's state, and the two rules that make "render only when dirty"
//! survive contact with data that never stops arriving.
//!
//! ## Rule one: arriving is not the same as changing
//!
//! A sample lands every frame whether or not it carries news. If the
//! application writes every field of it into a signal on arrival, every frame
//! marks something dirty, and the window never sleeps — not because anything
//! moved but because something was *told*. So every write here goes through
//! [`Signal::set_if_changed`], and the process list — which changes far more
//! slowly than the CPU counter — gets a signal of its own so that a CPU-only
//! update does not rebuild sixty-four table rows.
//!
//! ## Rule two: a frame-time readout must not be driven by frames
//!
//! The obvious way to build the frame-time indicator is to read
//! [`AppRuntime::frame_stats`](silka_core::app::AppRuntime::frame_stats) at the
//! end of every frame and write it into a signal. That is a perpetual motion
//! machine: the write dirties the readout, the readout schedules a frame, the
//! frame produces a new timing, and the machine has invented a reason to redraw
//! forever. The whole "idle costs nothing" claim dies, and it dies in the one
//! widget that exists to measure it.
//!
//! So [`Monitor::record_frame`] is called from the **sampling** path, not the
//! frame path. The readout refreshes when a sample arrives, at the sample
//! cadence, and freezes when sampling stops — which is the honest thing for it
//! to display, because when sampling stops there are no frames to time.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::animation::Tick;
use silka_core::scheduler::{Dirty, FrameStats, Vsync};
use silka_core::signals::{Runtime, Signal};

use crate::sample::{ProcessRow, Sample, Snapshot};
use crate::smooth::Smoothed;

/// The two springs that follow the machine, quantised to what a reader can
/// actually see.
///
/// Quantisation is what stops the readout from rebuilding on a change of
/// 3 bytes. The spring keeps its full precision — it is the *display* that is
/// coarse, so the text stops changing several frames before the spring stops
/// moving, and neither one lies to the other.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GaugeReading {
    /// Memory in use, in bytes, rounded to the nearest 10 MB.
    pub memory_bytes: f64,
    /// Total CPU load in percent, rounded to a tenth.
    pub cpu_percent: f32,
}

impl GaugeReading {
    /// The smallest change in memory the readout can show: below this, two
    /// values render as the same string.
    const MEMORY_STEP: f64 = 10_000_000.0;

    /// Round a raw pair of spring values to what the screen can distinguish.
    pub fn quantized(memory_bytes: f64, cpu_percent: f64) -> Self {
        Self {
            memory_bytes: (memory_bytes / Self::MEMORY_STEP).round() * Self::MEMORY_STEP,
            cpu_percent: ((cpu_percent * 10.0).round() / 10.0) as f32,
        }
    }
}

/// What the frame-time indicator shows.
///
/// A plain copyable summary rather than a borrow of
/// [`FrameStats`]: it lives in a signal, and
/// a signal holding a reference into the runtime it belongs to is not a thing
/// that can exist.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameSummary {
    /// Frames drawn since the monitor opened.
    pub frames: u64,
    /// The most recent frame's total time, in milliseconds.
    pub last_ms: f32,
    /// Median frame time, in milliseconds.
    pub p50_ms: f32,
    /// 95th-percentile frame time — the number that decides whether the
    /// application feels smooth, because the eye notices the slow frames and
    /// not the fast ones.
    pub p95_ms: f32,
    /// The worst frame seen, in milliseconds.
    pub worst_ms: f32,
    /// The display's frame budget, in milliseconds; zero when unknown.
    pub budget_ms: f32,
    /// How many frames have missed that budget.
    pub over_budget: u64,
}

impl FrameSummary {
    /// Summarise the runtime's statistics against the display's budget.
    pub fn from_stats(stats: &FrameStats, vsync: Vsync) -> Self {
        let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0;
        Self {
            frames: stats.frames(),
            last_ms: stats.last().map(|t| ms(t.total())).unwrap_or(0.0),
            p50_ms: stats.p50().map(ms).unwrap_or(0.0),
            p95_ms: stats.p95().map(ms).unwrap_or(0.0),
            worst_ms: ms(stats.worst()),
            budget_ms: vsync.budget().map(ms).unwrap_or(0.0),
            over_budget: stats.over_budget(),
        }
    }

    /// True when the 95th percentile fits the display's budget.
    ///
    /// Unknown budgets count as healthy: a headless test has no display link,
    /// and colouring the indicator red because nobody answered the question
    /// would be worse than saying nothing.
    pub fn healthy(&self) -> bool {
        self.budget_ms <= 0.0 || self.p95_ms <= self.budget_ms
    }
}

/// Everything the monitor knows, and the only thing the views read.
///
/// Clonable because it is a bundle of handles: signals are copyable references
/// into the runtime, and the springs sit behind one `Rc<RefCell<…>>` because
/// the animation callback and the sampling callback both need them and neither
/// owns the other.
#[derive(Clone)]
pub struct Monitor {
    /// The rolling window and the latest reading — what the charts read.
    pub data: Signal<Rc<Snapshot>>,
    /// The process list, on its own signal so that a CPU-only sample does not
    /// rebuild the table.
    pub processes: Signal<Rc<Vec<ProcessRow>>>,
    /// The spring-smoothed headline figures.
    pub reading: Signal<GaugeReading>,
    /// The frame-time indicator's data.
    pub frame: Signal<FrameSummary>,
    /// False while the monitor is paused: no samples, no springs, no frames.
    pub running: Signal<bool>,
    springs: Rc<RefCell<Springs>>,
}

/// The springs, and nothing else.
struct Springs {
    memory: Smoothed,
    cpu: Smoothed,
    /// True until the first sample. The first reading must *jump*: the machine
    /// has been running for hours and springing its memory up from zero is a
    /// lie told with an animation.
    first: bool,
}

impl Monitor {
    /// Build the state. Called once, before the first frame.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            data: runtime.signal(Rc::new(Snapshot::new())),
            processes: runtime.signal(Rc::new(Vec::new())),
            reading: runtime.signal(GaugeReading::default()),
            frame: runtime.signal(FrameSummary::default()),
            running: runtime.signal(true),
            springs: Rc::new(RefCell::new(Springs {
                // The scale is a placeholder until the first sample says how
                // much memory the machine has; `rescale` converts rather than
                // reinterprets, so nothing jumps when the answer arrives.
                memory: Smoothed::new(1.0, 0.0),
                cpu: Smoothed::new(100.0, 0.0),
                first: true,
            })),
        }
    }

    /// Record one reading.
    ///
    /// Three separate decisions, and the middle one is the interesting one:
    ///
    /// 1. The snapshot always changes, because it always gains a point — that
    ///    is what a scrolling chart *is*.
    /// 2. The process list changes only when it changes. On an idle machine the
    ///    same sixty-four rows arrive over and over, and `set_if_changed` is
    ///    the difference between rebuilding them sixty times a second and not
    ///    at all.
    /// 3. The springs are retargeted rather than set, so a value that is
    ///    already moving carries its velocity into the new destination instead
    ///    of restarting (§3.5).
    pub fn push(&self, sample: Sample) {
        {
            let mut springs = self.springs.borrow_mut();
            springs.memory.rescale(sample.memory_total.max(1) as f64);
            if springs.first {
                springs.first = false;
                springs.memory.jump_to(sample.memory_used as f64);
                springs.cpu.jump_to(sample.cpu as f64);
            } else {
                springs.memory.set_target(sample.memory_used as f64);
                springs.cpu.set_target(sample.cpu as f64);
            }
        }

        let processes = Rc::new(sample.processes.clone());
        self.processes.set_if_changed(processes);

        let next = self.data.peek().pushed(sample);
        self.data.set(Rc::new(next));

        // The first sample jumps rather than springs, so the readout has to be
        // published here as well — otherwise nothing would move it until the
        // second sample arrived.
        self.publish_reading();
    }

    /// Push the springs' current values into the readout signal, if they have
    /// moved far enough to be visible.
    ///
    /// Returns whether anything was written — which is exactly "is there a
    /// reason to rebuild".
    fn publish_reading(&self) -> bool {
        let springs = self.springs.borrow();
        let reading = GaugeReading::quantized(springs.memory.value(), springs.cpu.value());
        drop(springs);
        self.reading.set_if_changed(reading)
    }

    /// Advance the springs by one frame.
    ///
    /// The returned [`Dirty`] is the whole contract with the scheduler:
    /// [`Dirty::ANIMATION`] while a spring is still moving, and nothing at all
    /// once they have all arrived. The moment this returns [`Dirty::NONE`] and
    /// no signal has changed, the window is allowed to stop asking for frames.
    ///
    /// [`Dirty::LAYOUT`] is never returned, for the same reason `silka-chart`
    /// never returns it: a number changing must not make the page around it
    /// re-measure sixty times a second.
    pub fn advance(&self, tick: &Tick) -> Dirty {
        let moving = {
            let mut springs = self.springs.borrow_mut();
            let dt = tick.dt();
            let motion = tick.motion();
            let memory = springs.memory.advance(dt, motion);
            let cpu = springs.cpu.advance(dt, motion);
            memory || cpu
        };
        if !moving {
            return Dirty::NONE;
        }
        // Returning `Dirty::ANIMATION` is not enough on its own, and finding
        // that out cost an afternoon. `AppRuntime::frame` **serves** the dirty
        // set it was handed, and then re-raises `ANIMATION` only if the
        // animation driver says something is still moving. The driver learns
        // that from the tick, so a spring that lives outside the render tree —
        // as these two do — has to say so here. Without this line the readout
        // advances exactly one frame and then freezes until the next input
        // event happens to wake the window.
        tick.keep_awake();
        let mut dirty = Dirty::ANIMATION;
        if self.publish_reading() {
            dirty |= Dirty::PAINT;
        }
        dirty
    }

    /// True when no spring is moving.
    pub fn is_settled(&self) -> bool {
        let springs = self.springs.borrow();
        !springs.memory.is_animating() && !springs.cpu.is_animating()
    }

    /// Finish every spring instantly — for snapshots, where "halfway through a
    /// spring" is not a state worth photographing.
    pub fn settle(&self) {
        let mut springs = self.springs.borrow_mut();
        springs.memory.settle();
        springs.cpu.settle();
        drop(springs);
        self.publish_reading();
    }

    /// Publish the runtime's frame statistics.
    ///
    /// Called from the sampling path and **never** from the frame path — see
    /// this module's header for why that distinction is the difference between
    /// an idle window and a busy one.
    pub fn record_frame(&self, stats: &FrameStats, vsync: Vsync) {
        self.frame
            .set_if_changed(FrameSummary::from_stats(stats, vsync));
    }

    /// Flip between running and paused.
    ///
    /// Pausing **finishes the springs instead of letting them coast**. A paused
    /// monitor should show the last reading it took, exactly; leaving a spring
    /// mid-flight would mean the number on screen is one the machine never
    /// reported, frozen there until sampling resumes. It also means pausing
    /// puts the window to sleep on the very next frame rather than half a
    /// second later.
    pub fn toggle_running(&self) {
        let next = !self.running.peek();
        self.running.set(next);
        if !next && !self.is_settled() {
            self.settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::ProcessRow;
    use silka_core::animation::Motion;
    use std::time::Duration;

    fn sample(at: f64, cpu: f32, used: u64, processes: Vec<ProcessRow>) -> Sample {
        Sample {
            at,
            cpu,
            cores: vec![cpu, cpu],
            memory_used: used,
            memory_total: 17_179_869_184,
            processes,
        }
    }

    fn proc(pid: u32, cpu: f32) -> ProcessRow {
        ProcessRow {
            pid,
            name: format!("p{pid}"),
            cpu,
            memory: 1_000_000 * pid as u64,
        }
    }

    fn tick() -> Tick {
        Tick::manual(Duration::from_micros(16_667), Motion::Full)
    }

    #[test]
    fn kuantisasi_menahan_perubahan_yang_tidak_terlihat() {
        let a = GaugeReading::quantized(8_000_000_000.0, 41.23);
        let b = GaugeReading::quantized(8_000_000_003.0, 41.24);
        assert_eq!(a, b, "tiga byte tidak boleh memicu bangun ulang");
        let c = GaugeReading::quantized(8_020_000_000.0, 41.23);
        assert_ne!(a, c, "20 MB jelas terlihat dan harus terbit");
    }

    #[test]
    fn ringkasan_frame_tanpa_anggaran_dianggap_sehat() {
        // A headless run has no display link, and colouring the badge red
        // because nobody answered the question is worse than saying nothing.
        let s = FrameSummary::default();
        assert!(s.healthy());
        let over = FrameSummary {
            p95_ms: 14.0,
            budget_ms: 8.3,
            ..Default::default()
        };
        assert!(!over.healthy());
    }

    #[test]
    fn sampel_pertama_melompat_dan_tidak_meminta_frame() {
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        m.push(sample(0.0, 40.0, 8_000_000_000, vec![proc(1, 5.0)]));
        assert!(
            m.is_settled(),
            "pembacaan pertama harus langsung benar, bukan tumbuh dari nol"
        );
        assert_eq!(m.advance(&tick()), Dirty::NONE);
        let r = m.reading.peek();
        assert!((r.memory_bytes - 8_000_000_000.0).abs() < 20_000_000.0);
        assert!((r.cpu_percent - 40.0).abs() < 0.2);
    }

    #[test]
    fn sampel_kedua_beranimasi_lalu_berhenti() {
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        m.push(sample(0.0, 10.0, 8_000_000_000, vec![]));
        m.push(sample(1.0, 90.0, 14_000_000_000, vec![]));
        assert!(!m.is_settled());

        // The number that matters: how many frames a spring costs after the
        // data stops. Two seconds is four times the preset's own duration.
        let mut frames = 0;
        while !m.is_settled() && frames < 120 {
            m.advance(&tick());
            frames += 1;
        }
        assert!(frames < 120, "spring tidak pernah selesai");
        assert_eq!(
            m.advance(&tick()),
            Dirty::NONE,
            "spring yang sudah diam tidak boleh minta frame lagi"
        );
        let r = m.reading.peek();
        assert!((r.memory_bytes - 14_000_000_000.0).abs() < 20_000_000.0);
    }

    #[test]
    fn daftar_proses_yang_tidak_berubah_tidak_ditulis_ulang() {
        // The rebuild the table would otherwise pay sixty times a second on a
        // machine where nothing is happening.
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        let rows = vec![proc(1, 5.0), proc(2, 1.0)];
        m.push(sample(0.0, 10.0, 8_000_000_000, rows.clone()));
        let first = m.processes.peek();

        for i in 1..30 {
            m.push(sample(i as f64, 10.0, 8_000_000_000, rows.clone()));
        }
        assert!(
            Rc::ptr_eq(&first, &m.processes.peek()),
            "daftar proses yang identik tidak boleh diganti"
        );

        // …and a list that really did change is written.
        m.push(sample(30.0, 10.0, 8_000_000_000, vec![proc(1, 99.0)]));
        assert!(!Rc::ptr_eq(&first, &m.processes.peek()));
    }

    #[test]
    fn riwayat_terus_bertambah_walau_angkanya_tetap() {
        // A scrolling chart is not "the same data" when the same value arrives
        // again: the x axis moved, so the picture moved.
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        for i in 0..10 {
            m.push(sample(i as f64, 10.0, 8_000_000_000, vec![]));
        }
        let snap = m.data.peek();
        assert_eq!(snap.sequence, 10);
        assert_eq!(snap.history.len(), 10);
    }

    #[test]
    fn indikator_frame_hanya_berubah_saat_angkanya_berubah() {
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        let stats = FrameStats::new();
        m.record_frame(&stats, Vsync::UNKNOWN);
        let first = m.frame.peek();
        m.record_frame(&stats, Vsync::UNKNOWN);
        assert_eq!(first, m.frame.peek());
    }

    #[test]
    fn jeda_dapat_dinyalakan_dan_dimatikan() {
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        assert!(m.running.peek());
        m.toggle_running();
        assert!(!m.running.peek());
        m.toggle_running();
        assert!(m.running.peek());
    }

    #[test]
    fn settle_menyelesaikan_seketika() {
        let rt = Runtime::new();
        let m = Monitor::new(&rt);
        m.push(sample(0.0, 10.0, 8_000_000_000, vec![]));
        m.push(sample(1.0, 90.0, 15_000_000_000, vec![]));
        m.settle();
        assert!(m.is_settled());
        assert_eq!(m.advance(&tick()), Dirty::NONE);
    }
}
