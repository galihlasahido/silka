//! Bukti nyata scheduler render-on-dirty (REKOMENDASI §3.5).
//!
//! Jalankan, lalu perhatikan stderr:
//!
//! ```text
//! cargo run -p silka-platform --example frame_scheduling
//! ```
//!
//! Yang seharusnya terlihat:
//!
//! 1. Baris pembuka menyebut sumber vsync — di Mac ProMotion ia berbunyi
//!    `vsync 120.0 Hz (display-link) (CADisplayLink)`, bukan 60 Hz dan bukan
//!    angka yang dikonstanta.
//! 2. Selama tiga detik pertama, latar berdenyut karena tiap frame memanggil
//!    [`silka_platform::FrameContext::request_animation_frame`]; log frame
//!    time mengalir.
//! 3. Setelah animasi selesai, **log berhenti total** — tidak ada satu pun
//!    frame yang digambar sampai window di-resize, dark mode OS berubah, atau
//!    window ditutup. Itulah inti "render hanya saat dirty".

use std::time::Duration;

use silka_paint::Scene;
use silka_platform::{window, PlatformError};

/// Berapa lama denyut berjalan sebelum window kembali benar-benar diam.
const DURASI_ANIMASI: Duration = Duration::from_secs(3);

fn main() -> Result<(), PlatformError> {
    window("silka — frame scheduling")
        .size(720.0, 480.0)
        .on_frame(|frame| {
            let t = frame.elapsed();
            if t >= DURASI_ANIMASI {
                // Tidak meminta frame lagi → window tidur.
                return Scene::new(frame.theme().color.background);
            }

            // Selama masih beranimasi, tiap frame memesan penerusnya. Nanti
            // pemanggil ini adalah spring yang belum mencapai target (§3.5).
            frame.request_animation_frame();

            let fase = (t.as_secs_f32() * 2.0).sin() * 0.5 + 0.5;
            Scene::new(
                frame
                    .theme()
                    .color
                    .background
                    .lerp(frame.theme().color.accent, fase * 0.35),
            )
        })
        // Ringkasan tiap 30 frame supaya denyut singkat pun kelihatan.
        .frame_log_every(30)
        .run()
}
