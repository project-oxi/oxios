//! Local Telegram Bot API test double (unit tests only).
//!
//! Serves canned JSON for every request so `validate_token` / plugin setup
//! can be exercised without touching api.telegram.org.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Canned response the fake server serves for every request.
pub(crate) enum FakeResponse {
    /// `200` + getMe-style ok payload carrying this bot username.
    Ok {
        /// Username reported by the fake bot.
        username: String,
    },
    /// `401` + Telegram-style error body.
    Unauthorized,
}

/// A running fake server. Keep it alive for the duration of the test;
/// dropping it aborts the accept loop.
pub(crate) struct FakeServer {
    /// Base URL (`http://127.0.0.1:PORT`).
    pub base_url: String,
    /// Accept-loop task; aborts when the handle drops.
    _task: tokio::task::JoinHandle<()>,
}

/// Spawn a fake Bot API server on a random loopback port.
pub(crate) async fn spawn(response: FakeResponse) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake telegram server");
    let addr = listener.local_addr().expect("fake server local addr");
    let canned: Arc<(&'static str, String)> = Arc::new(match response {
        FakeResponse::Ok { username } => (
            "200 OK",
            format!(
                r#"{{"ok":true,"result":{{"id":1,"is_bot":true,"first_name":"Oxios","username":"{username}"}}}}"#
            ),
        ),
        FakeResponse::Unauthorized => (
            "401 Unauthorized",
            r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#.to_string(),
        ),
    });

    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let canned = canned.clone();
            tokio::spawn(async move {
                // Drain the request (headers + Content-Length body). Requests
                // from reqwest are small; 16 KiB is a generous ceiling.
                let mut buf = vec![0u8; 16 * 1024];
                let mut total = 0usize;
                loop {
                    if total >= buf.len() {
                        break;
                    }
                    match socket.read(&mut buf[total..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            total += n;
                            let req = String::from_utf8_lossy(&buf[..total]);
                            if request_complete(&req) {
                                break;
                            }
                        }
                    }
                }
                let (status, json) = &*canned;
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                    json.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    FakeServer {
        base_url: format!("http://{addr}"),
        _task: task,
    }
}

/// Whether a raw HTTP/1.1 request has been fully received.
fn request_complete(req: &str) -> bool {
    let Some(header_end) = req.find("\r\n\r\n") else {
        return false;
    };
    let content_length = req
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    req.len() >= header_end + 4 + content_length
}
