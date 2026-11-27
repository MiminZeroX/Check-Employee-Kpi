//! Time-based one-time passwords (RFC 6238, SHA-1, 6 digits, 30 s) as used by
//! Google Authenticator, Microsoft Authenticator, 1Password, etc.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
pub const STEP: i64 = 30;

pub fn new_secret() -> String {
    let mut b = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut b);
    base32_encode(&b)
}

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(ALPHABET[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '=') {
        let v = ALPHABET.iter().position(|&a| a as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

pub fn code_at(key: &[u8], step: i64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("hmac key");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    let bin = ((h[o] as u32 & 0x7f) << 24) | ((h[o + 1] as u32) << 16) | ((h[o + 2] as u32) << 8) | h[o + 3] as u32;
    bin % 1_000_000
}

/// Checks `code` against the current step ±1 (clock drift). Returns the
/// matching step, which must be greater than the last one used (no replay).
pub fn verify(secret_b32: &str, code: &str, now_unix: i64, last_step: Option<i64>) -> Option<i64> {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let want: u32 = code.parse().ok()?;
    let key = base32_decode(secret_b32)?;
    let cur = now_unix.div_euclid(STEP);
    (cur - 1..=cur + 1)
        .filter(|s| last_step.map_or(true, |l| *s > l))
        .find(|s| code_at(&key, *s) == want)
}

pub fn otpauth_uri(issuer: &str, account: &str, secret: &str) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect::<String>()
    };
    format!("otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30", enc(issuer), enc(account), secret, enc(issuer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors() {
        // RFC 6238 appendix B, SHA-1 key "12345678901234567890" (8 digits there; 6 = last six)
        let key = b"12345678901234567890";
        assert_eq!(code_at(key, 59 / 30), 287082);
        assert_eq!(code_at(key, 1111111109 / 30), 81804);
        assert_eq!(code_at(key, 2000000000 / 30), 279037);
    }

    #[test]
    fn base32_roundtrip_and_replay() {
        let s = new_secret();
        assert_eq!(s.len(), 32);
        let key = base32_decode(&s).unwrap();
        assert_eq!(base32_encode(&key), s);
        let now = 1_800_000_000;
        let c = format!("{:06}", code_at(&key, now / 30));
        let step = verify(&s, &c, now, None).unwrap();
        assert!(verify(&s, &c, now, Some(step)).is_none());
        assert!(verify(&s, "000000x", now, None).is_none());
    }
}
