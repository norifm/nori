//! A profile's client certificate: its PKCS#12 file (`.p12`), kept under the data directory, read into
//! the key and chain TLS sends. Its bags may be in the clear, encrypted with PBES2 (PBKDF2 and AES-CBC:
//! OpenSSL 3's default) or with PKCS#12's own PBE (3DES and RC2 over SHA-1: older exports); its MAC is
//! checked with the password first.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, InnerIvInit, KeyIvInit};
use nori_settings::settings::SavedServer;
use ring::{digest, hmac, pbkdf2};

/// Why a client certificate was not taken. Messages are for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum CertError {
    /// Its file could not be read or kept.
    #[error("the file could not be read or kept")]
    File,
    /// The password does not open it.
    #[error("the password does not open it")]
    Password,
    /// It is no PKCS#12 file, or one without a key and a certificate.
    #[error("no PKCS#12 file with a key and a certificate")]
    NotPkcs12,
    /// It is encrypted in a way not read here.
    #[error("encrypted in a way not read here")]
    Unsupported,
}

/// A client certificate as TLS sends it: its private key (PKCS#8 DER) and its chain (DER), its own first.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Identity {
    pub key: Vec<u8>,
    pub chain: Vec<Vec<u8>>,
}

/// The directory under the data directory the client certificates are kept in.
fn certs(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("certs")
}

/// `profile` with PKCS#12 file `p12` as its client certificate, opened with `password`: checked, then
/// kept under `data_dir` in a file of its own, named by the profile's id and the file's contents. The
/// file the saved profile uses stays as it is until the profile is saved.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn import_client_cert(data_dir: &str, profile: SavedServer, p12: Vec<u8>, password: String) -> Result<SavedServer, CertError> {
    read(&p12, &password)?;
    let hash = digest::digest(&digest::SHA256, &p12);
    let name = format!("{}-{}.p12", profile.id, hash.as_ref()[..8].iter().map(|b| format!("{b:02x}")).collect::<String>());
    let dir = certs(data_dir);
    std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(&name), &p12)).map_err(|_| CertError::File)?;
    Ok(SavedServer { client_cert: name, client_cert_password: password, ..profile })
}

/// [`import_client_cert`] of the PKCS#12 file at `from`.
pub fn import_client_cert_file(data_dir: &Path, profile: SavedServer, from: &Path, password: String) -> Result<SavedServer, CertError> {
    let p12 = std::fs::read(from).map_err(|_| CertError::File)?;
    import_client_cert(&data_dir.to_string_lossy(), profile, p12, password)
}

/// `profile`'s client certificate, kept under `data_dir`; None without one.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn client_identity(data_dir: &str, profile: SavedServer) -> Result<Option<Identity>, CertError> {
    if profile.client_cert.is_empty() {
        return Ok(None);
    }
    let p12 = std::fs::read(certs(data_dir).join(&profile.client_cert)).map_err(|_| CertError::File)?;
    read(&p12, &profile.client_cert_password).map(Some)
}

/// Removes the client certificates kept under `data_dir` that none of `servers` uses: those of profiles
/// removed or given another, and those imported in a form left unsaved. Called as a client starts, before
/// a form can import one.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn forget_unused_client_certs(data_dir: &str, servers: Vec<SavedServer>) {
    let Ok(kept) = std::fs::read_dir(certs(data_dir)) else { return };
    for file in kept.flatten() {
        if !servers.iter().any(|s| !s.client_cert.is_empty() && file.file_name() == s.client_cert.as_str()) {
            let _ = std::fs::remove_file(file.path());
        }
    }
}

