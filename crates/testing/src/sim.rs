//! Driving a real app with synthetic input.
//!
//! The point of simulated input is not to save typing — it is that a UI bug
//! almost never lives in a widget's `build`. It lives in the seam between a
//! press, a rebuild, a layout pass and the next paint. A test that calls a
//! widget's methods directly never crosses that seam; a test that presses at a
//! coordinate crosses all of it, through the very same
//! [`InputRouter`](silka_core::input::InputRouter) a window uses.
//!
//! Three decisions make the harness trustworthy:
//!
//! 1. **Two clocks, both fake.** Event timestamps come from a counter, so the
//!    velocity tracker and the double-click detector see plausible, repeatable
//!    timing. Animation frames come from a second counter, so a spring
//!    genuinely advances 8.3 ms per frame instead of the two microseconds a
//!    tight loop of `Instant::now()` would report — the trap §3.5 warns about,
//!    and the one that makes a settle-loop spin forever.
//! 2. **Aim by accessible name, not by coordinate.** [`Simulator::click_label`]
//!    asks the accessibility tree where "Simpan" is and presses its centre. The
//!    coordinates then come from the real layout, the test survives a padding
//!    change, and it fails when the a11y contract (§3.8) is broken — which is
//!    exactly when it should.
//! 3. **Settling is bounded.** [`Simulator::settle`] pumps frames until the app
//!    is idle and panics if it never is. A UI that cannot come to rest is a bug
//!    (it keeps the GPU awake forever), so the harness reports it instead of
//!    hanging the suite.

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::animation::Tick;
use silka_core::app::{AppRuntime, BuildCtx, FrameReport, ScaleFactor};
use silka_core::input::{
    Buttons, Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerPhase, Response, ScrollDelta, ScrollEvent, ScrollPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::RenderTree;
use silka_core::view::View;
use silka_paint::{Point, Rect, Scene, Size};
use silka_platform::headless_app;
use silka_theme::Theme;

use crate::headless::Headless;
use crate::image::Image;
use crate::matrix::Case;

/// The default viewport a simulated app opens at.
pub const DEFAULT_SIZE: Size = Size::new(800.0, 600.0);
/// The default device pixel ratio: a Retina screen, because that is the path
/// that actually rounds — a 1.0 test never exercises the scale factor at all.
pub const DEFAULT_SCALE: f64 = 2.0;
/// One frame at 120 Hz, the rate this framework promises (§3.5).
pub const DEFAULT_FRAME_STEP: Duration = Duration::from_micros(8_333);
/// How far the event clock moves between two synthetic events.
pub const DEFAULT_EVENT_GAP: Duration = Duration::from_millis(8);
/// How many frames [`Simulator::settle`] will pump before declaring the app
/// unable to come to rest.
pub const DEFAULT_SETTLE_LIMIT: usize = 1_200;

/// The animation driver a simulator runs each frame — the same signature
/// `silka_widgets::advance` has, so an app's real driver plugs straight in.
pub type Animator = Box<dyn FnMut(&mut RenderTree, &Tick) -> Dirty>;

/// An [`AppRuntime`] plus a hand, a keyboard and two fake clocks.
pub struct Simulator {
    ui: AppRuntime,
    size: Size,
    scale: f64,
    /// The input timeline, as widgets see it (time since the window opened).
    clock: Duration,
    /// How far `clock` moves per synthetic event.
    gap: Duration,
    /// How far the animation clock moves per frame.
    step: Duration,
    origin: Instant,
    elapsed: Duration,
    position: Point,
    buttons: Buttons,
    modifiers: Modifiers,
    animator: Animator,
    settle_limit: usize,
    last: Response,
}

impl Simulator {
    /// Wrap an existing runtime.
    pub fn new(ui: AppRuntime) -> Self {
        let mut sim = Self {
            ui,
            size: DEFAULT_SIZE,
            scale: DEFAULT_SCALE,
            clock: Duration::ZERO,
            gap: DEFAULT_EVENT_GAP,
            step: DEFAULT_FRAME_STEP,
            origin: Instant::now(),
            elapsed: Duration::ZERO,
            position: Point::new(-1.0, -1.0),
            buttons: Buttons::NONE,
            modifiers: Modifiers::NONE,
            animator: Box::new(|_, _| Dirty::NONE),
            settle_limit: DEFAULT_SETTLE_LIMIT,
            last: Response::default(),
        };
        sim.apply_size();
        sim.apply_scale();
        sim
    }

    /// Build a headless app for `theme` — assembled by
    /// [`silka_platform::headless_app`], which is the same assembly `run_app`
    /// performs, so a test never drifts onto a runtime no user runs.
    pub fn app(theme: Theme, build: impl Fn(&BuildCtx) -> View + 'static) -> Self {
        Self::new(headless_app(theme, build))
    }

    /// [`Simulator::app`] for one cell of the preset matrix.
    pub fn case(case: Case, build: impl Fn(&BuildCtx) -> View + 'static) -> Self {
        Self::app(case.theme(), build)
    }

    // -- configuration ------------------------------------------------------

    /// Set the viewport in logical points.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size::new(width, height);
        self.apply_size();
        self
    }

    /// Set the device pixel ratio.
    pub fn scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self.apply_scale();
        self
    }

    /// Set how far the animation clock moves per [`Simulator::frame`].
    pub fn frame_step(mut self, step: Duration) -> Self {
        self.step = step;
        self
    }

    /// Set how far the event clock moves per synthetic event.
    pub fn event_gap(mut self, gap: Duration) -> Self {
        self.gap = gap;
        self
    }

    /// Set the frame budget [`Simulator::settle`] is allowed.
    pub fn settle_limit(mut self, frames: usize) -> Self {
        self.settle_limit = frames;
        self
    }

    /// Install the animation driver — `silka_widgets::advance` for an app made
    /// of this framework's widgets. Without one, springs never move.
    pub fn animator(mut self, f: impl FnMut(&mut RenderTree, &Tick) -> Dirty + 'static) -> Self {
        self.animator = Box::new(f);
        self
    }

    /// Hold these modifiers for every event from now on.
    pub fn hold_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    fn apply_size(&mut self) {
        self.ui.resize(self.size);
    }

    fn apply_scale(&mut self) {
        if let Some(signal) = self.ui.env::<Signal<ScaleFactor>>() {
            signal.set(ScaleFactor(self.scale as f32));
        }
    }

    // -- access -------------------------------------------------------------

    /// The runtime being driven.
    pub fn ui(&self) -> &AppRuntime {
        &self.ui
    }

    /// The runtime, mutably — for the rare case the harness has no verb for.
    pub fn ui_mut(&mut self) -> &mut AppRuntime {
        &mut self.ui
    }

    /// Give the runtime back.
    pub fn into_ui(self) -> AppRuntime {
        self.ui
    }

    /// The scene the last frame painted.
    pub fn scene(&self) -> &Scene {
        self.ui.scene()
    }

    /// The render tree.
    pub fn tree(&self) -> &RenderTree {
        self.ui.tree()
    }

    /// The viewport in logical points.
    pub fn viewport(&self) -> Size {
        self.size
    }

    /// The device pixel ratio.
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// The result of the most recent dispatch.
    pub fn last_response(&self) -> &Response {
        &self.last
    }

    /// The current pointer position.
    pub fn pointer_position(&self) -> Point {
        self.position
    }

    /// The event clock's current value.
    pub fn now(&self) -> Duration {
        self.clock
    }

    // -- frames -------------------------------------------------------------

    /// Advance one frame: animation first, then rebuild → layout → paint.
    ///
    /// The order matters and is the same one the shell uses — a value that
    /// moved this tick must be *this* frame's value, not the next one's.
    pub fn frame(&mut self) -> FrameReport {
        self.elapsed += self.step;
        let now = self.origin + self.elapsed;
        let Self { ui, animator, .. } = self;
        ui.animate_at(now, |tree, tick| animator(tree, tick));
        self.ui.frame()
    }

    /// Advance `count` frames.
    pub fn advance(&mut self, count: usize) -> &mut Self {
        for _ in 0..count {
            self.frame();
        }
        self
    }

    /// Pump frames until nothing is animating and nothing is dirty.
    ///
    /// Returns how many frames that took. Panics past
    /// [`Simulator::settle_limit`]: an app that never settles is a bug worth a
    /// loud failure, not a hung test run.
    pub fn settle(&mut self) -> usize {
        let mut frames = 0;
        // Always run at least one frame: before the first one the tree has not
        // even been built, and "idle" would be a lie.
        loop {
            self.frame();
            frames += 1;
            if !self.ui.is_animating() && self.ui.is_idle() {
                return frames;
            }
            if frames >= self.settle_limit {
                panic!(
                    "aplikasi tidak pernah tenang setelah {frames} frame \
                     (masih animasi: {}, masih kotor: {:?})",
                    self.ui.is_animating(),
                    self.ui.pending()
                );
            }
        }
    }

    /// Move the event clock without drawing anything — how a test separates
    /// two clicks so they are not read as a double click.
    pub fn wait(&mut self, duration: Duration) -> &mut Self {
        self.clock += duration;
        self
    }

    // -- pointer ------------------------------------------------------------

    /// Move the pointer to a point (no button).
    pub fn move_to(&mut self, position: Point) -> &mut Self {
        self.position = position;
        self.pointer(PointerPhase::Move, None);
        self
    }

    /// A synonym for [`Simulator::move_to`] that reads better in hover tests.
    pub fn hover(&mut self, position: Point) -> &mut Self {
        self.move_to(position)
    }

    /// Press the primary button where the pointer already is.
    pub fn press(&mut self) -> &mut Self {
        self.press_button(PointerButton::Primary)
    }

    /// Press a specific button.
    pub fn press_button(&mut self, button: PointerButton) -> &mut Self {
        self.buttons.insert(button);
        self.pointer(PointerPhase::Down, Some(button));
        self
    }

    /// Release the primary button.
    pub fn release(&mut self) -> &mut Self {
        self.release_button(PointerButton::Primary)
    }

    /// Release a specific button.
    pub fn release_button(&mut self, button: PointerButton) -> &mut Self {
        self.buttons.remove(button);
        self.pointer(PointerPhase::Up, Some(button));
        self
    }

    /// Cancel the gesture the way the OS does when it steals the pointer.
    ///
    /// Worth its own verb: a widget that treats cancel as a release fires a
    /// click that never happened, and that bug is invisible to any test that
    /// only ever presses and releases.
    pub fn cancel(&mut self) -> &mut Self {
        self.buttons.clear();
        self.pointer(PointerPhase::Cancel, None);
        self
    }

    /// Move, press, release — one click.
    pub fn click(&mut self, position: Point) -> &mut Self {
        self.move_to(position);
        self.press();
        self.release()
    }

    /// Two clicks close enough in time to be read as a double click.
    pub fn double_click(&mut self, position: Point) -> &mut Self {
        self.click(position);
        self.click(position)
    }

    /// Press at `from`, drag through `steps` intermediate points, release at
    /// `to`.
    ///
    /// The intermediate moves are what make this a drag rather than a teleport:
    /// the velocity tracker needs a series of timed samples before a fling can
    /// hand off to a spring (§3.5).
    pub fn drag(&mut self, from: Point, to: Point, steps: usize) -> &mut Self {
        self.move_to(from);
        self.press();
        let steps = steps.max(1);
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            self.move_to(Point::new(
                from.x + (to.x - from.x) * t,
                from.y + (to.y - from.y) * t,
            ));
        }
        self.release()
    }

    /// Scroll at the current pointer position.
    pub fn scroll(&mut self, delta: ScrollDelta, phase: ScrollPhase) -> &mut Self {
        self.clock += self.gap;
        let event = ScrollEvent {
            id: silka_core::input::PointerId::MOUSE,
            position: self.position,
            delta,
            phase,
            modifiers: self.modifiers,
            time: self.clock,
        };
        self.last = self.ui.dispatch(&Event::Scroll(event));
        self
    }

    /// A mouse-wheel scroll in logical points.
    pub fn scroll_by(&mut self, x: f32, y: f32) -> &mut Self {
        self.scroll(ScrollDelta::Points { x, y }, ScrollPhase::Wheel)
    }

    fn pointer(&mut self, phase: PointerPhase, button: Option<PointerButton>) {
        self.clock += self.gap;
        let mut event = PointerEvent::new(phase, self.position, self.clock);
        event.button = button;
        event.buttons = self.buttons;
        event.modifiers = self.modifiers;
        self.last = self.ui.dispatch(&Event::Pointer(event));
    }

    // -- keyboard -----------------------------------------------------------

    /// Press and release a named key.
    pub fn key(&mut self, key: NamedKey) -> &mut Self {
        self.key_code(KeyCode::Named(key))
    }

    /// Press and release any key code.
    pub fn key_code(&mut self, code: KeyCode) -> &mut Self {
        self.press_key(code.clone());
        self.release_key(code)
    }

    /// Press a key and hold it.
    pub fn press_key(&mut self, code: KeyCode) -> &mut Self {
        self.clock += self.gap;
        let text = match &code {
            KeyCode::Character(c) => Some(c.to_string()),
            KeyCode::Named(NamedKey::Space) => Some(" ".to_string()),
            _ => None,
        };
        let mut event = KeyEvent::pressed(code, self.clock).modifiers(self.modifiers);
        event.text = text;
        self.last = self.ui.dispatch(&Event::Key(event));
        self
    }

    /// Release a key.
    pub fn release_key(&mut self, code: KeyCode) -> &mut Self {
        self.clock += self.gap;
        let event = KeyEvent::released(code, self.clock).modifiers(self.modifiers);
        self.last = self.ui.dispatch(&Event::Key(event));
        self
    }

    /// Type a string, one character key at a time, exactly as a keyboard does.
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        for ch in text.chars() {
            let code = if ch == ' ' {
                KeyCode::Named(NamedKey::Space)
            } else {
                KeyCode::Character(ch)
            };
            self.key_code(code);
        }
        self
    }

    /// Change the modifiers held from now on.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) -> &mut Self {
        self.modifiers = modifiers;
        self
    }

    /// Move focus forward with Tab.
    pub fn tab(&mut self) -> &mut Self {
        self.key(NamedKey::Tab)
    }

    // -- aiming by accessible name ------------------------------------------

    /// The accessibility tree as of the last frame.
    pub fn access_tree(&self) -> AccessTree {
        self.ui.access_tree()
    }

    /// The box of the node with this accessible name.
    pub fn bounds_of(&self, label: &str) -> Option<Rect> {
        self.access_tree().find_label(label).map(|e| e.bounds)
    }

    /// The centre of the node with this accessible name.
    pub fn center_of(&self, label: &str) -> Option<Point> {
        self.bounds_of(label).map(center)
    }

    /// [`Simulator::center_of`], panicking with the whole a11y tree when the
    /// name is not there — the dump is what turns "None" into a diagnosis.
    pub fn require_center(&self, label: &str) -> Point {
        self.center_of(label).unwrap_or_else(|| {
            panic!(
                "tidak ada node beraksesibilitas berlabel {label:?}\n{}",
                self.access_tree().dump()
            )
        })
    }

    /// Click the node with this accessible name.
    pub fn click_label(&mut self, label: &str) -> &mut Self {
        let point = self.require_center(label);
        self.click(point)
    }

    /// Hover the node with this accessible name.
    pub fn hover_label(&mut self, label: &str) -> &mut Self {
        let point = self.require_center(label);
        self.hover(point)
    }

    /// True when something in the a11y tree carries this name.
    pub fn has_label(&self, label: &str) -> bool {
        self.access_tree().find_label(label).is_some()
    }

    /// The a11y tree as text — paste it into a failure message.
    pub fn access_dump(&self) -> String {
        self.access_tree().dump()
    }

    // -- capture ------------------------------------------------------------

    /// Render the current scene headlessly, without text.
    pub fn capture(&self, gpu: &mut Headless) -> Image {
        gpu.capture(self.ui.scene(), self.size, self.scale)
    }

    /// Render the current scene headlessly, with the caller's glyph source.
    pub fn capture_with_glyphs(
        &self,
        gpu: &mut Headless,
        glyphs: &mut dyn silka_paint::GlyphSource,
    ) -> Image {
        gpu.capture_with_glyphs(self.ui.scene(), self.size, self.scale, glyphs)
    }
}

