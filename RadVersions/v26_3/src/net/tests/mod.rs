pub(super) use super::*;
pub(super) use rad_core::PROTOCOL_VERSION;
pub(super) use rad_core::crypto::offline_player_uuid;
pub(super) use rad_core::forwarding::{ForwardingData, encode_velocity_forwarding};
pub(super) use tokio::net::TcpListener;

mod blocks_gameplay;
mod commands_access;
mod connection_login;
mod inventory_containers;
mod movement_chunks;
mod persistence;

fn test_server_config(mode: ServerForwardingMode) -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        forwarding_mode: mode,
        forwarding_secret: b"server-test-secret".to_vec(),
        motd: "RadServer Test".into(),
        max_players: 50,
        view_distance: 2,
        simulation_distance: 2,
        tps: 20,
        game_mode: 2,
        spawn_x: 0.5,
        spawn_y: 64.0,
        spawn_z: 0.5,
        brand: "RadServer".into(),
        welcome_message: Some("Welcome!".into()),
        lobby_server: Some("limbo".into()),
        max_packet_size: DEFAULT_MAX_PACKET_SIZE,
        login_timeout_secs: 10,
        forwarding_timeout_secs: 5,
        configuration_timeout_secs: 10,
        play_read_timeout_secs: 35,
        admins: std::collections::HashSet::new(),
        max_mobs: 256,
        autosave_interval_secs: 300,
        world_type: "flat".into(),
        world_seed: 0,
        level_name: "world".into(),
        difficulty: 1,
        pvp: true,
        force_gamemode: false,
        white_list: None,
        enforce_whitelist: false,
        op_permission_level: 4,
        spawn_protection: 0,
        enable_status: true,
        hide_online_players: false,
        hardcore: false,
        player_idle_timeout: 0,
        max_world_size: 29_999_984,
    }
}

async fn connect_test_player(addr: SocketAddr, name: &str) -> (TcpStream, BytesMut) {
    let (client, buf, _) = connect_test_player_with_login(addr, name).await;
    (client, buf)
}

async fn connect_test_player_with_login(
    addr: SocketAddr,
    name: &str,
) -> (TcpStream, BytesMut, PlayLogin) {
    let mut client = TcpStream::connect(addr).await.unwrap();
    let uuid = offline_player_uuid(name);
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Login,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut client,
        &LoginStart {
            username: name.into(),
            player_uuid: uuid,
        },
    )
    .await
    .unwrap();

    let mut buf = BytesMut::with_capacity(65536);
    let req: LoginPluginRequest = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    let payload = encode_velocity_forwarding(
        &ForwardingData {
            address: "127.0.0.1".into(),
            uuid,
            username: name.into(),
            properties: Vec::new(),
        },
        b"server-test-secret",
    )
    .unwrap();
    send_packet(
        &mut client,
        &LoginPluginResponse {
            message_id: req.message_id,
            successful: true,
            data: payload,
        },
    )
    .await
    .unwrap();

    let success: LoginSuccess = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(success.username, name);
    send_packet(&mut client, &LoginAcknowledged).await.unwrap();

    loop {
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == ClientboundKnownPacks::ID {
            let known = frame.decode_as::<ClientboundKnownPacks>().unwrap();
            send_packet(
                &mut client,
                &rad_core::packet::configuration::ServerboundKnownPacks { packs: known.packs },
            )
            .await
            .unwrap();
        } else if frame.id == FinishConfiguration::ID {
            break;
        }
    }
    send_packet(&mut client, &AcknowledgeFinishConfiguration)
        .await
        .unwrap();

    let login: PlayLogin = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();

    // Drain initial chunks until SynchronizePlayerPosition, then confirm teleportation.
    loop {
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == SynchronizePlayerPosition::ID {
            let tp = frame.decode_as::<SynchronizePlayerPosition>().unwrap();
            send_packet(
                &mut client,
                &ConfirmTeleportation {
                    teleport_id: tp.teleport_id,
                    x: tp.x,
                    y: tp.y,
                    z: tp.z,
                    yaw: tp.yaw,
                    pitch: tp.pitch,
                },
            )
            .await
            .unwrap();
            break;
        }
    }
    (client, buf, login)
}

async fn join_and_capture_set_time(
    addr: SocketAddr,
    name: &str,
) -> (TcpStream, BytesMut, rad_core::packet::play::SetTime) {
    use rad_core::packet::configuration::{
        AcknowledgeFinishConfiguration, ClientboundKnownPacks, FinishConfiguration,
    };
    use rad_core::packet::handshake::Handshake;
    use rad_core::packet::login::{
        LoginAcknowledged, LoginPluginRequest, LoginPluginResponse, LoginStart, LoginSuccess,
    };
    use rad_core::packet::play::{ConfirmTeleportation, SetTime, SynchronizePlayerPosition};

    let mut client = TcpStream::connect(addr).await.unwrap();
    let uuid = offline_player_uuid(name);
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Login,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut client,
        &LoginStart {
            username: name.into(),
            player_uuid: uuid,
        },
    )
    .await
    .unwrap();

    let mut buf = BytesMut::with_capacity(65536);
    let req: LoginPluginRequest = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    let payload = encode_velocity_forwarding(
        &ForwardingData {
            address: "127.0.0.1".into(),
            uuid,
            username: name.into(),
            properties: Vec::new(),
        },
        b"server-test-secret",
    )
    .unwrap();
    send_packet(
        &mut client,
        &LoginPluginResponse {
            message_id: req.message_id,
            successful: true,
            data: payload,
        },
    )
    .await
    .unwrap();

    let _success: LoginSuccess = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    send_packet(&mut client, &LoginAcknowledged).await.unwrap();

    let mut captured_time = None;
    loop {
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == ClientboundKnownPacks::ID {
            let known = frame.decode_as::<ClientboundKnownPacks>().unwrap();
            send_packet(
                &mut client,
                &rad_core::packet::configuration::ServerboundKnownPacks { packs: known.packs },
            )
            .await
            .unwrap();
        } else if frame.id == FinishConfiguration::ID {
            send_packet(&mut client, &AcknowledgeFinishConfiguration)
                .await
                .unwrap();
        } else if frame.id == SetTime::ID {
            captured_time = Some(frame.decode_as::<SetTime>().unwrap());
        } else if frame.id == SynchronizePlayerPosition::ID {
            let tp = frame.decode_as::<SynchronizePlayerPosition>().unwrap();
            send_packet(
                &mut client,
                &ConfirmTeleportation {
                    teleport_id: tp.teleport_id,
                    x: tp.x,
                    y: tp.y,
                    z: tp.z,
                    yaw: tp.yaw,
                    pitch: tp.pitch,
                },
            )
            .await
            .unwrap();
            break;
        }
    }
    (
        client,
        buf,
        captured_time.expect("player must receive SetTime before SynchronizePlayerPosition"),
    )
}
