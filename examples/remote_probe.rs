//! remote_probe: paired-client smoke test against a live `oxios serve --remote`.
//!
//! Usage:
//!
//!     cargo run --features remote --example remote_probe -- 'oxios://pair?code=...'
//!
//! The daemon prints the `oxios://pair?code=…` URL on stdout when it starts
//! with `--remote --pairing-address`. The probe takes that URL, decodes the
//! `PairingOffer`, connects to the offered endpoint, runs the Noise_XX
//! handshake (learning the server's static key ON THE WIRE and asserting it
//! matches the pin from the offer), then sends an encrypted `status.get`
//! and prints the JSON response.
//!
//! This is the manual / live acceptance check for RFC-044 §12 Phase 1.
//! The hermetic equivalent lives as `paired_client_round_trip_status_get`
//! in `src/remote/mod.rs`.

#![cfg(feature = "remote")]

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

const NOISE_XX: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const FRAME_HEADER_LEN: usize = 5;
const FRAME_MAX: usize = 65536;
const NOISE_TAG_LEN: usize = 16;

#[derive(Debug, Deserialize)]
struct PairingOffer {
    #[allow(dead_code)]
    v: u32,
    endpoint: String,
    device_id: String,
    public_key_b64: String,
}

fn decode_pair_url(url: &str) -> Result<PairingOffer> {
    let code = url
        .strip_prefix("oxios://pair?code=")
        .or_else(|| url.strip_prefix("oxios://pair#code="))
        .ok_or_else(|| anyhow!("not an oxios://pair URL: {url}"))?;
    let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(code.as_bytes())
        .context("base64 decode pairing code")?;
    serde_json::from_slice::<PairingOffer>(&json_bytes).context("parse PairingOffer JSON")
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum FrameType {
    Noise = 0x01,
    App = 0x02,
}

fn encode_frame(ty: FrameType, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > FRAME_MAX {
        return Err(anyhow!("payload exceeds FRAME_MAX"));
    }
    let len = u32::try_from(payload.len())?;
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    out.push(ty as u8);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

fn decode_frame(buf: &[u8]) -> Result<(FrameType, Vec<u8>)> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(anyhow!("frame too short"));
    }
    let ty = match buf[0] {
        0x01 => FrameType::Noise,
        0x02 => FrameType::App,
        other => return Err(anyhow!("unknown frame type {other}")),
    };
    let size = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if size > FRAME_MAX {
        return Err(anyhow!("frame size exceeds FRAME_MAX"));
    }
    let end = FRAME_HEADER_LEN + size;
    if end > buf.len() {
        return Err(anyhow!("frame truncated: want {end}, got {}", buf.len()));
    }
    Ok((ty, buf[FRAME_HEADER_LEN..end].to_vec()))
}