/// The key and chain of PKCS#12 file `p12`, opened with `password`.
fn read(p12: &[u8], password: &str) -> Result<Identity, CertError> {
    let mut pfx = Der(p12).seq()?;
    pfx.expect(INTEGER)?;
    let mut auth = pfx.seq()?;
    // The other integrity mode, a public-key signature, has no password.
    if auth.expect(OID)? != DATA {
        return Err(CertError::Unsupported);
    }
    let content = Der(auth.expect(EXPLICIT_0)?).expect(OCTET_STRING)?;
    let bmp = bmp(password);
    if !pfx.is_empty() {
        check_mac(pfx.seq()?, content, &bmp)?;
    }
    let pw = Password { text: password, bmp: &bmp };
    let (mut keys, mut certs) = (Vec::new(), Vec::new());
    let mut safes = Der(content).seq()?;
    while !safes.is_empty() {
        let mut info = safes.seq()?;
        let kind = info.expect(OID)?;
        let mut inner = Der(info.expect(EXPLICIT_0)?);
        let bags = match kind {
            DATA => inner.expect(OCTET_STRING)?.to_vec(),
            ENCRYPTED_DATA => {
                let mut data = inner.seq()?;
                data.expect(INTEGER)?;
                let mut content = data.seq()?;
                content.expect(OID)?;
                let algorithm = content.seq()?;
                decrypt(algorithm, content.expect(IMPLICIT_0)?, &pw)?
            }
            _ => return Err(CertError::Unsupported),
        };
        read_bags(&bags, &pw, &mut keys, &mut certs)?;
    }
    let Some((key, id)) = keys.into_iter().next() else { return Err(CertError::NotPkcs12) };
    // Its own certificate first: the one with the key's id, else the first.
    let own = certs.iter().position(|(_, c)| id.is_some() && *c == id).unwrap_or(0);
    if certs.is_empty() {
        return Err(CertError::NotPkcs12);
    }
    let first = certs.remove(own).0;
    Ok(Identity { key, chain: std::iter::once(first).chain(certs.into_iter().map(|(c, _)| c)).collect() })
}

/// A password as PBES2 takes it (its UTF-8) and as PKCS#12's own derivation does (BMP).
struct Password<'a> {
    text: &'a str,
    bmp: &'a [u8],
}

/// `password` as a BMPString with its two-byte end, as PKCS#12's derivation takes it.
fn bmp(password: &str) -> Vec<u8> {
    password.encode_utf16().flat_map(u16::to_be_bytes).chain([0, 0]).collect()
}

/// Keys or certificates found, each with its local key id.
type Found = Vec<(Vec<u8>, Option<Vec<u8>>)>;

/// The keys and certificates of SafeContents `safe`.
fn read_bags(safe: &[u8], pw: &Password, keys: &mut Found, certs: &mut Found) -> Result<(), CertError> {
    let mut bags = Der(safe).seq()?;
    while !bags.is_empty() {
        let mut bag = bags.seq()?;
        let kind = bag.expect(OID)?;
        let mut value = Der(bag.expect(EXPLICIT_0)?);
        let id = if bag.is_empty() { None } else { local_key_id(Der(bag.expect(SET)?))? };
        match kind {
            KEY_BAG => keys.push((value.element()?.to_vec(), id)),
            SHROUDED_KEY_BAG => {
                let mut sealed = value.seq()?;
                let algorithm = sealed.seq()?;
                keys.push((decrypt(algorithm, sealed.expect(OCTET_STRING)?, pw)?, id));
            }
            CERT_BAG => {
                let mut cert = value.seq()?;
                if cert.expect(OID)? == X509_CERTIFICATE {
                    certs.push((Der(cert.expect(EXPLICIT_0)?).expect(OCTET_STRING)?.to_vec(), id));
                }
            }
            // Secrets and revocation lists say nothing TLS sends.
            _ => {}
        }
    }
    Ok(())
}

/// The local key id among a bag's attributes.
fn local_key_id(mut attributes: Der) -> Result<Option<Vec<u8>>, CertError> {
    while !attributes.is_empty() {
        let mut attribute = attributes.seq()?;
        if attribute.expect(OID)? == LOCAL_KEY_ID {
            return Ok(Some(Der(attribute.expect(SET)?).expect(OCTET_STRING)?.to_vec()));
        }
    }
    Ok(None)
}

/// Checks MacData `mac` over `content` with password `bmp`.
fn check_mac(mut mac: Der, content: &[u8], bmp: &[u8]) -> Result<(), CertError> {
    let mut digest_info = mac.seq()?;
    let algorithm = digest_info.seq()?.expect(OID)?;
    let want = digest_info.expect(OCTET_STRING)?;
    let salt = mac.expect(OCTET_STRING)?;
    let rounds = if mac.is_empty() { 1 } else { number(mac.expect(INTEGER)?)? };
    let (hash, keyed) = match algorithm {
        SHA1 => (&digest::SHA1_FOR_LEGACY_USE_ONLY, hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY),
        SHA256 => (&digest::SHA256, hmac::HMAC_SHA256),
        SHA384 => (&digest::SHA384, hmac::HMAC_SHA384),
        SHA512 => (&digest::SHA512, hmac::HMAC_SHA512),
        _ => return Err(CertError::Unsupported),
    };
    let key = derive(hash, bmp, salt, MAC_KEY, rounds, hash.output_len());
    hmac::verify(&hmac::Key::new(keyed, &key), content, want).map_err(|_| CertError::Password)
}

