//! Lightweight Prometheus `/metrics` HTTP endpoint for `RadProxy`.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{error, info};

use crate::proxy::ProxyState;

/// Spawns an asynchronous HTTP/1.1 `/metrics` & `/health` server.
pub fn spawn_metrics_server(bind_addr: SocketAddr, state: Arc<ProxyState>) {
    tokio::spawn(async move {
        let listener = match TcpListener::bind(bind_addr).await {
            Ok(l) => l,
            Err(err) => {
                error!(%bind_addr, %err, "failed to bind Prometheus metrics endpoint");
                return;
            }
        };
        info!(%bind_addr, "Prometheus /metrics endpoint listening");
        let connections = Arc::new(Semaphore::new(128));

        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
                continue;
            };
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                let _permit = permit;
                let path =
                    match timeout(Duration::from_secs(5), read_request_path(&mut stream)).await {
                        Ok(Ok(path)) => path,
                        _ => return,
                    };
                let (status, content_type, body) = match path.as_str() {
                    "/health" => ("200 OK", "text/plain; charset=utf-8", "ok\n".to_owned()),
                    "/metrics" => (
                        "200 OK",
                        "text/plain; version=0.0.4; charset=utf-8",
                        format_prometheus(&state),
                    ),
                    _ => (
                        "404 Not Found",
                        "text/plain; charset=utf-8",
                        "not found\n".to_owned(),
                    ),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = timeout(Duration::from_secs(5), stream.write_all(resp.as_bytes())).await;
            });
        }
    });
}

async fn read_request_path(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<String> {
    let mut line = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        line.push(byte);
        if line.len() > 512 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP request line too long",
            ));
        }
        if line.ends_with(b"\r\n") {
            let request = std::str::from_utf8(&line[..line.len() - 2]).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP request line")
            })?;
            let mut fields = request.split(' ');
            let method = fields.next();
            let path = fields.next().unwrap_or("");
            let version = fields.next();
            if method != Some("GET")
                || !path.starts_with('/')
                || !matches!(version, Some("HTTP/1.0" | "HTTP/1.1"))
                || fields.next().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported HTTP request line",
                ));
            }
            return Ok(path.to_owned());
        }
    }
}

#[must_use]
pub fn format_prometheus(state: &ProxyState) -> String {
    let snap = state.metrics.snapshot();
    let online = state.online_players.load(Ordering::Relaxed);
    format!(
        "# HELP radproxy_online_players Current online players.\n\
         # TYPE radproxy_online_players gauge\n\
         radproxy_online_players {online}\n\
         # HELP radproxy_connections_total Total accepted TCP connections.\n\
         # TYPE radproxy_connections_total counter\n\
         radproxy_connections_total {}\n\
         # HELP radproxy_blocked_filter_total Connections blocked by IP filter.\n\
         # TYPE radproxy_blocked_filter_total counter\n\
         radproxy_blocked_filter_total {}\n\
         # HELP radproxy_blocked_radon_total Connections blocked by radons.\n\
         # TYPE radproxy_blocked_radon_total counter\n\
         radproxy_blocked_radon_total {}\n\
         # HELP radproxy_status_pings_total Total Status pings served.\n\
         # TYPE radproxy_status_pings_total counter\n\
         radproxy_status_pings_total {}\n\
         # HELP radproxy_server_switches_total Total backend server switches.\n\
         # TYPE radproxy_server_switches_total counter\n\
         radproxy_server_switches_total {}\n",
        snap.total_connections,
        snap.blocked_by_filter,
        snap.blocked_by_radon,
        snap.status_pings,
        snap.server_switches
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_fragmented_request_line_without_accepting_path_prefixes() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let write = tokio::spawn(async move {
            for part in [b"GE".as_slice(), b"T /health-extra HT", b"TP/1.1\r\n"] {
                writer.write_all(part).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert_eq!(
            read_request_path(&mut reader).await.unwrap(),
            "/health-extra"
        );
        write.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_truncated_malformed_and_oversized_requests() {
        for request in [
            b"GET /health HTTP/1.1".to_vec(),
            b"POST /metrics HTTP/1.1\r\n".to_vec(),
            vec![b'A'; 513],
        ] {
            let mut input = request.as_slice();
            assert!(read_request_path(&mut input).await.is_err());
        }
    }
}
