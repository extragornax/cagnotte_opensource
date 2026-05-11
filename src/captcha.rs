use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const MAX_NUMBER: u64 = 50_000;

#[derive(Serialize)]
pub struct Challenge {
    pub algorithm: &'static str,
    pub challenge: String,
    pub maxnumber: u64,
    pub salt: String,
    pub signature: String,
}

pub fn issue(secret: &[u8]) -> Challenge {
    let mut salt_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = hex::encode(salt_bytes);
    let number: u64 = (OsRng.next_u64() % MAX_NUMBER) + 1;

    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(number.to_string().as_bytes());
    let challenge = hex::encode(hasher.finalize());

    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(challenge.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    Challenge {
        algorithm: "SHA-256",
        challenge,
        maxnumber: MAX_NUMBER,
        salt,
        signature,
    }
}

#[derive(Deserialize)]
struct Payload {
    algorithm: String,
    challenge: String,
    number: u64,
    salt: String,
    signature: String,
}

pub fn verify(secret: &[u8], payload_b64: &str) -> bool {
    let bytes = match B64.decode(payload_b64) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let p: Payload = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if p.algorithm != "SHA-256" {
        return false;
    }

    let mut hasher = Sha256::new();
    hasher.update(p.salt.as_bytes());
    hasher.update(p.number.to_string().as_bytes());
    let computed = hex::encode(hasher.finalize());
    if computed != p.challenge {
        return false;
    }

    let sig_bytes = match hex::decode(&p.signature) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(p.challenge.as_bytes());
    mac.verify_slice(&sig_bytes).is_ok()
}
