//! FiSH-compatible Blowfish encryption and decryption (ECB and CBC).
//!
//! Wire behaviour follows the FiSH-irssi ECB layout and the FiSH 10 CBC
//! mode as documented by its author. The FiSH behaviour is not defined by the client
//! so no claim is made about the FiSH plugin itself.
//!
//! Two wire formats are supported:
//! - ECB: "+OK <ciphertext>" using the FiSH-specific base64 alphabet. Each 8-byte block is
//!   split into two 32-bit halves; the right half is written first, as 6-character groups.
//! - CBC (FiSH 10): "+OK *<payload>" where the payload is standard base64
//!   of an 8-byte IV followed by the Blowfish-CBC ciphertext (zero padded).
//!   Keys stored as "cbc:<key>" select CBC for outgoing messages; the prefix is not part
//!   of the Blowfish key.

use blowfish::Blowfish;
use byteorder::BigEndian;
use cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use cipher::generic_array::GenericArray;

const FISH_BASE64: &[u8; 64] = b"./0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const STD_BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Marker used in blow.ini / key entry to select CBC mode ("cbc:<key>").
const CBC_KEY_MARKER: &[u8] = b"cbc:";
const BLOCK_SIZE: usize = 8;

pub struct FishCipher {
    key: Vec<u8>,
    cbc: bool,
}

impl FishCipher {
    /// Creates a cipher from a stored key. A leading "cbc:" marker selects CBC mode for
    /// encryption and is not part of the actual Blowfish key.
    pub fn new(key: &str) -> Self {
        let raw = key.as_bytes();
        if raw.len() >= CBC_KEY_MARKER.len()
            && raw[..CBC_KEY_MARKER.len()].eq_ignore_ascii_case(CBC_KEY_MARKER)
        {
            Self {
                key: raw[CBC_KEY_MARKER.len()..].to_vec(),
                cbc: true,
            }
        } else {
            Self {
                key: raw.to_vec(),
                cbc: false,
            }
        }
    }

    /// Returns true if the key was marked as CBC ("cbc:" prefix).
    pub fn is_cbc(&self) -> bool {
        self.cbc
    }

    /// Encrypts plaintext message into FiSH format: "+OK *" (CBC) if the key is marked
    /// "cbc:", otherwise "+OK " (ECB).
    pub fn encrypt(&self, message: &str) -> Result<String, String> {
        if message.is_empty() {
            return Ok(String::new());
        }
        if self.cbc {
            self.encrypt_cbc(message)
        } else {
            self.encrypt_ecb(message)
        }
    }

    /// Encrypts into the CBC format "+OK *<base64(IV + ciphertext)>" with a random IV,
    /// regardless of the key marker. An empty message yields an IV-only payload.
    pub fn encrypt_cbc(&self, message: &str) -> Result<String, String> {
        let mut iv = [0u8; BLOCK_SIZE];
        getrandom::fill(&mut iv).map_err(|e| format!("Random IV generation failed: {}", e))?;
        self.encrypt_cbc_with_iv(message, iv)
    }

    fn encrypt_cbc_with_iv(&self, message: &str, iv: [u8; BLOCK_SIZE]) -> Result<String, String> {
        let cipher = Blowfish::<BigEndian>::new_from_slice(&self.key)
            .map_err(|e| format!("Invalid key length: {}", e))?;

        // Zero-pad to multiple of 8 bytes
        let mut padded = message.as_bytes().to_vec();
        let rem = padded.len() % BLOCK_SIZE;
        if rem != 0 {
            padded.resize(padded.len() + (BLOCK_SIZE - rem), 0);
        }

        let mut payload = Vec::with_capacity(BLOCK_SIZE + padded.len());
        payload.extend_from_slice(&iv);

        let mut prev = iv;
        for chunk in padded.chunks(BLOCK_SIZE) {
            let mut block = GenericArray::clone_from_slice(chunk);
            for (b, p) in block.iter_mut().zip(prev.iter()) {
                *b ^= p;
            }
            cipher.encrypt_block(&mut block);
            prev.copy_from_slice(&block);
            payload.extend_from_slice(&block);
        }

        Ok(format!("+OK *{}", std_b64_encode(&payload)))
    }

