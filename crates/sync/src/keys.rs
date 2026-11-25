//! The group key and what it seals, and a new device's one-time lock. Every primitive is an audited
//! crate's: XChaCha20-Poly1305 and HKDF-SHA256 (RustCrypto), HPKE (RFC 9180) over X25519
//! (x25519-dalek), Argon2id for the words.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hpke::{Deserializable, Kem as _, OpModeR, OpModeS, Serializable};
use sha2::Sha256;

type Kem = hpke::kem::X25519HkdfSha256;
type Kdf = hpke::kdf::HkdfSha256;
type Aead1 = hpke::aead::ChaCha20Poly1305;

/// `N` random bytes from the system.
pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut b = [0; N];
    getrandom::fill(&mut b).expect("the system has randomness");
    b
}

/// HPKE's randomness, from the system.
struct System;

impl rand_core::RngCore for System {
    fn next_u32(&mut self) -> u32 {
        rand_core::impls::next_u32_via_fill(self)
    }
    fn next_u64(&mut self) -> u64 {
        rand_core::impls::next_u64_via_fill(self)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        getrandom::fill(dest).expect("the system has randomness");
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl rand_core::CryptoRng for System {}

pub(crate) fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn unb64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}

/// 32 bytes from their base64url text (43 characters); None for anything else.
pub(crate) fn bytes32(text: &str) -> Option<[u8; 32]> {
    (text.len() == 43).then(|| unb64(text)?.try_into().ok()).flatten()
}

/// A room on the sync server: 32 bytes as 43 base64url characters. Whoever knows it may join.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoomName(String);

impl RoomName {
    /// A new random room, for a meeting.
    pub fn random() -> RoomName {
        RoomName(b64(&random::<32>()))
    }

    /// `text` when it is a room's name.
    pub fn parse(text: &str) -> Option<RoomName> {
        bytes32(text).map(|_| RoomName(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The key every device of a group holds; the server never has it.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupKey([u8; 32]);

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupKey(…)")
    }
}

/// What a group key's seal is for: a sealed message opens only as what it was sealed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sealed {
    /// Said by one device of the group to the others.
    Message,
    /// The kept copy on the server.
    Copy,
}

impl GroupKey {
    pub(crate) fn new() -> GroupKey {
        GroupKey(random())
    }

    pub(crate) fn from_text(text: &str) -> Option<GroupKey> {
        bytes32(text).map(GroupKey)
    }

    pub(crate) fn text(&self) -> String {
        b64(&self.0)
    }

    fn derive(&self, what: &[u8]) -> [u8; 32] {
        let mut out = [0; 32];
        Hkdf::<Sha256>::new(None, &self.0).expand(what, &mut out).expect("32 bytes is a valid HKDF length");
        out
    }

    /// The group room: a one-way fingerprint of the key.
    pub fn room(&self) -> RoomName {
        RoomName(b64(&self.derive(b"nori sync room")))
    }

    /// What the group's devices with synced profile `profile` open sign their requests to each other's
    /// LAN doors with, and announce those doors by: 32 bytes, hex.
    pub(crate) fn lan_secret(&self, profile: &str) -> String {
        self.derive(format!("nori lan door {profile}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// What the kept copy's puts and deletes present; the server learns it with the first put.
    pub(crate) fn copy_secret(&self) -> [u8; 32] {
        self.derive(b"nori sync copy secret")
    }

    /// `plain` sealed as `what`: a random nonce, then the ciphertext and its tag.
    pub(crate) fn seal(&self, what: Sealed, plain: &[u8]) -> Vec<u8> {
        let cipher = XChaCha20Poly1305::new(&self.derive(b"nori sync seal").into());
        let nonce = random::<24>();
        let sealed = cipher.encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: what.aad() }).expect("a message fits");
        [&nonce[..], &sealed].concat()
    }

    /// What [`GroupKey::seal`] sealed as `what` with this key; None for anything else.
    pub(crate) fn open(&self, what: Sealed, sealed: &[u8]) -> Option<Vec<u8>> {
        let (nonce, body) = (sealed.get(..24)?, sealed.get(24..)?);
        let cipher = XChaCha20Poly1305::new(&self.derive(b"nori sync seal").into());
        cipher.decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad: what.aad() }).ok()
    }

    /// A short check that a device holds this key, bound to `lock`; it tells nothing about the key.
    pub(crate) fn tag(&self, lock: &LockKey) -> String {
        let mut out = [0; 16];
        Hkdf::<Sha256>::new(Some(&lock.0), &self.0).expand(b"nori sync member tag", &mut out).expect("16 bytes is a valid HKDF length");
        b64(&out)
    }
}

impl Sealed {
    fn aad(self) -> &'static [u8] {
        match self {
            Sealed::Message => b"nori sync message",
            Sealed::Copy => b"nori sync copy",
        }
    }
}

