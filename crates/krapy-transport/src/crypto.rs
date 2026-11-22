use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use hkdf::Hkdf;
use sha2::Sha256;

const REPLAY_WINDOW: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionKeys {
    pub tcp_p2d: [u8; 32],
    pub tcp_d2p: [u8; 32],
    pub udp_p2d: [u8; 32],
    pub udp_d2p: [u8; 32],
}

pub fn pair_secret(k: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let _ = Hkdf::<Sha256>::new(Some(b"krapy-v1"), k).expand(b"pair", &mut out);
    out
}

impl SessionKeys {
    pub fn derive(s: &[u8; 32], nonce_p: &[u8; 16], nonce_d: &[u8; 16]) -> Self {
        let mut salt = [0u8; 32];
        salt[..16].copy_from_slice(nonce_p);
        salt[16..].copy_from_slice(nonce_d);
        let mut okm = [0u8; 128];
        let _ = Hkdf::<Sha256>::new(Some(&salt), s).expand(b"krapy session", &mut okm);
        let mut sk = SessionKeys {
            tcp_p2d: [0; 32],
            tcp_d2p: [0; 32],
            udp_p2d: [0; 32],
            udp_d2p: [0; 32],
        };
        sk.tcp_p2d.copy_from_slice(&okm[0..32]);
        sk.tcp_d2p.copy_from_slice(&okm[32..64]);
        sk.udp_p2d.copy_from_slice(&okm[64..96]);
        sk.udp_d2p.copy_from_slice(&okm[96..128]);
        sk
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    Tamper,
    Replay,
    OldCounter,
}

pub struct Sealer {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Sealer {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(key.into()),
            counter: 0,
        }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_be_bytes());
        n
    }

    pub fn seal_tcp(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let counter_bytes = self.counter.to_be_bytes();
        let mut out = Vec::with_capacity(8 + plaintext.len() + 16);
        out.extend_from_slice(&counter_bytes);
        let mut buf = plaintext.to_vec();
        let tag = self
            .cipher
            .encrypt_in_place_detached(&self.nonce().into(), &counter_bytes, &mut buf)
            .unwrap_or_default();
        out.extend_from_slice(&buf);
        out.extend_from_slice(&tag);
        self.counter += 1;
        out
    }

    pub fn seal_udp(&mut self, header9: &[u8; 9], plaintext: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + plaintext.len() + 16);
        out.extend_from_slice(header9);
        let mut buf = plaintext.to_vec();
        let tag = self
            .cipher
            .encrypt_in_place_detached(&self.nonce().into(), header9, &mut buf)
            .unwrap_or_default();
        out.extend_from_slice(&buf);
        out.extend_from_slice(&tag);
        self.counter += 1;
        out
    }

    pub fn counter(&self) -> u64 {
        self.counter
    }
}

pub struct Opener {
    cipher: ChaCha20Poly1305,
    highest: Option<u64>,
    seen: u128,
}

