//! SHA-256, and the hexadecimal digest a feed carries.
//!
//! This is **integrity, not authenticity**. It answers "are these the bytes the
//! feed described", which catches a truncated download, a corrupted mirror and a
//! cache that served the previous release. It does not answer "did we write this
//! feed", which is what a signature is for and what [`crate::update`] delegates
//! to a verifier the application supplies.
//!
//! Written out by hand rather than pulled in, for the reason the crate README
//! gives: the updater is the one component that cannot be repaired by an update,
//! so its dependency count is zero. FIPS 180-4, checked here against the
//! specification's own test vectors.
//!
//! ```
//! use silka_dist::sha256::{sha256, Digest};
//!
//! let digest = sha256(b"abc");
//! assert_eq!(
//!     digest.to_string(),
//!     "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
//! );
//!
//! // A feed writes it either bare or with the algorithm spelled out.
//! assert_eq!(Digest::parse("sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD").unwrap(), digest);
//! ```

use std::fmt;

/// The 32-byte output of SHA-256.
///
/// Compared with `==`, which for a 32-byte array is a constant-time-enough
/// comparison of a public value: a digest is not a secret, and an attacker who
/// can substitute the file can substitute the digest too. That is exactly why a
/// digest match is never the last word — see [`crate::update::verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// Wrap raw bytes that are already a digest.
    pub const fn from_bytes(bytes: [u8; 32]) -> Digest {
        Digest(bytes)
    }

    /// The raw bytes — the message an artifact signature is made over.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Read `"ba78…"` or `"sha256:BA78…"`; case-insensitive, exactly 64 hex digits.
    pub fn parse(text: &str) -> Result<Digest, DigestError> {
        let trimmed = text.trim();
        let body = match trimmed.split_once(':') {
            Some((algorithm, rest)) => {
                if !algorithm.eq_ignore_ascii_case("sha256") {
                    return Err(DigestError::UnknownAlgorithm);
                }
                rest
            }
            None => trimmed,
        };
        if body.len() != 64 {
            return Err(DigestError::WrongLength);
        }
        let bytes = body.as_bytes();
        let mut out = [0u8; 32];
        for index in 0..32 {
            let high = hex_value(bytes[index * 2])?;
            let low = hex_value(bytes[index * 2 + 1])?;
            out[index] = (high << 4) | low;
        }
        Ok(Digest(out))
    }
}

fn hex_value(byte: u8) -> Result<u8, DigestError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DigestError::NotHex),
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter() {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Why a digest string could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestError {
    /// The prefix before `:` named an algorithm this crate does not compute.
    UnknownAlgorithm,
    /// Not 64 hexadecimal digits.
    WrongLength,
    /// A character outside `[0-9a-fA-F]`.
    NotHex,
}

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            DigestError::UnknownAlgorithm => "digest names an algorithm other than sha256",
            DigestError::WrongLength => "digest is not 64 hexadecimal digits",
            DigestError::NotHex => "digest contains a non-hexadecimal character",
        };
        f.write_str(text)
    }
}

impl std::error::Error for DigestError {}

// ---------------------------------------------------------------------------
// The hash itself
// ---------------------------------------------------------------------------

/// Digest a whole slice in one call.
pub fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finish()
}

