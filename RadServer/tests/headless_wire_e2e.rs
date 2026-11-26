//! Independent headless MC 26.3 (`777`) wire client & full-process restart/persistence E2E test.
//!
//! Intentionally does NOT import `rad_core` or `rad_server` packet structs — implements its own
//! raw VarInt/frame/packet codecs to independently verify wire compatibility (T03–T07).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn write_varint(out: &mut Vec<u8>, mut val: i32) {
    let mut u = val as u32;
    loop {
        let mut b = (u & 0x7F) as u8;
        u >>= 7;
        if u != 0 {
            b |= 0x80;
        }
        out.push(b);
        if u == 0 {
            break;
        }
        val = u as i32;
        let _ = val;
    }
}

fn read_varint_slice(buf: &[u8], pos: &mut usize) -> Result<i32, String> {
    let mut num_read = 0;
    let mut result = 0i32;
    loop {
        if *pos >= buf.len() {
            return Err("unexpected EOF reading VarInt".into());
        }
        let byte = buf[*pos];
        *pos += 1;
        result |= i32::from(byte & 0x7F) << (7 * num_read);
        num_read += 1;
        if num_read > 5 {
            return Err("VarInt too long".into());
        }
        if (byte & 0x80) == 0 {
            return Ok(result);
        }
    }
}