impl core::fmt::Debug for Simulator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Simulator")
            .field("size", &self.size)
            .field("scale", &self.scale)
            .field("clock", &self.clock)
            .field("position", &self.position)
            .finish()
    }
}

/// The centre of a rect.
fn center(rect: Rect) -> Point {
    Point::new(
        rect.origin.x + rect.size.width * 0.5,
        rect.origin.y + rect.size.height * 0.5,
    )
}

#[cfg(test)]
mod tests {
    use silka_core::view::fixed;
    use silka_theme::{Appearance, Preset};

    use super::*;

    fn sim() -> Simulator {
        Simulator::app(Theme::new(Preset::Cupertino, Appearance::Light), |_cx| {
            fixed(120.0, 40.0).into()
        })
        .size(200.0, 120.0)
    }

    #[test]
    fn ukuran_dan_skala_terpasang_di_runtime() {
        let mut s = sim().scale(3.0);
        s.frame();
        assert_eq!(s.viewport(), Size::new(200.0, 120.0));
        assert_eq!(
            s.ui()
                .env::<Signal<ScaleFactor>>()
                .expect("headless_app menitipkan scale factor")
                .get()
                .get(),
            3.0
        );
    }

    #[test]
    fn jam_kejadian_maju_setiap_kejadian() {
        let mut s = sim();
        s.frame();
        assert_eq!(s.now(), Duration::ZERO);
        s.click(Point::new(10.0, 10.0));
        // move + down + up
        assert_eq!(s.now(), DEFAULT_EVENT_GAP * 3);
        s.wait(Duration::from_millis(500));
        assert_eq!(s.now(), DEFAULT_EVENT_GAP * 3 + Duration::from_millis(500));
    }

