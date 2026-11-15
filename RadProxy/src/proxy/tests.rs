use std::collections::HashMap;
use std::time::Duration;

use tokio::net::TcpListener;

use super::*;

fn test_config(backend_addr: Option<SocketAddr>, forwarding_mode: ForwardingMode) -> ProxyConfig {
    ProxyConfig {
        config_version: 1,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        backend_addr,
        servers: HashMap::new(),
        try_servers: Vec::new(),
        forced_hosts: HashMap::new(),
        haproxy_protocol: false,
        haproxy_trusted_proxies: std::collections::HashSet::new(),
        haproxy_allow_untrusted: false,
        forwarding_mode,
        forwarding_secret: b"test-secret-key".to_vec(),
        online_mode: false,
        motd: TextComponent::text("Test Proxy"),
        motds: Vec::new(),
        favicon: None,
        max_players: 100,
        enforce_max_players: false,
        ping_passthrough: PingPassthroughMode::None,
        sample_players_in_ping: false,
        handshake_timeout: Duration::from_secs(2),
        connect_timeout: Duration::from_secs(2),
        max_packet_size: 64 * 1024,
        max_connections_per_ip: 16,
        rate_limit_per_sec: 50,
        kick_existing_players: true,
        whitelist_enabled: false,
        whitelist: std::collections::HashSet::new(),
        banned_players: std::collections::HashSet::new(),
        banned_ips: std::collections::HashSet::new(),
        admins: std::collections::HashSet::new(),
        restricted_servers: HashMap::new(),
        metrics_bind_addr: None,
        log_player_connections: true,
        log_command_executions: true,
        radons_dir: "radons".into(),
        radonfigs_dir: "radonfigs".into(),
        load_balance_strategy: LoadBalanceStrategy::LeastPlayers,
        server_groups: HashMap::new(),
        bungeecord_messaging: true,
        proxy_brand: Some("RadProxy".into()),
    }
}

#[tokio::test]
async fn e2e_status_and_ping() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(
        test_config(None, ForwardingMode::None),
        RadonManager::new(),
        None,
    ));

    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Status,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(&StatusRequest, &mut out).unwrap();
    encode_packet(&PingRequest { timestamp: 777_123 }, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let status_frame = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap();
    let status: StatusResponse = status_frame.decode_as().unwrap();
    let payload = status.parse_payload().unwrap();
    assert_eq!(payload.version.protocol, PROTOCOL_VERSION);

    let pong_frame = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap();
    let pong: PongResponse = pong_frame.decode_as().unwrap();
    assert_eq!(pong.timestamp, 777_123);
}