fn encrypt(ts: &mut snow::TransportState, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; plaintext.len() + NOISE_TAG_LEN];
    let n = ts
        .write_message(plaintext, &mut buf)
        .map_err(|e| anyhow!("encrypt: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

fn decrypt(ts: &mut snow::TransportState, ciphertext: &[u8]) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; FRAME_MAX + NOISE_TAG_LEN];
    let n = ts
        .read_message(ciphertext, &mut buf)
        .map_err(|e| anyhow!("decrypt: {e}"))?;
    buf.truncate(n);
    Ok(buf)
}

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: remote_probe <oxios://pair?code=...>"))?;
    let offer = decode_pair_url(&url).context("decode pairing URL")?;

    eprintln!(
        "pairing offer: device_id={}, endpoint={}",
        offer.device_id, offer.endpoint
    );

    let pinned_server_pub = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(offer.public_key_b64.as_bytes())
        .context("base64 decode server public key")?;
    if pinned_server_pub.len() != 32 {
        return Err(anyhow!(
            "server public key must be 32 bytes, got {}",
            pinned_server_pub.len()
        ));
    }

    // Initiator: ephemeral static key. XX carries our static in msg3; the
    // server static is learned ON THE WIRE in msg2 and verified against the
    // offer's pin below. Do NOT pre-feed `remote_public_key` — that would
    // defeat the whole point of pinning.
    let pattern: snow::params::NoiseParams = NOISE_XX.parse().context("parse noise pattern")?;
    let client_kp = snow::Builder::new(pattern.clone())
        .generate_keypair()
        .context("generate client keypair")?;
    let mut initiator = snow::Builder::new(pattern)
        .local_private_key(&client_kp.private)
        .context("set client static")?
        .build_initiator()
        .context("build initiator")?;

    let (mut ws, _resp) = tokio_tungstenite::connect_async(&offer.endpoint)
        .await
        .context("WebSocket connect to daemon")?;
    eprintln!("WebSocket connected to {}", offer.endpoint);

    // msg1: -> e, es
    let mut msg1 = vec![0u8; 1024];
    let n = initiator
        .write_message(&[], &mut msg1)
        .context("init write msg1")?;
    msg1.truncate(n);
    let frame1 = encode_frame(FrameType::Noise, &msg1)?;
    ws.send(Message::Binary(frame1))
        .await
        .context("send msg1")?;
    eprintln!("sent msg1 ({} bytes)", msg1.len());

    // msg2: <- e, ee, s, es
    let msg2_msg = timeout(Duration::from_secs(5), ws.next())
        .await
        .map_err(|_| anyhow!("timeout waiting for msg2"))?
        .context("ws err on msg2")?
        .context("server closed before msg2")?;
    let msg2_bytes = msg2_msg.into_data();
    let (ft2, msg2) = decode_frame(&msg2_bytes)?;
    if !matches!(ft2, FrameType::Noise) {
        return Err(anyhow!("msg2 must be Noise frame, got {:?}", ft2));
    }
    let mut buf2 = [0u8; 1024];
    initiator
        .read_message(&msg2, &mut buf2)
        .context("init read msg2 (XX authenticates the server here)")?;

    // PIN VERIFICATION: the XX initiator learns the server's static public
    // key on the wire in msg2. Assert it byte-equals the offer's pin. This
    // is what makes the QR pairing offer a trust root — without this check,
    // a successful AEAD round-trip would only prove "encryption works with
    // someone", not "we reached the *pinned* daemon".
    let learned_static = initiator
        .get_remote_static()
        .ok_or_else(|| anyhow!("XX initiator must know the server static after msg2"))?;
    if learned_static != pinned_server_pub.as_slice() {
        return Err(anyhow!(
            "PIN MISMATCH: server static learned in XX does not match offer.public_key_b64"
        ));
    }
    eprintln!("recv msg2 ({} bytes) — server static pinned", msg2.len());

    // msg3: -> s, se
    let mut msg3 = vec![0u8; 1024];
    let n = initiator
        .write_message(&[], &mut msg3)
        .context("init write msg3")?;
    msg3.truncate(n);
    let frame3 = encode_frame(FrameType::Noise, &msg3)?;
    ws.send(Message::Binary(frame3))
        .await
        .context("send msg3")?;
    eprintln!("sent msg3 ({} bytes) — handshake complete", msg3.len());

    let mut transport = initiator
        .into_transport_mode()
        .context("init -> transport")?;

    // Send an encrypted status.get.
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "status.get",
    });
    let request_bytes = serde_json::to_vec(&request)?;
    let ct = encrypt(&mut transport, &request_bytes).context("encrypt status.get")?;
    let app_frame = encode_frame(FrameType::App, &ct)?;
    ws.send(Message::Binary(app_frame))
        .await
        .context("send app frame")?;
    eprintln!("sent encrypted status.get");

    // Recv the encrypted reply.
    let reply_msg = timeout(Duration::from_secs(5), ws.next())
        .await
        .map_err(|_| anyhow!("timeout waiting for reply"))?
        .context("ws err on reply")?
        .context("server closed before reply")?;
    let reply_bytes = reply_msg.into_data();
    let (ft, payload) = decode_frame(&reply_bytes)?;
    if !matches!(ft, FrameType::App) {
        return Err(anyhow!("reply must be App frame, got {:?}", ft));
    }
    let plaintext = decrypt(&mut transport, &payload)
        .context("decrypt reply (E2EE: only the paired Noise peer can produce this)")?;
    let resp: Value = serde_json::from_slice(&plaintext).context("parse JSON-RPC reply")?;

    println!("status.get OK response: {resp}");

    let result = resp
        .get("result")
        .ok_or_else(|| anyhow!("reply missing `result` field"))?;
    if result.get("protocol_version").and_then(Value::as_u64) != Some(1) {
        return Err(anyhow!(
            "protocol_version mismatch: got {:?}",
            result.get("protocol_version")
        ));
    }
    if result.get("device_id").and_then(Value::as_str) != Some(offer.device_id.as_str()) {
        return Err(anyhow!(
            "device_id mismatch: got {:?}, expected {}",
            result.get("device_id"),
            offer.device_id
        ));
    }

    eprintln!("OK protocol_version=1 device_id={}", offer.device_id);
    Ok(())
}