    #[test]
    fn tombol_yang_ditekan_terbawa_di_kejadian() {
        let mut s = sim();
        s.frame();
        s.move_to(Point::new(5.0, 5.0));
        s.press();
        assert!(s.buttons.contains(PointerButton::Primary));
        s.release();
        assert!(s.buttons.is_empty());
        s.press();
        s.cancel();
        assert!(s.buttons.is_empty(), "cancel melepas semua tombol");
    }

    #[test]
    fn drag_menghasilkan_langkah_antara_bukan_teleportasi() {
        let mut s = sim();
        s.frame();
        let sebelum = s.now();
        s.drag(Point::new(0.0, 0.0), Point::new(40.0, 0.0), 4);
        // 1 move + 1 down + 4 moves + 1 up = 7 events.
        assert_eq!(s.now() - sebelum, DEFAULT_EVENT_GAP * 7);
        assert_eq!(s.pointer_position(), Point::new(40.0, 0.0));
    }

    #[test]
    fn mengetik_mengirim_teks_pada_kejadian_tombol() {
        // A key event without `text` is a key a text field cannot insert; the
        // harness must not make that mistake on the app's behalf.
        let mut s = Simulator::app(Theme::default(), |_cx| fixed(10.0, 10.0).into());
        s.frame();
        s.type_text("ab");
        assert_eq!(s.now(), DEFAULT_EVENT_GAP * 4, "dua tombol, tekan + lepas");
    }

