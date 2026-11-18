//! Real QUIC handshakes over loopback UDP: identity pinning, channel
//! binding and framed I/O limits.

use lanlink_auth::Identity;
use lanlink_protocol::{ControlMsg, StreamKind};
use lanlink_transport::*;
use std::net::SocketAddr;

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

async fn connect(
    server: &quinn::Endpoint,
    client_id: &Identity,
    trust: ServerTrust,
) -> Result<(quinn::Connection, Option<quinn::Connection>), String> {
    let client = client_endpoint(loopback()).unwrap();
    let cfg = client_config(client_id, trust).unwrap();
    let addr = server.local_addr().unwrap();
    let connecting = client.connect_with(cfg, addr, SERVER_NAME).unwrap();
    let server = server.clone();
    let accept = tokio::spawn(async move {
        let incoming = server.accept().await?;
        incoming.await.ok()
    });
    match connecting.await {
        Ok(c) => Ok((c, accept.await.unwrap())),
        Err(e) => Err(e.to_string()),
    }
}

#[tokio::test]
async fn pinned_fingerprint_connects_and_both_sides_see_each_other() {
    let host = Identity::generate("host").unwrap();
    let phone = Identity::generate("phone").unwrap();
    let server = server_endpoint(&host, loopback()).unwrap();

    let (c, s) = connect(&server, &phone, ServerTrust::Pinned(host.fingerprint()))
        .await
        .unwrap();
    let s = s.unwrap();
    assert_eq!(peer_fingerprint(&c).unwrap(), host.fingerprint());
    assert_eq!(peer_fingerprint(&s).unwrap(), phone.fingerprint());
    // Both ends derive the same binding, and it is not all zeros.
    let b = channel_binding(&c).unwrap();
    assert_eq!(b, channel_binding(&s).unwrap());
    assert_ne!(b, [0u8; 32]);
}

#[tokio::test]
async fn wrong_pinned_fingerprint_is_rejected_in_the_handshake() {
    let host = Identity::generate("host").unwrap();
    let impostor = Identity::generate("impostor").unwrap();
    let phone = Identity::generate("phone").unwrap();
    // The phone expects `host` but reaches `impostor`.
    let server = server_endpoint(&impostor, loopback()).unwrap();
    let err = connect(&server, &phone, ServerTrust::Pinned(host.fingerprint()))
        .await
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("certificate") || err.to_lowercase().contains("crypto"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn channel_binding_differs_per_connection() {
    let host = Identity::generate("host").unwrap();
    let phone = Identity::generate("phone").unwrap();
    let server = server_endpoint(&host, loopback()).unwrap();
    let (c1, _s1) = connect(&server, &phone, ServerTrust::Unverified)
        .await
        .unwrap();
    let (c2, _s2) = connect(&server, &phone, ServerTrust::Unverified)
        .await
        .unwrap();
    assert_ne!(channel_binding(&c1).unwrap(), channel_binding(&c2).unwrap());
}

#[tokio::test]
async fn control_roundtrip_and_oversize_rejection() {
    let host = Identity::generate("host").unwrap();
    let phone = Identity::generate("phone").unwrap();
    let server = server_endpoint(&host, loopback()).unwrap();
    let (c, s) = connect(&server, &phone, ServerTrust::Pinned(host.fingerprint()))
        .await
        .unwrap();
    let s = s.unwrap();

    let (mut send, _recv) = c.open_bi().await.unwrap();
    write_stream_kind(&mut send, StreamKind::Control)
        .await
        .unwrap();
    write_control(&mut send, &ControlMsg::KeyframeRequest)
        .await
        .unwrap();
    // A length prefix far beyond the limit, with no body behind it.
    send.write_all(&u32::MAX.to_le_bytes()).await.unwrap();

    let (_s_send, mut s_recv) = s.accept_bi().await.unwrap();
    assert_eq!(
        read_stream_kind(&mut s_recv).await.unwrap(),
        StreamKind::Control
    );
    assert_eq!(
        read_control(&mut s_recv).await.unwrap(),
        Some(ControlMsg::KeyframeRequest)
    );
    // Rejected immediately from the 4-byte prefix: no 4 GiB allocation, no hang.
    assert!(matches!(
        read_control(&mut s_recv).await,
        Err(TransportError::Malformed(_))
    ));
}

#[tokio::test]
async fn clean_finish_reads_as_none() {
    let host = Identity::generate("host").unwrap();
    let phone = Identity::generate("phone").unwrap();
    let server = server_endpoint(&host, loopback()).unwrap();
    let (c, s) = connect(&server, &phone, ServerTrust::Pinned(host.fingerprint()))
        .await
        .unwrap();
    let s = s.unwrap();
    let (mut send, _recv) = c.open_bi().await.unwrap();
    write_stream_kind(&mut send, StreamKind::Control)
        .await
        .unwrap();
    send.finish().unwrap();
    let (_s_send, mut s_recv) = s.accept_bi().await.unwrap();
    read_stream_kind(&mut s_recv).await.unwrap();
    assert_eq!(read_control(&mut s_recv).await.unwrap(), None);
}
