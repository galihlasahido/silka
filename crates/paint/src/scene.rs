//! Scene: satu frame yang siap digambar, dinyatakan sebagai daftar perintah.
//!
//! `Scene` adalah satu-satunya hal yang menyeberang dari framework ke backend.
//! Backend mana pun (`rustui-renderer` di wgpu hari ini; GL/CPU nanti) menerima
//! `&Scene` dan tidak pernah menerima tipe grafis milik dirinya sendiri dari
//! sisi pemanggil (REKOMENDASI §3.2, §5 failure mode #7).

use crate::color::Color;
use crate::corner::Corners;
use crate::geometry::Rect;
use crate::glyph::GlyphRun;
use crate::shadow::{Shadow, ShadowPair};

/// Kumpulan perintah gambar untuk satu frame, plus warna latar.
///
/// ```
/// use rustui_paint::{Color, Scene};
///
/// let scene = Scene::new(Color::hex(0x1C1C1E));
/// assert!(scene.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct Scene {
    clear_color: Color,
    commands: Vec<Command>,
}

impl Scene {
    /// Scene kosong dengan warna latar tertentu.
    ///
    /// Warna latar selalu datang dari token theme (`background`), tidak pernah
    /// dari literal di kode widget.
    pub fn new(clear_color: Color) -> Self {
        Self {
            clear_color,
            commands: Vec::new(),
        }
    }

    /// Warna latar frame ini.
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Ganti warna latar (mis. setelah dark mode berubah).
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Perintah gambar frame ini, urut dari belakang ke depan.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Tambah satu perintah.
    pub fn push(&mut self, command: impl Into<Command>) -> &mut Self {
        self.commands.push(command.into());
        self
    }

    /// Tambah sederet perintah yang sudah jadi.
    ///
    /// Ada untuk pass paint: subtree yang **bersih** menyalin kembali perintah
    /// yang sudah dihitung frame sebelumnya, tanpa menjalankan ulang logika
    /// gambarnya.
    pub fn push_all(&mut self, commands: &[Command]) -> &mut Self {
        self.commands.extend_from_slice(commands);
        self
    }

    /// Buang perintah setelah indeks `len`.
    ///
    /// Dipakai pass paint untuk membatalkan pembuka clip yang ternyata tidak
    /// membungkus apa pun — clip kosong bukan perintah, ia hanya sampah.
    pub fn truncate(&mut self, len: usize) {
        self.commands.truncate(len);
    }

    /// Tambah sebuah quad **beserta bayangan gandanya** (ambient + key).
    ///
    /// Urutannya yang menentukan hasil: ambient, lalu key, baru kotaknya —
    /// sehingga kotak selalu menutupi bagian bayangan yang berada di bawahnya.
    /// Lapis yang transparan penuh tidak menghasilkan perintah sama sekali,
    /// jadi elevasi 0 benar-benar gratis.
    ///
    /// ```
    /// use rustui_paint::{Color, Quad, Rect, Scene, Shadow, ShadowPair};
    ///
    /// let mut scene = Scene::new(Color::WHITE);
    /// let bayangan = ShadowPair::new(
    ///     Shadow::new(Color::BLACK.with_alpha(0.08), 40.0),
    ///     Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
    /// );
    /// scene.push_shadowed(Quad::new(Rect::new(0.0, 0.0, 80.0, 40.0)), bayangan);
    /// assert_eq!(scene.len(), 3);
    /// ```
    pub fn push_shadowed(&mut self, quad: Quad, shadows: ShadowPair) -> &mut Self {
        for lapis in shadows.layers() {
            if lapis.is_visible() {
                self.push(ShadowQuad::for_quad(&quad, lapis));
            }
        }
        self.push(quad)
    }

    /// Benar bila belum ada perintah apa pun (frame hanya berisi clear).
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Jumlah perintah.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Kosongkan daftar perintah tanpa melepas alokasi — dipakai scheduler
    /// agar frame berikutnya tidak mengalokasi ulang.
    pub fn reset(&mut self, clear_color: Color) {
        self.clear_color = clear_color;
        self.commands.clear();
    }
}

