//! Loadable Minecraft Java 26.3 (`protocol 777`) server & protocol runtime (`rad_version_26_3`).

#![warn(missing_docs)]

pub mod abi;
pub mod config;
pub mod gameplay;
pub mod net;
pub mod radon;
pub mod registries;
pub mod world;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rad_core::version_package::LoadedVersionPackage;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::config::{SERVER_CONFIG_VERSION, ServerConfig};
use crate::net::{ServerState, handle_connection, spawn_tick_loop_with_shutdown};

async fn shutdown_signal() {
    #[cfg(windows)]
    {
        match tokio::signal::windows::ctrl_close() {
            Ok(mut close) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = close.recv() => {} }
            }
            Err(e) => {
                tracing::warn!(error = %e, "unable to register console-close handler");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Runs the Minecraft 26.3 (`protocol 777`) server using `cfg_path` (`rad.properties`)
/// and `versions_dir` (`versions/`). Returns `0` on clean shutdown or non-zero on error.
pub fn run_server(cfg_path: &Path, versions_dir: &Path) -> i32 {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let cfg = match ServerConfig::load_or_create(cfg_path) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, path = %cfg_path.display(), "failed to load config");
            return 1;
        }
    };

    let version_package = match LoadedVersionPackage::load_from_versions_dir(
        versions_dir,
        rad_core::MINECRAFT_VERSION,
    ) {
        Ok(pkg) => {
            let pkg = Arc::new(pkg);
            abi::set_active_package(Arc::clone(&pkg));
            pkg
        }
        Err(e) => {
            error!(
                error = %e,
                versions_dir = %versions_dir.display(),
                "failed to load required Minecraft version package before opening port"
            );
            return 1;
        }
    };

    let world_root = Path::new(&cfg.level_name);
    if let Err(error) = crate::world::containers::ContainerStore::try_load_from_dir(world_root) {
        error!(world = %world_root.display(), %error, "refusing startup with invalid container storage");
        return 1;
    }
    match rad_version::check_world_compatibility(world_root, &rad_version::MC_26_3) {
        Ok(
            rad_version::WorldCompatibility::Compatible
            | rad_version::WorldCompatibility::EmptyNewWorld
            | rad_version::WorldCompatibility::ConfirmedLegacyRadByteWorld,
        ) => {}
        Ok(rad_version::WorldCompatibility::UnmarkedInsufficientData { reason })
        | Ok(rad_version::WorldCompatibility::Incompatible { reason })
        | Ok(rad_version::WorldCompatibility::CorruptMetadata { reason }) => {
            error!(world = %world_root.display(), "{reason}");
            return 1;
        }
        Err(e) => {
            error!(error = %e, world = %world_root.display(), "failed to verify world profile");
            return 1;
        }
    }

    for dir in [
        format!("{}/region", cfg.level_name),
        format!("{}/players", cfg.level_name),
        "radons".to_owned(),
        "radonfigs".to_owned(),
    ] {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            error!(error = %e, dir, "failed to create server directory");
            return 1;
        }
    }
    info!(
        config_version = SERVER_CONFIG_VERSION,
        config = %cfg_path.display(),
        level_name = %cfg.level_name,
        version = version_package.game_version(),
        protocol = version_package.protocol_version(),
        version_package = %version_package.package_dir().display(),
        "loaded config and version package inside rad_version_26_3"
    );

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(error = %e, "failed to build tokio runtime");
            return 1;
        }
    };

    rt.block_on(async move {
        let bind_addr = cfg.bind_addr;
        let forwarding = cfg.forwarding_mode;
        let tps = cfg.tps;
        let view_distance = cfg.view_distance;
        let simulation_distance = cfg.simulation_distance;
        let world_dir = PathBuf::from(&cfg.level_name);
        let state = Arc::new(ServerState::new_with_world_dir_and_package(
            cfg,
            Some(world_dir),
            version_package,
        ));
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let tick_handle = spawn_tick_loop_with_shutdown(Arc::clone(&state), shutdown_rx);

        let listener = match TcpListener::bind(bind_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(%bind_addr, error = %e, "failed to bind RadServer listener");
                return 1;
            }
        };

        info!(
            %bind_addr,
            ?forwarding,
            tps,
            view_distance,
            simulation_distance,
            config = %cfg_path.display(),
            "RadServer listening (via rad_version_26_3)"
        );

        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                let trimmed = line.trim().to_owned();
                if !trimmed.is_empty() && cmd_tx.send(trimmed).is_err() {
                    break;
                }
            }
        });

        let stop_file = PathBuf::from(".radserver.stop");
        let _ = std::fs::remove_file(&stop_file);
        let mut signal_poll = tokio::time::interval(std::time::Duration::from_millis(200));
        let mut session_tasks = tokio::task::JoinSet::new();
        let console_shutdown = shutdown_signal();
        tokio::pin!(console_shutdown);

        loop {
            while session_tasks.try_join_next().is_some() {}
            tokio::select! {
                _ = &mut console_shutdown => {
                    info!("shutting down RadServer (console signal), disconnecting players and saving world/playerdata");
                    break;
                }
                _ = signal_poll.tick() => {
                    if stop_file.exists() {
                        let _ = std::fs::remove_file(&stop_file);
                        info!("shutting down RadServer (stop signal), disconnecting players and saving world/playerdata");
                        break;
                    }
                }
                Some(cmd) = cmd_rx.recv() => {
                    if crate::net::handle_console_command(&state, &cmd).await {
                        info!("shutting down RadServer (console stop), disconnecting players and saving world/playerdata");
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer_addr)) => {
                            let st = Arc::clone(&state);
                            session_tasks.spawn(async move {
                                handle_connection(stream, peer_addr, st).await;
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "accept error");
                        }
                    }
                }
            }
        }

        let _ = shutdown_tx.send(true);
        let _ = tick_handle.await;
        state.disconnect_all("Server shutting down.").await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while session_tasks.join_next().await.is_some() {}
        })
        .await;
        info!("shutdown: calling save_all");
        match state.save_all().await {
            Ok(saved) => info!(saved_chunks = saved, "RadServer save complete"),
            Err(e) => error!(error = %e, "failed to save world on shutdown"),
        }
        info!("shutdown: exiting");
        0
    })
}

