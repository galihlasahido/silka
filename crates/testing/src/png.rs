//! A self-contained PNG codec for golden files — no dependency.
//!
//! ## Why golden files are PNGs at all
//!
//! A golden test that stores raw bytes is a golden test nobody looks at. When a
//! snapshot fails, the first thing a human does is open the expected image, the
//! actual image, and the diff side by side; that only works if the files are a
//! format every operating system already previews. So goldens are real PNGs.
//!
//! ## Why the codec is ours
//!
//! Golden files are committed, one per widget per preset per appearance, and
//! they only ever accumulate. An uncompressed 400×320 capture is half a
//! megabyte; twenty widgets across the four-cell matrix would be forty
//! megabytes of repository for a single screen size. So this module implements
//! DEFLATE properly — LZ77 matching emitted with the **fixed Huffman** tables
//! from RFC 1951 — which takes flat UI artwork down by one to two orders of
//! magnitude. A dependency would cost less code and more supply chain; the
//! algorithm is thirty years old and does not move.
//!
//! What is deliberately *not* implemented is **dynamic** Huffman decoding: we
//! read our own files, and our own files are fixed-Huffman. A golden re-saved
//! by another tool would use dynamic blocks and fail with
//! [`PngError::Compressed`], whose message says to regenerate it. Golden files
//! are generated artefacts, so that is a cost we can pay.
//!
//! The output is validated two ways: round-tripped through this module's own
//! decoder, and — the check that actually matters — opened by the operating
//! system's image stack, which is what a reviewer will use.
//!
//! ```
//! use silka_testing::png;
//! use silka_testing::Image;
//!
//! let mut capture = Image::filled(64, 64, [28, 28, 30, 255]);
//! capture.set_pixel(10, 10, [10, 132, 255, 255]);
//!
//! let bytes = png::encode(&capture);
//!
//! // A real PNG, so a reviewer can double-click the failing golden.
//! assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
//!
//! // Flat UI artwork compresses hard, which is the point: goldens are
//! // committed, one per widget per cell, and they only ever accumulate.
//! assert!(bytes.len() < capture.pixels().len() / 4);
//!
//! // And the round trip is lossless — a golden is a comparison target, so
//! // "close enough" would defeat the whole exercise.
//! let decoded = png::decode(&bytes).expect("our own encoder writes fixed-Huffman blocks");
//! assert_eq!(decoded.pixels(), capture.pixels());
//! ```

use core::fmt;

use crate::image::{Image, ImageError, CHANNELS};

/// The eight bytes every PNG starts with.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The largest payload a single DEFLATE stored block can carry — only the
/// test-only stored encoder needs it now that real blocks are Huffman-coded.
#[cfg(test)]
const MAX_STORED: usize = 0xFFFF;

/// PNG colour type 6: truecolour with alpha.
const COLOR_RGBA: u8 = 6;

/// Why a PNG could not be read.
///
/// ```
/// use silka_testing::png::{self, PngError};
///
/// // Anything that is not a PNG is rejected before it can be misread.
/// assert_eq!(png::decode(b"not a png at all"), Err(PngError::NotPng));
/// ```
///
/// [`PngError::Compressed`] is the one worth recognising: this decoder handles
/// only the stored DEFLATE blocks the matching encoder writes, so a golden file
/// produced by another tool has to be regenerated with `SILKA_GOLDEN=update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngError {
    /// The file does not start with the PNG signature.
    NotPng,
    /// The file ended in the middle of a structure.
    Truncated,
    /// A chunk's CRC did not match its contents.
    BadCrc {
        /// The four-letter chunk type.
        chunk: String,
    },
    /// The zlib wrapper's Adler-32 checksum did not match.
    BadAdler,
    /// The image is not 8-bit RGBA, or is interlaced.
    Unsupported {
        /// What exactly is unsupported.
        detail: String,
    },
    /// The DEFLATE stream uses Huffman blocks, which this decoder does not
    /// implement. Regenerate the golden file (`SILKA_GOLDEN=update`).
    Compressed,
    /// The decoded pixel data does not fit the declared size.
    Malformed(ImageError),
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngError::NotPng => f.write_str("bukan berkas PNG (tanda tangan salah)"),
            PngError::Truncated => f.write_str("berkas PNG terpotong"),
            PngError::BadCrc { chunk } => write!(f, "CRC chunk {chunk} tidak cocok"),
            PngError::BadAdler => f.write_str("checksum Adler-32 zlib tidak cocok"),
            PngError::Unsupported { detail } => write!(f, "PNG tidak didukung: {detail}"),
            PngError::Compressed => f.write_str(
                "PNG memakai blok DEFLATE terkompresi; berkas golden harus \
                 dibuat ulang dengan SILKA_GOLDEN=update",
            ),
            PngError::Malformed(e) => write!(f, "isi PNG tidak konsisten: {e}"),
        }
    }
}

