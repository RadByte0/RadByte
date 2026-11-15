//! Concurrent TCP Status Ping load generator for benchmarking Minecraft proxies.
//!
//! Usage:
//!   cargo run --release -p rad-proxy --example status_load_bench -- <addr> <concurrency> <warmup_secs> <duration_secs>

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use rad_core::packet::handshake::Handshake;
use rad_core::packet::status::{PingRequest, StatusRequest};
use rad_core::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(1).map_or("127.0.0.1:25565", String::as_str);
    let concurrency: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(64);
    let warmup_secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
    let duration_secs: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    // Pre-build Handshake + StatusRequest + PingRequest wire bytes once.
    let mut req_buf = BytesMut::with_capacity(128);

    let handshake = Handshake {
        protocol_version: VarInt(PROTOCOL_VERSION),
        server_address: "localhost".to_string(),
        server_port: 25565,
        next_state: ConnectionState::Status,
    };
    encode_packet(&handshake, &mut req_buf).unwrap();
    encode_packet(&StatusRequest, &mut req_buf).unwrap();
    let hs_status_bytes = Arc::new(req_buf.split().freeze());

    let ping = PingRequest {
        timestamp: 1_700_000_000_000,
    };
    encode_packet(&ping, &mut req_buf).unwrap();
    let ping_bytes = Arc::new(req_buf.freeze());

    let record_phase = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let addr = addr.to_string();
        let hs_status_bytes = Arc::clone(&hs_status_bytes);
        let ping_bytes = Arc::clone(&ping_bytes);
        let record_phase = Arc::clone(&record_phase);
        let stop_flag = Arc::clone(&stop_flag);
        let errors = Arc::clone(&errors);

        handles.push(tokio::spawn(async move {
            let mut latencies_us: Vec<u64> = Vec::with_capacity(4096);
            let mut rx_buf = BytesMut::with_capacity(4096);

            while !stop_flag.load(Ordering::Relaxed) {
                let start = Instant::now();
                let res =
                    run_one_status_ping(&addr, &hs_status_bytes, &ping_bytes, &mut rx_buf).await;

                if record_phase.load(Ordering::Relaxed) {
                    match res {
                        Ok(()) => {
                            latencies_us.push(start.elapsed().as_micros() as u64);
                        }
                        Err(_) => {
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            latencies_us
        }));
    }

    tokio::time::sleep(Duration::from_secs(warmup_secs)).await;
    let measure_start = Instant::now();
    record_phase.store(true, Ordering::Release);
    tokio::time::sleep(Duration::from_secs(duration_secs)).await;
    stop_flag.store(true, Ordering::Release);
    let elapsed = measure_start.elapsed();

    let mut all_latencies = Vec::new();
    for h in handles {
        if let Ok(mut v) = h.await {
            all_latencies.append(&mut v);
        }
    }

    let total_ok = all_latencies.len() as u64;
    let total_err = errors.load(Ordering::Relaxed);
    let rps = total_ok as f64 / elapsed.as_secs_f64();

    if all_latencies.is_empty() {
        println!("target={addr} ok=0 errors={total_err} rps=0.0");
        return;
    }

    all_latencies.sort_unstable();
    let sum: u64 = all_latencies.iter().sum();
    let avg_us = sum as f64 / all_latencies.len() as f64;
    let pct = |p: f64| -> u64 {
        let idx = ((all_latencies.len() - 1) as f64 * p).round() as usize;
        all_latencies[idx]
    };

    println!(
        "target={addr} concurrency={concurrency} duration={:.2}s ok={total_ok} errors={total_err} rps={rps:.1} avg_us={avg_us:.1} p50_us={} p95_us={} p99_us={} max_us={}",
        elapsed.as_secs_f64(),
        pct(0.50),
        pct(0.95),
        pct(0.99),
        all_latencies[all_latencies.len() - 1]
    );
}

async fn run_one_status_ping(
    addr: &str,
    hs_status_bytes: &[u8],
    ping_bytes: &[u8],
    rx_buf: &mut BytesMut,
) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    stream.write_all(hs_status_bytes).await?;

    rx_buf.clear();
    // Read StatusResponse (0x00)
    loop {
        if let Ok(Some(frame)) = decode_frame(rx_buf, DEFAULT_MAX_PACKET_SIZE) {
            if frame.id != 0x00 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unexpected status packet id",
                ));
            }
            break;
        }
        if !rx_buf.has_remaining_mut() {
            rx_buf.reserve(4096);
        }
        if stream.read_buf(rx_buf).await? == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
    }

    // Send PingRequest (0x01)
    stream.write_all(ping_bytes).await?;

    // Read PongResponse (0x01)
    loop {
        if let Ok(Some(frame)) = decode_frame(rx_buf, DEFAULT_MAX_PACKET_SIZE) {
            if frame.id != 0x01 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unexpected pong packet id",
                ));
            }
            break;
        }
        if !rx_buf.has_remaining_mut() {
            rx_buf.reserve(4096);
        }
        if stream.read_buf(rx_buf).await? == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
    }

    // Avoid Windows ephemeral port exhaustion (TIME_WAIT) across 300k+ rapid connections.
    let _ = socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO));

    Ok(())
}
