use anyhow::{bail, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD as BASE64, STANDARD_NO_PAD},
    Engine,
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use shadowvpn_core::PeerIdentity;
use x25519_dalek::{PublicKey, StaticSecret};
use xeddsa::xed25519::{PrivateKey as SigningKey, PublicKey as VerifyingKey};
use xeddsa::{Sign, Verify};

const IDENTITY_CONTEXT: &[u8] = b"ShadowVPN peer identity v1\0";
const PSK_CONTEXT: &[u8] = b"ShadowVPN pairwise psk v1\0";

pub fn peer_fingerprint(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    STANDARD_NO_PAD.encode(digest)
}

pub fn public_key_b64(secret: &[u8; 32]) -> String {
    BASE64.encode(PublicKey::from(&StaticSecret::from(*secret)).to_bytes())
}

pub fn create_peer_identity(
    secret: &[u8; 32],
    group_name: &str,
    public_key: &str,
) -> Result<PeerIdentity> {
    let public_key = decode_public_key(public_key)?;
    let secret = StaticSecret::from(*secret);
    let signing_key = SigningKey::from(&secret);
    let public_key_b64 = BASE64.encode(public_key);
    let message = identity_message(group_name, &public_key_b64);
    let signature: [u8; 64] = signing_key.sign(&message, rand::rngs::OsRng);
    Ok(PeerIdentity {
        signature: BASE64.encode(signature),
    })
}

pub fn verify_peer_identity(
    public_key: &str,
    group_name: &str,
    identity: &PeerIdentity,
) -> Result<String> {
    let public_key = decode_public_key(public_key)?;
    let signature = decode_signature(&identity.signature)?;
    let message = identity_message(group_name, &BASE64.encode(public_key));
    let verifying_key = VerifyingKey::from(&PublicKey::from(public_key));
    verifying_key
        .verify(&message, &signature)
        .context("assinatura de identidade não confere")?;
    Ok(peer_fingerprint(&public_key))
}

pub fn derive_pairwise_psk(
    secret: &[u8; 32],
    peer_public_key: &str,
    group_name: &str,
) -> Result<[u8; 32]> {
    let peer_public_key = decode_public_key(peer_public_key)?;
    let secret = StaticSecret::from(*secret);
    let local_public = PublicKey::from(&secret);
    let peer_public = PublicKey::from(peer_public_key);
    let shared = secret.diffie_hellman(&peer_public);
    if shared.as_bytes().iter().all(|byte| *byte == 0) {
        bail!("chave pública WireGuard inválida para o PSK");
    }

    let mut public_keys = [local_public.as_bytes().to_owned(), peer_public_key];
    public_keys.sort();
    let mut info = Vec::with_capacity(PSK_CONTEXT.len() + 2 + group_name.len() + 64);
    info.extend_from_slice(PSK_CONTEXT);
    append_field(&mut info, group_name.as_bytes());
    append_field(&mut info, &public_keys[0]);
    append_field(&mut info, &public_keys[1]);

    let hkdf = Hkdf::<Sha256>::new(Some(b"ShadowVPN pairwise psk salt v1"), shared.as_bytes());
    let mut psk = [0u8; 32];
    hkdf.expand(&info, &mut psk)
        .map_err(|_| anyhow::anyhow!("não foi possível derivar o PSK"))?;
    Ok(psk)
}

fn identity_message(group_name: &str, public_key_b64: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(
        IDENTITY_CONTEXT.len() + 2 + group_name.len() + 2 + public_key_b64.len(),
    );
    message.extend_from_slice(IDENTITY_CONTEXT);
    append_field(&mut message, group_name.as_bytes());
    append_field(&mut message, public_key_b64.as_bytes());
    message
}

fn append_field(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value);
}

pub fn decode_public_key(encoded: &str) -> Result<[u8; 32]> {
    let decoded = BASE64.decode(encoded)?;
    decoded
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("chave pública inválida"))
}

fn decode_signature(encoded: &str) -> Result<[u8; 64]> {
    let decoded = BASE64.decode(encoded)?;
    decoded
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("tamanho da assinatura inválido"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public_key(secret: &[u8; 32]) -> [u8; 32] {
        PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
    }

    #[test]
    fn identity_proof_binds_group_and_key() {
        let secret = [7u8; 32];
        let public = public_key(&secret);
        let identity = create_peer_identity(&secret, "minha-rede", &BASE64.encode(public)).unwrap();

        let encoded = BASE64.encode(public);
        let fingerprint = verify_peer_identity(&encoded, "minha-rede", &identity).unwrap();
        assert_eq!(fingerprint, peer_fingerprint(&public));
        assert!(verify_peer_identity(&encoded, "outra-rede", &identity).is_err());

        let other_public = BASE64.encode(public_key(&[8u8; 32]));
        assert!(verify_peer_identity(&other_public, "minha-rede", &identity).is_err());
    }

    #[test]
    fn rejects_tampered_identity_proof() {
        let secret = [9u8; 32];
        let public = public_key(&secret);
        let mut identity =
            create_peer_identity(&secret, "minha-rede", &BASE64.encode(public)).unwrap();
        let mut signature = BASE64.decode(&identity.signature).unwrap();
        signature[0] ^= 0x80;
        identity.signature = BASE64.encode(signature);

        assert!(verify_peer_identity(&BASE64.encode(public), "minha-rede", &identity).is_err());
    }

    #[test]
    fn derives_the_same_pairwise_psk_for_both_peers() {
        let secret_a = [11u8; 32];
        let secret_b = [12u8; 32];
        let public_a = public_key(&secret_a);
        let public_b = public_key(&secret_b);

        let psk_a = derive_pairwise_psk(&secret_a, &BASE64.encode(public_b), "minha-rede").unwrap();
        let psk_b = derive_pairwise_psk(&secret_b, &BASE64.encode(public_a), "minha-rede").unwrap();
        assert_eq!(psk_a, psk_b);

        let other_group =
            derive_pairwise_psk(&secret_a, &BASE64.encode(public_b), "outra-rede").unwrap();
        assert_ne!(psk_a, other_group);
    }

    #[test]
    fn rejects_low_order_public_key_for_psk() {
        let secret = [13u8; 32];
        assert!(derive_pairwise_psk(&secret, &BASE64.encode([0u8; 32]), "minha-rede").is_err());
    }
}