/// `data` decrypted as AlgorithmIdentifier `algorithm` says.
fn decrypt(mut algorithm: Der, data: &[u8], pw: &Password) -> Result<Vec<u8>, CertError> {
    let kind = algorithm.expect(OID)?;
    let mut params = algorithm.seq()?;
    match kind {
        PBES2 => {
            let mut kdf = params.seq()?;
            if kdf.expect(OID)? != PBKDF2 {
                return Err(CertError::Unsupported);
            }
            let mut kdf = kdf.seq()?;
            let salt = kdf.expect(OCTET_STRING)?;
            let rounds = number(kdf.expect(INTEGER)?)?;
            let mut prf = pbkdf2::PBKDF2_HMAC_SHA1;
            while !kdf.is_empty() {
                match kdf.next()? {
                    // The key's length, which the cipher says too.
                    (INTEGER, _) => {}
                    (SEQUENCE, f) => {
                        prf = match Der(f).expect(OID)? {
                            HMAC_SHA1 => pbkdf2::PBKDF2_HMAC_SHA1,
                            HMAC_SHA256 => pbkdf2::PBKDF2_HMAC_SHA256,
                            HMAC_SHA384 => pbkdf2::PBKDF2_HMAC_SHA384,
                            HMAC_SHA512 => pbkdf2::PBKDF2_HMAC_SHA512,
                            _ => return Err(CertError::Unsupported),
                        }
                    }
                    _ => return Err(CertError::NotPkcs12),
                }
            }
            let mut scheme = params.seq()?;
            let cipher = scheme.expect(OID)?;
            let iv = scheme.expect(OCTET_STRING)?;
            let mut key = vec![0; key_len(cipher)?];
            pbkdf2::derive(prf, NonZeroU32::new(rounds).ok_or(CertError::NotPkcs12)?, salt, pw.text.as_bytes(), &mut key);
            cbc_decrypt(cipher, &key, iv, data)
        }
        PBE_SHA1_3DES | PBE_SHA1_RC2_128 | PBE_SHA1_RC2_40 => {
            let salt = params.expect(OCTET_STRING)?;
            let rounds = number(params.expect(INTEGER)?)?;
            let sha1 = &digest::SHA1_FOR_LEGACY_USE_ONLY;
            let key = derive(sha1, pw.bmp, salt, CIPHER_KEY, rounds, key_len(kind)?);
            let iv = derive(sha1, pw.bmp, salt, CIPHER_IV, rounds, 8);
            cbc_decrypt(kind, &key, &iv, data)
        }
        _ => Err(CertError::Unsupported),
    }
}

/// How long cipher `kind`'s key is.
fn key_len(kind: &[u8]) -> Result<usize, CertError> {
    Ok(match kind {
        AES_128_CBC | PBE_SHA1_RC2_128 => 16,
        AES_192_CBC | DES_EDE3_CBC | PBE_SHA1_3DES => 24,
        AES_256_CBC => 32,
        PBE_SHA1_RC2_40 => 5,
        _ => return Err(CertError::Unsupported),
    })
}

/// `data` decrypted by cipher `kind` in CBC mode, its padding taken off. Bad padding is a wrong password.
fn cbc_decrypt(kind: &[u8], key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, CertError> {
    fn open<C: BlockDecryptMut>(c: C, data: &[u8]) -> Result<Vec<u8>, CertError> {
        c.decrypt_padded_vec_mut::<Pkcs7>(data).map_err(|_| CertError::Password)
    }
    let bad = |_| CertError::NotPkcs12;
    match kind {
        AES_128_CBC => open(cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv).map_err(bad)?, data),
        AES_192_CBC => open(cbc::Decryptor::<aes::Aes192>::new_from_slices(key, iv).map_err(bad)?, data),
        AES_256_CBC => open(cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv).map_err(bad)?, data),
        DES_EDE3_CBC | PBE_SHA1_3DES => open(cbc::Decryptor::<des::TdesEde3>::new_from_slices(key, iv).map_err(bad)?, data),
        PBE_SHA1_RC2_128 | PBE_SHA1_RC2_40 => {
            let rc2 = rc2::Rc2::new_with_eff_key_len(key, key.len() * 8);
            open(cbc::Decryptor::inner_iv_slice_init(rc2, iv).map_err(bad)?, data)
        }
        _ => Err(CertError::Unsupported),
    }
}