impl std::error::Error for PngError {}

// ---------------------------------------------------------------------------
// Checksums
// ---------------------------------------------------------------------------

/// The CRC-32 table PNG chunks use, built at compile time.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in bytes {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode an image as a PNG byte stream.
pub fn encode(image: &Image) -> Vec<u8> {
    let raw = scanlines(image);

    let mut out = Vec::with_capacity(raw.len() + 512);
    out.extend_from_slice(&SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&image.width().to_be_bytes());
    ihdr.extend_from_slice(&image.height().to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(COLOR_RGBA);
    ihdr.push(0); // compression: deflate
    ihdr.push(0); // filter method: adaptive
    ihdr.push(0); // interlace: none
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Prefix every row with filter byte 0 ("None").
///
/// A real encoder would pick a filter per row to help the compressor; with
/// stored blocks there is no compressor to help, so the honest choice is the
/// filter that costs nothing to write and nothing to undo.
fn scanlines(image: &Image) -> Vec<u8> {
    let stride = image.width() as usize * CHANNELS;
    let mut raw = Vec::with_capacity((stride + 1) * image.height() as usize);
    for row in image.pixels().chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    raw
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Wrap `raw` in a zlib stream: header, one fixed-Huffman DEFLATE block, and
/// the Adler-32 trailer.
fn zlib(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() / 4 + 64);
    out.push(0x78); // CM = deflate, CINFO = 32K window
    out.push(0x01); // no preset dictionary, fastest level; (0x7801 % 31 == 0)
    out.extend_from_slice(&deflate_fixed(raw));
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// The same wrapper built from **stored** blocks.
///
/// Kept because the decoder must handle stored blocks — any other encoder may
/// emit them — and a code path with no test is a code path that is wrong.
#[cfg(test)]
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / MAX_STORED * 5 + 16);
    out.push(0x78);
    out.push(0x01);

    if raw.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        let mut offset = 0;
        while offset < raw.len() {
            let len = (raw.len() - offset).min(MAX_STORED);
            let last = offset + len == raw.len();
            out.push(u8::from(last));
            out.extend_from_slice(&(len as u16).to_le_bytes());
            out.extend_from_slice(&(!(len as u16)).to_le_bytes());
            out.extend_from_slice(&raw[offset..offset + len]);
            offset += len;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

// ---------------------------------------------------------------------------
// DEFLATE (RFC 1951), fixed Huffman
// ---------------------------------------------------------------------------

/// The sliding window a match may reach back into.
const WINDOW: usize = 32_768;
/// The shortest run worth encoding as a match rather than as literals.
const MIN_MATCH: usize = 3;
/// The longest run one match can express.
const MAX_MATCH: usize = 258;
/// How many candidates at the same hash we are willing to inspect.
///
/// The knob that trades ratio for time. Golden files are written rarely and
/// read often, but they are also written inside a test run, so the chain stays
/// short: flat UI artwork finds its long match almost immediately.
const MAX_CHAIN: usize = 32;
/// Size of the 3-byte hash table.
const HASH_SIZE: usize = 1 << 15;

/// The first length each length code covers (codes 257–285).
const LENGTH_BASE: [usize; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// How many extra bits each length code carries.
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// The first distance each distance code covers.
const DIST_BASE: [usize; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12_289, 16_385, 24_577,
];
/// How many extra bits each distance code carries.
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// DEFLATE packs plain values least-significant bit first, but Huffman codes
/// most-significant bit first. Getting that backwards produces a file that
/// looks right until a real decoder reads it, so the two are separate methods.
struct BitWriter {
    out: Vec<u8>,
    buffer: u32,
    bits: u32,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
            buffer: 0,
            bits: 0,
        }
    }

    /// Write `count` bits of `value`, least significant first.
    fn write(&mut self, value: u32, count: u32) {
        self.buffer |= (value & ((1 << count) - 1)) << self.bits;
        self.bits += count;
        while self.bits >= 8 {
            self.out.push((self.buffer & 0xFF) as u8);
            self.buffer >>= 8;
            self.bits -= 8;
        }
    }

    /// Write a Huffman code, most significant bit first.
    fn write_code(&mut self, code: u16, length: u32) {
        for i in (0..length).rev() {
            self.write((code as u32 >> i) & 1, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.out.push((self.buffer & 0xFF) as u8);
        }
        self.out
    }
}

/// The fixed literal/length code for a symbol, as `(code, bit length)`.
fn fixed_literal_code(symbol: u16) -> (u16, u32) {
    match symbol {
        0..=143 => (0x30 + symbol, 8),
        144..=255 => (0x190 + (symbol - 144), 9),
        256..=279 => (symbol - 256, 7),
        _ => (0xC0 + (symbol - 280), 8),
    }
}

/// Which length code covers `length`.
fn length_code(length: usize) -> usize {
    let mut i = 0;
    while i + 1 < LENGTH_BASE.len() && LENGTH_BASE[i + 1] <= length {
        i += 1;
    }
    i
}

/// Which distance code covers `distance`.
fn distance_code(distance: usize) -> usize {
    let mut i = 0;
    while i + 1 < DIST_BASE.len() && DIST_BASE[i + 1] <= distance {
        i += 1;
    }
    i
}

fn hash3(data: &[u8]) -> usize {
    let v = (data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32;
    ((v.wrapping_mul(0x9E37_79B1)) >> (32 - 15)) as usize & (HASH_SIZE - 1)
}

/// Compress with LZ77 + the fixed Huffman tables, as a single final block.
fn deflate_fixed(raw: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new(raw.len() / 4 + 16);
    w.write(1, 1); // BFINAL
    w.write(1, 2); // BTYPE = 01, fixed Huffman

    let mut head = vec![u32::MAX; HASH_SIZE];
    let mut prev = vec![u32::MAX; raw.len()];
    let mut at = 0usize;

    while at < raw.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;

        if at + MIN_MATCH <= raw.len() {
            let h = hash3(&raw[at..]);
            let mut candidate = head[h];
            let mut chain = 0;
            while candidate != u32::MAX && chain < MAX_CHAIN {
                let c = candidate as usize;
                let distance = at - c;
                if distance > WINDOW {
                    break;
                }
                let limit = (raw.len() - at).min(MAX_MATCH);
                let mut len = 0;
                while len < limit && raw[c + len] == raw[at + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = distance;
                    if len >= MAX_MATCH {
                        break;
                    }
                }
                candidate = prev[c];
                chain += 1;
            }
        }

        let advance = if best_len >= MIN_MATCH {
            let li = length_code(best_len);
            let (code, bits) = fixed_literal_code(257 + li as u16);
            w.write_code(code, bits);
            w.write((best_len - LENGTH_BASE[li]) as u32, LENGTH_EXTRA[li]);
            let di = distance_code(best_dist);
            // Distance codes are five fixed bits, still most significant first.
            w.write_code(di as u16, 5);
            w.write((best_dist - DIST_BASE[di]) as u32, DIST_EXTRA[di]);
            best_len
        } else {
            let (code, bits) = fixed_literal_code(raw[at] as u16);
            w.write_code(code, bits);
            1
        };

        // Every position the match covered still has to enter the hash chain,
        // or later matches cannot reach back into it.
        for k in 0..advance {
            let p = at + k;
            if p + MIN_MATCH <= raw.len() {
                let h = hash3(&raw[p..]);
                prev[p] = head[h];
                head[h] = p as u32;
            }
        }
        at += advance;
    }

    let (code, bits) = fixed_literal_code(256); // end of block
    w.write_code(code, bits);
    w.finish()
}

/// The mirror image of [`BitWriter`].
struct BitReader<'a> {
    data: &'a [u8],
    at: usize,
    bit: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            bit: 0,
        }
    }

    fn read_bit(&mut self) -> Result<u32, PngError> {
        let byte = *self.data.get(self.at).ok_or(PngError::Truncated)?;
        let value = (byte >> self.bit) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.at += 1;
        }
        Ok(value as u32)
    }

    /// `count` bits, least significant first.
    fn read(&mut self, count: u32) -> Result<u32, PngError> {
        let mut value = 0;
        for i in 0..count {
            value |= self.read_bit()? << i;
        }
        Ok(value)
    }

    /// `count` bits of a Huffman code, most significant first.
    fn read_code(&mut self, count: u32) -> Result<u16, PngError> {
        let mut value = 0u16;
        for _ in 0..count {
            value = (value << 1) | self.read_bit()? as u16;
        }
        Ok(value)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.at += 1;
        }
    }
}