    /// Decrypts the payload of a CBC message (standard base64 of IV + ciphertext).
    fn decrypt_cbc_payload(&self, payload: &str) -> Result<String, String> {
        // Take only the base64 run (ignoring trailing crypt marks)
        let b64: Vec<u8> = payload
            .bytes()
            .take_while(|b| *b == b'=' || STD_BASE64.contains(b))
            .collect();
        let data = std_b64_decode(&b64)?;

        if data.len() < BLOCK_SIZE {
            return Err("FiSH CBC payload too short (missing IV)".into());
        }
        if data.len() % BLOCK_SIZE != 0 {
            return Err("FiSH CBC payload length is not a multiple of 8 bytes".into());
        }

        let cipher = Blowfish::<BigEndian>::new_from_slice(&self.key)
            .map_err(|e| format!("Invalid key length: {}", e))?;

        let (iv, ciphertext) = data.split_at(BLOCK_SIZE);
        let mut prev = [0u8; BLOCK_SIZE];
        prev.copy_from_slice(iv);

        let mut decrypted_bytes = Vec::with_capacity(ciphertext.len());
        for chunk in ciphertext.chunks(BLOCK_SIZE) {
            let mut block = GenericArray::clone_from_slice(chunk);
            cipher.decrypt_block(&mut block);
            for (b, p) in block.iter_mut().zip(prev.iter()) {
                *b ^= p;
            }
            prev.copy_from_slice(chunk);
            decrypted_bytes.extend_from_slice(&block);
        }

        // Trim trailing zeros
        while decrypted_bytes.last() == Some(&0) {
            decrypted_bytes.pop();
        }

        Ok(bytes_to_text(decrypted_bytes))
    }

    fn encrypt_ecb(&self, message: &str) -> Result<String, String> {
        let msg_bytes = message.as_bytes();
        if msg_bytes.is_empty() {
            return Ok(String::new());
        }

        let cipher = Blowfish::<BigEndian>::new_from_slice(&self.key)
            .map_err(|e| format!("Invalid key length: {}", e))?;

        // Zero-pad to multiple of 8 bytes
        let mut padded = msg_bytes.to_vec();
        let rem = padded.len() % 8;
        if rem != 0 {
            padded.resize(padded.len() + (8 - rem), 0);
        }

        let mut output = String::from("+OK ");

        // Process 8-byte blocks
        for chunk in padded.chunks(8) {
            let mut block = GenericArray::clone_from_slice(chunk);
            cipher.encrypt_block(&mut block);

            let left = u32::from_be_bytes([block[0], block[1], block[2], block[3]]);
            let right = u32::from_be_bytes([block[4], block[5], block[6], block[7]]);

            // Right word -> 6 base64 chars (little-endian base64 bits)
            let mut r = right;
            for _ in 0..6 {
                let idx = (r & 0x3F) as usize;
                output.push(FISH_BASE64[idx] as char);
                r >>= 6;
            }

            // Left word -> 6 base64 chars
            let mut l = left;
            for _ in 0..6 {
                let idx = (l & 0x3F) as usize;
                output.push(FISH_BASE64[idx] as char);
                l >>= 6;
            }
        }

        Ok(output)
    }

    /// Decrypts "+OK <ciphertext>", "mcps <ciphertext>", or raw FiSH string into plaintext.
    /// CBC messages ("+OK *<payload>" / "mcps *<payload>") are detected by prefix, independent
    /// of the key marker.
    /// Tolerates trailing crypt-marks (e.g. " 12· "), padding, and non-UTF8/Windows-1252 bytes.
    pub fn decrypt(&self, ciphertext: &str) -> Result<String, String> {
        let mut raw = ciphertext.trim();
        if let Some(payload) = cbc_payload(raw) {
            return self.decrypt_cbc_payload(payload);
        }
        if let Some(rest) = raw.strip_prefix("+OK ") {
            raw = rest;
        } else if let Some(rest) = raw.strip_prefix("mcps ") {
            raw = rest;
        } else if let Some(rest) = raw.strip_prefix("+OK*") {
            raw = rest;
        } else if let Some(rest) = raw.strip_prefix("+OK") {
            raw = rest;
        }

        // Take only valid FiSH Base64 characters (ignoring trailing crypt marks like "   \x0312·  ")
        let valid_b64: Vec<u8> = raw
            .bytes()
            .take_while(|b| FISH_BASE64.contains(b))
            .collect();

        // Must be multiple of 12 (each 12 Base64 chars decode to an 8-byte Blowfish block)
        let block_len = valid_b64.len() - (valid_b64.len() % 12);
        if block_len == 0 {
            return Err("FiSH ciphertext contains no complete 12-char blocks".into());
        }

        let bytes = &valid_b64[..block_len];

        let cipher = Blowfish::<BigEndian>::new_from_slice(&self.key)
            .map_err(|e| format!("Invalid key length: {}", e))?;

        let mut decrypted_bytes = Vec::new();

        for chunk in bytes.chunks(12) {
            // First 6 chars = right word
            let mut right: u32 = 0;
            for (i, &b) in chunk[0..6].iter().enumerate() {
                let val = b64_index(b)? as u32;
                right |= val << (i * 6);
            }

            // Next 6 chars = left word
            let mut left: u32 = 0;
            for (i, &b) in chunk[6..12].iter().enumerate() {
                let val = b64_index(b)? as u32;
                left |= val << (i * 6);
            }

            let mut block = GenericArray::clone_from_slice(&[
                (left >> 24) as u8,
                (left >> 16) as u8,
                (left >> 8) as u8,
                left as u8,
                (right >> 24) as u8,
                (right >> 16) as u8,
                (right >> 8) as u8,
                right as u8,
            ]);

            cipher.decrypt_block(&mut block);
            decrypted_bytes.extend_from_slice(&block);
        }

        // Trim trailing zeros
        while decrypted_bytes.last() == Some(&0) {
            decrypted_bytes.pop();
        }

        Ok(bytes_to_text(decrypted_bytes))
    }
}

