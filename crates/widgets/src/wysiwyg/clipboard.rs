//! The **internal** clipboard format: copy inside the application keeps its
//! styling, copy out of it degrades to readable plain text.
//!
//! The clipboard itself belongs to `silka-platform` (INTEGRASI-NATIVE §4) and
//! this crate may not depend on it, so what lives here is only the part that is
//! genuinely ours: turning a [`Fragment`] into a string and back. The shell
//! puts [`Clipping::rich`](super::Clipping) on the pasteboard under a private
//! flavour and [`Clipping::plain`](super::Clipping) under the public one; an
//! application that has no pasteboard yet (the gallery) keeps the rich string
//! in a signal and gets the same behaviour.
//!
//! ## The format
//!
//! Deliberately dull — a line-based text format, versioned by its first line:
//!
//! ```text
//! silka-rich/1
//! #0                          a block, kind index 0 (paragraph)
//! >0,0|lihat                  a span: mark bits 0, link length 0, then the text
//! >0,17|https://silka.dev situs   mark bits 0, 17 bytes of link, then the text
//! ```
//!
//! A span line carries the **byte length** of its link rather than a delimiter,
//! so a URL containing the separator cannot corrupt the parse. Block text can
//! never contain a newline (that is what a block break *is*), which is what
//! makes lines safe as the outer delimiter.
//!
//! Anything that does not begin with the magic line is not ours — [`decode`]
//! answers `None` and the caller pastes it as plain text. That is the whole
//! contract with the outside world: **never** guess.

use super::document::{Block, BlockKind, Fragment, InlineStyle, Marks, Piece, Span};

/// The magic first line — the format's version.
pub const MAGIC: &str = "silka-rich/1";

/// Serialize a fragment into the internal format.
pub fn encode(fragment: &Fragment) -> String {
    let mut out = String::from(MAGIC);
    for piece in &fragment.pieces {
        out.push_str(&format!("\n#{}", kind_index(piece.kind)));
        for span in &piece.spans {
            let tautan = span.style.link.clone().unwrap_or_default();
            out.push_str(&format!(
                "\n>{},{}|{}{}",
                span.style.marks.bits(),
                tautan.len(),
                tautan,
                span.text
            ));
        }
    }
    out
}

/// Parse the internal format, or `None` when the text came from elsewhere.
pub fn decode(text: &str) -> Option<Fragment> {
    let mut baris = text.split('\n');
    if baris.next()? != MAGIC {
        return None;
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for l in baris {
        if let Some(sisa) = l.strip_prefix('#') {
            let kind = kind_from_index(sisa.trim().parse::<usize>().ok()?)?;
            pieces.push(Piece {
                kind,
                spans: Vec::new(),
            });
            continue;
        }
        let sisa = l.strip_prefix('>')?;
        let (kepala, isi) = sisa.split_once('|')?;
        let (bits, panjang) = kepala.split_once(',')?;
        let bits: u8 = bits.parse().ok()?;
        let panjang: usize = panjang.parse().ok()?;
        if panjang > isi.len() || !isi.is_char_boundary(panjang) {
            return None;
        }
        let (tautan, teks) = isi.split_at(panjang);
        let style = InlineStyle {
            marks: marks_from_bits(bits),
            link: (!tautan.is_empty()).then(|| tautan.to_string()),
        };
        // A span line before any block line means a malformed payload; refusing
        // it is the whole point of having a format.
        pieces.last_mut()?.spans.push(Span::new(teks, style));
    }
    if pieces.is_empty() {
        return None;
    }
    for p in &mut pieces {
        super::document::normalize(&mut p.spans);
    }
    Some(Fragment { pieces })
}

/// The blocks a fragment stands for — what `Document::from_blocks` wants.
pub fn blocks_of(fragment: &Fragment) -> Vec<Block> {
    fragment
        .pieces
        .iter()
        .map(|p| Block::new(p.kind, p.spans.clone()))
        .collect()
}

/// Marks are a bit set; only the bits we define survive a round trip.
fn marks_from_bits(bits: u8) -> Marks {
    Marks::ALL
        .iter()
        .fold(Marks::NONE, |acc, m| acc.with(*m, bits & m.bits() != 0))
}

fn kind_index(kind: BlockKind) -> usize {
    BlockKind::ALL.iter().position(|k| *k == kind).unwrap_or(0)
}

fn kind_from_index(index: usize) -> Option<BlockKind> {
    BlockKind::ALL.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn potongan() -> Fragment {
        Fragment {
            pieces: vec![
                Piece {
                    kind: BlockKind::Heading2,
                    spans: vec![
                        Span::plain("Rilis "),
                        Span::new("1.0", InlineStyle::with_marks(Marks::BOLD)),
                    ],
                },
                Piece {
                    kind: BlockKind::Bullet,
                    spans: vec![Span::new(
                        "catatan",
                        InlineStyle::link("https://silka.dev/a|b"),
                    )],
                },
            ],
        }
    }

    #[test]
    fn bolak_balik_mempertahankan_gaya_dan_jenis_blok() {
        let asal = potongan();
        let hasil = decode(&encode(&asal)).expect("format sendiri harus terbaca");
        assert_eq!(hasil, asal);
    }

    #[test]
    fn tautan_yang_memuat_pemisah_tetap_utuh() {
        let hasil = decode(&encode(&potongan())).expect("terbaca");
        assert_eq!(
            hasil.pieces[1].spans[0].style.link.as_deref(),
            Some("https://silka.dev/a|b"),
        );
    }

    #[test]
    fn teks_dari_luar_aplikasi_ditolak_bukan_ditebak() {
        assert!(decode("halo dunia").is_none());
        assert!(decode("silka-rich/9\n#0").is_none());
        assert!(decode("").is_none());
    }

    #[test]
    fn teks_polos_menurunkan_gaya_tapi_bukan_isinya() {
        assert_eq!(potongan().plain_text(), "Rilis 1.0\n• catatan");
    }
}
