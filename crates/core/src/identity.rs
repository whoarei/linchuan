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
    let bytes =
        serde_json::to_vec(&json!({"host_id":host,"public_key":public_key,"payload":payload}))
            .unwrap();
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
    let bytes = serde_json::to_vec(
        &json!({"host_id":envelope.host_id,"public_key":envelope.public_key,"payload":envelope.payload}),
    )?;
    VerifyingKey::from_bytes(&key)?.verify_strict(&bytes, &signature)?;
    Ok(())
}
pub fn offer_payload(offer: &Offer) -> Value {
    json!({"type":"message","protocol":offer.protocol,"message_id":offer.message_id,"group_id":offer.group_id,"sender_id":offer.sender_id,"target_id":offer.target_id,"sender_name":offer.sender_name,"text":offer.text,"files":offer.files,"created_at":offer.created_at})
}
