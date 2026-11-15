use super::*;
use rad_core::crypto::offline_player_uuid;
use rad_core::forwarding::{ForwardingData, encode_velocity_forwarding};
use tokio::net::TcpListener;

fn test_limbo_config(mode: LimboForwardingMode) -> LimboConfig {
    LimboConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        forwarding_mode: mode,
        forwarding_secret: b"limbo-test-secret".to_vec(),
        motd: "Limbo Test".into(),
        max_players: 100,
        view_distance: 2,
        game_mode: 3,
        spawn_x: 0.5,
        spawn_y: 64.0,
        spawn_z: 0.5,
        brand: "RadLimbo".into(),
        welcome_message: Some("Welcome to Limbo!".into()),
        reconnect_server: Some("lobby".into()),
        keep_alive_interval: Duration::from_secs(30),
        max_packet_size: DEFAULT_MAX_PACKET_SIZE,
    }
}

#[tokio::test]
async fn e2e_limbo_status_and_ping() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(LimboState::new(test_limbo_config(
        LimboForwardingMode::None,
    )));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, server_state).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Status,
        },
    )
    .await
    .unwrap();
    send_packet(&mut client, &StatusRequest).await.unwrap();

    let mut buf = BytesMut::new();
    let resp: StatusResponse = read_typed_packet(&mut client, &mut buf, 65536)
        .await
        .unwrap();
    assert!(resp.json.contains("Limbo Test"));

    send_packet(&mut client, &PingRequest { timestamp: 42 })
        .await
        .unwrap();
    let pong: PongResponse = read_typed_packet(&mut client, &mut buf, 65536)
        .await
        .unwrap();
    assert_eq!(pong.timestamp, 42);
}

#[tokio::test]
async fn e2e_limbo_rejects_invalid_modern_forwarding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(LimboState::with_registry_provider(
        test_limbo_config(LimboForwardingMode::Modern),
        Some(Arc::new(
            rad_core::registry_provider::SyntheticFixtureRegistryProvider::minimal_valid(),
        )),
    ));
    let server_state = Arc::clone(&state);
    tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, server_state).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    let uuid = offline_player_uuid("BadProxy");
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
            username: "BadProxy".into(),
            player_uuid: uuid,
        },
    )
    .await
    .unwrap();

    let mut buf = BytesMut::new();
    let req: LoginPluginRequest = read_typed_packet(&mut client, &mut buf, 65536)
        .await
        .unwrap();
    let bad_payload = encode_velocity_forwarding(
        &ForwardingData {
            address: "127.0.0.1".into(),
            uuid,
            username: "BadProxy".into(),
            properties: Vec::new(),
        },
        b"wrong-secret",
    )
    .unwrap();
    send_packet(
        &mut client,
        &LoginPluginResponse {
            message_id: req.message_id,
            successful: true,
            data: bad_payload,
        },
    )
    .await
    .unwrap();

    let disconnect: LoginDisconnect = read_typed_packet(&mut client, &mut buf, 65536)
        .await
        .unwrap();
    assert!(
        disconnect
            .reason_json
            .contains("Invalid Velocity forwarding")
    );
}

#[test]
fn limbo_configuration_tags_work_with_synthetic_provider() {
    use rad_core::packet::configuration::{ConfigUpdateTags, RegistryData};
    use rad_core::registry_provider::SyntheticFixtureRegistryProvider;

    let provider = SyntheticFixtureRegistryProvider::minimal_valid();
    let registries = RegistryData::minimal_vanilla_26_3();
    let tags = ConfigUpdateTags::for_registries_with_provider(&registries, &provider)
        .expect("synthetic provider should build ConfigUpdateTags");
    assert!(
        tags.registries
            .iter()
            .any(|r| r.registry == "minecraft:block")
    );
}

#[tokio::test]
async fn e2e_limbo_refuses_configuration_without_real_tags() {
    use rad_core::packet::configuration::{ConfigUpdateTags, RegistryData};
    use rad_core::registry_provider::{
        BlockRegistryEntry, RegistryProvider, RegistryProviderError, RegistryTagSet, TagDefinition,
        VersionMetadata,
    };

    struct NoTagsProvider;
    impl RegistryProvider for NoTagsProvider {
        fn metadata(&self) -> VersionMetadata {
            VersionMetadata {
                game_version: "0.0-synthetic".into(),
                protocol_version: 0,
                world_data_version: 0,
                source: "synthetic-test".into(),
                is_synthetic_fixture: true,
            }
        }
        fn block_by_name(&self, _name: &str) -> Option<BlockRegistryEntry> {
            None
        }
        fn block_by_state(&self, _state: u16) -> Option<BlockRegistryEntry> {
            None
        }
        fn item_by_name(&self, _name: &str) -> Option<i32> {
            None
        }
        fn item_name_by_id(&self, _id: i32) -> Option<&str> {
            None
        }
        fn entity_by_name(&self, _name: &str) -> Option<i32> {
            None
        }
        fn menu_by_name(&self, _name: &str) -> Option<i32> {
            None
        }
        fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
            Err(RegistryProviderError::MissingData(
                "block tags not packaged".into(),
            ))
        }
        fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
            Err(RegistryProviderError::MissingData(
                "item tags not packaged".into(),
            ))
        }
        fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError> {
            Err(RegistryProviderError::MissingData(
                "core tags not packaged".into(),
            ))
        }
    }

    let registries = RegistryData::minimal_vanilla_26_3();
    let err =
        ConfigUpdateTags::for_registries_with_provider(&registries, &NoTagsProvider).unwrap_err();
    assert!(matches!(err, RegistryProviderError::MissingData(_)));
}