/// Streaming SHA-256, for a download that arrives in chunks.
///
/// The point of the streaming form is that a 200 MB installer never has to be
/// resident in memory to be verified: hash it as it lands, compare once at the
/// end, and only then hand the file to the installer.
#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    /// A fresh hasher.
    pub fn new() -> Sha256 {
        Sha256 {
            state: INITIAL_STATE,
            buffer: [0u8; 64],
            buffered: 0,
            length: 0,
        }
    }

    /// Feed more bytes.
    pub fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);

        if self.buffered > 0 {
            let want = 64 - self.buffered;
            let take = want.min(data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                compress(&mut self.state, &block);
                self.buffered = 0;
            }
        }

        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            compress(&mut self.state, &block);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffered = data.len();
        }
    }

    /// Finish and produce the digest.
    pub fn finish(mut self) -> Digest {
        let bit_length = self.length.wrapping_mul(8);

        // Padding: one `1` bit, then zeros, then the length in bits as a 64-bit
        // big-endian number, landing exactly on a block boundary.
        let mut tail = [0u8; 128];
        tail[0] = 0x80;
        let zeros = if self.buffered < 56 {
            55 - self.buffered
        } else {
            119 - self.buffered
        };
        let total = 1 + zeros + 8;
        tail[1 + zeros..total].copy_from_slice(&bit_length.to_be_bytes());
        self.update_without_length(&tail[..total]);

        let mut out = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        Digest(out)
    }

    /// The body of [`Sha256::update`] without the length accounting.
    ///
    /// Padding must not be counted as message bytes, and the alternative —
    /// subtracting the padding length afterwards — is the kind of correction
    /// that survives review and then goes wrong on a 4 GB input.
    fn update_without_length(&mut self, mut data: &[u8]) {
        if self.buffered > 0 {
            let want = 64 - self.buffered;
            let take = want.min(data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                compress(&mut self.state, &block);
                self.buffered = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            compress(&mut self.state, &block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffered = data.len();
        }
    }
}

const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0u32; 64];
    for index in 0..16 {
        schedule[index] = u32::from_be_bytes([
            block[index * 4],
            block[index * 4 + 1],
            block[index * 4 + 2],
            block[index * 4 + 3],
        ]);
    }
    for index in 16..64 {
        let s0 = {
            let x = schedule[index - 15];
            x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
        };
        let s1 = {
            let x = schedule[index - 2];
            x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
        };
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for index in 0..64 {
        let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(big_s1)
            .wrapping_add(choose)
            .wrapping_add(ROUND_CONSTANTS[index])
            .wrapping_add(schedule[index]);
        let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = big_s0.wrapping_add(majority);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    // FIPS 180-4 second vector: 56 bytes, which lands the padding in a second block.
    const TWO_BLOCK: &str = "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1";
    // A million 'a' — the vector that catches a length counted in bytes.
    const MILLION_A: &str = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";

    #[test]
    fn vektor_kosong() {
        assert_eq!(sha256(b"").to_string(), EMPTY);
    }

    #[test]
    fn vektor_abc() {
        assert_eq!(sha256(b"abc").to_string(), ABC);
    }

    #[test]
    fn vektor_dua_blok() {
        let message = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(message.len(), 56, "vektor ini justru menguji batas padding");
        assert_eq!(sha256(message).to_string(), TWO_BLOCK);
    }

    #[test]
    fn vektor_sejuta_a() {
        let mut hasher = Sha256::new();
        let chunk = [b'a'; 1000];
        for _ in 0..1000 {
            hasher.update(&chunk);
        }
        assert_eq!(hasher.finish().to_string(), MILLION_A);
    }

    #[test]
    fn potongan_tak_rata_sama_dengan_sekali_jalan() {
        let message: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let once = sha256(&message);

        let mut hasher = Sha256::new();
        let mut offset = 0usize;
        for size in [1usize, 7, 63, 64, 65, 100, 200, 300, 200] {
            let end = (offset + size).min(message.len());
            hasher.update(&message[offset..end]);
            offset = end;
        }
        hasher.update(&message[offset..]);
        assert_eq!(hasher.finish(), once);
    }

    #[test]
    fn panjang_tepat_di_batas_padding() {
        // 55, 56, 63, 64 and 119/120 are every place the padding branch changes.
        for length in [54usize, 55, 56, 57, 63, 64, 65, 119, 120, 121] {
            let message = vec![b'x'; length];
            let mut streamed = Sha256::new();
            streamed.update(&message);
            assert_eq!(
                streamed.finish(),
                sha256(&message),
                "panjang {length} berbeda antara sekali jalan dan streaming"
            );
        }
    }

    #[test]
    fn digest_bolak_balik() {
        let digest = sha256(b"abc");
        assert_eq!(Digest::parse(ABC).unwrap(), digest);
        assert_eq!(Digest::parse(&format!("sha256:{ABC}")).unwrap(), digest);
        assert_eq!(Digest::parse(&ABC.to_uppercase()).unwrap(), digest);
        assert_eq!(digest.as_bytes().len(), 32);
    }

    #[test]
    fn digest_yang_ditolak() {
        assert_eq!(Digest::parse("abcd"), Err(DigestError::WrongLength));
        assert_eq!(Digest::parse(&"z".repeat(64)), Err(DigestError::NotHex));
        assert_eq!(
            Digest::parse(&format!("md5:{ABC}")),
            Err(DigestError::UnknownAlgorithm)
        );
    }

    #[test]
    fn from_bytes_bolak_balik() {
        let digest = sha256(b"abc");
        assert_eq!(Digest::from_bytes(*digest.as_bytes()), digest);
    }
}
