use crate::model::*;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
pub fn create_key() -> String {
    hex::encode(SigningKey::generate(&mut rand_core::OsRng).to_bytes())
}
pub fn signing_key(secret: &str) -> anyhow::Result<SigningKey> {
    let bytes: [u8; 32] = hex::decode(secret)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("设备密钥格式错误"))?;
    Ok(SigningKey::from_bytes(&bytes))
}
pub fn sign(key: &SigningKey, host: &str, payload: Value) -> SignedEnvelope {
    let public_key = hex::encode(key.verifying_key().to_bytes());
    let bytes = canonical(&json!({"host_id":host,"public_key":public_key,"payload":payload}));
    SignedEnvelope {
        host_id: host.into(),
        public_key,
        payload,
        signature: hex::encode(key.sign(&bytes).to_bytes()),
    }
}
pub fn verify(envelope: &SignedEnvelope) -> anyhow::Result<()> {
    anyhow::ensure!(
        uuid::Uuid::parse_str(&envelope.host_id).is_ok(),
        "宿主身份无效"
    );
    let key: [u8; 32] = hex::decode(&envelope.public_key)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("公钥无效"))?;
    let signature = ed25519_dalek::Signature::from_slice(&hex::decode(&envelope.signature)?)?;
    let bytes = canonical(
        &json!({"host_id":envelope.host_id,"public_key":envelope.public_key,"payload":envelope.payload}),
    );
    VerifyingKey::from_bytes(&key)?.verify_strict(&bytes, &signature)?;
    Ok(())
}
pub fn offer_payload(offer: &Offer) -> Value {
    json!({"type":"message","protocol":offer.protocol,"message_id":offer.message_id,"group_id":offer.group_id,"sender_id":offer.sender_id,"target_id":offer.target_id,"sender_name":offer.sender_name,"text":offer.text,"files":offer.files,"created_at":offer.created_at})
}

// Sort recursively, independently of serde_json feature unification in a native build.
fn canonical(value: &Value) -> Vec<u8> {
    fn write(value: &Value, out: &mut Vec<u8>) {
        match value {
            Value::Object(map) => {
                out.push(b'{');
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    serde_json::to_writer(&mut *out, key).unwrap();
                    out.push(b':');
                    write(&map[key], out);
                }
                out.push(b'}');
            }
            Value::Array(values) => {
                out.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    write(value, out);
                }
                out.push(b']');
            }
            _ => serde_json::to_writer(out, value).unwrap(),
        }
    }
    let mut result = Vec::new();
    write(value, &mut result);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_matches_python_ed25519_canonical_json_vector() {
        // Generated independently using Python cryptography, UTF-8 and sorted compact JSON.
        let key = SigningKey::from_bytes(&[7; 32]);
        let value = json!({"type":"message","text":"你好","members":["设备A","B"]});
        let envelope = sign(&key, "11111111-1111-4111-8111-111111111111", value);
        assert_eq!(
            envelope.public_key,
            "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
        );
        assert_eq!(envelope.signature,"abde7144e22e7e46c556147751dc9e2e62786dc9743e844f06d2e6348160e485ca2d3515886226d9d1b2b8099fd3cdc17273738b255ab615c9ed4e9312b77f02");
        verify(&envelope).unwrap();
    }
}