fn write_mc_string(out: &mut Vec<u8>, s: &str) {
    write_varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

fn encode_block_pos(x: i32, y: i32, z: i32) -> i64 {
    ((i64::from(x) & 0x3FF_FFFF) << 38)
        | ((i64::from(z) & 0x3FF_FFFF) << 12)
        | (i64::from(y) & 0xFFF)
}

fn offline_uuid_bytes(name: &str) -> [u8; 16] {
    // Simple deterministic UUID bytes for test client; server uses OfflinePlayer:<name>
    let mut b = [0u8; 16];
    for (i, byte) in name.bytes().take(16).enumerate() {
        b[i] = byte;
    }
    b[6] = (b[6] & 0x0F) | 0x30;
    b[8] = (b[8] & 0x3F) | 0x80;
    b
}

async fn send_raw_packet(stream: &mut TcpStream, id: i32, body: &[u8]) {
    let mut inner = Vec::new();
    write_varint(&mut inner, id);
    inner.extend_from_slice(body);
    let mut framed = Vec::new();
    write_varint(&mut framed, inner.len() as i32);
    framed.extend_from_slice(&inner);
    stream.write_all(&framed).await.unwrap();
}

async fn read_raw_packet(stream: &mut TcpStream, rx_buf: &mut Vec<u8>) -> (i32, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let mut pos = 0usize;
        if let Ok(frame_len) = read_varint_slice(rx_buf, &mut pos) {
            let len = frame_len as usize;
            if rx_buf.len() >= pos + len {
                let frame = rx_buf[pos..pos + len].to_vec();
                rx_buf.drain(..pos + len);
                let mut fpos = 0usize;
                let id = read_varint_slice(&frame, &mut fpos).unwrap();
                return (id, frame[fpos..].to_vec());
            }
        }
        assert!(Instant::now() < deadline, "timed out reading wire packet");
        let mut tmp = [0u8; 8192];
        let n = tokio::time::timeout(Duration::from_secs(4), stream.read(&mut tmp))
            .await
            .expect("socket read timeout")
            .expect("socket read error");
        assert!(n > 0, "unexpected EOF from RadServer");
        rx_buf.extend_from_slice(&tmp[..n]);
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

struct ServerChildGuard(Option<Child>);

impl ServerChildGuard {
    fn id(&self) -> u32 {
        self.0.as_ref().expect("child already consumed").id()
    }
}

impl Drop for ServerChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn read_mc_string_slice(buf: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = read_varint_slice(buf, pos)? as usize;
    if *pos + len > buf.len() {
        return Err("unexpected EOF reading string".into());
    }
    let s = std::str::from_utf8(&buf[*pos..*pos + len])
        .map_err(|e| format!("invalid utf8 string: {e}"))?
        .to_owned();
    *pos += len;
    Ok(s)
}

fn spawn_rad_server_with_log(
    dir: &Path,
    port: u16,
    rust_log: &str,
) -> Result<ServerChildGuard, String> {
    let bin = env!("CARGO_BIN_EXE_rad-server");
    stage_version_package(dir);
    let cfg = format!(
        r#"config_version = 1
bind_addr = "127.0.0.1:{port}"
forwarding_mode = "none"
forwarding_secret = "independent-test-secret"
motd = "Headless Wire Test"
max_players = 20
view_distance = 2
tps = 20
game_mode = 0
spawn_x = 0.5
spawn_y = 64.0
spawn_z = 0.5
brand = "RadServer"
max_packet_size = 2097152
admins = ["WireAlice"]
max_mobs = 64
autosave_interval_secs = 1
"#
    );
    std::fs::write(dir.join("radserver.toml"), cfg).map_err(|e| e.to_string())?;
    let properties = dir.join("rad.properties");
    if properties.exists() {
        let raw = std::fs::read_to_string(&properties).map_err(|e| e.to_string())?;
        let updated = raw
            .lines()
            .map(|line| {
                if line.starts_with("bind-addr=") {
                    format!("bind-addr=127.0.0.1:{port}")
                } else if line.starts_with("server-port=") {
                    format!("server-port={port}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(properties, updated).map_err(|e| e.to_string())?;
    }

    let mut child = Command::new(bin)
        .arg("--nogui")
        .current_dir(dir)
        .env("RUST_LOG", rust_log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn rad-server binary: {e}"))?;

    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut sink = String::new();
            while reader.read_line(&mut sink).unwrap_or(0) > 0 {
                eprintln!("[rad-server:out] {}", sink.trim_end());
                sink.clear();
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut sink = String::new();
            while reader.read_line(&mut sink).unwrap_or(0) > 0 {
                eprintln!("[rad-server:err] {}", sink.trim_end());
                sink.clear();
            }
        });
    }

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("try_wait failed: {e}"))?
        {
            return Err(format!("rad-server exited early with status {status}"));
        }
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
            return Ok(ServerChildGuard(Some(child)));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out waiting for rad-server TCP readiness".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn spawn_rad_server(dir: &Path, port: u16) -> ServerChildGuard {
    spawn_rad_server_with_log(dir, port, "info").expect("failed to start rad-server")
}

fn stop_rad_server(mut guard: ServerChildGuard) {
    let mut child = guard.0.take().expect("child already stopped");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"stop\n");
        let _ = stdin.flush();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("failed polling rad-server exit") {
            assert!(
                status.success(),
                "rad-server exited with non-zero: {status}"
            );
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("rad-server did not exit within 10s after 'stop'");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

async fn connect_raw_mc777(
    addr: &str,
    port: u16,
    username: &str,
) -> (TcpStream, Vec<u8>, Vec<(i32, i32)>) {
    connect_raw_mc777_opts(addr, port, username, true).await
}

async fn connect_raw_mc777_opts(
    addr: &str,
    port: u16,
    username: &str,
    expect_chunks: bool,
) -> (TcpStream, Vec<u8>, Vec<(i32, i32)>) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut buf = Vec::new();

    // 1. Handshake (0x00): protocol=777, host="localhost", port, next_state=2 (Login)
    let mut hs = Vec::new();
    write_varint(&mut hs, 777);
    write_mc_string(&mut hs, "localhost");
    hs.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut hs, 2);
    send_raw_packet(&mut stream, 0x00, &hs).await;

    // 2. LoginStart (0x00): username + uuid
    let mut ls = Vec::new();
    write_mc_string(&mut ls, username);
    ls.extend_from_slice(&offline_uuid_bytes(username));
    send_raw_packet(&mut stream, 0x00, &ls).await;

    // Expect LoginSuccess (0x02) and decode full payload
    let (id, payload) = read_raw_packet(&mut stream, &mut buf).await;
    assert_eq!(id, 0x02, "expected LoginSuccess (0x02)");
    {
        let mut pos = 0usize;
        assert!(payload.len() >= 16, "LoginSuccess missing UUID");
        pos += 16;
        let parsed_name = read_mc_string_slice(&payload, &mut pos).unwrap();
        assert_eq!(parsed_name, username);
        let prop_count = read_varint_slice(&payload, &mut pos).unwrap();
        assert_eq!(prop_count, 0);
        assert!(pos + 16 <= payload.len(), "LoginSuccess missing session_id");
        pos += 16; // session_id (Uuid)
        assert_eq!(pos, payload.len(), "LoginSuccess trailing bytes");
    }

    // 3. LoginAcknowledged (0x03) -> Configuration state
    send_raw_packet(&mut stream, 0x03, &[]).await;

    let mut registry_ids = Vec::new();
    loop {
        let (cid, cbody) = read_raw_packet(&mut stream, &mut buf).await;
        match cid {
            0x0F => {
                // ClientboundKnownPacks: echo matching ServerboundKnownPacks (0x07)
                send_raw_packet(&mut stream, 0x07, &cbody).await;
            }
            0x07 => {
                // RegistryData: identifier + entry count
                let mut pos = 0usize;
                let reg_id = read_mc_string_slice(&cbody, &mut pos).unwrap();
                let entries = read_varint_slice(&cbody, &mut pos).unwrap();
                assert!(entries > 0, "RegistryData {reg_id} has 0 entries");
                registry_ids.push(reg_id);
            }
            0x03 => {
                // FinishConfiguration
                assert!(cbody.is_empty(), "FinishConfiguration should be empty");
                break;
            }
            _ => {}
        }
    }
    assert!(
        !registry_ids.is_empty(),
        "expected RegistryData in Configuration"
    );

    // AcknowledgeFinishConfiguration (0x03) -> Play state
    send_raw_packet(&mut stream, 0x03, &[]).await;

    // 4. Play: expect PlayLogin (0x32), Chunks (0x2E), SynchronizePlayerPosition (0x49), ContainerSetContent (0x12)
    let mut saw_play_login = false;
    let mut saw_chunk = false;
    let mut initial_inv = Vec::new();

    for _ in 0..80 {
        let (pid, pbody) = read_raw_packet(&mut stream, &mut buf).await;
        match pid {
            0x32 => {
                // PlayLogin: entity_id (i32) + is_hardcore (bool) + dimension_names (Vec<String>) + ...
                let mut pos = 0usize;
                let eid = i32::from_be_bytes(pbody[pos..pos + 4].try_into().unwrap());
                pos += 4;
                assert!(eid > 0, "PlayLogin entity_id must be > 0");
                let _is_hardcore = pbody[pos];
                pos += 1;
                let dim_count = read_varint_slice(&pbody, &mut pos).unwrap();
                assert!(dim_count >= 1, "PlayLogin must list >= 1 dimension");
                saw_play_login = true;
            }
            0x2E => {
                // LevelChunkWithLight: chunk_x (i32), chunk_z (i32), ...
                assert!(pbody.len() > 16, "LevelChunkWithLight too short");
                saw_chunk = true;
            }
            0x12 => {
                // ContainerSetContent: window_id, state_id, count, slots..., carried_item
                let mut pos = 0;
                let wid = read_varint_slice(&pbody, &mut pos).unwrap();
                let _sid = read_varint_slice(&pbody, &mut pos).unwrap();
                let count = read_varint_slice(&pbody, &mut pos).unwrap() as usize;
                if wid == 0 {
                    initial_inv.clear();
                    for _ in 0..count {
                        let sc = read_varint_slice(&pbody, &mut pos).unwrap();
                        let iid = if sc > 0 {
                            let id = read_varint_slice(&pbody, &mut pos).unwrap();
                            let _add = read_varint_slice(&pbody, &mut pos).unwrap();
                            let _rem = read_varint_slice(&pbody, &mut pos).unwrap();
                            id
                        } else {
                            0
                        };
                        initial_inv.push((sc, iid));
                    }
                    // carried_item
                    let carried_c = read_varint_slice(&pbody, &mut pos).unwrap();
                    if carried_c > 0 {
                        let _ = read_varint_slice(&pbody, &mut pos).unwrap();
                        let _ = read_varint_slice(&pbody, &mut pos).unwrap();
                        let _ = read_varint_slice(&pbody, &mut pos).unwrap();
                    }
                    assert_eq!(pos, pbody.len(), "ContainerSetContent trailing bytes");
                }
            }
            0x49 => {
                // SynchronizePlayerPosition: teleport_id + 5 doubles/floats...
                let mut pos = 0;
                let tp_id = read_varint_slice(&pbody, &mut pos).unwrap();
                let mut ack = Vec::new();
                write_varint(&mut ack, tp_id);
                // x, y, z (3 * f64), yaw, pitch (2 * f32)
                ack.extend_from_slice(&pbody[pos..pos + 32]);
                send_raw_packet(&mut stream, 0x00, &ack).await;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_play_login, "expected PlayLogin (0x32)");
    if expect_chunks {
        assert!(saw_chunk, "expected LevelChunkWithLight (0x2E)");
    }
    (stream, buf, initial_inv)
}

fn encode_simple_slot(out: &mut Vec<u8>, count: i32, item_id: i32) {
    if count <= 0 || item_id <= 0 {
        write_varint(out, 0);
    } else {
        write_varint(out, count);
        write_varint(out, item_id);
        write_varint(out, 0);
        write_varint(out, 0);
    }
}

async fn send_container_click(
    stream: &mut TcpStream,
    window_id: i32,
    state_id: i32,
    slot: i16,
    button: i8,
    mode: i32,
) {
    let mut b = Vec::new();
    write_varint(&mut b, window_id);
    write_varint(&mut b, state_id);
    b.extend_from_slice(&slot.to_be_bytes());
    b.push(button as u8);
    write_varint(&mut b, mode);
    write_varint(&mut b, 0); // changed_slots
    encode_simple_slot(&mut b, 0, 0);
    send_raw_packet(stream, 0x12, &b).await;
}

async fn read_window_content(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    expected_wid: i32,
) -> (i32, Vec<(i32, i32)>) {
    for _ in 0..30 {
        let (pid, pbody) = read_raw_packet(stream, buf).await;
        if pid == 0x12 {
            let mut pos = 0;
            let wid = read_varint_slice(&pbody, &mut pos).unwrap();
            let sid = read_varint_slice(&pbody, &mut pos).unwrap();
            let count = read_varint_slice(&pbody, &mut pos).unwrap() as usize;
            if wid == expected_wid {
                let mut slots = Vec::with_capacity(count);
                for _ in 0..count {
                    let sc = read_varint_slice(&pbody, &mut pos).unwrap();
                    let iid = if sc > 0 {
                        let id = read_varint_slice(&pbody, &mut pos).unwrap();
                        let _add = read_varint_slice(&pbody, &mut pos).unwrap();
                        let _rem = read_varint_slice(&pbody, &mut pos).unwrap();
                        id
                    } else {
                        0
                    };
                    slots.push((sc, iid));
                }
                return (sid, slots);
            }
        }
    }
    panic!("timed out waiting for ContainerSetContent wid={expected_wid}");
}

#[tokio::test]
#[ignore = "requires staged versions/26.3 data package"]
async fn headless_wire_two_players_craft_chest_autosave_and_full_process_restart() {
    let dir = std::env::temp_dir().join(format!("rad_wire_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let child1 = spawn_rad_server(&dir, port);
    let pid1 = child1.id();

    let (mut alice, mut abuf, alice_inv) = connect_raw_mc777(&addr, port, "WireAlice").await;
    let (mut bob, mut bbuf, _bob_inv) = connect_raw_mc777(&addr, port, "WireBob").await;

    // Starter planks at slot 40: count 16, item_id 63 (minecraft:oak_planks in 777)
    assert_eq!(alice_inv[40], (16, 63));

    // 1. Craft 1x Crafting Table (405) in 2x2 grid: 16 -> 12 planks at slot 40, 1x 405 at slot 41
    let mut state_id = 1;
    send_container_click(&mut alice, 0, state_id, 40, 0, 0).await;
    state_id = read_window_content(&mut alice, &mut abuf, 0).await.0;
    for g in 1..=4i16 {
        send_container_click(&mut alice, 0, state_id, g, 1, 0).await;
        state_id = read_window_content(&mut alice, &mut abuf, 0).await.0;
    }
    send_container_click(&mut alice, 0, state_id, 40, 0, 0).await;
    state_id = read_window_content(&mut alice, &mut abuf, 0).await.0;
    send_container_click(&mut alice, 0, state_id, 0, 0, 1).await;
    let (_sid_after_craft, inv_after_craft) = read_window_content(&mut alice, &mut abuf, 0).await;
    assert_eq!(inv_after_craft[40], (12, 63));
    assert_eq!(inv_after_craft[41], (1, 405));

    // Place block in negative chunk (-2, 64, -2) using slot 0 (stone)
    let mut place_neg = Vec::new();
    write_varint(&mut place_neg, 0); // main hand
    place_neg.extend_from_slice(&encode_block_pos(-2, 63, -2).to_be_bytes());
    write_varint(&mut place_neg, 1); // Top face -> (-2, 64, -2)
    place_neg.extend_from_slice(&0.5f32.to_be_bytes());
    place_neg.extend_from_slice(&1.0f32.to_be_bytes());
    place_neg.extend_from_slice(&0.5f32.to_be_bytes());
    place_neg.push(0); // inside_block
    place_neg.push(0); // world_border_hit
    write_varint(&mut place_neg, 1); // sequence
    send_raw_packet(&mut alice, 0x42, &place_neg).await;

    // Select hotbar slot 5 (inventory slot 41 = crafting_table) and place at (1, 64, 1)
    send_raw_packet(&mut alice, 0x36, &5i16.to_be_bytes()).await;
    let mut place_table = Vec::new();
    write_varint(&mut place_table, 0);
    place_table.extend_from_slice(&encode_block_pos(1, 63, 1).to_be_bytes());
    write_varint(&mut place_table, 1);
    place_table.extend_from_slice(&0.5f32.to_be_bytes());
    place_table.extend_from_slice(&1.0f32.to_be_bytes());
    place_table.extend_from_slice(&0.5f32.to_be_bytes());
    place_table.push(0);
    place_table.push(0);
    write_varint(&mut place_table, 2);
    send_raw_packet(&mut alice, 0x42, &place_table).await;

    // Right-click Crafting Table at (1, 64, 1) -> wid=2 (CRAFTING_WINDOW_ID), craft Chest (404) from 8 planks
    let mut open_table = Vec::new();
    write_varint(&mut open_table, 0);
    open_table.extend_from_slice(&encode_block_pos(1, 64, 1).to_be_bytes());
    write_varint(&mut open_table, 1);
    open_table.extend_from_slice(&0.5f32.to_be_bytes());
    open_table.extend_from_slice(&0.5f32.to_be_bytes());
    open_table.extend_from_slice(&0.5f32.to_be_bytes());
    open_table.push(0);
    open_table.push(0);
    write_varint(&mut open_table, 3);
    send_raw_packet(&mut alice, 0x42, &open_table).await;

    let (table_sid, _) = read_window_content(&mut alice, &mut abuf, 2).await;
    state_id = table_sid;
    // In 3x3 table (10..46), player slot 40 is index 10 + (40 - 9) = 41
    send_container_click(&mut alice, 2, state_id, 41, 0, 0).await;
    state_id = read_window_content(&mut alice, &mut abuf, 2).await.0;
    for g in [1i16, 2, 3, 4, 6, 7, 8, 9] {
        send_container_click(&mut alice, 2, state_id, g, 1, 0).await;
        state_id = read_window_content(&mut alice, &mut abuf, 2).await.0;
    }
    send_container_click(&mut alice, 2, state_id, 41, 0, 0).await;
    state_id = read_window_content(&mut alice, &mut abuf, 2).await.0;
    send_container_click(&mut alice, 2, state_id, 0, 0, 1).await;
    let (_sid_table_done, table_inv_after) = read_window_content(&mut alice, &mut abuf, 2).await;
    assert_eq!(table_inv_after[41], (4, 63)); // 12 - 8 = 4 planks
    assert_eq!(table_inv_after[42], (1, 404)); // 1x crafted chest at slot 41 (index 42 in 3x3 window)
    send_raw_packet(&mut alice, 0x13, &[2u8]).await; // ContainerClose (0x13) wid=2

    // Select hotbar slot 5 (inventory slot 41 = crafted chest) and place Chest at (2, 64, 2)
    send_raw_packet(&mut alice, 0x36, &5i16.to_be_bytes()).await;
    let mut place_chest = Vec::new();
    write_varint(&mut place_chest, 0);
    place_chest.extend_from_slice(&encode_block_pos(2, 63, 2).to_be_bytes());
    write_varint(&mut place_chest, 1);
    place_chest.extend_from_slice(&0.5f32.to_be_bytes());
    place_chest.extend_from_slice(&1.0f32.to_be_bytes());
    place_chest.extend_from_slice(&0.5f32.to_be_bytes());
    place_chest.push(0);
    place_chest.push(0);
    write_varint(&mut place_chest, 4);
    send_raw_packet(&mut alice, 0x42, &place_chest).await;

    // Right-click Chest at (2, 64, 2) to open it (OpenScreen 0x39 + ContainerSetContent wid=1)
    let mut open_chest = Vec::new();
    write_varint(&mut open_chest, 0);
    open_chest.extend_from_slice(&encode_block_pos(2, 64, 2).to_be_bytes());
    write_varint(&mut open_chest, 1);
    open_chest.extend_from_slice(&0.5f32.to_be_bytes());
    open_chest.extend_from_slice(&0.5f32.to_be_bytes());
    open_chest.extend_from_slice(&0.5f32.to_be_bytes());
    open_chest.push(0);
    open_chest.push(0);
    write_varint(&mut open_chest, 5);
    send_raw_packet(&mut alice, 0x42, &open_chest).await;

    let (chest_sid, _chest_slots) = read_window_content(&mut alice, &mut abuf, 1).await;
    state_id = chest_sid;
    // Shift-click 4 oak_planks (slot 40 -> chest window index 27 + (40 - 9) = 58) into chest
    send_container_click(&mut alice, 1, state_id, 58, 0, 1).await;
    let (_sid2, chest_after) = read_window_content(&mut alice, &mut abuf, 1).await;
    assert_eq!(chest_after[0], (4, 63));

    // Bob observes chat broadcast from Alice
    let mut chat = Vec::new();
    write_mc_string(&mut chat, "hello-wire");
    chat.extend_from_slice(&0i64.to_be_bytes());
    chat.extend_from_slice(&0i64.to_be_bytes());
    chat.push(0); // no signature
    write_varint(&mut chat, 0);
    chat.extend_from_slice(&[0u8; 3]);
    chat.push(0);
    send_raw_packet(&mut alice, 0x09, &chat).await;

    let mut bob_saw_chat = false;
    for _ in 0..30 {
        let (pid, pbody) = read_raw_packet(&mut bob, &mut bbuf).await;
        if pid == 0x7C && String::from_utf8_lossy(&pbody).contains("hello-wire") {
            bob_saw_chat = true;
            break;
        }
    }
    assert!(bob_saw_chat, "Bob should receive Alice's chat");

    // Wait for autosave_interval_secs = 1 WITHOUT /save -> check disk files!
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let containers_disk =
        std::fs::read_to_string(dir.join("world").join("containers.json")).unwrap();
    let containers_json: serde_json::Value = serde_json::from_str(&containers_disk).unwrap();
    assert_eq!(containers_json[0]["x"], 2);
    assert_eq!(containers_json[0]["y"], 64);
    assert_eq!(containers_json[0]["z"], 2);
    assert_eq!(containers_json[0]["slots"][0]["count"], 4);
    assert_eq!(containers_json[0]["slots"][0]["item_id"], 63);

    drop(alice);
    drop(bob);
    stop_rad_server(child1);

    // Start a BRAND NEW rad-server process on the same world/ directory!
    let port2 = free_port();
    let addr2 = format!("127.0.0.1:{port2}");
    let child2 = spawn_rad_server(&dir, port2);
    let pid2 = child2.id();
    assert_ne!(pid1, pid2);

    let (mut alice2, mut abuf2, inv_restored) = connect_raw_mc777(&addr2, port2, "WireAlice").await;
    // Alice's stone was decremented to 63, planks (40) empty (moved to chest), chest (41) empty (placed)
    assert_eq!(inv_restored[36], (63, 1));
    assert_eq!(inv_restored[40], (0, 0));
    assert_eq!(inv_restored[41], (0, 0));

    // Re-open Chest at (2, 64, 2) on fresh process -> 4 oak_planks still inside!
    send_raw_packet(&mut alice2, 0x42, &open_chest).await;
    let (_sid3, chest_restored) = read_window_content(&mut alice2, &mut abuf2, 1).await;
    assert_eq!(chest_restored[0], (4, 63));

    // Break (-2, 64, -2) in negative chunk (-1, -1) on fresh process -> BlockUpdate(air) proves it persisted!
    let mut start_dig = Vec::new();
    write_varint(&mut start_dig, 0); // Start digging
    start_dig.extend_from_slice(&encode_block_pos(-2, 64, -2).to_be_bytes());
    start_dig.push(1);
    write_varint(&mut start_dig, 10);
    send_raw_packet(&mut alice2, 0x29, &start_dig).await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let mut finish_dig = Vec::new();
    write_varint(&mut finish_dig, 2); // Finish digging
    finish_dig.extend_from_slice(&encode_block_pos(-2, 64, -2).to_be_bytes());
    finish_dig.push(1);
    write_varint(&mut finish_dig, 11);
    send_raw_packet(&mut alice2, 0x29, &finish_dig).await;

    let mut saw_neg_chunk_break = false;
    let expected_neg_pos = encode_block_pos(-2, 64, -2);
    for _ in 0..30 {
        let (pid, pbody) = read_raw_packet(&mut alice2, &mut abuf2).await;
        if pid == 0x08 && pbody.len() >= 9 {
            let pos = i64::from_be_bytes(pbody[0..8].try_into().unwrap());
            let mut pos_idx = 8usize;
            let state_id = read_varint_slice(&pbody, &mut pos_idx).unwrap_or(-1);
            if pos == expected_neg_pos && state_id == 0 {
                saw_neg_chunk_break = true;
                break;
            }
        }
    }
    assert!(
        saw_neg_chunk_break,
        "(-2, 64, -2) in negative chunk (-1, -1) should persist across restart and break to air"
    );

    drop(alice2);
    stop_rad_server(child2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "requires staged versions/26.3 data package"]
async fn headless_startup_works_under_warn_logging_and_bounded_failure() {
    let dir = std::env::temp_dir().join(format!("rad_warn_boot_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. Verify startup succeeds under RUST_LOG=warn without hanging on INFO logs.
    let port = free_port();
    let guard = spawn_rad_server_with_log(&dir, port, "warn")
        .expect("rad-server must start under RUST_LOG=warn");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut buf = Vec::new();
    let mut hs = Vec::new();
    write_varint(&mut hs, 777);
    write_mc_string(&mut hs, "localhost");
    hs.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut hs, 1); // Status
    send_raw_packet(&mut stream, 0x00, &hs).await;
    send_raw_packet(&mut stream, 0x00, &[]).await;

    let (id, sbody) = read_raw_packet(&mut stream, &mut buf).await;
    assert_eq!(id, 0x00, "expected StatusResponse (0x00)");
    let mut pos = 0usize;
    let json = read_mc_string_slice(&sbody, &mut pos).unwrap();
    assert_eq!(pos, sbody.len());
    assert!(
        json.contains("\"protocol\":777"),
        "expected protocol 777 in status JSON: {json}"
    );
    drop(stream);
    stop_rad_server(guard);

    // 2. Verify invalid bind_addr fails fast in a bounded way with no orphan child.
    let bad_dir = dir.join("bad_cfg");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(
        bad_dir.join("radserver.toml"),
        "config_version = 1\nbind_addr = \"not-an-ip:bad\"\n",
    )
    .unwrap();
    let bin = env!("CARGO_BIN_EXE_rad-server");
    let mut bad_child = Command::new(bin)
        .arg("--nogui")
        .current_dir(&bad_dir)
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let exited = loop {
        if let Some(status) = bad_child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = bad_child.kill();
            let _ = bad_child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let status = exited.expect("invalid config must exit within 5s");
    assert!(!status.success(), "invalid config must exit non-zero");

    let _ = std::fs::remove_dir_all(&dir);
}

fn ensure_sibling_bin(name: &str) -> std::path::PathBuf {
    let server_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rad-server"));
    let debug_dir = server_bin.parent().expect("bin parent");
    let exe_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let candidate = debug_dir.join(&exe_name);
    if candidate.exists() {
        return candidate;
    }
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", name, "--bin", name])
        .status()
        .expect("failed to invoke cargo build for sibling binary");
    assert!(status.success(), "cargo build --bin {name} failed");
    assert!(
        candidate.exists(),
        "expected built binary at {}",
        candidate.display()
    );
    candidate
}

fn version_library_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "rad_version_26_3.dll"
    } else if cfg!(target_os = "macos") {
        "librad_version_26_3.dylib"
    } else {
        "librad_version_26_3.so"
    }
}

fn stage_version_package(dir: &Path) {
    let lib_name = version_library_filename();
    let dst = dir.join("versions").join("26.3");
    if dst.join("version.json").exists() && dst.join(lib_name).exists() {
        return;
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("versions")
        .join("26.3");
    std::fs::create_dir_all(dst.join("data")).expect("create versions/26.3/data");
    std::fs::copy(src.join("version.json"), dst.join("version.json")).expect("copy version.json");
    std::fs::copy(src.join(lib_name), dst.join(lib_name)).expect("copy version library");
    for f in ["items.json", "blocks.json", "entities.json", "menus.json"] {
        std::fs::copy(src.join("data").join(f), dst.join("data").join(f)).expect("copy data json");
    }
}

fn spawn_bin_and_wait_tcp(bin: &Path, dir: &Path, port: u16, label: &str) -> ServerChildGuard {
    if bin.file_stem().and_then(|s| s.to_str()) == Some("rad-server") {
        stage_version_package(dir);
    }
    let mut child = Command::new(bin)
        .args(
            if bin.file_stem().and_then(|s| s.to_str()) == Some("rad-server") {
                vec!["--nogui"]
            } else {
                vec![]
            },
        )
        .current_dir(dir)
        .env("RUST_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {label}: {e}"));

    if let Some(stdout) = child.stdout.take() {
        let lbl = label.to_owned();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut sink = String::new();
            while reader.read_line(&mut sink).unwrap_or(0) > 0 {
                eprintln!("[{lbl}:out] {}", sink.trim_end());
                sink.clear();
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let lbl = label.to_owned();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut sink = String::new();
            while reader.read_line(&mut sink).unwrap_or(0) > 0 {
                eprintln!("[{lbl}:err] {}", sink.trim_end());
                sink.clear();
            }
        });
    }

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait failed") {
            panic!("{label} exited early with status {status}");
        }
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
            return ServerChildGuard(Some(child));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for {label} TCP readiness on {addr}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn parse_play_login_entity_and_gamemode(pbody: &[u8]) -> (i32, u8) {
    let mut pos = 0usize;
    let entity_id = i32::from_be_bytes(pbody[pos..pos + 4].try_into().unwrap());
    pos += 4;
    pos += 1; // is_hardcore
    let dim_count = read_varint_slice(pbody, &mut pos).unwrap();
    for _ in 0..dim_count {
        let _ = read_mc_string_slice(pbody, &mut pos).unwrap();
    }
    let _max_players = read_varint_slice(pbody, &mut pos).unwrap();
    let _view_distance = read_varint_slice(pbody, &mut pos).unwrap();
    let _sim_distance = read_varint_slice(pbody, &mut pos).unwrap();
    pos += 3; // reduced_debug_info, enable_respawn_screen, do_limited_crafting
    let _dim_type = read_varint_slice(pbody, &mut pos).unwrap();
    let _dim_name = read_mc_string_slice(pbody, &mut pos).unwrap();
    pos += 8; // hashed_seed
    let game_mode = pbody[pos];
    (entity_id, game_mode)
}

async fn finish_switch_configuration_and_enter_play(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    label: &str,
) -> (i32, u8, bool) {
    // Wait for StartConfiguration (0x78)
    let mut saw_start_cfg = false;
    for _ in 0..80 {
        let (pid, _) = read_raw_packet(stream, buf).await;
        if pid == 0x78 {
            saw_start_cfg = true;
            break;
        }
    }
    assert!(
        saw_start_cfg,
        "expected StartConfiguration (0x78) during {label}"
    );
    send_raw_packet(stream, 0x10, &[]).await; // ConfigurationAcknowledged

    // Configuration loop until FinishConfiguration (0x03)
    loop {
        let (cid, cbody) = read_raw_packet(stream, buf).await;
        match cid {
            0x0F => send_raw_packet(stream, 0x07, &cbody).await, // KnownPacks
            0x03 => break,                                       // FinishConfiguration
            _ => {}
        }
    }
    send_raw_packet(stream, 0x03, &[]).await; // AcknowledgeFinishConfiguration

    // PlayLogin (0x32) + SynchronizePlayerPosition (0x49) + optional SystemChatMessage (0x73)
    let mut login_info = None;
    let mut saw_limbo_welcome = false;
    for _ in 0..80 {
        let (pid, pbody) = read_raw_packet(stream, buf).await;
        if pid == 0x32 {
            login_info = Some(parse_play_login_entity_and_gamemode(&pbody));
        } else if pid == 0x7C
            && pbody
                .windows(b"LimboFallbackWelcome".len())
                .any(|w| w == b"LimboFallbackWelcome")
        {
            saw_limbo_welcome = true;
        } else if pid == 0x49 {
            let mut pos = 0;
            let tp_id = read_varint_slice(&pbody, &mut pos).unwrap();
            let mut ack = Vec::new();
            write_varint(&mut ack, tp_id);
            ack.extend_from_slice(&pbody[pos..pos + 32]);
            send_raw_packet(stream, 0x00, &ack).await;
            break;
        }
    }
    let (eid, gm) = login_info.unwrap_or_else(|| panic!("expected PlayLogin (0x32) for {label}"));
    // Drain any immediate SystemChatMessage right after SynchronizePlayerPosition
    if !saw_limbo_welcome {
        for _ in 0..8 {
            let Ok((pid, pbody)) =
                tokio::time::timeout(Duration::from_millis(40), read_raw_packet(stream, buf)).await
            else {
                break;
            };
            if pid == 0x7C
                && pbody
                    .windows(b"LimboFallbackWelcome".len())
                    .any(|w| w == b"LimboFallbackWelcome")
            {
                saw_limbo_welcome = true;
                break;
            }
        }
    }
    (eid, gm, saw_limbo_welcome)
}

async fn perform_raw_switch_and_wait_play_login(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    target_server: &str,
) -> (i32, u8) {
    let mut cmd = Vec::new();
    write_mc_string(&mut cmd, &format!("server {target_server}"));
    send_raw_packet(stream, 0x07, &cmd).await; // ChatCommand
    let (eid, gm, _) = finish_switch_configuration_and_enter_play(stream, buf, target_server).await;
    (eid, gm)
}

#[tokio::test]
#[ignore = "requires staged versions/26.3 data package"]
async fn headless_proxy_limbo_server_switch_and_fallback() {
    let proxy_bin = ensure_sibling_bin("rad-proxy");
    let limbo_bin = ensure_sibling_bin("rad-limbo");
    let server_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rad-server"));

    let dir = std::env::temp_dir().join("radbyte_headless_proxy_limbo_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let limbo_dir = dir.join("limbo");
    let server_dir = dir.join("server");
    let proxy_dir = dir.join("proxy");
    std::fs::create_dir_all(&limbo_dir).unwrap();
    std::fs::create_dir_all(&server_dir).unwrap();
    std::fs::create_dir_all(&proxy_dir).unwrap();

    let limbo_port = free_port();
    let server_port = free_port();
    let proxy_port = free_port();
    let secret = "radbyte-ecosystem-test-secret";

    std::fs::write(
        limbo_dir.join("radlimbo.toml"),
        format!(
            "config_version = 1\nbind_addr = \"127.0.0.1:{limbo_port}\"\nforwarding_mode = \"modern\"\nforwarding_secret = \"{secret}\"\nmotd = \"Limbo E2E\"\nmax_players = 20\nview_distance = 2\ngame_mode = 3\nspawn_x = 0.5\nspawn_y = 64.0\nspawn_z = 0.5\nbrand = \"RadLimbo\"\nwelcome_message = \"LimboFallbackWelcome\"\nreconnect_server = \"survival\"\nkeep_alive_interval_secs = 15\n"
        ),
    )
    .unwrap();

    std::fs::write(
        server_dir.join("radserver.toml"),
        format!(
            "config_version = 1\nbind_addr = \"127.0.0.1:{server_port}\"\nforwarding_mode = \"modern\"\nforwarding_secret = \"{secret}\"\nmotd = \"Server E2E\"\nmax_players = 20\nview_distance = 2\ngame_mode = 0\ncompression_threshold = 64\nworld_dir = \"world\"\nspawn_x = 0.5\nspawn_y = 64.0\nspawn_z = 0.5\nautosave_interval_secs = 60\nmax_mobs = 0\n"
        ),
    )
    .unwrap();

    std::fs::write(
        proxy_dir.join("radproxy.toml"),
        format!(
            "config_version = 1\nbind_addr = \"127.0.0.1:{proxy_port}\"\ndefault_server = \"survival\"\ntry_servers = [\"survival\", \"limbo\"]\nforwarding_mode = \"modern\"\nforwarding_secret = \"{secret}\"\nonline_mode = false\nmotd = \"Proxy E2E\"\nmax_players = 50\nproxy_brand = \"RadProxy\"\nbungeecord_messaging = true\nradons_dir = \"radons\"\nradonfigs_dir = \"radonfigs\"\n\n[servers]\nsurvival = \"127.0.0.1:{server_port}\"\nlimbo = \"127.0.0.1:{limbo_port}\"\n"
        ),
    )
    .unwrap();

    let mut limbo_guard = spawn_bin_and_wait_tcp(&limbo_bin, &limbo_dir, limbo_port, "rad-limbo");
    let server_guard = spawn_bin_and_wait_tcp(&server_bin, &server_dir, server_port, "rad-server");
    let _proxy_guard = spawn_bin_and_wait_tcp(&proxy_bin, &proxy_dir, proxy_port, "rad-proxy");

    // 1. Client connects to RadProxy -> lands on RadServer (survival) with backend compression=64
    let proxy_addr = format!("127.0.0.1:{proxy_port}");
    let (mut stream, mut buf, _inv) =
        connect_raw_mc777(&proxy_addr, proxy_port, "eco_player").await;

    // 2. Switch RadServer -> RadLimbo via /server limbo (verify game_mode == 3)
    let (limbo_eid, limbo_gm) =
        perform_raw_switch_and_wait_play_login(&mut stream, &mut buf, "limbo").await;
    assert!(
        limbo_eid > 0 && limbo_gm == 3,
        "expected RadLimbo game_mode=3, got eid={limbo_eid} gm={limbo_gm}"
    );

    // 3. Switch RadLimbo -> RadServer via /server survival (verify game_mode == 0)
    let (srv_eid, srv_gm) =
        perform_raw_switch_and_wait_play_login(&mut stream, &mut buf, "survival").await;
    assert!(
        srv_eid > 0 && srv_gm == 0,
        "expected RadServer game_mode=0, got eid={srv_eid} gm={srv_gm}"
    );

    // 4. ACTIVE-SESSION FALLBACK (graceful backend stop): keep `stream` connected on RadServer,
    //    stop RadServer, and verify the SAME session transitions into RadLimbo (game_mode=3 + welcome)!
    stop_rad_server(server_guard);
    let (fb_eid, fb_gm, saw_welcome) =
        finish_switch_configuration_and_enter_play(&mut stream, &mut buf, "active-stop-fallback")
            .await;
    assert!(
        fb_eid > 0 && fb_gm == 3,
        "active session must fallback to RadLimbo (gm=3)"
    );
    assert!(
        saw_welcome,
        "active session fallback must receive RadLimbo welcome chat"
    );

    // 5. ACTIVE-SESSION FALLBACK (abrupt backend crash): restart RadServer #2, switch active session
    //    back to survival, kill() RadServer #2 abruptly, and verify SAME session returns to RadLimbo!
    let mut server_guard_2 =
        spawn_bin_and_wait_tcp(&server_bin, &server_dir, server_port, "rad-server-2");
    let (srv2_eid, srv2_gm) =
        perform_raw_switch_and_wait_play_login(&mut stream, &mut buf, "survival").await;
    assert!(
        srv2_eid > 0 && srv2_gm == 0,
        "expected RadServer #2 game_mode=0"
    );

    if let Some(mut child) = server_guard_2.0.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let (crash_eid, crash_gm, crash_welcome) =
        finish_switch_configuration_and_enter_play(&mut stream, &mut buf, "active-crash-fallback")
            .await;
    assert!(
        crash_eid > 0 && crash_gm == 3,
        "active session after backend crash must fallback to RadLimbo (gm=3)"
    );
    assert!(crash_welcome, "crash fallback must enter RadLimbo");
    drop(stream);

    // 6. All backends unavailable: kill RadLimbo too and verify new client gets clean disconnect (not hang)
    if let Some(mut child) = limbo_guard.0.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let mut dc_stream = TcpStream::connect(&proxy_addr).await.unwrap();
    let mut hs = Vec::new();
    write_varint(&mut hs, 777);
    write_mc_string(&mut hs, "localhost");
    hs.extend_from_slice(&proxy_port.to_be_bytes());
    write_varint(&mut hs, 2);
    send_raw_packet(&mut dc_stream, 0x00, &hs).await;
    let mut ls = Vec::new();
    write_mc_string(&mut ls, "no_backend");
    ls.extend_from_slice(&offline_uuid_bytes("no_backend"));
    send_raw_packet(&mut dc_stream, 0x00, &ls).await;

    let mut dc_pkt_buf = Vec::new();
    let mut tmp = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(8), dc_stream.read(&mut tmp))
        .await
        .expect("proxy must not hang when all fallback backends are unavailable")
        .expect("socket read error waiting for Login Disconnect");
    assert!(
        n > 0,
        "expected Login Disconnect packet bytes, got immediate EOF"
    );
    dc_pkt_buf.extend_from_slice(&tmp[..n]);
    let mut fpos = 0usize;
    let pkt_len = read_varint_slice(&dc_pkt_buf, &mut fpos)
        .expect("Login Disconnect must start with valid VarInt packet length")
        as usize;
    while dc_pkt_buf.len() < fpos + pkt_len {
        let more = tokio::time::timeout(Duration::from_secs(2), dc_stream.read(&mut tmp))
            .await
            .expect("timed out reading rest of Login Disconnect packet")
            .expect("socket read error");
        assert!(more > 0, "unexpected EOF mid-Login Disconnect packet");
        dc_pkt_buf.extend_from_slice(&tmp[..more]);
    }
    let frame = &dc_pkt_buf[fpos..fpos + pkt_len];
    let mut bpos = 0usize;
    let dc_pid = read_varint_slice(frame, &mut bpos).expect("valid packet id");
    let dc_body = &frame[bpos..];
    assert_eq!(
        dc_pid, 0x00,
        "expected Login Disconnect (0x00) when all backends are unavailable"
    );
    let mut pos = 0usize;
    let reason = read_mc_string_slice(dc_body, &mut pos)
        .expect("Login Disconnect must contain valid UTF-8 reason string");
    assert_eq!(
        pos,
        dc_body.len(),
        "Login Disconnect packet must have no trailing bytes"
    );
    assert!(
        reason.contains("Backend server is currently unavailable."),
        "unexpected Login Disconnect reason: {reason}"
    );
    let mut eof_buf = [0u8; 64];
    let eof_res = if dc_pkt_buf.len() > fpos + pkt_len {
        panic!(
            "unexpected trailing bytes after Login Disconnect: {}",
            dc_pkt_buf.len() - (fpos + pkt_len)
        );
    } else {
        tokio::time::timeout(Duration::from_secs(2), dc_stream.read(&mut eof_buf))
            .await
            .expect("proxy must close TCP connection after Login Disconnect")
            .expect("read after Login Disconnect must not error")
    };
    assert_eq!(
        eof_res, 0,
        "expected EOF (0 bytes) after Login Disconnect, got {eof_res} bytes"
    );
    drop(dc_stream);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "requires staged versions/26.3 data package"]
async fn headless_production_persistence_shutdown_crash_backup_restore() {
    let base_dir = std::env::temp_dir().join(format!(
        "rad_prod_persist_e2e_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base_dir);
    let server_dir = base_dir.join("server");
    let world_dir = server_dir.join("world");
    let backup_dir = base_dir.join("backups");
    let restored_server_dir = base_dir.join("restored_server");
    let restored_world_dir = restored_server_dir.join("world");
    std::fs::create_dir_all(world_dir.join("playerdata")).unwrap();
    std::fs::create_dir_all(&restored_server_dir).unwrap();

    let server_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rad-server"));
    let server_port = free_port();

    let uuid_bytes = offline_uuid_bytes("persist_user");
    let uuid_str = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid_bytes[0],
        uuid_bytes[1],
        uuid_bytes[2],
        uuid_bytes[3],
        uuid_bytes[4],
        uuid_bytes[5],
        uuid_bytes[6],
        uuid_bytes[7],
        uuid_bytes[8],
        uuid_bytes[9],
        uuid_bytes[10],
        uuid_bytes[11],
        uuid_bytes[12],
        uuid_bytes[13],
        uuid_bytes[14],
        uuid_bytes[15]
    );
    stage_version_package(&server_dir);
    let pkg = rad_core::version_package::LoadedVersionPackage::load_from_versions_dir(
        &server_dir.join("versions"),
        rad_core::MINECRAFT_VERSION,
    )
    .expect("staged 26.3 version package");
    let iron_sword_id = pkg
        .item_id_by_name("minecraft:iron_sword")
        .expect("minecraft:iron_sword in version package");
    let chest_id = pkg
        .item_id_by_name("minecraft:chest")
        .expect("minecraft:chest in version package");

    let mut inv_json = Vec::new();
    for idx in 0..46 {
        if idx == 36 {
            inv_json.push(serde_json::json!({
                "count": 1,
                "item_id": iron_sword_id,
                "components": {
                    "custom_name": "SaveBlade",
                    "damage": 5,
                    "max_damage": 250,
                    "repair_cost": 2,
                    "unbreakable": true,
                    "removed": [3]
                }
            }));
        } else if idx == 37 {
            inv_json.push(serde_json::json!({ "count": 4, "item_id": chest_id }));
        } else {
            inv_json.push(serde_json::json!({ "count": 0, "item_id": 0 }));
        }
    }
    let player_json = serde_json::json!({
        "dimension": "Overworld",
        "x": 0.5,
        "y": 64.0,
        "z": 0.5,
        "yaw": 0.0,
        "pitch": 0.0,
        "hp": 20.0,
        "food": 20,
        "saturation": 5.0,
        "game_mode": 0,
        "selected_hotbar": 0,
        "inventory": inv_json
    });
    std::fs::write(
        world_dir
            .join("playerdata")
            .join(format!("{uuid_str}.json")),
        serde_json::to_vec_pretty(&player_json).unwrap(),
    )
    .unwrap();
    let world_profile_json = serde_json::json!({
        "profile_id": "mc_26_3",
        "game_version": "26.3",
        "protocol_version": 777,
        "world_data_version": 4790,
        "storage_schema_version": 1
    });
    std::fs::write(
        world_dir.join("radbyte_world_profile.json"),
        serde_json::to_vec_pretty(&world_profile_json).unwrap(),
    )
    .unwrap();

    let cfg_text = format!(
        "config_version = 1\nbind_addr = \"127.0.0.1:{server_port}\"\nforwarding_mode = \"none\"\nmotd = \"Persist E2E\"\nmax_players = 20\nview_distance = 2\ngame_mode = 0\ncompression_threshold = -1\nworld_dir = \"world\"\nspawn_x = 0.5\nspawn_y = 64.0\nspawn_z = 0.5\nspawn_protection = 0\nautosave_interval_secs = 60\nmax_mobs = 0\n"
    );
    std::fs::write(server_dir.join("radserver.toml"), &cfg_text).unwrap();

    async fn connect_direct(port: u16, username: &str) -> (TcpStream, Vec<u8>, (f64, f64, f64)) {
        let addr = format!("127.0.0.1:{port}");
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        let mut buf = Vec::new();
        let mut hs = Vec::new();
        write_varint(&mut hs, 777);
        write_mc_string(&mut hs, "localhost");
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 2);
        send_raw_packet(&mut stream, 0x00, &hs).await;
        let mut ls = Vec::new();
        write_mc_string(&mut ls, username);
        ls.extend_from_slice(&offline_uuid_bytes(username));
        send_raw_packet(&mut stream, 0x00, &ls).await;

        let (pid, _) = read_raw_packet(&mut stream, &mut buf).await;
        assert_eq!(pid, 0x02, "expected LoginSuccess");
        send_raw_packet(&mut stream, 0x03, &[]).await; // LoginAcknowledged

        loop {
            let (cid, cbody) = read_raw_packet(&mut stream, &mut buf).await;
            match cid {
                0x0F => send_raw_packet(&mut stream, 0x07, &cbody).await,
                0x03 => break,
                _ => {}
            }
        }
        send_raw_packet(&mut stream, 0x03, &[]).await; // AcknowledgeFinishConfiguration

        let mut spawn_pos = (0.0, 0.0, 0.0);
        for _ in 0..80 {
            let (pid, pbody) = read_raw_packet(&mut stream, &mut buf).await;
            if pid == 0x49 {
                let mut pos = 0usize;
                let tp_id = read_varint_slice(&pbody, &mut pos).unwrap();
                let x = f64::from_be_bytes(pbody[pos..pos + 8].try_into().unwrap());
                let y = f64::from_be_bytes(pbody[pos + 8..pos + 16].try_into().unwrap());
                let z = f64::from_be_bytes(pbody[pos + 16..pos + 24].try_into().unwrap());
                spawn_pos = (x, y, z);
                let mut ack = Vec::new();
                write_varint(&mut ack, tp_id);
                ack.extend_from_slice(&x.to_be_bytes());
                ack.extend_from_slice(&y.to_be_bytes());
                ack.extend_from_slice(&z.to_be_bytes());
                ack.extend_from_slice(&0.0f32.to_be_bytes());
                ack.extend_from_slice(&0.0f32.to_be_bytes());
                send_raw_packet(&mut stream, 0x00, &ack).await;
                break;
            }
        }
        (stream, buf, spawn_pos)
    }

    async fn send_use_item_on_top(stream: &mut TcpStream, x: i32, y: i32, z: i32, seq: i32) {
        let mut b = Vec::new();
        write_varint(&mut b, 0);
        let packed = (((x as i64) & 0x3FF_FFFF) << 38)
            | (((z as i64) & 0x3FF_FFFF) << 12)
            | ((y as i64) & 0xFFF);
        b.extend_from_slice(&packed.to_be_bytes());
        write_varint(&mut b, 1);
        b.extend_from_slice(&0.5f32.to_be_bytes());
        b.extend_from_slice(&1.0f32.to_be_bytes());
        b.extend_from_slice(&0.5f32.to_be_bytes());
        b.push(0);
        b.push(0);
        write_varint(&mut b, seq);
        send_raw_packet(stream, 0x42, &b).await;
    }

    // 1. Start RadServer #1, place chest at (2, 64, 2), deposit SaveBlade, move player, graceful shutdown.
    let guard1 = spawn_bin_and_wait_tcp(&server_bin, &server_dir, server_port, "persist-srv-1");
    {
        let (mut stream, mut buf, spawn_pos) = connect_direct(server_port, "persist_user").await;
        assert!((spawn_pos.1 - 64.0).abs() < 0.1);

        send_raw_packet(&mut stream, 0x36, &1i16.to_be_bytes()).await;
        send_use_item_on_top(&mut stream, 2, 63, 2, 1).await;
        tokio::time::sleep(Duration::from_millis(60)).await;

        send_use_item_on_top(&mut stream, 2, 64, 2, 2).await;
        let (chest_sid, _slots) = read_window_content(&mut stream, &mut buf, 1).await;
        assert!(chest_sid > 0);

        send_container_click(&mut stream, 1, chest_sid, 54, 0, 1).await;
        let (_sid2, slots2) = read_window_content(&mut stream, &mut buf, 1).await;
        assert_eq!(
            slots2[0],
            (1, iron_sword_id),
            "SaveBlade must be in chest slot 0"
        );

        send_raw_packet(&mut stream, 0x13, &[1u8]).await;

        tokio::time::sleep(Duration::from_millis(200)).await;
        for &(mx, mz) in &[(1.0f64, 0.0f64), (1.5f64, -0.5f64)] {
            let mut mv = Vec::new();
            mv.extend_from_slice(&mx.to_be_bytes());
            mv.extend_from_slice(&64.0f64.to_be_bytes());
            mv.extend_from_slice(&mz.to_be_bytes());
            mv.push(0x01);
            send_raw_packet(&mut stream, 0x1E, &mv).await;
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        stop_rad_server(guard1);
    }

    // 2. Restart RadServer #2 -> verify position, chest + SaveBlade components, then simulate crash mid-save!
    let mut guard2 = spawn_bin_and_wait_tcp(&server_bin, &server_dir, server_port, "persist-srv-2");
    {
        let (mut stream, mut buf, spawn_pos) = connect_direct(server_port, "persist_user").await;
        assert!(
            (spawn_pos.0 - 1.5).abs() < 0.01 && (spawn_pos.2 - (-0.5)).abs() < 0.01,
            "graceful shutdown must persist player position, got {spawn_pos:?}"
        );
        send_use_item_on_top(&mut stream, 2, 64, 2, 1).await;
        let mut saw_save_blade = false;
        for _ in 0..30 {
            let (pid, pbody) = read_raw_packet(&mut stream, &mut buf).await;
            if pid == 0x12 && pbody.windows(b"SaveBlade".len()).any(|w| w == b"SaveBlade") {
                saw_save_blade = true;
                break;
            }
        }
        assert!(
            saw_save_blade,
            "chest must retain SaveBlade ItemComponentPatch after graceful shutdown"
        );
        drop(stream);
    }
    std::fs::write(
        world_dir.join("containers.json.99999.0.tmp"),
        b"{corrupt_partial",
    )
    .unwrap();
    std::fs::write(world_dir.join("level.dat.99999.1.tmp"), b"bad_gzip").unwrap();
    std::fs::write(
        world_dir
            .join("playerdata")
            .join(format!("{uuid_str}.json.99999.2.tmp")),
        b"{truncated",
    )
    .unwrap();
    let _ = std::fs::create_dir_all(world_dir.join("region"));
    std::fs::write(
        world_dir.join("region").join("r.0.0.mca.99999.3.tmp"),
        b"partial_mca",
    )
    .unwrap();
    if let Some(mut child) = guard2.0.take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    // 3. Exercise the backup copy and restore in Rust, without a shell dependency.
    fn copy_tree(source: &Path, destination: &Path) {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let src = entry.path();
            let dst = destination.join(entry.file_name());
            if src.is_dir() {
                copy_tree(&src, &dst);
            } else {
                std::fs::copy(src, dst).unwrap();
            }
        }
    }
    fn assert_tree_equal(expected: &Path, actual: &Path) {
        for entry in std::fs::read_dir(expected).unwrap() {
            let entry = entry.unwrap();
            let expected_path = entry.path();
            let actual_path = actual.join(entry.file_name());
            if expected_path.is_dir() {
                assert!(
                    actual_path.is_dir(),
                    "missing directory {}",
                    actual_path.display()
                );
                assert_tree_equal(&expected_path, &actual_path);
            } else {
                assert_eq!(
                    std::fs::read(&expected_path).unwrap(),
                    std::fs::read(&actual_path).unwrap(),
                    "restored file differs: {}",
                    actual_path.display()
                );
            }
        }
    }
    let backup_world = backup_dir.join("world");
    copy_tree(&world_dir, &backup_world);
    copy_tree(&backup_world, &restored_world_dir);
    assert_tree_equal(&world_dir, &restored_world_dir);

    // 4. Boot RadServer #3 on restored_world_dir and verify complete integrity!
    let restored_port = free_port();
    let restored_cfg = format!(
        "config_version = 1\nbind_addr = \"127.0.0.1:{restored_port}\"\nforwarding_mode = \"none\"\nmotd = \"Restored E2E\"\nmax_players = 20\nview_distance = 2\ngame_mode = 0\ncompression_threshold = -1\nworld_dir = \"world\"\nspawn_x = 0.5\nspawn_y = 64.0\nspawn_z = 0.5\nspawn_protection = 0\nautosave_interval_secs = 60\nmax_mobs = 0\n"
    );
    std::fs::write(restored_server_dir.join("radserver.toml"), restored_cfg).unwrap();
    let guard3 = spawn_bin_and_wait_tcp(
        &server_bin,
        &restored_server_dir,
        restored_port,
        "persist-srv-restored",
    );
    {
        let (mut stream, mut buf, spawn_pos) = connect_direct(restored_port, "persist_user").await;
        assert!(
            (spawn_pos.0 - 1.5).abs() < 0.01 && (spawn_pos.2 - (-0.5)).abs() < 0.01,
            "restored world must preserve playerdata position after crash + backup/restore, got {spawn_pos:?}"
        );
        send_use_item_on_top(&mut stream, 2, 64, 2, 1).await;
        let mut saw_save_blade = false;
        for _ in 0..30 {
            let (pid, pbody) = read_raw_packet(&mut stream, &mut buf).await;
            if pid == 0x12 && pbody.windows(b"SaveBlade".len()).any(|w| w == b"SaveBlade") {
                saw_save_blade = true;
                break;
            }
        }
        assert!(
            saw_save_blade,
            "restored world must preserve chest block, container contents, and SaveBlade ItemComponentPatch"
        );
        drop(stream);
    }
    stop_rad_server(guard3);
    let _ = std::fs::remove_dir_all(&base_dir);
}

#[tokio::test]
#[ignore = "requires staged versions/26.3 data package"]
async fn headless_whitelist_ops_console_and_restart_e2e() {
    use rad_core::crypto::offline_player_uuid;
    use rad_core::packet::Packet;
    use rad_core::packet::play::{CommandNodeKind, DeclareCommands};
    use std::io::Write;

    let server_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rad-server"));
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base_dir =
        std::env::temp_dir().join(format!("rad_wl_ops_e2e_{}_{}", std::process::id(), uniq));
    let _ = std::fs::remove_dir_all(&base_dir);
    std::fs::create_dir_all(&base_dir).unwrap();

    let port = free_port();
    let cfg = format!(
        "config_version = 1\nbind_addr = \"127.0.0.1:{port}\"\nforwarding_mode = \"none\"\nmotd = \"Whitelist Ops E2E\"\nmax_players = 5\nview_distance = 2\ngame_mode = 0\nlevel_name = \"custom_world\"\ndifficulty = \"normal\"\npvp = false\nwhite_list = true\nenforce_whitelist = true\nop_permission_level = 3\nspawn_protection = 4\n"
    );
    std::fs::write(base_dir.join("radserver.toml"), cfg).unwrap();

    async fn try_connect_and_read_commands(
        port: u16,
        username: &str,
    ) -> Result<(TcpStream, Vec<u8>, Vec<String>), String> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let uuid = offline_player_uuid(username);
        let mut hs = Vec::new();
        write_varint(&mut hs, 777);
        write_mc_string(&mut hs, "127.0.0.1");
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 2);
        send_raw_packet(&mut stream, 0x00, &hs).await;

        let mut ls = Vec::new();
        write_mc_string(&mut ls, username);
        ls.extend_from_slice(uuid.as_bytes());
        send_raw_packet(&mut stream, 0x00, &ls).await;

        let mut buf = Vec::new();
        let (pid, pbody) = read_raw_packet(&mut stream, &mut buf).await;
        if pid == 0x00 {
            let mut pos = 0usize;
            let reason = read_mc_string_slice(&pbody, &mut pos).unwrap_or_default();
            return Err(reason);
        }
        assert_eq!(pid, 0x02, "expected LoginSuccess");
        send_raw_packet(&mut stream, 0x03, &[]).await;

        loop {
            let (cid, cbody) = read_raw_packet(&mut stream, &mut buf).await;
            match cid {
                0x0F => send_raw_packet(&mut stream, 0x07, &cbody).await,
                0x03 => break,
                _ => {}
            }
        }
        send_raw_packet(&mut stream, 0x03, &[]).await;

        let mut cmds = Vec::new();
        for _ in 0..80 {
            let (pid, pbody) = read_raw_packet(&mut stream, &mut buf).await;
            if pid == DeclareCommands::ID {
                let mut slice = pbody.as_slice();
                if let Ok(dc) = DeclareCommands::decode_body(&mut slice) {
                    cmds = dc
                        .nodes
                        .iter()
                        .filter_map(|n| match &n.kind {
                            CommandNodeKind::Literal { name } => Some(name.clone()),
                            _ => None,
                        })
                        .collect();
                }
            } else if pid == 0x49 {
                let mut pos = 0usize;
                let tp_id = read_varint_slice(&pbody, &mut pos).unwrap();
                let x = f64::from_be_bytes(pbody[pos..pos + 8].try_into().unwrap());
                let y = f64::from_be_bytes(pbody[pos + 8..pos + 16].try_into().unwrap());
                let z = f64::from_be_bytes(pbody[pos + 16..pos + 24].try_into().unwrap());
                let mut ack = Vec::new();
                write_varint(&mut ack, tp_id);
                ack.extend_from_slice(&x.to_be_bytes());
                ack.extend_from_slice(&y.to_be_bytes());
                ack.extend_from_slice(&z.to_be_bytes());
                ack.extend_from_slice(&0.0f32.to_be_bytes());
                ack.extend_from_slice(&0.0f32.to_be_bytes());
                send_raw_packet(&mut stream, 0x00, &ack).await;
                break;
            }
        }
        Ok((stream, buf, cmds))
    }

    async fn send_chat_cmd(stream: &mut TcpStream, command: &str) {
        let mut body = Vec::new();
        write_mc_string(&mut body, command);
        send_raw_packet(stream, 0x07, &body).await;
    }

    // 1. Start RadServer #1, verify unlisted player is rejected, grant whitelist + op from console.
    let mut guard1 = spawn_bin_and_wait_tcp(&server_bin, &base_dir, port, "wl-ops-1");
    assert!(
        base_dir.join("custom_world").exists(),
        "level_name = \"custom_world\" must create custom_world directory"
    );

    let err = try_connect_and_read_commands(port, "Stranger")
        .await
        .unwrap_err();
    assert!(
        err.contains("not white-listed"),
        "unlisted player must be rejected with whitelist message, got {err}"
    );

    // Issue console commands via stdin!
    {
        let stdin = guard1.0.as_mut().unwrap().stdin.as_mut().unwrap();
        writeln!(stdin, "whitelist add Alice").unwrap();
        writeln!(stdin, "whitelist add Bob").unwrap();
        writeln!(stdin, "op Alice").unwrap();
        writeln!(stdin, "ops").unwrap();
        writeln!(stdin, "save-all").unwrap();
        stdin.flush().unwrap();
    }
    tokio::time::sleep(Duration::from_millis(120)).await;

    // Alice (op level 3 from op_permission_level = 3) logs in
    let (mut alice, mut alice_buf, alice_cmds) =
        try_connect_and_read_commands(port, "Alice").await.unwrap();
    assert!(alice_cmds.contains(&"op".to_owned()));
    assert!(alice_cmds.contains(&"whitelist".to_owned()));
    assert!(!alice_cmds.contains(&"save".to_owned()));

    // Bob (whitelisted non-op) logs in
    let (mut bob, mut bob_buf, bob_cmds) =
        try_connect_and_read_commands(port, "Bob").await.unwrap();
    assert!(!bob_cmds.contains(&"op".to_owned()));
    assert!(!bob_cmds.contains(&"whitelist".to_owned()));
    assert!(!bob_cmds.contains(&"gamemode".to_owned()));

    // Bob attempts /op Bob -> denied
    send_chat_cmd(&mut bob, "op Bob").await;
    let mut bob_denied = false;
    for _ in 0..20 {
        if let Ok((pid, body)) = tokio::time::timeout(
            Duration::from_millis(250),
            read_raw_packet(&mut bob, &mut bob_buf),
        )
        .await
            && pid == 0x7C
            && body
                .windows(b"do not have permission".len())
                .any(|w| w == b"do not have permission")
        {
            bob_denied = true;
            break;
        }
    }
    assert!(bob_denied, "non-op Bob must be denied /op");

    // Alice runs /op Bob 2 -> Bob immediately receives updated DeclareCommands with gamemode (level 2), still no op (level 3)
    send_chat_cmd(&mut alice, "op Bob 2").await;
    let mut bob_updated_cmds = Vec::new();
    for _ in 0..30 {
        if let Ok((pid, body)) = tokio::time::timeout(
            Duration::from_millis(300),
            read_raw_packet(&mut bob, &mut bob_buf),
        )
        .await
        {
            if pid == DeclareCommands::ID {
                let mut slice = body.as_slice();
                if let Ok(dc) = DeclareCommands::decode_body(&mut slice) {
                    bob_updated_cmds = dc
                        .nodes
                        .iter()
                        .filter_map(|n| match &n.kind {
                            CommandNodeKind::Literal { name } => Some(name.clone()),
                            _ => None,
                        })
                        .collect();
                    break;
                }
            }
        } else {
            break;
        }
    }
    assert!(bob_updated_cmds.contains(&"gamemode".to_owned()));
    assert!(!bob_updated_cmds.contains(&"op".to_owned()));
    let _ = (&mut alice, &mut alice_buf);
    drop(alice);
    drop(bob);

    stop_rad_server(guard1);
    assert!(
        !base_dir
            .join("custom_world")
            .join("access_control.json")
            .exists(),
        "redundant access_control.json must not be created"
    );
    for name in ["ops.json", "whitelist.json", "banned-players.json"] {
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(base_dir.join(name)).expect("list must be next to server configuration"),
        )
        .unwrap();
        assert!(json.is_array(), "{name} must contain a JSON array");
    }

    // 2. Restart RadServer #2 on the same base_dir and verify persistence of whitelist & op levels!
    let guard2 = spawn_bin_and_wait_tcp(&server_bin, &base_dir, port, "wl-ops-2");
    let (_alice2, _abuf2, alice2_cmds) =
        try_connect_and_read_commands(port, "Alice").await.unwrap();
    assert!(alice2_cmds.contains(&"op".to_owned()));
    let (_bob2, _bbuf2, bob2_cmds) = try_connect_and_read_commands(port, "Bob").await.unwrap();
    assert!(bob2_cmds.contains(&"gamemode".to_owned()));
    assert!(!bob2_cmds.contains(&"op".to_owned()));
    assert!(
        try_connect_and_read_commands(port, "Stranger")
            .await
            .is_err()
    );

    stop_rad_server(guard2);
    let _ = std::fs::remove_dir_all(&base_dir);
}

#[tokio::test]
#[ignore = "requires staged versions/26.3 data package"]
async fn headless_version_package_validation_cross_dir_and_selection() {
    use rad_core::PROTOCOL_VERSION;
    use rad_core::crypto::offline_player_uuid;
    use rad_core::packet::Packet;
    use rad_core::packet::configuration::{
        AcknowledgeFinishConfiguration, ClientboundKnownPacks, FinishConfiguration,
        ServerboundKnownPacks,
    };
    use rad_core::packet::handshake::Handshake;
    use rad_core::packet::login::{LoginAcknowledged, LoginStart, LoginSuccess};
    use rad_core::packet::play::PlayLogin;
    use rad_core::packet::status::{StatusRequest, StatusResponse};
    use rad_core::state::ConnectionState;
    use rad_core::types::VarInt;
    use tokio::io::AsyncWriteExt;

    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rad_verpkg_e2e_{}_{}", std::process::id(), uniq));
    let _ = std::fs::remove_dir_all(&root);
    let install_dir = root.join("install");
    let other_cwd = root.join("other_cwd");
    std::fs::create_dir_all(&install_dir).unwrap();
    std::fs::create_dir_all(&other_cwd).unwrap();

    let server_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rad-server"));
    let port = free_port();
    let props = format!(
        "server-ip=127.0.0.1\nserver-port={port}\nmotd=VerPkg E2E\nview-distance=2\nsimulation-distance=2\n"
    );
    std::fs::write(install_dir.join("rad.properties"), &props).unwrap();
    std::fs::write(install_dir.join("ops.json"), "[]").unwrap();
    std::fs::write(install_dir.join("whitelist.json"), "[]").unwrap();

    // 1. Missing versions/26.3 must fail BEFORE opening TCP port
    let missing_out = Command::new(&server_bin)
        .arg("--nogui")
        .current_dir(&other_cwd)
        .arg("--config")
        .arg(install_dir.join("rad.properties"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        !missing_out.status.success(),
        "rad-server must refuse to start when versions/26.3 is missing"
    );

    // 2. Corrupt versions/26.3/data/items.json must fail BEFORE opening TCP port
    stage_version_package(&install_dir);
    let items_path = install_dir.join("versions/26.3/data/items.json");
    let orig_items = std::fs::read(&items_path).unwrap();
    std::fs::write(&items_path, b"{\"entries\":[]}").unwrap();
    let corrupt_out = Command::new(&server_bin)
        .arg("--nogui")
        .current_dir(&other_cwd)
        .arg("--config")
        .arg(install_dir.join("rad.properties"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        !corrupt_out.status.success(),
        "rad-server must refuse to start when version package hash mismatches"
    );
    std::fs::write(&items_path, orig_items).unwrap();

    // 2b. Corrupt native version library must also fail BEFORE opening TCP port
    let lib_path = install_dir
        .join("versions/26.3")
        .join(version_library_filename());
    let orig_lib = std::fs::read(&lib_path).unwrap();
    std::fs::write(&lib_path, b"not-a-valid-dll").unwrap();
    let corrupt_lib_out = Command::new(&server_bin)
        .arg("--nogui")
        .current_dir(&other_cwd)
        .arg("--config")
        .arg(install_dir.join("rad.properties"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        !corrupt_lib_out.status.success(),
        "rad-server must refuse to start when native version library hash mismatches"
    );
    std::fs::write(&lib_path, orig_lib).unwrap();

    // 2c. Selecting an uninstalled version in version.toml must fail BEFORE opening TCP port
    let version_toml_path = install_dir.join("versions/version.toml");
    std::fs::write(&version_toml_path, "version = \"26.4\"\n").unwrap();
    let missing_selected_out = Command::new(&server_bin)
        .arg("--nogui")
        .current_dir(&other_cwd)
        .arg("--config")
        .arg(install_dir.join("rad.properties"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        !missing_selected_out.status.success(),
        "rad-server must refuse to start when selected version package is missing"
    );

    // 3. Preserve the user's version selection while validating the installed package.
    let preserved_toml = "# user-preserved\nversion = \"26.3\"\n";
    std::fs::write(&version_toml_path, preserved_toml).unwrap();
    let selected_package = rad_core::version_package::LoadedVersionPackage::load_from_versions_dir(
        &install_dir.join("versions"),
        rad_core::MINECRAFT_VERSION,
    )
    .expect("preserved version selection still loads its package");
    selected_package
        .verify_and_resolve_library_path()
        .expect("preserved package library remains valid");
    assert_eq!(
        std::fs::read_to_string(install_dir.join("rad.properties")).unwrap(),
        props
    );
    assert_eq!(
        std::fs::read_to_string(&version_toml_path).unwrap(),
        preserved_toml
    );

    // 4. Launch from a DIFFERENT working directory (--config <install_dir>/rad.properties)
    // and verify Status + Login + Configuration + Play + clean stop!
    let mut child = Command::new(&server_bin)
        .arg("--nogui")
        .current_dir(&other_cwd)
        .arg("--config")
        .arg(install_dir.join("rad.properties"))
        .env("RUST_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let addr = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(&addr).is_ok() {
            break;
        }
        if let Some(st) = child.try_wait().unwrap() {
            panic!("rad-server exited early with {st}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let guard = ServerChildGuard(Some(child));

    // Status ping
    let mut stream = TcpStream::connect(&addr).await.unwrap();
    let mut buf = Vec::new();
    let mut out = bytes::BytesMut::new();
    rad_core::codec::encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: port,
            next_state: ConnectionState::Status,
        },
        &mut out,
    )
    .unwrap();
    rad_core::codec::encode_packet(&StatusRequest, &mut out).unwrap();
    stream.write_all(&out).await.unwrap();
    let (id, body) = read_raw_packet(&mut stream, &mut buf).await;
    assert_eq!(id, StatusResponse::ID);
    let status = StatusResponse::decode_body(&mut body.as_slice()).unwrap();
    let status_json: serde_json::Value = serde_json::from_str(&status.json).unwrap();
    assert_eq!(status_json["version"]["name"], "26.3");
    assert_eq!(status_json["version"]["protocol"], PROTOCOL_VERSION);
    drop(stream);

    // Login -> Configuration -> Play
    let mut play_stream = TcpStream::connect(&addr).await.unwrap();
    let mut pbuf = Vec::new();
    let mut pout = bytes::BytesMut::new();
    rad_core::codec::encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: port,
            next_state: ConnectionState::Login,
        },
        &mut pout,
    )
    .unwrap();
    rad_core::codec::encode_packet(
        &LoginStart {
            username: "VerPkgPlayer".into(),
            player_uuid: offline_player_uuid("VerPkgPlayer"),
        },
        &mut pout,
    )
    .unwrap();
    play_stream.write_all(&pout).await.unwrap();
    let (lid, _) = read_raw_packet(&mut play_stream, &mut pbuf).await;
    assert_eq!(lid, LoginSuccess::ID);

    pout.clear();
    rad_core::codec::encode_packet(&LoginAcknowledged, &mut pout).unwrap();
    play_stream.write_all(&pout).await.unwrap();
    loop {
        let (pid, _) = read_raw_packet(&mut play_stream, &mut pbuf).await;
        if pid == ClientboundKnownPacks::ID {
            pout.clear();
            rad_core::codec::encode_packet(&ServerboundKnownPacks { packs: vec![] }, &mut pout)
                .unwrap();
            play_stream.write_all(&pout).await.unwrap();
        } else if pid == FinishConfiguration::ID {
            pout.clear();
            rad_core::codec::encode_packet(&AcknowledgeFinishConfiguration, &mut pout).unwrap();
            play_stream.write_all(&pout).await.unwrap();
            break;
        }
    }
    let (play_id, _) = read_raw_packet(&mut play_stream, &mut pbuf).await;
    assert_eq!(play_id, PlayLogin::ID);
    drop(play_stream);

    stop_rad_server(guard);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn headless_refuses_missing_version_package_without_opening_port() {
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rad_missing_pkg_{}_{}", std::process::id(), uniq));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let server_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rad-server"));
    let port = free_port();
    let props = format!(
        "server-ip=127.0.0.1\nserver-port={port}\nmotd=Missing Pkg\nview-distance=2\nsimulation-distance=2\n"
    );
    std::fs::write(root.join("rad.properties"), &props).unwrap();
    std::fs::write(root.join("ops.json"), "[]").unwrap();
    std::fs::write(root.join("whitelist.json"), "[]").unwrap();

    let out = Command::new(&server_bin)
        .arg("--nogui")
        .current_dir(&root)
        .arg("--config")
        .arg(root.join("rad.properties"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "rad-server must refuse to start when versions/26.3 is missing"
    );
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "game port must not be opened when version package is missing"
    );

    let _ = std::fs::remove_dir_all(&root);
}