#[cfg(test)]
mod abi_tests {
    use super::abi::*;
    use rad_core::version_package::{RAD_VERSION_ABI_VERSION, RAD_VERSION_OK, RadVersionInfo};

    #[test]
    fn c_abi_version_info_and_registry_queries_work() {
        assert_eq!(rad_version_abi_version(), RAD_VERSION_ABI_VERSION);
        assert_eq!(rad_version_protocol_version(), rad_core::PROTOCOL_VERSION);

        let mut info = RadVersionInfo {
            struct_size: 0,
            abi_version: 0,
            protocol_version: 0,
            world_data_version: 0,
            game_version: [0; 32],
            target_os: [0; 32],
            target_arch: [0; 32],
        };
        assert_eq!(unsafe { rad_version_info(&mut info) }, RAD_VERSION_OK);
        assert_eq!(info.protocol_version, 777);
        assert_eq!(info.abi_version, 1);

        let stone = b"minecraft:stone";
        assert_eq!(
            unsafe { rad_version_item_id(stone.as_ptr(), stone.len()) },
            rad_core::version_package::RAD_VERSION_ERR_NOT_FOUND
        );
        assert_eq!(
            unsafe { rad_version_block_default_state(stone.as_ptr(), stone.len()) },
            rad_core::version_package::RAD_VERSION_ERR_NOT_FOUND
        );

        let mut status_buf = [0u8; 512];
        let mut status_len = 0usize;
        let motd = b"RadByte DLL";
        assert_eq!(
            unsafe {
                rad_version_format_status_json(
                    motd.as_ptr(),
                    motd.len(),
                    3,
                    20,
                    status_buf.as_mut_ptr(),
                    status_buf.len(),
                    &mut status_len,
                )
            },
            RAD_VERSION_OK
        );
        let status_str = std::str::from_utf8(&status_buf[..status_len]).unwrap();
        assert!(status_str.contains("\"protocol\":777"));
        assert!(status_str.contains("RadByte DLL"));
    }
}