/// Returns the base64 payload if `raw` is a CBC message ("+OK *..." or "mcps *...").
fn cbc_payload(raw: &str) -> Option<&str> {
    raw.strip_prefix("+OK *")
        .or_else(|| raw.strip_prefix("mcps *"))
}

/// Decode as UTF-8 with fallback to Windows-1252 / ISO-8859-1 for IRC compatibility
fn bytes_to_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            // Fallback byte-to-char mapping for Windows-1252 / Latin-1
            e.into_bytes().iter().map(|&b| b as char).collect()
        }
    }
}

/// Standard base64 (RFC 4648, with '=' padding) as used by the CBC format.
fn std_b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(STD_BASE64[(n >> 18) as usize & 0x3F] as char);
        out.push(STD_BASE64[(n >> 12) as usize & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(STD_BASE64[(n >> 6) as usize & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(STD_BASE64[n as usize & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Standard base64 decoder. Accepts missing padding (truncated or stripped), rejects
/// invalid characters, misplaced padding and impossible lengths.
fn std_b64_decode(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut end = input.len();
    while end > 0 && input[end - 1] == b'=' {
        end -= 1;
    }
    let pad = input.len() - end;
    if pad > 2 || (pad > 0 && input.len() % 4 != 0) {
        return Err("Invalid base64 padding".into());
    }
    let body = &input[..end];
    if body.len() % 4 == 1 {
        return Err("Invalid base64 length".into());
    }

    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in body {
        let val = STD_BASE64
            .iter()
            .position(|&x| x == c)
            .ok_or_else(|| format!("Invalid base64 char: '{}'", c as char))? as u32;
        acc = (acc << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Decrypts a key from blow.ini if it was encrypted with the standard master key "blowinikey"
pub fn decrypt_ini_key(raw_val: &str) -> String {
    let trimmed = raw_val.trim();
    if trimmed.starts_with("+OK ") {
        let master = FishCipher::new("blowinikey");
        master.decrypt(trimmed).unwrap_or_else(|_| trimmed.to_string())
    } else {
        trimmed.to_string()
    }
}

/// Encrypts a plaintext key for saving into blow.ini using master key "blowinikey" with "+OK " prefix
pub fn encrypt_ini_key(plain_key: &str) -> String {
    let trimmed = plain_key.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let master = FishCipher::new("blowinikey");
    master.encrypt(trimmed).unwrap_or_else(|_| trimmed.to_string())
}

fn b64_index(b: u8) -> Result<usize, String> {
    FISH_BASE64
        .iter()
        .position(|&x| x == b)
        .ok_or_else(|| format!("Invalid FiSH base64 char: '{}'", b as char))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fish_roundtrip() {
        let fish = FishCipher::new("secret_ferric_key_123");
        let plain = "Hello from Ferric secure chat!";
        let enc = fish.encrypt(plain).unwrap();
        assert!(enc.starts_with("+OK "));
        let dec = fish.decrypt(&enc).unwrap();
        assert_eq!(plain, dec);

        // Test blowinikey decryption on real blow.ini sample:
        // [NUKLEOTIDE:#demo] -> key=+OK EF1B81aGvQA.
        let dec_key = decrypt_ini_key("+OK EF1B81aGvQA.");
        println!("Decrypted key from blow.ini: '{}'", dec_key);
        assert!(!dec_key.is_empty());

        // Test roundtrip with encrypt_ini_key
        let enc_key = encrypt_ini_key(&dec_key);
        let dec_again = decrypt_ini_key(&enc_key);
        assert_eq!(dec_key, dec_again);

        // Test with trailing crypt-mark (e.g. \x0312· or spaces)
        let msg = "Confidential chat line";
        let cipher = FishCipher::new("secret123");
        let enc_msg = cipher.encrypt(msg).unwrap();
        let enc_with_mark = format!("{}   \x0312\u{00B7}  ", enc_msg);
        let dec_marked = cipher.decrypt(&enc_with_mark).unwrap();
        assert_eq!(msg, dec_marked);

        // Test with mcps prefix
        let enc_mcps = enc_msg.replace("+OK ", "mcps ");
        let dec_mcps = cipher.decrypt(&enc_mcps).unwrap();
        assert_eq!(msg, dec_mcps);
    }

    const TEST_CBC_KEY: &str = "testkey_cbc_123";

    #[test]
    fn test_cbc_roundtrip_lengths() {
        let fish = FishCipher::new("cbc:testkey_cbc_123");
        for len in [0usize, 1, 7, 8, 9, 31] {
            let plain: String = "abcdefghijklmnopqrstuvwxyz0123456789"
                .chars()
                .cycle()
                .take(len)
                .collect();
            let enc = fish.encrypt_cbc(&plain).unwrap();
            assert!(enc.starts_with("+OK *"), "len {}", len);
            let dec = fish.decrypt(&enc).unwrap();
            assert_eq!(plain, dec, "len {}", len);
        }
    }

    #[test]
    fn test_cbc_random_iv_differs() {
        let fish = FishCipher::new("cbc:testkey_cbc_123");
        let a = fish.encrypt("same message").unwrap();
        let b = fish.encrypt("same message").unwrap();
        assert_ne!(a, b);
        assert_eq!(fish.decrypt(&a).unwrap(), "same message");
        assert_eq!(fish.decrypt(&b).unwrap(), "same message");
    }

    #[test]
    fn test_cbc_utf8_roundtrip() {
        let fish = FishCipher::new("cbc:testkey_cbc_123");
        let plain = "Gr\u{00FC}\u{00DF}e aus K\u{00F6}ln \u{1F600}";
        let enc = fish.encrypt(plain).unwrap();
        assert_eq!(fish.decrypt(&enc).unwrap(), plain);
    }

    #[test]
    fn test_cbc_known_vectors() {
        // Independently computed with OpenSSL Blowfish-CBC (Python cryptography), not
        // produced by FiSH itself. Layout: base64(IV || CBC(zero padded plaintext)).
        let fish = FishCipher::new(TEST_CBC_KEY);

        let iv = [1u8, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            fish.encrypt_cbc_with_iv("Hello CBC world!", iv).unwrap(),
            "+OK *AQIDBAUGBwijzCLvx1TjT6jupzGtj0Tg"
        );
        assert_eq!(
            fish.decrypt("+OK *AQIDBAUGBwijzCLvx1TjT6jupzGtj0Tg").unwrap(),
            "Hello CBC world!"
        );
        assert_eq!(
            fish.encrypt_cbc_with_iv("hi", iv).unwrap(),
            "+OK *AQIDBAUGBwhxElMj1lqF2A=="
        );

        // FiSH-irssi style: zero IV, first block is encrypted random data (discarded on
        // decrypt), which is equivalent to treating that first ciphertext block as IV.
        assert_eq!(fish.decrypt("+OK *nnoVhVtuADOeDzPO1Bdobg==").unwrap(), "hi");
    }

    #[test]
    fn test_cbc_malformed_inputs() {
        let fish = FishCipher::new(TEST_CBC_KEY);
        // empty payload
        assert!(fish.decrypt("+OK *").is_err());
        // bad base64 characters
        assert!(fish.decrypt("+OK *!!!!!!!!!!!!").is_err());
        // too short: fewer than 8 bytes (no complete IV)
        assert!(fish.decrypt("+OK *AQIDBA==").is_err());
        // base64 length impossible (4n+1)
        assert!(fish.decrypt("+OK *AQIDB").is_err());
        // misplaced / excessive padding
        assert!(fish.decrypt("+OK *AQID=BAU").is_err());
        assert!(fish.decrypt("+OK *A===").is_err());
        // 9 bytes: IV plus partial block (not a multiple of 8)
        assert!(fish.decrypt("+OK *AQIDBAUGBwhh").is_err());
        // 12 bytes: not a multiple of 8
        assert!(fish.decrypt("+OK *AQIDBAUGBwgJCgsM").is_err());
        // no panic on odd inputs
        for s in ["+OK *=", "+OK *==", "mcps *", "+OK * ", "+OK *\u{00B7}", "+OK **"] {
            let _ = fish.decrypt(s);
        }
    }

    #[test]
    fn test_cbc_trailing_mark_and_mcps_prefix() {
        let fish = FishCipher::new("cbc:testkey_cbc_123");
        let enc = fish.encrypt("marked line").unwrap();
        let marked = format!("{}   \x0312\u{00B7}  ", enc);
        assert_eq!(fish.decrypt(&marked).unwrap(), "marked line");
        let mcps = enc.replacen("+OK *", "mcps *", 1);
        assert_eq!(fish.decrypt(&mcps).unwrap(), "marked line");
    }

    #[test]
    fn test_cbc_wrong_key_does_not_panic() {
        let enc = FishCipher::new("cbc:testkey_cbc_123").encrypt("secret").unwrap();
        let other = FishCipher::new("another_test_key");
        if let Ok(dec) = other.decrypt(&enc) {
            assert_ne!(dec, "secret");
        }
    }

    #[test]
    fn test_ecb_still_works() {
        let fish = FishCipher::new("plain_ecb_key");
        assert!(!fish.is_cbc());
        let enc = fish.encrypt("ecb message").unwrap();
        assert!(enc.starts_with("+OK "));
        assert!(!enc.starts_with("+OK *"));
        assert_eq!(fish.decrypt(&enc).unwrap(), "ecb message");
        // Empty messages stay empty for both modes
        assert_eq!(fish.encrypt("").unwrap(), "");
        assert_eq!(FishCipher::new("cbc:k").encrypt("").unwrap(), "");
    }

    #[test]
    fn test_external_interop_vectors() {
        // ECB vectors from whiym/fish (Go, MIT; FiSH-irssi layout), key "blowfish".
        // Checked in both directions, so the encoder and decoder are tested independently.
        let ecb = FishCipher::new("blowfish");
        let ecb_vectors = [
            ("qwertyui", "+OK qKKBR.riU410"),
            ("qwerty", "+OK Ee5Cz.Dgr1q1"),
            ("qwertyuiop", "+OK qKKBR.riU410bpm4f/1UAmL0"),
            (
                "qwertyuiopasdfghjklzxcvbnm1234567890",
                "+OK qKKBR.riU410sK.30/GVDts.ewTqf.5tm3p/Ol.d.0DH23r/m9tTw.Rmpbj1",
            ),
        ];
        for (plain, wire) in ecb_vectors {
            assert_eq!(ecb.encrypt(plain).unwrap(), wire, "encrypt {plain}");
            assert_eq!(ecb.decrypt(wire).unwrap(), plain, "decrypt {plain}");
        }

        // CBC vectors from the same project, key "blowfish" with the "cbc:" marker. That
        // project omits the "*" CBC marker; this crate requires it, so it is added here.
        // Only the payload layout (IV block, then CBC blocks) is tested by this.
        let cbc = FishCipher::new("cbc:blowfish");
        for wire in [
            "+OK o2BNTv01lAIXRSFP+Efw/w==",
            "+OK JU548T93fNLB+N+cnOZJpA==",
            "+OK d6HO0xqR1yrWngq6XxEYqA==",
            "+OK IDOeaXOWlGhDWxKXX4VRow==",
        ] {
            let starred = wire.replacen("+OK ", "+OK *", 1);
            assert_eq!(cbc.decrypt(&starred).unwrap(), "qwertyui", "{starred}");
        }
    }

    #[test]
    fn test_cbc_key_marker_handling() {
        let cbc = FishCipher::new("cbc:testkey_cbc_123");
        assert!(cbc.is_cbc());
        let upper = FishCipher::new("CBC:testkey_cbc_123");
        assert!(upper.is_cbc());

        // Marker is stripped: the key without marker decrypts the same message
        let enc = cbc.encrypt("marker test").unwrap();
        assert!(enc.starts_with("+OK *"));
        let bare = FishCipher::new("testkey_cbc_123");
        assert!(!bare.is_cbc());
        assert_eq!(bare.decrypt(&enc).unwrap(), "marker test");
        assert_eq!(upper.decrypt(&enc).unwrap(), "marker test");

        // Incoming detection is by prefix, regardless of key marker
        let ecb_enc = bare.encrypt("ecb via bare").unwrap();
        assert_eq!(cbc.decrypt(&ecb_enc).unwrap(), "ecb via bare");

        // Unmarked key encrypts ECB, marked key encrypts CBC
        assert!(bare.encrypt("x").unwrap().starts_with("+OK "));
        assert!(!bare.encrypt("x").unwrap().starts_with("+OK *"));

        // Marker only, no key: invalid key length error, no panic
        assert!(FishCipher::new("cbc:").encrypt("x").is_err());
        // Short non-marker strings
        assert!(!FishCipher::new("cb").is_cbc());
    }
}
