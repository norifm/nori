//! The three words both screens show while pairing, made from the new device's lock. They are the EFF's
//! short word list 2.0 (words.txt, by the Electronic Frontier Foundation, CC BY 3.0 US): 1,296 words,
//! no two of them within two letters' change of each other, so none is mistaken for another.
//!
//! Three words are 31 bits. Argon2id makes each guess cost a fraction of a second, so a lock with the
//! same words as another cannot be searched for while a code lives.

use crate::keys::LockKey;

const LIST: &str = include_str!("words.txt");
const COUNT: u64 = 1296;

/// The words `lock` reads as. They are English on every client, so two screens in different languages
/// still compare.
pub(crate) fn words(lock: &LockKey) -> Vec<String> {
    let params = argon2::Params::new(19 * 1024, 2, 1, Some(8)).expect("valid Argon2 parameters");
    let mut out = [0u8; 8];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(lock.bytes(), b"nori pairing words", &mut out)
        .expect("valid Argon2 input");
    let list: Vec<&str> = LIST.lines().collect();
    let mut n = u64::from_le_bytes(out) % COUNT.pow(3);
    (0..3)
        .map(|_| {
            let word = list[(n % COUNT) as usize];
            n /= COUNT;
            word.to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Lock;

    #[test]
    fn the_list_is_the_eff_short_list() {
        let list: Vec<&str> = LIST.lines().collect();
        assert_eq!(list.len() as u64, COUNT);
        assert_eq!((list[0], list[1295]), ("aardvark", "zucchini"));
        let mut sorted = list.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len(), "no word twice");
    }

    #[test]
    fn one_lock_reads_as_the_same_words_and_another_as_others() {
        let (a, b) = (Lock::new(), Lock::new());
        let words_a = words(a.key());
        assert_eq!(words_a.len(), 3);
        assert_eq!(words(&LockKey::from_text(&a.key().text()).unwrap()), words_a);
        assert_ne!(words(b.key()), words_a);
    }
}