/// What PKCS#12's derivation makes: a cipher's key, its IV, or a MAC's key.
const CIPHER_KEY: u8 = 1;
const CIPHER_IV: u8 = 2;
const MAC_KEY: u8 = 3;

/// `n` bytes for `purpose` from password `bmp` and `salt`, as PKCS#12 derives them (RFC 7292, B.2).
fn derive(hash: &'static digest::Algorithm, bmp: &[u8], salt: &[u8], purpose: u8, rounds: u32, n: usize) -> Vec<u8> {
    let v = hash.block_len();
    let filled = |s: &[u8]| -> Vec<u8> { s.iter().copied().cycle().take(s.len().div_ceil(v) * v).collect() };
    let mut input = filled(salt);
    input.extend(filled(bmp));
    let mut out = Vec::with_capacity(n);
    loop {
        let mut round = digest::Context::new(hash);
        round.update(&vec![purpose; v]);
        round.update(&input);
        let mut a = round.finish();
        for _ in 1..rounds {
            a = digest::digest(hash, a.as_ref());
        }
        out.extend_from_slice(a.as_ref());
        if out.len() >= n {
            out.truncate(n);
            return out;
        }
        let b: Vec<u8> = a.as_ref().iter().copied().cycle().take(v).collect();
        for block in input.chunks_mut(v) {
            let mut carry = 1u16;
            for (x, y) in block.iter_mut().zip(&b).rev() {
                let sum = *x as u16 + *y as u16 + carry;
                *x = sum as u8;
                carry = sum >> 8;
            }
        }
    }
}

/// A small unsigned DER INTEGER's value.
fn number(bytes: &[u8]) -> Result<u32, CertError> {
    let bytes = bytes.strip_prefix(&[0]).unwrap_or(bytes);
    if bytes.len() > 4 {
        return Err(CertError::Unsupported);
    }
    Ok(bytes.iter().fold(0, |n, b| n << 8 | *b as u32))
}

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const INTEGER: u8 = 0x02;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;
const EXPLICIT_0: u8 = 0xa0;
const IMPLICIT_0: u8 = 0x80;

const DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01];
const ENCRYPTED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x06];
const KEY_BAG: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x0a, 0x01, 0x01];
const SHROUDED_KEY_BAG: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x0a, 0x01, 0x02];
const CERT_BAG: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x0a, 0x01, 0x03];
const X509_CERTIFICATE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x16, 0x01];
const LOCAL_KEY_ID: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x15];
const PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];
const PBKDF2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0c];
const HMAC_SHA1: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x07];
const HMAC_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x09];
const HMAC_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x0a];
const HMAC_SHA512: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x0b];
const AES_128_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x02];
const AES_192_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x16];
const AES_256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a];
const DES_EDE3_CBC: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x03, 0x07];
const PBE_SHA1_3DES: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x01, 0x03];
const PBE_SHA1_RC2_128: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x01, 0x05];
const PBE_SHA1_RC2_40: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x01, 0x06];
const SHA1: &[u8] = &[0x2b, 0x0e, 0x03, 0x02, 0x1a];
const SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const SHA384: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02];
const SHA512: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03];

/// DER elements still to read, in order.
struct Der<'a>(&'a [u8]);

