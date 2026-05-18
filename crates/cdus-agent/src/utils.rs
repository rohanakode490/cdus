use anyhow::Result;
use libp2p::PeerId;

pub fn hex_to_peer_id(hex_pk: &str) -> Result<String> {
    let bytes = hex::decode(hex_pk)?;
    let pk = libp2p::identity::ed25519::PublicKey::try_from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Invalid ed25519 public key: {}", e))?;
    Ok(PeerId::from_public_key(&libp2p::identity::PublicKey::from(pk)).to_string())
}
