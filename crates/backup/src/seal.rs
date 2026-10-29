//! A backup file: a header, then the contents sealed with XChaCha20-Poly1305 under a key Argon2id
//! makes of the password. The header is the AEAD's associated data, so no byte of the file can change
//! unnoticed.
//!
//! | bytes | what |
//! |---|---|
//! | 8 | `NORIBACK` |
//! | 1 | format version |
//! | 4, 4, 4 | Argon2id memory (KiB), passes, lanes; little-endian |
//! | 16 | salt |
//! | 24 | nonce |
//! | 32 | the password's check: the second half of Argon2id's 64 bytes (the key is the first) |
//! | rest | the sealed contents and their 16-byte tag |

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::BackupError;

const MAGIC: &[u8; 8] = b"NORIBACK";
pub(crate) const FORMAT: u8 = 1;
const HEADER: usize = 8 + 1 + 12 + 16 + 24 + 32;

/// What Argon2id spends on a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cost {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

/// RFC 9106's second recommended option (64 MiB, 3 passes) in one lane: under a second on a phone.
pub(crate) const COST: Cost = Cost { memory_kib: 64 * 1024, passes: 3, lanes: 1 };

/// The most a file may ask Argon2id for: more is a damaged header, not a backup this nori made.
const MOST: Cost = Cost { memory_kib: 1024 * 1024, passes: 16, lanes: 8 };

/// The key and the password's check.
fn derive(password: &str, salt: &[u8], cost: Cost) -> Result<([u8; 32], [u8; 32]), BackupError> {
    let params = Params::new(cost.memory_kib, cost.passes, cost.lanes, Some(64)).map_err(|_| BackupError::Damaged)?;
    let mut out = [0u8; 64];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(password.as_bytes(), salt, &mut out).map_err(|_| BackupError::Damaged)?;
    let (mut key, mut check) = ([0u8; 32], [0u8; 32]);
    key.copy_from_slice(&out[..32]);
    check.copy_from_slice(&out[32..]);
    Ok((key, check))
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the system has randomness");
    b
}

/// `plain` sealed with `password`.
pub(crate) fn seal(plain: &[u8], password: &str, cost: Cost) -> Result<Vec<u8>, BackupError> {
    if password.is_empty() {
        return Err(BackupError::NoPassword);
    }
    let (salt, nonce) = (random::<16>(), random::<24>());
    let (key, check) = derive(password, &salt, cost)?;
    let mut file = Vec::with_capacity(HEADER + plain.len() + 16);
    file.extend_from_slice(MAGIC);
    file.push(FORMAT);
    for n in [cost.memory_kib, cost.passes, cost.lanes] {
        file.extend_from_slice(&n.to_le_bytes());
    }
    file.extend_from_slice(&salt);
    file.extend_from_slice(&nonce);
    file.extend_from_slice(&check);
    let sealed = XChaCha20Poly1305::new(&key.into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: &file }).expect("a backup fits the cipher");
    file.extend_from_slice(&sealed);
    Ok(file)
}

/// The contents of `file`, opened with `password`.
pub(crate) fn open(file: &[u8], password: &str) -> Result<Vec<u8>, BackupError> {
    if file.len() < MAGIC.len() || &file[..MAGIC.len()] != MAGIC {
        return Err(BackupError::NotABackup);
    }
    if file.len() < HEADER + 16 {
        return Err(BackupError::Damaged);
    }
    if file[8] > FORMAT {
        return Err(BackupError::Newer);
    }
    let n = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().expect("four bytes"));
    let cost = Cost { memory_kib: n(9), passes: n(13), lanes: n(17) };
    if cost.memory_kib > MOST.memory_kib || cost.passes > MOST.passes || cost.lanes > MOST.lanes {
        return Err(BackupError::Damaged);
    }
    let (salt, nonce, check) = (&file[21..37], &file[37..61], &file[61..HEADER]);
    let (key, derived) = derive(password, salt, cost)?;
    if derived != check {
        return Err(BackupError::WrongPassword);
    }
    let (header, sealed) = file.split_at(HEADER);
    XChaCha20Poly1305::new(&key.into()).decrypt(XNonce::from_slice(nonce), Payload { msg: sealed, aad: header }).map_err(|_| BackupError::Damaged)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHEAP: Cost = Cost { memory_kib: 64, passes: 1, lanes: 1 };

    #[test]
    fn opens_with_its_password_only() {
        let file = seal(b"settings", "correct horse", CHEAP).unwrap();
        assert_eq!(open(&file, "correct horse").unwrap(), b"settings");
        assert!(!file.windows(8).any(|w| w == b"settings"), "the contents are not in the clear");
        assert_eq!(open(&file, "correct horsE"), Err(BackupError::WrongPassword));
        assert_eq!(seal(b"x", "", CHEAP), Err(BackupError::NoPassword));
        assert_ne!(seal(b"x", "p", CHEAP).unwrap(), seal(b"x", "p", CHEAP).unwrap(), "a new salt and nonce each time");
    }

    #[test]
    fn any_change_is_refused() {
        let file = seal(b"settings and plugins", "pw", CHEAP).unwrap();
        // Each part of the header, and the contents and tag.
        for at in [8, 9, 21, 37, HEADER, file.len() - 1] {
            let mut bad = file.clone();
            bad[at] ^= 1;
            let got = open(&bad, "pw");
            assert!(got.is_err(), "byte {at} changed unnoticed");
        }
        let mut contents = file.clone();
        contents[HEADER + 3] ^= 0x80;
        assert_eq!(open(&contents, "pw"), Err(BackupError::Damaged));
        let mut salt = file.clone();
        salt[21] ^= 1;
        assert_eq!(open(&salt, "pw"), Err(BackupError::WrongPassword), "another salt is another key");
        assert_eq!(open(&file[..file.len() - 1], "pw"), Err(BackupError::Damaged), "cut short");
        assert_eq!(open(&file[..HEADER], "pw"), Err(BackupError::Damaged));
        assert_eq!(open(b"PK\x03\x04 a zip", "pw"), Err(BackupError::NotABackup));
        let mut newer = file.clone();
        newer[8] = FORMAT + 1;
        assert_eq!(open(&newer, "pw"), Err(BackupError::Newer));
        let mut greedy = file;
        greedy[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(open(&greedy, "pw"), Err(BackupError::Damaged), "a header asking for 4 TiB");
    }
}