impl Opener {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(key.into()),
            highest: None,
            seen: 0,
        }
    }

    fn nonce(counter: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&counter.to_be_bytes());
        n
    }

    fn check_replay(&mut self, counter: u64) -> Result<(), CryptoError> {
        let high = match self.highest {
            Some(h) => h,
            None => {
                self.highest = Some(counter);
                self.seen = 1;
                return Ok(());
            }
        };
        if counter > high {
            let shift = counter - high;
            if shift >= REPLAY_WINDOW {
                self.seen = 1;
            } else {
                self.seen <<= shift;
                self.seen |= 1;
            }
            self.highest = Some(counter);
            return Ok(());
        }
        let behind = high - counter;
        if behind >= REPLAY_WINDOW {
            return Err(CryptoError::OldCounter);
        }
        let bit = 1u128 << behind;
        if self.seen & bit != 0 {
            return Err(CryptoError::Replay);
        }
        self.seen |= bit;
        Ok(())
    }

    pub fn open_tcp(&mut self, wire: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if wire.len() < 8 + 16 {
            return Err(CryptoError::Tamper);
        }
        let counter_arr: [u8; 8] = wire[0..8].try_into().map_err(|_| CryptoError::Tamper)?;
        let counter = u64::from_be_bytes(counter_arr);
        self.check_replay(counter)?;
        let counter_bytes = counter_arr;
        let ct_len = wire.len() - 8 - 16;
        let mut buf = wire[8..8 + ct_len].to_vec();
        let tag: [u8; 16] = wire[8 + ct_len..].try_into().expect("tag");
        self.cipher
            .decrypt_in_place_detached(
                &Self::nonce(counter).into(),
                &counter_bytes,
                &mut buf,
                &tag.into(),
            )
            .map_err(|_| CryptoError::Tamper)?;
        Ok(buf)
    }

    pub fn open_udp(&mut self, wire: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if wire.len() < 9 + 16 {
            return Err(CryptoError::Tamper);
        }
        let counter = u64::from_be_bytes(wire[1..9].try_into().expect("len"));
        self.check_replay(counter)?;
        let aad: [u8; 9] = wire[0..9].try_into().expect("len");
        let ct_len = wire.len() - 9 - 16;
        let mut buf = wire[9..9 + ct_len].to_vec();
        let tag: [u8; 16] = wire[9 + ct_len..].try_into().expect("tag");
        self.cipher
            .decrypt_in_place_detached(&Self::nonce(counter).into(), &aad, &mut buf, &tag.into())
            .map_err(|_| CryptoError::Tamper)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        hex::decode(s).expect("hex")
    }

    #[test]
    fn hkdf_rfc5869_case1() {
        let salt: Vec<u8> = (0x00..=0x0c).collect();
        let info: Vec<u8> = (0xf0..=0xf9).collect();
        let mut okm = [0u8; 42];
        Hkdf::<Sha256>::new(Some(&salt), &[0x0b; 22])
            .expand(&info, &mut okm)
            .expect("expand");
        assert_eq!(
            okm.to_vec(),
            unhex(
                "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
            )
        );
    }

    #[test]
    fn protocol_vectors() {
        let k: Vec<u8> = (0..32).collect();
        let s = pair_secret(&k);
        assert_eq!(
            s.to_vec(),
            unhex("652e39ae96eb24d8546fe67d80acb68c9710e974f062d7c4486e0be6657fb38b")
        );
        let keys = SessionKeys::derive(&s, &[1; 16], &[2; 16]);
        assert_eq!(
            keys.tcp_p2d.to_vec(),
            unhex("c14af774e5c109ed4ef46e6b15c8718ac80b0d50eeb8f754def92a441e6fb3b4")
        );
        assert_eq!(
            keys.udp_d2p.to_vec(),
            unhex("f8ce0d3e603791fe2648a067c98a537969b190eb20a9379c20dd3e4d5f06ac7b")
        );
        let sealed = Sealer::new(&keys.tcp_p2d).seal_tcp(b"krapy");
        assert_eq!(
            sealed,
            unhex("0000000000000000d483e26a1b70807bc388d46700f5be3dac1fe382e4")
        );
    }

    #[test]
    fn seal_open_roundtrip() {
        let key = [7u8; 32];
        let mut seal = Sealer::new(&key);
        let mut open = Opener::new(&key);
        let pt = b"hello control";
        let wire = seal.seal_tcp(pt);
        let out = open.open_tcp(&wire).expect("open");
        assert_eq!(&out, pt);
    }

    #[test]
    fn tamper_rejected() {
        let key = [1u8; 32];
        let mut seal = Sealer::new(&key);
        let mut open = Opener::new(&key);
        let mut wire = seal.seal_tcp(b"x");
        wire[10] ^= 0xff;
        assert_eq!(open.open_tcp(&wire), Err(CryptoError::Tamper));
    }

    #[test]
    fn replay_rejected() {
        let key = [2u8; 32];
        let mut seal = Sealer::new(&key);
        let mut open = Opener::new(&key);
        let wire = seal.seal_tcp(b"a");
        let _ = open.open_tcp(&wire).expect("first");
        assert_eq!(open.open_tcp(&wire), Err(CryptoError::Replay));
    }

    #[test]
    fn old_counter_rejected() {
        let key = [3u8; 32];
        let mut seal = Sealer::new(&key);
        let mut open = Opener::new(&key);
        let mut saved = None;
        for i in 0..1040 {
            let wire = seal.seal_tcp(b"z");
            if i == 5 {
                saved = Some(wire);
                continue;
            }
            let _ = open.open_tcp(&wire).expect("ok");
        }
        assert_eq!(
            open.open_tcp(&saved.expect("saved")),
            Err(CryptoError::OldCounter)
        );
    }
}