#[tokio::test]
async fn e2e_velocity_modern_forwarding() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_addr = backend_listener.local_addr().unwrap();

    // Spawn mock backend that requests velocity:player_info and verifies HMAC.
    let backend_task = tokio::spawn(async move {
        let (mut sock, _) = backend_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let hs = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<Handshake>()
            .unwrap();
        assert_eq!(hs.next_state, ConnectionState::Login);

        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();
        assert_eq!(login.username, "RadTester");

        let mut out = BytesMut::new();
        encode_packet(
            &LoginPluginRequest {
                message_id: VarInt(1),
                channel: VELOCITY_FORWARDING_CHANNEL.into(),
                data: Vec::new(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let resp = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginPluginResponse>()
            .unwrap();
        assert!(resp.successful);
        let fwd = decode_velocity_forwarding(&resp.data, b"test-secret-key").unwrap();
        assert_eq!(fwd.username, "RadTester");
        assert_eq!(fwd.address, "127.0.0.1");

        out.clear();
        encode_packet(
            &LoginSuccess {
                uuid: fwd.uuid,
                username: fwd.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();
    });

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(
        test_config(Some(backend_addr), ForwardingMode::Modern),
        RadonManager::new(),
        None,
    ));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = proxy_listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: proxy_addr.port(),
            next_state: ConnectionState::Login,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(
        &LoginStart {
            username: "RadTester".into(),
            player_uuid: offline_player_uuid("RadTester"),
        },
        &mut out,
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let success = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<LoginSuccess>()
        .unwrap();
    assert_eq!(success.username, "RadTester");

    backend_task.await.unwrap();
}

#[test]
fn parses_proxy_chat_commands() {
    let mut out = BytesMut::new();
    encode_packet(
        &ChatCommand {
            command: "server".into(),
        },
        &mut out,
    )
    .unwrap();
    assert!(matches!(
        inspect_proxy_command(&out, None),
        Some(ProxyCommand::ServerList)
    ));

    out.clear();
    encode_packet(
        &ChatCommand {
            command: "server survival".into(),
        },
        &mut out,
    )
    .unwrap();
    assert!(matches!(
        inspect_proxy_command(&out, None),
        Some(ProxyCommand::ServerSwitch("survival"))
    ));

    out.clear();
    encode_packet(
        &ChatCommand {
            command: "glist".into(),
        },
        &mut out,
    )
    .unwrap();
    assert!(matches!(
        inspect_proxy_command(&out, None),
        Some(ProxyCommand::Glist)
    ));
}

#[test]
fn resolves_initial_and_fallback_servers() {
    let mut cfg = test_config(None, ForwardingMode::None);
    let lobby: SocketAddr = "127.0.0.1:25566".parse().unwrap();
    let survival: SocketAddr = "127.0.0.1:25567".parse().unwrap();
    cfg.servers.insert("lobby".into(), lobby);
    cfg.servers.insert("survival".into(), survival);
    cfg.try_servers = vec!["lobby".into(), "survival".into()];

    let (name, addr) = resolve_initial_server("Player", None, &cfg).unwrap();
    assert_eq!(name.as_deref(), Some("lobby"));
    assert_eq!(addr, lobby);

    let fb = find_fallback_server("Player", Some("lobby"), lobby, &cfg).unwrap();
    assert_eq!(fb, ("survival".to_owned(), survival));
}

#[test]
fn resolves_forced_hosts_and_legacy_forwarding() {
    let mut cfg = test_config(None, ForwardingMode::BungeeGuard);
    let skyblock: SocketAddr = "127.0.0.1:25568".parse().unwrap();
    cfg.servers.insert("skyblock".into(), skyblock);
    cfg.forced_hosts
        .insert("sky.example.com".into(), "skyblock".into());

    let (name, addr) = resolve_forced_host("SKY.EXAMPLE.COM.:25565\0FML3", &cfg).unwrap();
    assert_eq!(name, "skyblock");
    assert_eq!(addr, skyblock);

    let hs = Handshake {
        protocol_version: VarInt(PROTOCOL_VERSION),
        server_address: "sky.example.com\0FML3".into(),
        server_port: 25565,
        next_state: ConnectionState::Login,
    };
    let identity = PlayerIdentity {
        uuid: offline_player_uuid("Steve"),
        username: "Steve".into(),
        properties: Vec::new(),
    };
    let forwarded =
        format_forwarded_handshake(&hs, "10.0.0.5:41234".parse().unwrap(), &identity, &cfg);
    assert!(forwarded.server_address.contains("10.0.0.5"));
    assert!(forwarded.server_address.contains("bungeeguard-token"));
    assert!(forwarded.server_address.contains("test-secret-key"));
}

#[test]
fn load_balances_server_groups() {
    let mut cfg = test_config(None, ForwardingMode::None);
    let l1: SocketAddr = "127.0.0.1:25566".parse().unwrap();
    let l2: SocketAddr = "127.0.0.1:25567".parse().unwrap();
    cfg.servers.insert("lobby-1".into(), l1);
    cfg.servers.insert("lobby-2".into(), l2);
    cfg.server_groups
        .insert("lobby".into(), vec!["lobby-1".into(), "lobby-2".into()]);

    let state = ProxyState::new(cfg.clone(), RadonManager::new(), None);
    state.increment_server_player("lobby-1");
    state.increment_server_player("lobby-1");
    let (picked, addr) = state.resolve_named_target("lobby", &cfg).unwrap();
    assert_eq!(picked, "lobby-2");
    assert_eq!(addr, l2);
}

#[tokio::test]
async fn e2e_play_brand_commands_and_bungeecord_switch() {
    use super::plugin_msg::{encode_brand_payload, write_bungee_utf};
    use rad_core::packet::configuration::{
        AcknowledgeFinishConfiguration, ConfigClientboundPluginMessage,
    };
    use rad_core::packet::play::{
        ConfigurationAcknowledged, DeclareCommands, PlayClientboundPluginMessage,
        StartConfiguration,
    };

    let lobby_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let lobby_addr = lobby_listener.local_addr().unwrap();
    let survival_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let survival_addr = survival_listener.local_addr().unwrap();
    let (_login_done_tx, _login_done_rx) = tokio::sync::oneshot::channel::<()>();

    let lobby_task = tokio::spawn(async move {
        let (mut sock, _) = lobby_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let hs = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<Handshake>()
            .unwrap();
        assert_eq!(hs.next_state, ConnectionState::Login);
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();
        assert_eq!(login.username, "Switcher");

        let mut out = BytesMut::new();
        encode_packet(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();

        out.clear();
        encode_packet(
            &ConfigClientboundPluginMessage {
                channel: "minecraft:brand".into(),
                data: encode_brand_payload("vanilla"),
            },
            &mut out,
        )
        .unwrap();
        encode_packet(
            &rad_core::packet::configuration::FinishConfiguration,
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let ack = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<AcknowledgeFinishConfiguration>()
            .unwrap();
        let _ = ack;

        let mut bungee_data = BytesMut::new();
        write_bungee_utf(&mut bungee_data, "Connect");
        write_bungee_utf(&mut bungee_data, "survival");
        out.clear();
        encode_packet(
            &PlayClientboundPluginMessage {
                channel: "bungeecord:main".into(),
                data: bungee_data.to_vec(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();
    });

    let survival_task = tokio::spawn(async move {
        let (mut sock, _) = survival_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let hs = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<Handshake>()
            .unwrap();
        assert_eq!(hs.next_state, ConnectionState::Login);
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();
        assert_eq!(login.username, "Switcher");

        let mut out = BytesMut::new();
        encode_packet(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();
    });

    let mut cfg = test_config(Some(lobby_addr), ForwardingMode::None);
    cfg.servers.insert("survival".into(), survival_addr);
    cfg.proxy_brand = Some("RadProxy".into());

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(cfg, RadonManager::new(), None));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = proxy_listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: proxy_addr.port(),
            next_state: ConnectionState::Login,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(
        &LoginStart {
            username: "Switcher".into(),
            player_uuid: offline_player_uuid("Switcher"),
        },
        &mut out,
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let success = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<LoginSuccess>()
        .unwrap();
    assert_eq!(success.username, "Switcher");
    let _ = _login_done_tx.send(());

    out.clear();
    encode_packet(&rad_core::packet::login::LoginAcknowledged, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let brand_pkt = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<ConfigClientboundPluginMessage>()
        .unwrap();
    assert_eq!(brand_pkt.channel, "minecraft:brand");
    assert_eq!(brand_pkt.data, encode_brand_payload("RadProxy"));

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::FinishConfiguration>()
        .unwrap();

    out.clear();
    encode_packet(&AcknowledgeFinishConfiguration, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let cmds = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<DeclareCommands>()
        .unwrap();
    assert!(!cmds.nodes.is_empty());

    let start_cfg = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<StartConfiguration>()
        .unwrap();
    let _ = start_cfg;

    out.clear();
    encode_packet(&ConfigurationAcknowledged, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    lobby_task.await.unwrap();
    survival_task.await.unwrap();
}

#[tokio::test]
async fn e2e_play_disconnect_moves_to_fallback() {
    use rad_core::nbt::NbtTag;
    use rad_core::packet::configuration::AcknowledgeFinishConfiguration;
    use rad_core::packet::play::{
        ConfigurationAcknowledged, DeclareCommands, PlayDisconnect, StartConfiguration,
        SystemChatMessage,
    };

    let lobby_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let lobby_addr = lobby_listener.local_addr().unwrap();
    let fallback_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fallback_addr = fallback_listener.local_addr().unwrap();

    let lobby_task = tokio::spawn(async move {
        let (mut sock, _) = lobby_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<Handshake>()
            .unwrap();
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();

        let mut out = BytesMut::new();
        encode_packet(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();

        out.clear();
        encode_packet(
            &rad_core::packet::configuration::FinishConfiguration,
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<AcknowledgeFinishConfiguration>()
            .unwrap();

        out.clear();
        encode_packet(
            &PlayDisconnect {
                reason: NbtTag::String("Maintenance".into()),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();
    });

    let fallback_task = tokio::spawn(async move {
        let (mut sock, _) = fallback_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<Handshake>()
            .unwrap();
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();

        let mut out = BytesMut::new();
        encode_packet(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();

        out.clear();
        encode_packet(
            &rad_core::packet::configuration::FinishConfiguration,
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<AcknowledgeFinishConfiguration>()
            .unwrap();
    });

    let mut cfg = test_config(Some(lobby_addr), ForwardingMode::None);
    cfg.servers.insert("fallback".into(), fallback_addr);
    cfg.try_servers = vec!["lobby".into(), "fallback".into()];

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(cfg, RadonManager::new(), None));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = proxy_listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: proxy_addr.port(),
            next_state: ConnectionState::Login,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(
        &LoginStart {
            username: "FallbackUser".into(),
            player_uuid: offline_player_uuid("FallbackUser"),
        },
        &mut out,
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let success = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<LoginSuccess>()
        .unwrap();
    assert_eq!(success.username, "FallbackUser");

    out.clear();
    encode_packet(&rad_core::packet::login::LoginAcknowledged, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::FinishConfiguration>()
        .unwrap();

    out.clear();
    encode_packet(&AcknowledgeFinishConfiguration, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<DeclareCommands>()
        .unwrap();

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<StartConfiguration>()
        .unwrap();

    out.clear();
    encode_packet(&ConfigurationAcknowledged, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::FinishConfiguration>()
        .unwrap();

    out.clear();
    encode_packet(&AcknowledgeFinishConfiguration, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<DeclareCommands>()
        .unwrap();

    let chat = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<SystemChatMessage>()
        .unwrap();
    let NbtTag::String(text) = chat.content else {
        panic!("expected string chat message, got {:?}", chat.content);
    };
    assert!(text.contains("Maintenance"), "unexpected chat text: {text}");

    lobby_task.await.unwrap();
    fallback_task.await.unwrap();
}

#[tokio::test]
async fn e2e_initial_login_acknowledged_stays_in_configuration_and_merges_commands() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend_addr = backend_listener.local_addr().unwrap();

    let backend_task = tokio::spawn(async move {
        let (mut sock, _) = backend_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap();
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();

        let mut out = BytesMut::new();
        encode_packet(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();

        out.clear();
        encode_packet(
            &rad_core::packet::configuration::ConfigClientboundPluginMessage {
                channel: "minecraft:brand".into(),
                data: encode_brand_payload("vanilla"),
            },
            &mut out,
        )
        .unwrap();
        encode_packet(
            &rad_core::packet::configuration::FinishConfiguration,
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<AcknowledgeFinishConfiguration>()
            .unwrap();

        out.clear();
        let backend_cmds = DeclareCommands {
            nodes: vec![
                rad_core::packet::play::CommandNode {
                    kind: rad_core::packet::play::CommandNodeKind::Root,
                    executable: false,
                    children: vec![VarInt(1)],
                },
                rad_core::packet::play::CommandNode {
                    kind: rad_core::packet::play::CommandNodeKind::Literal { name: "tps".into() },
                    executable: true,
                    children: vec![],
                },
            ],
            root_index: VarInt(0),
        };
        encode_packet(&backend_cmds, &mut out).unwrap();
        sock.write_all(&out).await.unwrap();
    });

    let mut cfg = test_config(Some(backend_addr), ForwardingMode::None);
    cfg.proxy_brand = Some("RadProxy-Test".into());
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(cfg, RadonManager::new(), None));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = proxy_listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: proxy_addr.port(),
            next_state: ConnectionState::Login,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(
        &LoginStart {
            username: "ConfigCheck".into(),
            player_uuid: offline_player_uuid("ConfigCheck"),
        },
        &mut out,
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<LoginSuccess>()
        .unwrap();

    out.clear();
    encode_packet(&rad_core::packet::login::LoginAcknowledged, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    // Must receive rewritten minecraft:brand and FinishConfiguration — NOT DeclareCommands!
    let brand_pkt = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::ConfigClientboundPluginMessage>()
        .unwrap();
    assert_eq!(brand_pkt.channel, "minecraft:brand");
    assert_eq!(brand_pkt.data, encode_brand_payload("RadProxy-Test"));

    let _ = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::FinishConfiguration>()
        .unwrap();

    out.clear();
    encode_packet(&AcknowledgeFinishConfiguration, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    // Read DeclareCommands (first proxy-only on AckFinishConfig, then merged when backend sends it).
    let mut saw_merged = false;
    for _ in 0..2 {
        let cmds = read_next_frame(&mut client, &mut in_buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<DeclareCommands>()
            .unwrap();
        let has_tps = cmds.nodes.iter().any(|n| {
            matches!(
                &n.kind,
                rad_core::packet::play::CommandNodeKind::Literal { name } if name == "tps"
            )
        });
        let has_server = cmds.nodes.iter().any(|n| {
            matches!(
                &n.kind,
                rad_core::packet::play::CommandNodeKind::Literal { name } if name == "server"
            )
        });
        if has_tps && has_server {
            saw_merged = true;
            break;
        }
    }
    assert!(
        saw_merged,
        "expected merged DeclareCommands with both tps and server"
    );

    backend_task.await.unwrap();
}

#[tokio::test]
async fn e2e_server_switch_compression_transcoding_and_fragmented_frames() {
    let srv1_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv1_addr = srv1_listener.local_addr().unwrap();
    let srv2_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv2_addr = srv2_listener.local_addr().unwrap();

    let client_comp = Some(CompressionConfig::new(8));

    let srv1_task = tokio::spawn(async move {
        let (mut sock, _) = srv1_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap();
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();

        let mut out = BytesMut::new();
        encode_packet(
            &SetCompression {
                threshold: VarInt(8),
            },
            &mut out,
        )
        .unwrap();
        encode_packet_with_compression(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
            Some(8),
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame_with_compression(&mut sock, &mut buf, 65536, client_comp)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();

        out.clear();
        encode_packet_with_compression(
            &rad_core::packet::configuration::FinishConfiguration,
            &mut out,
            Some(8),
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame_with_compression(&mut sock, &mut buf, 65536, client_comp)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<AcknowledgeFinishConfiguration>()
            .unwrap();

        // Keep socket open briefly while client sends compressed `/server srv2`.
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    // Backend 2 uses NO compression (None) while client keeps threshold=8!
    let srv2_task = tokio::spawn(async move {
        let (mut sock, _) = srv2_listener.accept().await.unwrap();
        let mut buf = BytesMut::new();
        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap();
        let login = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<LoginStart>()
            .unwrap();

        let mut out = BytesMut::new();
        encode_packet(
            &LoginSuccess {
                uuid: login.player_uuid,
                username: login.username,
                properties: Vec::new(),
                session_id: Uuid::nil(),
            },
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<rad_core::packet::login::LoginAcknowledged>()
            .unwrap();

        out.clear();
        encode_packet(
            &rad_core::packet::configuration::FinishConfiguration,
            &mut out,
        )
        .unwrap();
        sock.write_all(&out).await.unwrap();

        let _ = read_next_frame(&mut sock, &mut buf, 65536)
            .await
            .unwrap()
            .unwrap()
            .decode_as::<AcknowledgeFinishConfiguration>()
            .unwrap();

        // Send a fragmented SystemChatMessage across two TCP writes.
        out.clear();
        encode_packet(
            &SystemChatMessage {
                content: NbtTag::String("Welcome to uncompressed srv2!".into()),
                overlay: false,
            },
            &mut out,
        )
        .unwrap();
        let split = out.len() / 2;
        sock.write_all(&out[..split]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        sock.write_all(&out[split..]).await.unwrap();
    });

    let mut cfg = test_config(Some(srv1_addr), ForwardingMode::None);
    cfg.servers.insert("srv2".into(), srv2_addr);
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(cfg, RadonManager::new(), None));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = proxy_listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: proxy_addr.port(),
            next_state: ConnectionState::Login,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(
        &LoginStart {
            username: "CompUser".into(),
            player_uuid: offline_player_uuid("CompUser"),
        },
        &mut out,
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let set_comp = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<SetCompression>()
        .unwrap();
    assert_eq!(set_comp.threshold.0, 8);

    let _ = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<LoginSuccess>()
        .unwrap();

    out.clear();
    encode_packet_with_compression(
        &rad_core::packet::login::LoginAcknowledged,
        &mut out,
        Some(8),
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::FinishConfiguration>()
        .unwrap();

    out.clear();
    encode_packet_with_compression(&AcknowledgeFinishConfiguration, &mut out, Some(8)).unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<DeclareCommands>()
        .unwrap();

    // Send COMPRESSED `/server srv2` (>8 bytes so zlib-compressed!)
    out.clear();
    encode_packet_with_compression(
        &ChatCommand {
            command: "server srv2".into(),
        },
        &mut out,
        Some(8),
    )
    .unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<StartConfiguration>()
        .unwrap();

    out.clear();
    encode_packet_with_compression(&ConfigurationAcknowledged, &mut out, Some(8)).unwrap();
    client.write_all(&out).await.unwrap();

    // Receive FinishConfiguration transcoded from uncompressed srv2 -> compressed client!
    let _ = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<rad_core::packet::configuration::FinishConfiguration>()
        .unwrap();

    out.clear();
    encode_packet_with_compression(&AcknowledgeFinishConfiguration, &mut out, Some(8)).unwrap();
    client.write_all(&out).await.unwrap();

    let _ = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<DeclareCommands>()
        .unwrap();

    // Receive the fragmented SystemChatMessage reassembled & transcoded with compression=8!
    let chat = read_next_frame_with_compression(&mut client, &mut in_buf, 65536, client_comp)
        .await
        .unwrap()
        .unwrap()
        .decode_as::<SystemChatMessage>()
        .unwrap();
    assert_eq!(
        chat.content,
        NbtTag::String("Welcome to uncompressed srv2!".into())
    );

    srv1_task.await.unwrap();
    srv2_task.await.unwrap();
}

#[test]
fn restricted_server_enforced_across_initial_group_and_fallback() {
    let mut cfg = test_config(None, ForwardingMode::None);
    let vip_addr: SocketAddr = "127.0.0.1:25571".parse().unwrap();
    let pub_addr: SocketAddr = "127.0.0.1:25572".parse().unwrap();
    cfg.servers.insert("vip".into(), vip_addr);
    cfg.servers.insert("public".into(), pub_addr);
    cfg.try_servers = vec!["vip".into(), "public".into()];
    cfg.server_groups
        .insert("all".into(), vec!["vip".into(), "public".into()]);
    cfg.restricted_servers
        .insert("vip".into(), ["vipuser".to_owned()].into_iter().collect());

    // Non-VIP skips restricted "vip" on initial, fallback, and group resolution.
    assert_eq!(
        resolve_initial_server("guest", None, &cfg),
        Some((Some("public".into()), pub_addr))
    );
    assert_eq!(
        find_fallback_server("guest", Some("public"), pub_addr, &cfg),
        None
    );

    let state = ProxyState::new(cfg.clone(), RadonManager::new(), None);
    assert_eq!(
        state.resolve_named_target_for_user("guest", "vip", &cfg),
        None
    );
    assert_eq!(
        state.resolve_named_target_for_user("guest", "all", &cfg),
        Some(("public".into(), pub_addr))
    );
    assert_eq!(
        state.resolve_named_target_for_user("vipuser", "vip", &cfg),
        Some(("vip".into(), vip_addr))
    );
}

#[tokio::test]
async fn e2e_haproxy_rejects_untrusted_peer_when_trusted_list_empty() {
    let mut cfg = test_config(None, ForwardingMode::None);
    cfg.haproxy_protocol = true;
    // haproxy_trusted_proxies is empty and haproxy_allow_untrusted is false.

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let state = Arc::new(ProxyState::new(cfg, RadonManager::new(), None));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = proxy_listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"PROXY TCP4 1.2.3.4 127.0.0.1 12345 25565\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 16];
    let res = client.read(&mut buf).await;
    assert!(
        matches!(res, Ok(0) | Err(_)),
        "untrusted HAProxy peer must be closed immediately, got {res:?}"
    );
}

#[tokio::test]
async fn e2e_forwarding_profiles_none_legacy_bungeeguard() {
    for mode in [
        ForwardingMode::None,
        ForwardingMode::Legacy,
        ForwardingMode::BungeeGuard,
    ] {
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_addr = backend_listener.local_addr().unwrap();
        let (addr_tx, addr_rx) = tokio::sync::oneshot::channel::<String>();

        let backend_task = tokio::spawn(async move {
            let (mut sock, _) = backend_listener.accept().await.unwrap();
            let mut buf = BytesMut::new();
            let hs: Handshake = read_next_frame(&mut sock, &mut buf, 65536)
                .await
                .unwrap()
                .unwrap()
                .decode_as()
                .unwrap();
            let _ = addr_tx.send(hs.server_address.clone());
            let login: LoginStart = read_next_frame(&mut sock, &mut buf, 65536)
                .await
                .unwrap()
                .unwrap()
                .decode_as()
                .unwrap();
            let mut out = BytesMut::new();
            encode_packet(
                &LoginSuccess {
                    uuid: login.player_uuid,
                    username: login.username,
                    properties: Vec::new(),
                    session_id: Uuid::from_u128(1),
                },
                &mut out,
            )
            .unwrap();
            sock.write_all(&out).await.unwrap();
        });

        let mut cfg = test_config(Some(backend_addr), mode);
        cfg.forwarding_secret = b"bg-secret-token".to_vec();
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let state = Arc::new(ProxyState::new(cfg, RadonManager::new(), None));
        let server_state = Arc::clone(&state);
        tokio::spawn(async move {
            let (stream, peer) = proxy_listener.accept().await.unwrap();
            handle_connection(stream, peer, server_state).await;
        });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let mut out = BytesMut::new();
        encode_packet(
            &Handshake {
                protocol_version: VarInt(PROTOCOL_VERSION),
                server_address: "play.example.com".into(),
                server_port: proxy_addr.port(),
                next_state: ConnectionState::Login,
            },
            &mut out,
        )
        .unwrap();
        encode_packet(
            &LoginStart {
                username: "ModeTester".into(),
                player_uuid: Uuid::from_u128(0x5555),
            },
            &mut out,
        )
        .unwrap();
        client.write_all(&out).await.unwrap();

        let mut in_buf = BytesMut::new();
        let frame = read_next_frame(&mut client, &mut in_buf, 65536)
            .await
            .unwrap()
            .unwrap();
        let ls: LoginSuccess = frame.decode_as().unwrap();
        assert_eq!(ls.username, "ModeTester");

        let forwarded_addr = addr_rx.await.unwrap();
        match mode {
            ForwardingMode::None => assert_eq!(forwarded_addr, "play.example.com"),
            ForwardingMode::Legacy => {
                let parts: Vec<&str> = forwarded_addr.split('\0').collect();
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0], "play.example.com");
                assert!(Uuid::parse_str(parts[2]).is_ok());
            }
            ForwardingMode::BungeeGuard => {
                let parts: Vec<&str> = forwarded_addr.split('\0').collect();
                assert_eq!(parts.len(), 4);
                assert!(parts[3].contains("bungeeguard-token"));
                assert!(parts[3].contains("bg-secret-token"));
            }
            ForwardingMode::Modern => unreachable!(),
        }
        backend_task.await.unwrap();
    }
}

#[tokio::test]
async fn e2e_proxy_hot_reload_from_disk() {
    let dir = std::env::temp_dir().join(format!(
        "radproxy_reload_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("radons")).unwrap();
    std::fs::create_dir_all(dir.join("radonfigs")).unwrap();

    let cfg_path = dir.join("radproxy.toml");
    let cfg = ProxyConfig::load_or_create(&cfg_path).unwrap();
    let state = Arc::new(
        ProxyState::new(cfg, RadonManager::new(), None)
            .with_config_path(cfg_path.to_string_lossy().into_owned()),
    );

    let updated = format!(
        r#"config_version = 1
bind = "127.0.0.1:25565"
motd = "Reloaded Proxy MOTD"
max_players = 99
default_server = "lobby"
radons_dir = "{}"
radonfigs_dir = "{}"

[servers]
lobby = "127.0.0.1:25566"
"#,
        dir.join("radons").display().to_string().replace('\\', "/"),
        dir.join("radonfigs")
            .display()
            .to_string()
            .replace('\\', "/"),
    );
    std::fs::write(&cfg_path, updated).unwrap();
    state.reload_current().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    let mut out = BytesMut::new();
    encode_packet(
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Status,
        },
        &mut out,
    )
    .unwrap();
    encode_packet(&StatusRequest, &mut out).unwrap();
    client.write_all(&out).await.unwrap();

    let mut in_buf = BytesMut::new();
    let status: StatusResponse = read_next_frame(&mut client, &mut in_buf, 65536)
        .await
        .unwrap()
        .unwrap()
        .decode_as()
        .unwrap();
    let payload = status.parse_payload().unwrap();
    assert_eq!(payload.players.unwrap().max, 99);
    assert!(status.json.contains("Reloaded Proxy MOTD"));

    let _ = std::fs::remove_dir_all(&dir);
}