/// Satu perintah gambar.
///
/// Sengaja `#[non_exhaustive]`: kosakata masih tumbuh (glyph, shadow ganda,
/// blur/material, layer offscreen) tanpa memecah backend yang sudah ada.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Command {
    /// Kotak dengan sudut membulat — primitif yang menutupi ~95% UI.
    Quad(Quad),
    /// Satu lapis bayangan ber-blur di belakang sebuah kotak.
    ///
    /// Bayangan ganda ala HIG = dua perintah ini berurutan; lihat
    /// [`Scene::push_shadowed`].
    Shadow(ShadowQuad),
    /// Sekumpulan glyph sewarna dari atlas `rustui-text`.
    ///
    /// Perintah ini hanya membawa id atlas + kotak tujuan — tidak ada font,
    /// tidak ada shaping, tidak ada DPI (lihat modul [`crate::glyph`]).
    GlyphRun(GlyphRun),
    /// Batasi perintah berikutnya ke sebuah kotak, sampai [`Command::PopClip`].
    ///
    /// Kotaknya **absolut** (poin logis, relatif sudut kiri-atas window) dan
    /// sudah merupakan irisan dengan clip di luarnya, sehingga backend cukup
    /// menyetel satu scissor rect dan tidak perlu memelihara tumpukan sendiri.
    ///
    /// Pass paint sudah membuang perintah yang **seluruhnya** di luar kotak
    /// ini; yang tersisa untuk backend hanyalah memotong yang tertimpa
    /// sebagian. Pasangannya selalu seimbang dalam satu `Scene`.
    PushClip(Rect),
    /// Kembalikan clip ke kotak sebelum [`Command::PushClip`] terakhir.
    PopClip,
}

impl From<Quad> for Command {
    fn from(q: Quad) -> Self {
        Command::Quad(q)
    }
}

impl From<ShadowQuad> for Command {
    fn from(s: ShadowQuad) -> Self {
        Command::Shadow(s)
    }
}

impl From<GlyphRun> for Command {
    fn from(r: GlyphRun) -> Self {
        Command::GlyphRun(r)
    }
}

/// Kotak bersudut membulat dengan isi dan border opsional.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    /// Kotak dalam poin logis.
    pub rect: Rect,
    /// Geometri sudut — arc atau squircle, datang dari token theme.
    pub corners: Corners,
    /// Warna isi.
    pub background: Color,
    /// Tebal border (0.0 = tanpa border).
    pub border_width: f32,
    /// Warna border.
    pub border_color: Color,
}

impl Quad {
    /// Kotak polos tanpa lengkung dan tanpa border.
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            corners: Corners::SHARP,
            background: Color::TRANSPARENT,
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        }
    }

    /// Setel warna isi.
    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Setel geometri sudut.
    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    /// Setel border.
    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.border_width = width.max(0.0);
        self.border_color = color;
        self
    }

    /// Versi yang radius sudutnya sudah dibatasi terhadap ukuran kotak.
    pub fn normalized(mut self) -> Self {
        self.corners = self.corners.clamp_to(self.rect.size);
        self
    }
}

/// Satu lapis bayangan yang siap digambar.
///
/// Geometrinya sudah **final**: `offset` dan `spread` dari [`Shadow`] sudah
/// diterapkan ke `rect` dan `corners` di sini (CPU, bisa diuji), sehingga
/// backend cukup mem-blur bentuk apa adanya. Bentuk sudutnya sengaja diwarisi
/// dari kotak yang dibayangi — bayangan squircle tetap squircle.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowQuad {
    /// Bentuk bayangan setelah offset dan spread, poin logis.
    pub rect: Rect,
    /// Geometri sudut bayangan (radius sudah ikut tumbuh bersama spread).
    pub corners: Corners,
    /// Warna bayangan.
    pub color: Color,
    /// Diameter blur, poin logis (sigma = `blur / 2`).
    pub blur: f32,
}

impl ShadowQuad {
    /// Bayangan untuk sebuah quad: mewarisi bentuk sudutnya, lalu menerapkan
    /// offset dan spread.
    pub fn for_quad(quad: &Quad, shadow: Shadow) -> Self {
        let rect = shadow.shape(quad.rect);
        Self {
            rect,
            corners: shadow.shape_corners(quad.corners).clamp_to(rect.size),
            color: shadow.color,
            blur: shadow.blur.max(0.0),
        }
    }

    /// Sigma gaussian yang dipakai shader.
    pub fn sigma(&self) -> f32 {
        self.blur * 0.5
    }

    /// Kotak pembatas termasuk ekor gaussian (3σ) — untuk dirty region.
    pub fn bounds(&self) -> Rect {
        let margin = self.sigma() * 3.0;
        Rect::new(
            self.rect.origin.x - margin,
            self.rect.origin.y - margin,
            self.rect.size.width + margin * 2.0,
            self.rect.size.height + margin * 2.0,
        )
    }

    /// Benar bila lapis ini menyumbang piksel sama sekali.
    pub fn is_visible(&self) -> bool {
        self.color.a > 0.0 && !self.rect.size.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner::CornerStyle;

    #[test]
    fn scene_baru_hanya_berisi_clear() {
        let s = Scene::new(Color::hex(0x101010));
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.clear_color(), Color::hex(0x101010));
    }