    #[test]
    fn settle_berhenti_saat_aplikasi_tenang() {
        let mut s = sim();
        let frames = s.settle();
        assert_eq!(frames, 1, "pohon statis tenang setelah satu frame");
        assert!(s.ui().is_idle());
    }

    #[test]
    #[should_panic(expected = "tidak pernah tenang")]
    fn settle_menyerah_dan_melapor_bukan_menggantung() {
        let mut s = sim().settle_limit(5).animator(|_tree, tick| {
            // An animation that never finishes: exactly the bug the limit is
            // there to name.
            tick.keep_awake();
            Dirty::ANIMATION
        });
        s.settle();
    }

    #[test]
    fn membidik_lewat_nama_aksesibilitas_gagal_dengan_dump() {
        let s = sim();
        let hasil = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.require_center("tidak ada")
        }));
        let e = hasil.unwrap_err();
        let pesan = e
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "?".into());
        assert!(pesan.contains("tidak ada"), "{pesan}");
    }

    #[test]
    fn frame_memajukan_jam_animasi_dengan_langkah_tetap() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let dt: Rc<RefCell<Vec<Duration>>> = Rc::default();
        let rekam = dt.clone();
        let mut s = sim().animator(move |_tree, tick| {
            rekam.borrow_mut().push(tick.dt());
            // Something must stay in motion for the driver to keep its clock:
            // once everything settles it deliberately forgets it, which is how
            // an idle window costs nothing (§3.5).
            tick.keep_awake();
            Dirty::ANIMATION
        });
        s.advance(3);
        let dt = dt.borrow();
        assert_eq!(dt.len(), 3);
        assert_eq!(
            dt[0],
            Duration::ZERO,
            "frame pertama tidak punya pembanding"
        );
        // The rest must be the fixed step, not whatever the wall clock did in
        // a tight loop — that is the entire reason the clock is injected.
        assert_eq!(dt[1], DEFAULT_FRAME_STEP);
        assert_eq!(dt[2], DEFAULT_FRAME_STEP);
    }
}
