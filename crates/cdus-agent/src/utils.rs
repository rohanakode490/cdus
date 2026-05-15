use anyhow::Result;
use libp2p::PeerId;

pub fn hex_to_peer_id(hex_pk: &str) -> Result<String> {
    let bytes = hex::decode(hex_pk)?;
    let pk = libp2p::identity::ed25519::PublicKey::try_from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Invalid ed25519 public key: {}", e))?;
    Ok(PeerId::from_public_key(&libp2p::identity::PublicKey::from(pk)).to_string())
}

pub fn peer_id_to_hex(_peer_id: &str) -> Result<String> {
    // This is harder because PeerId is a hash.
    // For ed25519, the PeerId can sometimes contain the public key if it's small enough,
    // but in libp2p 0.50+ it's always a CID/multihash.
    // However, if we only use ed25519, we might be able to extract it if we use the unhashed format.
    // BUT libp2p defaults to hashing.

    // Better strategy: always use PeerId string as the Node ID.
    Err(anyhow::anyhow!(
        "Conversion from PeerId to hex not supported. Use PeerId string as Node ID everywhere."
    ))
}
