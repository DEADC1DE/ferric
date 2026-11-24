//! DH1080 key exchange as used by FiSH and FiSH 10.
//!
//! Pure computation only: no network, no files. The caller sends the strings returned by
//! [`Dh1080Keypair::init_message`] / [`Dh1080Keypair::finish_message`] as the text of an
//! IRC NOTICE to the peer and feeds received text to [`parse_message`].
//!
//! Wire format (standard base64 alphabet, no '=' padding, one extra 'A' appended when the
//! byte length is a multiple of 3, so a 135-byte value becomes 181 characters):
//! - `DH1080_INIT <public>[ CBC]`
//! - `DH1080_FINISH <public>[ CBC]`
//!
//! The Blowfish key is the 43-character base64 text of SHA-256(shared secret), used as
//! ASCII bytes (not the raw digest).
//!
//! No authentication exists in this protocol: anyone on the path can mount a
//! man-in-the-middle attack.

use std::fmt;

use num_bigint::BigUint;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

/// The 1080-bit safe prime shared by all FiSH implementations (hex, big-endian).
const PRIME_HEX: &[u8] = b"FBE1022E23D213E8ACFA9AE8B9DFADA3EA6B7AC7A7B7E95AB5EB2DF858921FEADE95E6AC7BE7DE6ADBAB8A783E7AF7A7FA6A2B7BEB1E72EAE2B72F9FA2BFB2A2EFBEFAC868BADB3E828FA8BADFADA3E4CC1BE7E8AFE85E9698A783EB68FA07A77AB6AD7BEB618ACF9CA2897EB28A6189EFA07AB99A8A7FA9AE299EFA7BA66DEAFEFBEFBF0B7D8B";

/// Size of the prime in bytes (1080 bits).
const KEY_BYTES: usize = 135;
/// Private exponent size, matching OpenSSL 3 for this prime (bits(p) - 2).
const PRIVATE_BITS: usize = 1078;
/// Longest accepted public value text (135 bytes -> 180 characters + 'A').
const MAX_PUBLIC_CHARS: usize = 181;
/// Longest accepted message line.
const MAX_MESSAGE_BYTES: usize = 512;

const INIT_PREFIX: &str = "DH1080_INIT ";
const FINISH_PREFIX: &str = "DH1080_FINISH ";
const CBC_SUFFIX: &str = " CBC";

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dh1080Error {
    /// The text is not valid DH1080 base64.
    InvalidEncoding,
    /// The value is 0, 1, p-1, or not smaller than p.
    InvalidPublicKey,
    /// The OS random source failed.
    Random,
}

impl fmt::Display for Dh1080Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEncoding => "invalid DH1080 encoding",
            Self::InvalidPublicKey => "invalid DH1080 public value",
            Self::Random => "random number generation failed",
        })
    }
}

impl std::error::Error for Dh1080Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dh1080Kind {
    Init,
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dh1080Message {
    pub kind: Dh1080Kind,
    /// Peer public value as received (validated to be alphabet-only, not yet range-checked).
    pub public: String,
    pub cbc: bool,
}

pub struct Dh1080Keypair {
    private: BigUint,
    public: BigUint,
}

// The private exponent must never appear in logs.
impl fmt::Debug for Dh1080Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dh1080Keypair").finish_non_exhaustive()
    }
}

fn prime() -> BigUint {
    BigUint::parse_bytes(PRIME_HEX, 16).expect("valid prime constant")
}

/// DH1080 base64: standard alphabet, '=' removed, one 'A' appended when the length is a
/// multiple of 3 (so no '=' was needed).
fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() / 3 * 4 + 5);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    if data.len() % 3 == 0 {
        out.push('A');
    }
    out
}

