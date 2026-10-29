//! A provider's encrypted stream ([`nori_core::providers::StreamCipher`]) decrypted as it is read, from
//! any byte: counter mode needs only the offset.

use std::io::{self, Read};

use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};

type Ctr = ctr::Ctr128BE<Aes128>;

pub(crate) struct Decrypting {
    reader: Box<dyn Read + Send>,
    ctr: Ctr,
}

impl Decrypting {
    /// `reader`, whose first byte is the file's byte `at`.
    pub(crate) fn new(reader: Box<dyn Read + Send>, cipher: &nori_core::providers::StreamCipher, at: u64) -> Decrypting {
        let mut ctr = Ctr::new(&cipher.key.into(), &cipher.iv.into());
        ctr.seek(at);
        Decrypting { reader, ctr }
    }
}

impl Read for Decrypting {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        self.ctr.apply_keystream(&mut buf[..n]);
        Ok(n)
    }
}