/// A new device's one-time lock: anyone may seal to its [`LockKey`], only this opens.
pub struct Lock {
    secret: <Kem as hpke::Kem>::PrivateKey,
    key: LockKey,
}

/// A lock's public half, as a pairing code carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockKey([u8; 32]);

impl Lock {
    pub(crate) fn new() -> Lock {
        let (secret, public) = Kem::derive_keypair(&random::<32>());
        Lock { secret, key: LockKey(public.to_bytes().into()) }
    }

    pub(crate) fn key(&self) -> &LockKey {
        &self.key
    }

    /// What [`LockKey::seal`] sealed to this lock for `meeting`; None for anything else.
    pub(crate) fn open(&self, sealed: &[u8], meeting: &RoomName) -> Option<Vec<u8>> {
        let (encapped, body) = (sealed.get(..32)?, sealed.get(32..)?);
        let encapped = <Kem as hpke::Kem>::EncappedKey::from_bytes(encapped).ok()?;
        hpke::single_shot_open::<Aead1, Kdf, Kem>(&OpModeR::Base, &self.secret, &encapped, &self.key.info(meeting), body, &[]).ok()
    }
}

impl LockKey {
    pub(crate) fn from_text(text: &str) -> Option<LockKey> {
        bytes32(text).map(LockKey)
    }

    pub(crate) fn text(&self) -> String {
        b64(&self.0)
    }

    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The HPKE context: a seal opens only for the lock and meeting it was made for.
    fn info(&self, meeting: &RoomName) -> Vec<u8> {
        [b"nori pairing ".as_slice(), meeting.as_str().as_bytes(), b" ", &self.0].concat()
    }

    /// `plain` sealed to this lock for `meeting`: the encapsulated key, then the ciphertext.
    pub(crate) fn seal(&self, plain: &[u8], meeting: &RoomName) -> Option<Vec<u8>> {
        let public = <Kem as hpke::Kem>::PublicKey::from_bytes(&self.0).ok()?;
        let (encapped, sealed) = hpke::single_shot_seal::<Aead1, Kdf, Kem, _>(&OpModeS::Base, &public, &self.info(meeting), plain, &[], &mut System).ok()?;
        Some([&encapped.to_bytes()[..], &sealed].concat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_seal_opens_only_with_its_key_as_what_it_was_sealed_as() {
        let (key, other) = (GroupKey::new(), GroupKey::new());
        let sealed = key.seal(Sealed::Message, b"hello");
        assert_eq!(key.open(Sealed::Message, &sealed).as_deref(), Some(&b"hello"[..]));
        assert_eq!(other.open(Sealed::Message, &sealed), None, "another group's key");
        assert_eq!(key.open(Sealed::Copy, &sealed), None, "a message is no kept copy");
        let mut tampered = sealed.clone();
        tampered[30] ^= 1;
        assert_eq!(key.open(Sealed::Message, &tampered), None, "tampered");
        assert_eq!(key.open(Sealed::Message, &sealed[..20]), None, "cut short");
        assert_ne!(key.seal(Sealed::Message, b"hello"), sealed, "a new nonce each time");
    }

    #[test]
    fn the_room_is_a_fingerprint_of_the_key() {
        let key = GroupKey::new();
        let room = key.room();
        assert_eq!(room.as_str().len(), 43);
        assert_eq!(RoomName::parse(room.as_str()), Some(room.clone()));
        assert_eq!(GroupKey::from_text(&key.text()).unwrap().room(), room, "the same key, the same room");
        assert_ne!(GroupKey::new().room(), room);
        assert!(!room.as_str().contains(&key.text()) && room.as_str() != key.text());
        assert_ne!(key.copy_secret(), key.derive(b"nori sync seal"), "each use its own key");
    }

    #[test]
    fn a_seal_to_a_lock_opens_only_with_that_lock_for_that_meeting() {
        let (lock, other) = (Lock::new(), Lock::new());
        let (meeting, elsewhere) = (RoomName::random(), RoomName::random());
        let sealed = lock.key().seal(b"key", &meeting).unwrap();
        assert_eq!(lock.open(&sealed, &meeting).as_deref(), Some(&b"key"[..]));
        assert_eq!(other.open(&sealed, &meeting), None, "another lock");
        assert_eq!(lock.open(&sealed, &elsewhere), None, "another meeting");
        let mut tampered = sealed.clone();
        tampered[40] ^= 1;
        assert_eq!(lock.open(&tampered, &meeting), None, "tampered");
        assert_eq!(LockKey::from_text(&lock.key().text()), Some(lock.key().clone()));
    }

    #[test]
    fn texts_that_are_not_32_bytes_are_refused() {
        for text in ["", "abc", &"A".repeat(42), &"A".repeat(44), &format!("{}*", "A".repeat(42))] {
            assert_eq!(RoomName::parse(text), None, "{text:?}");
            assert!(GroupKey::from_text(text).is_none(), "{text:?}");
        }
    }
}