impl<'a> Der<'a> {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The next element, whole.
    fn element(&mut self) -> Result<&'a [u8], CertError> {
        let all = self.0;
        self.next()?;
        Ok(&all[..all.len() - self.0.len()])
    }

    /// The next element's tag and content. Only definite lengths are DER's.
    fn next(&mut self) -> Result<(u8, &'a [u8]), CertError> {
        let [tag, first, rest @ ..] = self.0 else { return Err(CertError::NotPkcs12) };
        let (len, rest) = match *first {
            short if short < 0x80 => (short as usize, rest),
            long => {
                let n = (long & 0x7f) as usize;
                if n == 0 || n > 4 || rest.len() < n {
                    return Err(CertError::NotPkcs12);
                }
                (rest[..n].iter().fold(0, |len, b| len << 8 | *b as usize), &rest[n..])
            }
        };
        if rest.len() < len {
            return Err(CertError::NotPkcs12);
        }
        let (content, after) = rest.split_at(len);
        self.0 = after;
        Ok((*tag, content))
    }

    /// The content of the next element, which has tag `tag`.
    fn expect(&mut self, tag: u8) -> Result<&'a [u8], CertError> {
        match self.next()? {
            (t, content) if t == tag => Ok(content),
            _ => Err(CertError::NotPkcs12),
        }
    }

    /// The elements of the next one, a SEQUENCE.
    fn seq(&mut self) -> Result<Der<'a>, CertError> {
        self.expect(SEQUENCE).map(Der)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &[u8] = include_bytes!("../tests/data/client-cert.der");
    const CA: &[u8] = include_bytes!("../tests/data/client-ca.der");

    /// OpenSSL 3's AES and SHA-256, its legacy 3DES, RC2-40 and SHA-1, and bags in the clear: each gives
    /// the client's key and its certificate first.
    #[test]
    fn reads_each_encryption() {
        let aes = read(include_bytes!("../tests/data/client-aes.p12"), "secret").unwrap();
        let legacy = read(include_bytes!("../tests/data/client-legacy.p12"), "secret").unwrap();
        let clear = read(include_bytes!("../tests/data/client-clear.p12"), "").unwrap();
        assert_eq!(aes.chain, [CERT, CA], "its own certificate, then the CA's");
        assert_eq!(legacy.chain, [CERT]);
        assert_eq!(clear.chain, [CERT]);
        assert_eq!(aes.key, legacy.key, "one key whatever it was sealed with");
        assert_eq!(aes.key, clear.key);
    }

    #[test]
    fn a_wrong_password_or_another_file_is_told() {
        assert_eq!(read(include_bytes!("../tests/data/client-aes.p12"), "wrong"), Err(CertError::Password));
        assert_eq!(read(include_bytes!("../tests/data/client-legacy.p12"), ""), Err(CertError::Password));
        assert_eq!(read(CERT, "secret"), Err(CertError::NotPkcs12));
        assert_eq!(read(b"not a certificate", ""), Err(CertError::NotPkcs12));
    }

    /// An import is kept in a file of its own, so the one the saved profile uses stays until the profile
    /// is saved; the files no profile uses go when a client starts.
    #[test]
    fn an_import_leaves_the_saved_certificate_until_unused() {
        let dir = nori_testdir::TempDir::new("cert-import");
        let data = dir.path().to_str().unwrap();
        let (aes, legacy) = (include_bytes!("../tests/data/client-aes.p12"), include_bytes!("../tests/data/client-legacy.p12"));
        let saved = import_client_cert(data, SavedServer { id: "p1".into(), ..Default::default() }, aes.to_vec(), "secret".into()).unwrap();
        let draft = import_client_cert(data, saved.clone(), legacy.to_vec(), "secret".into()).unwrap();
        assert_eq!(client_identity(data, saved.clone()).unwrap(), Some(read(aes, "secret").unwrap()), "the saved one as it was");
        assert_eq!(client_identity(data, draft.clone()).unwrap(), Some(read(legacy, "secret").unwrap()));
        forget_unused_client_certs(data, vec![saved.clone(), SavedServer::default()]);
        assert_eq!(client_identity(data, draft), Err(CertError::File), "the unsaved import is gone");
        assert!(client_identity(data, saved).unwrap().is_some());
    }

    #[test]
    fn a_certificate_is_imported_only_when_it_opens() {
        let dir = nori_testdir::TempDir::new("cert-refused");
        let data = dir.path().to_str().unwrap();
        let legacy = include_bytes!("../tests/data/client-legacy.p12").to_vec();
        assert_eq!(import_client_cert(data, SavedServer::default(), legacy, "wrong".into()), Err(CertError::Password));
        assert_eq!(import_client_cert_file(dir.path(), SavedServer::default(), &dir.path().join("missing.p12"), String::new()), Err(CertError::File));
        assert!(!dir.path().join("certs").exists(), "nothing kept");
    }

    /// RFC 7292's derivation against OpenSSL's `PKCS12_key_gen` (a known answer from its test suite).
    #[test]
    fn derives_as_pkcs12_does() {
        let salt = [0x0a, 0x58, 0xcf, 0x64, 0x53, 0x0d, 0x82, 0x3f];
        let key = derive(&digest::SHA1_FOR_LEGACY_USE_ONLY, &bmp("smeg"), &salt, CIPHER_KEY, 1, 24);
        assert_eq!(key, [0x8a, 0xaa, 0xe6, 0x29, 0x7b, 0x6c, 0xb0, 0x46, 0x42, 0xab, 0x5b, 0x07, 0x78, 0x51, 0x28, 0x4e, 0xb7, 0x12, 0x8f, 0x1a, 0x2a, 0x7f, 0xbc, 0xa3]);
    }
}
