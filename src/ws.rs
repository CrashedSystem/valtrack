//! Riot Client local WebSocket.
//!
//! The overlay used to wake on a blind 5 s timer: every tick re-read the
//! lockfile, fetched a fresh access token and re-checked the lobby, even while
//! sitting in the menu. The client publishes the same information over a local
//! WebSocket (`wss://127.0.0.1:{port}`) and pushes
//! `OnJsonApiEvent_chat_v4_presences` whenever the session state changes
//! (menu -> agent select -> in game -> back). Subscribing lets the UI react
//! immediately and lets the fallback timer run slowly while idle.
//!
//! The endpoint only exists while the Riot Client is up, so a failed connect is
//! normal and must never be fatal: we retry with a fixed delay and the polling
//! fallback keeps working throughout.

use std::sync::mpsc::Sender;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use tungstenite::client::IntoClientRequest;
use tungstenite::{client_tls_with_config, Connector};

/// Session-state event. Other traffic is ignored.
const PRESENCE_EVENT: &str = "OnJsonApiEvent_chat_v4_presences";

const RECONNECT_DELAY: Duration = Duration::from_secs(3);

/// Reads the lockfile, connects, and forwards one wake-up per presence event.
/// Runs forever on its own thread; the process exit kills it.
pub fn spawn(tx: Sender<()>) {
    let _ = std::thread::Builder::new()
        .name("valtrack-ws".into())
        .spawn(move || loop {
            match run_once(&tx) {
                Ok(reason) => crate::log(&format!("ws: closed ({reason})")),
                Err(err) => crate::log(&format!("ws: {err:#}")),
            }
            std::thread::sleep(RECONNECT_DELAY);
        });
}

fn run_once(tx: &Sender<()>) -> Result<&'static str> {
    let lockfile = crate::paths::read_lockfile()?;

    // The client serves a self-signed certificate for 127.0.0.1; verification
    // is meaningless on loopback and would only produce spurious failures.
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .context("build TLS connector")?;

    let stream = std::net::TcpStream::connect(("127.0.0.1", lockfile.port))
        .with_context(|| format!("connect 127.0.0.1:{}", lockfile.port))?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    let _ = stream.set_nodelay(true);

    let creds = base64::engine::general_purpose::STANDARD
        .encode(format!("riot:{}", lockfile.password));
    let mut request = format!("wss://127.0.0.1:{}/", lockfile.port).into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Basic {creds}").parse()?);

    let (mut socket, _response) = client_tls_with_config(
        request,
        stream,
        None,
        Some(Connector::NativeTls(tls)),
    )
    .context("websocket handshake")?;
    crate::log("ws: connected");

    let mut seen_text = false;
    loop {
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                if !seen_text {
                    // Proves the subscription is live rather than merely connected.
                    seen_text = true;
                    crate::log(&format!("ws: first message {} bytes", text.len()));
                }
                if text.contains(PRESENCE_EVENT) {
                    // A closed receiver just means the overlay is shutting down.
                    if tx.send(()).is_err() {
                        return Ok("receiver gone");
                    }
                }
            }
            Ok(tungstenite::Message::Close(_)) => return Ok("server closed"),
            Ok(_) => {}
            // Read timeout: stay connected, the timer is only a liveness check.
            Err(tungstenite::Error::Io(err))
                if matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(err) => return Err(err).context("websocket read"),
        }
    }
}