/// Decode one symbol with the fixed literal/length table.
fn fixed_symbol(r: &mut BitReader<'_>) -> Result<u16, PngError> {
    let mut code = r.read_code(7)?;
    if code <= 0x17 {
        return Ok(256 + code);
    }
    code = (code << 1) | r.read_bit()? as u16;
    if (0x30..=0xBF).contains(&code) {
        return Ok(code - 0x30);
    }
    if (0xC0..=0xC7).contains(&code) {
        return Ok(280 + code - 0xC0);
    }
    code = (code << 1) | r.read_bit()? as u16;
    if (0x190..=0x1FF).contains(&code) {
        return Ok(144 + code - 0x190);
    }
    Err(PngError::Unsupported {
        detail: "kode Huffman tetap tidak valid".to_string(),
    })
}

/// Undo the zlib wrapper and every DEFLATE block inside it.
fn inflate(stream: &[u8]) -> Result<Vec<u8>, PngError> {
    if stream.len() < 6 {
        return Err(PngError::Truncated);
    }
    let cmf = stream[0];
    if cmf & 0x0F != 8 {
        return Err(PngError::Unsupported {
            detail: format!("metode kompresi zlib {}", cmf & 0x0F),
        });
    }
    let body = &stream[2..stream.len() - 4];
    let checksum = u32::from_be_bytes([
        stream[stream.len() - 4],
        stream[stream.len() - 3],
        stream[stream.len() - 2],
        stream[stream.len() - 1],
    ]);

    let mut out = Vec::new();
    let mut r = BitReader::new(body);
    loop {
        let last = r.read_bit()? == 1;
        match r.read(2)? {
            0 => {
                r.align();
                if r.at + 4 > body.len() {
                    return Err(PngError::Truncated);
                }
                let len = u16::from_le_bytes([body[r.at], body[r.at + 1]]) as usize;
                let nlen = u16::from_le_bytes([body[r.at + 2], body[r.at + 3]]);
                if nlen != !(len as u16) {
                    return Err(PngError::Truncated);
                }
                r.at += 4;
                if r.at + len > body.len() {
                    return Err(PngError::Truncated);
                }
                out.extend_from_slice(&body[r.at..r.at + len]);
                r.at += len;
            }
            1 => inflate_fixed_block(&mut r, &mut out)?,
            _ => return Err(PngError::Compressed),
        }
        if last {
            break;
        }
    }

    if adler32(&out) != checksum {
        return Err(PngError::BadAdler);
    }
    Ok(out)
}