fn decode(text: &str) -> Result<Vec<u8>, Dh1080Error> {
    let mut bytes = text.as_bytes();
    if bytes.len() > MAX_PUBLIC_CHARS {
        return Err(Dh1080Error::InvalidEncoding);
    }
    if bytes.len() % 4 == 1 && bytes.last() == Some(&b'A') {
        bytes = &bytes[..bytes.len() - 1];
    }
    if bytes.len() % 4 == 1 {
        return Err(Dh1080Error::InvalidEncoding);
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in bytes {
        let v = B64.iter().position(|&a| a == c).ok_or(Dh1080Error::InvalidEncoding)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn to_padded(value: &BigUint) -> Vec<u8> {
    let raw = value.to_bytes_be();
    let mut out = vec![0u8; KEY_BYTES.saturating_sub(raw.len())];
    out.extend_from_slice(&raw);
    out
}

impl Dh1080Keypair {
    /// Generates a keypair from the OS random source: ~1078-bit, top bit set, even exponent
    /// (what OpenSSL 3 produces for this prime; the public value is then a quadratic residue).
    pub fn generate() -> Result<Self, Dh1080Error> {
        let mut buf = [0u8; KEY_BYTES];
        getrandom::fill(&mut buf).map_err(|_| Dh1080Error::Random)?;
        let top_bits = PRIVATE_BITS - (KEY_BYTES - 1) * 8; // bits used in the first byte
        buf[0] &= ((1u16 << top_bits) - 1) as u8;
        buf[0] |= 1 << (top_bits - 1);
        buf[KEY_BYTES - 1] &= !1;
        let private = BigUint::from_bytes_be(&buf);
        buf.zeroize();
        Ok(Self::from_private(private))
    }

    fn from_private(private: BigUint) -> Self {
        let public = BigUint::from(2u8).modpow(&private, &prime());
        Self { private, public }
    }

    /// Public value for the wire: 135 big-endian bytes, 181 characters.
    pub fn public_encoded(&self) -> String {
        encode(&to_padded(&self.public))
    }

    /// Shared secret as 135 left-padded bytes.
    fn shared_secret(&self, peer_public: &str) -> Result<Vec<u8>, Dh1080Error> {
        let p = prime();
        let peer = BigUint::from_bytes_be(&decode(peer_public)?);
        let one = BigUint::from(1u8);
        if peer <= one || peer >= &p - &one {
            return Err(Dh1080Error::InvalidPublicKey);
        }
        Ok(to_padded(&peer.modpow(&self.private, &p)))
    }

    /// Derives the Blowfish key (43 ASCII characters) from the peer's public value.
    ///
    /// The shared secret is left-padded to 135 bytes before hashing, as FiSH 10 does.
    /// FiSH-irssi and weechat hash the minimal-length secret instead (OpenSSL
    /// `DH_compute_key` strips leading zero bytes), so against irssi the keys differ when
    /// the secret has a leading zero byte: roughly 1 in 256 exchanges fail to interoperate.
    pub fn shared_key(&self, peer_public: &str) -> Result<String, Dh1080Error> {
        let mut secret = self.shared_secret(peer_public)?;
        let digest = Sha256::digest(&secret);
        secret.zeroize();
        Ok(encode(&digest))
    }

    /// `DH1080_INIT <public>`, plus ` CBC` when `cbc` is true.
    pub fn init_message(&self, cbc: bool) -> String {
        self.message(INIT_PREFIX, cbc)
    }

    /// `DH1080_FINISH <public>`, plus ` CBC` when `cbc` is true.
    pub fn finish_message(&self, cbc: bool) -> String {
        self.message(FINISH_PREFIX, cbc)
    }

    fn message(&self, prefix: &str, cbc: bool) -> String {
        format!("{prefix}{}{}", self.public_encoded(), if cbc { CBC_SUFFIX } else { "" })
    }
}

/// Checks a peer's public value: its encoding and its range. Nothing is computed, so
/// this can run before the costly exponentiation.
pub fn check_peer_public(peer_public: &str) -> Result<(), Dh1080Error> {
    let p = prime();
    let peer = BigUint::from_bytes_be(&decode(peer_public)?);
    let one = BigUint::from(1u8);
    if peer <= one || peer >= &p - &one {
        return Err(Dh1080Error::InvalidPublicKey);
    }
    Ok(())
}

/// Parses `DH1080_INIT <pub>[ CBC]` / `DH1080_FINISH <pub>[ CBC]`. Anything else is `None`.
pub fn parse_message(line: &str) -> Option<Dh1080Message> {
    if line.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    let (kind, rest) = if let Some(r) = line.strip_prefix(INIT_PREFIX) {
        (Dh1080Kind::Init, r)
    } else if let Some(r) = line.strip_prefix(FINISH_PREFIX) {
        (Dh1080Kind::Finish, r)
    } else {
        return None;
    };
    let (public, cbc) = match rest.strip_suffix(CBC_SUFFIX) {
        Some(r) => (r, true),
        None => (rest, false),
    };
    if public.is_empty() || public.len() > MAX_PUBLIC_CHARS || !public.bytes().all(|c| B64.contains(&c)) {
        return None;
    }
    Some(Dh1080Message { kind, public: public.to_string(), cbc })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_peer_public_accepts_own_value_and_refuses_junk() {
        let kp = Dh1080Keypair::generate().unwrap();
        assert!(check_peer_public(&kp.public_encoded()).is_ok());
        assert!(check_peer_public("not*base64").is_err());
        assert!(check_peer_public("B").is_err());
    }

    // Self-computed vector from the research note: NOT published, NOT checked against a
    // real FiSH client.
    const VEC_A_PUB: &str = "QjjFg9ZygNXuVXLNVs2kKK/9DpVkUAqPPqPs1WETbSwErPzexAKLESwMu9UrkhZ4C8MVjKmbsMlb7sxN4j4riv8PCijVLSTw4vFAMqhLojF/5CidMWl50wNYdWIXPDUjIaXkgBx7ffycUQMtVOmI3yrzQPYZkRjiDqe74vsiYkgEmm0VuHg4A";
    const VEC_B_PUB: &str = "mTSwMqEzd0STKT7TVDTyfAN6N2dGPFfzKcdyeDhj5KWRD/QwwAT9ajpBfR9WSpaBtWZbTocrLjaocQuV/PuWYU6YU95OWJ43RUNd/cWTk3zug17xlzJF1yUdds0+8GI1HiM6+QKClHubSh+YAylJB5g3bs1i6MG1uUg/oyrKT2ODOrQW3ZAIA";
    const VEC_SECRET_HEX: &str = "f1affbbbcbe889f681f96c4e9ff4de50e609f3846e7231dcc3378423ddc1743bdccfd84e577be30ef298a30faee70443a071a16fb08fabc7cee5de384cd9b612bb1de245b35c13ae15ea677b15d3fa68a6de348d7af08c44b364c4d4bf452e2db6a82ec77707be2dce3dd8e4f4fd29f400c0c1ec8853f1d8b7b1b893db5197737767d36736a322";
    const VEC_KEY: &str = "SGd0E5jaMn5Wd94oTo0EbSSPNW4jWRVoRdeVelFS2Hs";

    fn vector_pairs() -> (Dh1080Keypair, Dh1080Keypair) {
        let a = (BigUint::from(1u8) << 1000) + BigUint::from(0x123456u32);
        let b = (BigUint::from(1u8) << 1000) + BigUint::from(0x6789ACu32);
        (Dh1080Keypair::from_private(a), Dh1080Keypair::from_private(b))
    }

    #[test]
    fn dh1080_two_keypairs_agree() {
        let a = Dh1080Keypair::generate().unwrap();
        let b = Dh1080Keypair::generate().unwrap();
        let ka = a.shared_key(&b.public_encoded()).unwrap();
        let kb = b.shared_key(&a.public_encoded()).unwrap();
        assert_eq!(ka, kb);
        assert_eq!(ka.len(), 43);
    }

    #[test]
    fn dh1080_public_encoding_is_181_chars() {
        let a = Dh1080Keypair::generate().unwrap();
        let e = a.public_encoded();
        assert_eq!(e.len(), 181);
        assert!(e.ends_with('A'));
        assert!(!e.contains('='));
    }

    #[test]
    fn dh1080_encoding_lengths() {
        assert_eq!(encode(&[7u8; 135]).len(), 181);
        assert_eq!(encode(&[7u8; 134]).len(), 179);
        assert_eq!(encode(&[7u8; 32]).len(), 43);
        for n in [1usize, 2, 3, 31, 32, 134, 135] {
            let data: Vec<u8> = (0..n).map(|i| (i * 31 + 5) as u8).collect();
            assert_eq!(decode(&encode(&data)).unwrap(), data);
        }
    }

    #[test]
    fn dh1080_rejects_bad_public_values() {
        let kp = Dh1080Keypair::generate().unwrap();
        let p = prime();
        let one = BigUint::from(1u8);
        for bad in [BigUint::from(0u8), one.clone(), &p - &one, p.clone(), &p + &one] {
            let text = encode(&to_padded(&bad));
            assert_eq!(kp.shared_key(&text), Err(Dh1080Error::InvalidPublicKey));
        }
        assert_eq!(kp.shared_key("not*base64"), Err(Dh1080Error::InvalidEncoding));
        assert_eq!(kp.shared_key("B"), Err(Dh1080Error::InvalidEncoding));
    }

    #[test]
    fn dh1080_message_formats() {
        let kp = Dh1080Keypair::generate().unwrap();
        let p = kp.public_encoded();
        assert_eq!(kp.init_message(false), format!("DH1080_INIT {p}"));
        assert_eq!(kp.init_message(true), format!("DH1080_INIT {p} CBC"));
        assert_eq!(kp.finish_message(false), format!("DH1080_FINISH {p}"));
        assert_eq!(kp.finish_message(true), format!("DH1080_FINISH {p} CBC"));
    }

    #[test]
    fn dh1080_parse_roundtrip() {
        let kp = Dh1080Keypair::generate().unwrap();
        let m = parse_message(&kp.init_message(true)).unwrap();
        assert_eq!(m.kind, Dh1080Kind::Init);
        assert!(m.cbc);
        assert_eq!(m.public, kp.public_encoded());
        let m = parse_message(&kp.finish_message(false)).unwrap();
        assert_eq!(m.kind, Dh1080Kind::Finish);
        assert!(!m.cbc);
    }

    #[test]
    fn dh1080_parse_rejects_other_input() {
        let p = Dh1080Keypair::generate().unwrap().public_encoded();
        assert!(parse_message(&format!("DH1080_INIT_cbc {p}")).is_none());
        assert!(parse_message(&format!("DH1080_INIT_CBC {p}")).is_none());
        assert!(parse_message(&format!("dh1080_init {p}")).is_none());
        assert!(parse_message(&format!("DH1080_INIT  {p}")).is_none());
        assert!(parse_message(&format!("DH1080_INIT {p} cbc")).is_none());
        assert!(parse_message(&format!("DH1080_INIT {p} CBC extra")).is_none());
        assert!(parse_message(&format!("DH1080_INIT {p}=")).is_none());
        assert!(parse_message("DH1080_INIT ").is_none());
        assert!(parse_message("DH1080_FINISH  CBC").is_none());
        assert!(parse_message(&format!("HELLO {p}")).is_none());
        assert!(parse_message(&format!("DH1080_INIT {}", "A".repeat(600))).is_none());
        assert!(parse_message(&format!("DH1080_INIT {}", "A".repeat(182))).is_none());
    }

    #[test]
    fn dh1080_self_computed_vector_unpublished() {
        let (a, b) = vector_pairs();
        assert_eq!(a.public_encoded(), VEC_A_PUB);
        assert_eq!(b.public_encoded(), VEC_B_PUB);
        let secret = a.shared_secret(VEC_B_PUB).unwrap();
        let hex: String = secret.iter().map(|x| format!("{x:02x}")).collect();
        assert_eq!(hex, VEC_SECRET_HEX);
        assert_eq!(a.shared_key(VEC_B_PUB).unwrap(), VEC_KEY);
        assert_eq!(b.shared_key(VEC_A_PUB).unwrap(), VEC_KEY);
    }
}
