// Encrypted, mutually-authenticated transport for Nutler peers.
//
// We use the Noise protocol (NNpsk0) with a pre-shared key derived from the host's
// room password:
//   - Confidentiality: every transport message is AEAD-encrypted (ChaCha20-Poly1305).
//   - Mutual auth: both sides must know the password (psk) or the handshake fails on
//     the very first message — no certificates to generate or distribute.
//
// Handshake messages and transport messages are framed identically to the rest of the
// protocol (a 4-byte big-endian length prefix), so this layers cleanly on top of TCP.
//
// TODO(phase-1): currently unit-tested in isolation; the next step wires these into
// sockets.rs (responder on accept, initiator on connect) and the send/broadcast paths.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use snow::{Builder, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Noise handshake pattern: no static keys, pre-shared key mixed in at position 0.
const NOISE_PARAMS: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";

/// Noise caps a single message at 65535 bytes; chat messages are far smaller.
const NOISE_MAX_MESSAGE: usize = 65535;

/// Derive a 32-byte pre-shared key from the room password. A domain-separation
/// prefix keeps this key distinct from any other use of the same password.
pub fn derive_psk(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"nutler-noise-psk-v1");
    hasher.update(password.as_bytes());
    let digest = hasher.finalize();
    let mut psk = [0u8; 32];
    psk.copy_from_slice(&digest);
    psk
}

/// Write a length-prefixed frame in the clear (used during the handshake).
async fn write_frame<W>(writer: &mut W, data: &[u8]) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    writer.write_all(&(data.len() as u32).to_be_bytes()).await?;
    writer.write_all(data).await?;
    Ok(())
}

/// Read a length-prefixed frame in the clear (used during the handshake).
async fn read_frame<R>(reader: &mut R) -> std::io::Result<Vec<u8>>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len > NOISE_MAX_MESSAGE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "handshake frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Perform the Noise handshake as the responder (the host side, per accepted client).
/// Returns the established transport state, or an error if the peer used the wrong
/// password (the AEAD tag on the first message fails to verify).
pub async fn responder_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    psk: &[u8; 32],
) -> Result<TransportState, String>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let params = NOISE_PARAMS.parse().map_err(|e| format!("bad noise params: {e}"))?;
    let mut handshake = Builder::new(params)
        .psk(0, psk)
        .build_responder()
        .map_err(|e| format!("build responder: {e}"))?;

    let mut buf = vec![0u8; NOISE_MAX_MESSAGE];

    // <- e  (read the initiator's first message; wrong psk fails the tag here)
    let msg1 = read_frame(reader).await.map_err(|e| format!("read handshake msg1: {e}"))?;
    handshake
        .read_message(&msg1, &mut buf)
        .map_err(|_| "handshake failed (wrong password?)".to_string())?;

    // -> e, ee  (respond)
    let n = handshake
        .write_message(&[], &mut buf)
        .map_err(|e| format!("write handshake msg2: {e}"))?;
    write_frame(writer, &buf[..n]).await.map_err(|e| format!("send handshake msg2: {e}"))?;

    handshake.into_transport_mode().map_err(|e| format!("enter transport mode: {e}"))
}

/// Perform the Noise handshake as the initiator (a client connecting to a host).
pub async fn initiator_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    psk: &[u8; 32],
) -> Result<TransportState, String>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let params = NOISE_PARAMS.parse().map_err(|e| format!("bad noise params: {e}"))?;
    let mut handshake = Builder::new(params)
        .psk(0, psk)
        .build_initiator()
        .map_err(|e| format!("build initiator: {e}"))?;

    let mut buf = vec![0u8; NOISE_MAX_MESSAGE];

    // -> e
    let n = handshake
        .write_message(&[], &mut buf)
        .map_err(|e| format!("write handshake msg1: {e}"))?;
    write_frame(writer, &buf[..n]).await.map_err(|e| format!("send handshake msg1: {e}"))?;

    // <- e, ee  (wrong psk fails the tag here on the initiator side)
    let msg2 = read_frame(reader).await.map_err(|e| format!("read handshake msg2: {e}"))?;
    handshake
        .read_message(&msg2, &mut buf)
        .map_err(|_| "handshake failed (wrong password?)".to_string())?;

    handshake.into_transport_mode().map_err(|e| format!("enter transport mode: {e}"))
}

/// Encrypt one plaintext message into a Noise transport message. The caller frames
/// the result (length-prefix) before sending.
pub fn encrypt(transport: &mut TransportState, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    if plaintext.len() + 16 > NOISE_MAX_MESSAGE {
        return Err("message too large to encrypt in a single Noise message".to_string());
    }
    let mut buf = vec![0u8; plaintext.len() + 16]; // + AEAD tag
    let n = transport
        .write_message(plaintext, &mut buf)
        .map_err(|e| format!("encrypt: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

/// Decrypt one Noise transport message back into plaintext.
pub fn decrypt(transport: &mut TransportState, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; ciphertext.len()];
    let n = transport
        .read_message(ciphertext, &mut buf)
        .map_err(|e| format!("decrypt: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn handshake_and_encrypted_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = sock.into_split();
            let mut ts = responder_handshake(&mut r, &mut w, &derive_psk("s3cret"))
                .await
                .expect("responder handshake");

            // Two messages each way to exercise nonce increment beyond the first.
            for expected in ["hello 1", "hello 2"] {
                let ct = read_frame(&mut r).await.unwrap();
                let pt = decrypt(&mut ts, &ct).unwrap();
                assert_eq!(pt, expected.as_bytes());
                let reply = encrypt(&mut ts, format!("ack {expected}").as_bytes()).unwrap();
                write_frame(&mut w, &reply).await.unwrap();
            }
        });

        let sock = TcpStream::connect(addr).await.unwrap();
        let (mut r, mut w) = sock.into_split();
        let mut ts = initiator_handshake(&mut r, &mut w, &derive_psk("s3cret"))
            .await
            .expect("initiator handshake");

        for msg in ["hello 1", "hello 2"] {
            let ct = encrypt(&mut ts, msg.as_bytes()).unwrap();
            write_frame(&mut w, &ct).await.unwrap();
            let reply_ct = read_frame(&mut r).await.unwrap();
            let reply = decrypt(&mut ts, &reply_ct).unwrap();
            assert_eq!(reply, format!("ack {msg}").as_bytes());
        }

        server.await.unwrap();
    }

    #[tokio::test]
    async fn wrong_password_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let (mut r, mut w) = sock.into_split();
            // Responder must reject the mismatched psk.
            let res = responder_handshake(&mut r, &mut w, &derive_psk("correct-horse")).await;
            assert!(res.is_err(), "responder should reject wrong password");
        });

        let sock = TcpStream::connect(addr).await.unwrap();
        let (mut r, mut w) = sock.into_split();
        let res = initiator_handshake(&mut r, &mut w, &derive_psk("wrong-password")).await;
        assert!(res.is_err(), "initiator should fail with wrong password");

        let _ = server.await;
    }
}