fn inflate_fixed_block(r: &mut BitReader<'_>, out: &mut Vec<u8>) -> Result<(), PngError> {
    loop {
        let symbol = fixed_symbol(r)?;
        match symbol {
            0..=255 => out.push(symbol as u8),
            256 => return Ok(()),
            _ => {
                let li = (symbol - 257) as usize;
                if li >= LENGTH_BASE.len() {
                    return Err(PngError::Unsupported {
                        detail: format!("kode panjang {symbol}"),
                    });
                }
                let length = LENGTH_BASE[li] + r.read(LENGTH_EXTRA[li])? as usize;
                let di = r.read_code(5)? as usize;
                if di >= DIST_BASE.len() {
                    return Err(PngError::Unsupported {
                        detail: format!("kode jarak {di}"),
                    });
                }
                let distance = DIST_BASE[di] + r.read(DIST_EXTRA[di])? as usize;
                if distance == 0 || distance > out.len() {
                    return Err(PngError::Truncated);
                }
                for _ in 0..length {
                    let byte = out[out.len() - distance];
                    out.push(byte);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode a PNG produced by [`encode`].
pub fn decode(bytes: &[u8]) -> Result<Image, PngError> {
    if bytes.len() < SIGNATURE.len() || bytes[..SIGNATURE.len()] != SIGNATURE {
        return Err(PngError::NotPng);
    }
    let mut cursor = SIGNATURE.len();
    let mut header: Option<(u32, u32)> = None;
    let mut idat: Vec<u8> = Vec::new();

    loop {
        if cursor + 8 > bytes.len() {
            return Err(PngError::Truncated);
        }
        let len = u32::from_be_bytes(take4(bytes, cursor)) as usize;
        let kind = &bytes[cursor + 4..cursor + 8];
        let body_start = cursor + 8;
        let body_end = body_start.checked_add(len).ok_or(PngError::Truncated)?;
        if body_end + 4 > bytes.len() {
            return Err(PngError::Truncated);
        }
        let expected = u32::from_be_bytes(take4(bytes, body_end));
        if crc32(&bytes[cursor + 4..body_end]) != expected {
            return Err(PngError::BadCrc {
                chunk: String::from_utf8_lossy(kind).into_owned(),
            });
        }
        let body = &bytes[body_start..body_end];

        match kind {
            b"IHDR" => header = Some(parse_ihdr(body)?),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        cursor = body_end + 4;
    }

    let (width, height) = header.ok_or(PngError::Truncated)?;
    let raw = inflate(&idat)?;
    let pixels = unfilter(&raw, width, height)?;
    Image::new(width, height, pixels).map_err(PngError::Malformed)
}

fn take4(bytes: &[u8], at: usize) -> [u8; 4] {
    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
}

fn parse_ihdr(body: &[u8]) -> Result<(u32, u32), PngError> {
    if body.len() < 13 {
        return Err(PngError::Truncated);
    }
    let width = u32::from_be_bytes(take4(body, 0));
    let height = u32::from_be_bytes(take4(body, 4));
    let (depth, color, interlace) = (body[8], body[9], body[12]);
    if depth != 8 || color != COLOR_RGBA {
        return Err(PngError::Unsupported {
            detail: format!("bit depth {depth}, color type {color}; hanya 8-bit RGBA"),
        });
    }
    if interlace != 0 {
        return Err(PngError::Unsupported {
            detail: "interlace Adam7".to_string(),
        });
    }
    Ok((width, height))
}

/// Reverse PNG's per-row filters.
///
/// Our encoder only ever writes filter 0, but all five are implemented: it
/// costs a few lines and means a golden file hand-made by another tool (as long
/// as it used stored blocks) still reads.
fn unfilter(raw: &[u8], width: u32, height: u32) -> Result<Vec<u8>, PngError> {
    let stride = width as usize * CHANNELS;
    let expected = (stride + 1) * height as usize;
    if raw.len() != expected {
        return Err(PngError::Truncated);
    }
    let mut out = vec![0u8; stride * height as usize];
    for y in 0..height as usize {
        let filter = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..y * (stride + 1) + 1 + stride];
        for x in 0..stride {
            let a = if x >= CHANNELS {
                out[y * stride + x - CHANNELS]
            } else {
                0
            };
            let b = if y > 0 { out[(y - 1) * stride + x] } else { 0 };
            let c = if y > 0 && x >= CHANNELS {
                out[(y - 1) * stride + x - CHANNELS]
            } else {
                0
            };
            let value = match filter {
                0 => src[x],
                1 => src[x].wrapping_add(a),
                2 => src[x].wrapping_add(b),
                3 => src[x].wrapping_add((((a as u16) + (b as u16)) / 2) as u8),
                4 => src[x].wrapping_add(paeth(a, b, c)),
                other => {
                    return Err(PngError::Unsupported {
                        detail: format!("filter baris {other}"),
                    })
                }
            };
            out[y * stride + x] = value;
        }
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contoh(width: u32, height: u32) -> Image {
        let mut img = Image::filled(width, height, [0, 0, 0, 255]);
        for y in 0..height {
            for x in 0..width {
                img.set_pixel(
                    x,
                    y,
                    [
                        (x % 251) as u8,
                        (y % 253) as u8,
                        ((x * 7 + y * 13) % 255) as u8,
                        255 - (x % 5) as u8,
                    ],
                );
            }
        }
        img
    }

    #[test]
    fn bolak_balik_menghasilkan_gambar_yang_sama() {
        let asli = contoh(23, 17);
        let bytes = encode(&asli);
        let kembali = decode(&bytes).expect("dekode PNG");
        assert_eq!(asli, kembali);
    }

    #[test]
    fn bolak_balik_gambar_besar() {
        // Big enough that matches reach back past a few thousand bytes, which
        // is where the larger distance codes start being used.
        let asli = contoh(200, 100);
        let bytes = encode(&asli);
        assert_eq!(decode(&bytes).expect("dekode"), asli);
    }

    #[test]
    fn tanda_tangan_dan_urutan_chunk_benar() {
        let bytes = encode(&contoh(4, 4));
        assert_eq!(&bytes[..8], &SIGNATURE);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert!(bytes.ends_with(b"\x00\x00\x00\x00IEND\xAE\x42\x60\x82"));
    }

    #[test]
    fn menolak_berkas_yang_bukan_png() {
        assert_eq!(decode(b"bukan png sama sekali"), Err(PngError::NotPng));
        assert_eq!(decode(&[]), Err(PngError::NotPng));
    }

    #[test]
    fn crc_rusak_ketahuan() {
        let mut bytes = encode(&contoh(3, 3));
        let n = bytes.len();
        // Corrupt a pixel byte but leave the CRC alone.
        bytes[n - 20] ^= 0xFF;
        assert!(matches!(decode(&bytes), Err(PngError::BadCrc { .. })));
    }

    #[test]
    fn terpotong_ketahuan() {
        let bytes = encode(&contoh(6, 6));
        assert_eq!(decode(&bytes[..bytes.len() - 9]), Err(PngError::Truncated));
    }

    #[test]
    fn blok_dinamis_ditolak_dengan_pesan_yang_menuntun() {
        // A dynamic-Huffman block (btype = 10, so bits 1|0<<1|1<<2) is what a
        // real compressor emits and what this decoder does not implement.
        let raw = scanlines(&contoh(2, 2));
        let mut zlib = vec![0x78, 0x01, 0b101];
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SIGNATURE);
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, COLOR_RGBA, 0, 0, 0]);
        chunk(&mut bytes, b"IHDR", &ihdr);
        chunk(&mut bytes, b"IDAT", &zlib);
        chunk(&mut bytes, b"IEND", &[]);
        assert_eq!(decode(&bytes), Err(PngError::Compressed));
        assert!(PngError::Compressed.to_string().contains("SILKA_GOLDEN"));
    }

    #[test]
    fn semua_filter_baris_didukung() {
        // Hand-build a two-row image that uses filter 4 (Paeth) on its second
        // row, proving the decoder is not limited to what our encoder writes.
        let width = 2u32;
        let stride = width as usize * CHANNELS;
        let mut raw = Vec::new();
        raw.push(0u8);
        raw.extend_from_slice(&[10, 20, 30, 255, 40, 50, 60, 255]);
        raw.push(4u8);
        raw.extend_from_slice(&[1, 1, 1, 0, 2, 2, 2, 0]);
        let pixels = unfilter(&raw, width, 2).expect("unfilter");
        assert_eq!(pixels.len(), stride * 2);
        // Row 1, pixel 0: a=0, b=row0 pixel0, c=0 -> Paeth picks b.
        assert_eq!(&pixels[stride..stride + 4], &[11, 21, 31, 255]);
    }

    #[test]
    fn adler_rusak_ketahuan() {
        let img = contoh(3, 2);
        let raw = scanlines(&img);
        let mut zlib = zlib_stored(&raw);
        let n = zlib.len();
        zlib[n - 1] ^= 0x01;
        assert_eq!(inflate(&zlib), Err(PngError::BadAdler));
    }

    #[test]
    fn blok_stored_dari_encoder_lain_tetap_terbaca() {
        // Our own files are fixed-Huffman, but stored blocks are legal DEFLATE
        // and the decoder must not have rotted since it stopped being the
        // encoder's output.
        let raw = scanlines(&contoh(40, 3));
        assert_eq!(inflate(&zlib_stored(&raw)).expect("inflate"), raw);
    }

    #[test]
    fn kompresi_benar_benar_mengecilkan_gambar_ui() {
        // The reason the LZ77 pass exists at all. A flat card on a flat
        // background is the shape almost every golden has, and storing it raw
        // would put half a megabyte per snapshot into the repository.
        let mut img = Image::filled(200, 200, [20, 22, 28, 255]);
        for y in 40..160 {
            for x in 40..160 {
                img.set_pixel(x, y, [64, 128, 255, 255]);
            }
        }
        let bytes = encode(&img);
        let mentah = 200 * 200 * 4;
        assert!(
            bytes.len() * 20 < mentah,
            "hanya {} byte dari {mentah} — kompresi tidak bekerja",
            bytes.len()
        );
        assert_eq!(
            decode(&bytes).expect("dekode"),
            img,
            "kecil tapi tetap utuh"
        );
    }

    #[test]
    fn kecocokan_terpanjang_dan_jarak_terjauh_bolak_balik() {
        // Exercises both ends of the match encoding: runs longer than the
        // 258-byte maximum (which must split into several matches) and a
        // repeat far enough back to need a large distance code.
        let mut raw = vec![7u8; 1000];
        raw.extend(std::iter::repeat(0u8).take(20_000));
        raw.extend_from_slice(&[7u8; 1000]);
        let stream = zlib(&raw);
        assert!(stream.len() < raw.len() / 4);
        assert_eq!(inflate(&stream).expect("inflate"), raw);
    }

    #[test]
    fn data_tak_termampatkan_tetap_bolak_balik() {
        // Pseudo-random bytes have no matches at all, so every symbol goes out
        // as a literal — the path where the fixed table's 9-bit codes are used.
        let raw: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        assert_eq!(inflate(&zlib(&raw)).expect("inflate"), raw);
    }

    #[test]
    fn aliran_kosong_tetap_sah() {
        assert_eq!(inflate(&zlib(&[])).expect("inflate"), Vec::<u8>::new());
    }
}