    #[test]
    fn push_menjaga_urutan() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Quad::new(Rect::new(0.0, 0.0, 1.0, 1.0)));
        s.push(Quad::new(Rect::new(0.0, 0.0, 2.0, 2.0)));
        assert_eq!(s.len(), 2);
        match &s.commands()[1] {
            Command::Quad(q) => assert_eq!(q.rect.size.width, 2.0),
            lain => panic!("perintah tak terduga: {lain:?}"),
        }
    }

    #[test]
    fn reset_mengganti_clear_dan_mengosongkan_perintah() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Quad::new(Rect::new(0.0, 0.0, 1.0, 1.0)));
        s.reset(Color::WHITE);
        assert!(s.is_empty());
        assert_eq!(s.clear_color(), Color::WHITE);
    }

    #[test]
    fn quad_normalized_membatasi_radius() {
        let q = Quad::new(Rect::new(0.0, 0.0, 100.0, 24.0))
            .corners(Corners::uniform(9999.0, CornerStyle::squircle()))
            .normalized();
        assert_eq!(q.corners.radii.max(), 12.0);
        // Bentuk sudut tidak boleh ikut hilang saat radius dibatasi.
        assert_eq!(q.corners.style, CornerStyle::squircle());
    }

    #[test]
    fn border_negatif_dinolkan() {
        let q = Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).border(-2.0, Color::WHITE);
        assert_eq!(q.border_width, 0.0);
    }

    fn kartu() -> Quad {
        Quad::new(Rect::new(40.0, 40.0, 200.0, 120.0))
            .background(Color::WHITE)
            .corners(Corners::uniform(14.0, CornerStyle::squircle()))
    }

    fn bayangan_ganda() -> ShadowPair {
        ShadowPair::new(
            Shadow::new(Color::BLACK.with_alpha(0.08), 40.0).offset(0.0, 12.0),
            Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
        )
    }

    #[test]
    fn push_shadowed_menggambar_ambient_key_lalu_kotak() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(kartu(), bayangan_ganda());
        assert_eq!(s.len(), 3);
        match s.commands() {
            [Command::Shadow(a), Command::Shadow(k), Command::Quad(q)] => {
                assert!(a.blur > k.blur, "ambient harus lapis paling lebar");
                assert_eq!(q.rect, kartu().rect);
            }
            lain => panic!("urutan perintah salah: {lain:?}"),
        }
    }

    #[test]
    fn bayangan_mewarisi_bentuk_sudut_kotaknya() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(kartu(), bayangan_ganda());
        match &s.commands()[0] {
            Command::Shadow(sh) => assert_eq!(sh.corners.style, CornerStyle::squircle()),
            lain => panic!("bukan bayangan: {lain:?}"),
        }
    }

    #[test]
    fn lapis_tak_terlihat_tidak_menghasilkan_perintah() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(kartu(), ShadowPair::NONE);
        assert_eq!(s.len(), 1, "elevasi 0 harus gratis");
    }

    #[test]
    fn shadow_quad_menerapkan_offset_spread_dan_membatasi_radius() {
        let q = Quad::new(Rect::new(0.0, 0.0, 40.0, 20.0))
            .corners(Corners::uniform(10.0, CornerStyle::Arc));
        let sh = ShadowQuad::for_quad(
            &q,
            Shadow::new(Color::BLACK.with_alpha(0.2), 16.0)
                .offset(0.0, 4.0)
                .spread(2.0),
        );
        assert_eq!(sh.rect, Rect::new(-2.0, 2.0, 44.0, 24.0));
        // radius 10 + spread 2 = 12, tapi setengah sisi terpendek = 12 → pas.
        assert_eq!(sh.corners.radii.max(), 12.0);
        assert_eq!(sh.sigma(), 8.0);
        assert!(sh.is_visible());
    }

    #[test]
    fn glyph_run_masuk_scene_sebagai_perintah_sendiri() {
        use crate::glyph::{Glyph, GlyphImageId, GlyphRun};

        let mut s = Scene::new(Color::BLACK);
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(
            GlyphImageId::from_raw(7),
            Rect::new(8.0, 8.0, 6.0, 10.0),
        ));
        s.push(run);
        match &s.commands()[0] {
            Command::GlyphRun(r) => {
                assert_eq!(r.len(), 1);
                assert_eq!(r.glyphs[0].image, GlyphImageId::from_raw(7));
            }
            lain => panic!("bukan glyph run: {lain:?}"),
        }
    }

    #[test]
    fn push_all_dan_truncate_menyalin_lalu_membatalkan() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)));
        let batas = s.len();
        s.push(Quad::new(Rect::new(0.0, 0.0, 1.0, 1.0)));
        let salinan = s.commands().to_vec();

        s.truncate(batas);
        assert_eq!(s.len(), 1, "clip tanpa isi harus bisa dibatalkan");
        s.push_all(&salinan);
        assert_eq!(s.len(), 3);
        assert!(matches!(s.commands()[1], Command::PushClip(_)));
    }

    #[test]
    fn bounds_bayangan_menyertakan_tiga_sigma() {
        let sh = ShadowQuad::for_quad(
            &Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Shadow::new(Color::BLACK, 4.0),
        );
        assert_eq!(sh.bounds(), Rect::new(-6.0, -6.0, 22.0, 22.0));
    }
}
