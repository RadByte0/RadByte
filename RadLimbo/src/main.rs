//! RadLimbo — ultra-lightweight Minecraft 26.3 (protocol 777) Limbo server.

#![warn(missing_docs)]

mod config;
mod net;

use std::path::Path;
use std::sync::Arc;

use mimalloc::MiMalloc;
use rad_core::cli::{print_startup_banner, resolve_config_path};
use rad_core::registry_provider::{
    BlockRegistryEntry, LoadedPackageRegistryProvider, RegistryProvider, RegistryProviderError,
    RegistryTagSet, TagDefinition, VersionMetadata,
};
use rad_core::version_package::LoadedVersionPackage;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::config::LimboConfig;
use crate::net::{LimboState, handle_connection};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const LIMBO_BANNER: &str = r"
  ____           _ _     _           _
 |  _ \ __ _  __| | |   (_)_ __ ___ | |__   ___
 | |_) / _` |/ _` | |   | | '_ ` _ \| '_ \ / _ \
 |  _ < (_| | (_| | |___| | | | | | | |_) | (_) |
 |_| \_\__,_|\__,_|_____|_|_| |_| |_|_.__/ \___/
";

struct LimboPackageProvider(Arc<LoadedVersionPackage>);

impl RegistryProvider for LimboPackageProvider {
    fn metadata(&self) -> VersionMetadata {
        LoadedPackageRegistryProvider::new(&self.0).metadata()
    }
    fn block_by_name(&self, name: &str) -> Option<BlockRegistryEntry> {
        LoadedPackageRegistryProvider::new(&self.0).block_by_name(name)
    }
    fn block_by_state(&self, state: u16) -> Option<BlockRegistryEntry> {
        LoadedPackageRegistryProvider::new(&self.0).block_by_state(state)
    }
    fn item_by_name(&self, name: &str) -> Option<i32> {
        LoadedPackageRegistryProvider::new(&self.0).item_by_name(name)
    }
    fn item_name_by_id(&self, id: i32) -> Option<&str> {
        self.0.item_name_by_id(id)
    }
    fn entity_by_name(&self, name: &str) -> Option<i32> {
        LoadedPackageRegistryProvider::new(&self.0).entity_by_name(name)
    }
    fn menu_by_name(&self, name: &str) -> Option<i32> {
        LoadedPackageRegistryProvider::new(&self.0).menu_by_name(name)
    }
    fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        LoadedPackageRegistryProvider::new(&self.0).block_tags()
    }
    fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        LoadedPackageRegistryProvider::new(&self.0).item_tags()
    }
    fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError> {
        LoadedPackageRegistryProvider::new(&self.0).core_registry_tags()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    print_startup_banner("RadLimbo", env!("CARGO_PKG_VERSION"), LIMBO_BANNER);

    let cfg_path = resolve_config_path("radlimbo.toml");
    let cfg = match LimboConfig::load_or_create(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, path = %cfg_path.display(), "failed to load config");
            std::process::exit(1);
        }
    };
    info!(config_version = crate::config::LIMBO_CONFIG_VERSION, config = %cfg_path.display(), "loaded config");

    let versions_dir = cfg_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("versions");
    let registry_provider: Option<Arc<dyn RegistryProvider>> =
        if versions_dir.join(rad_core::MINECRAFT_VERSION).exists() {
            match LoadedVersionPackage::load_from_versions_dir(
                &versions_dir,
                rad_core::MINECRAFT_VERSION,
            ) {
                Ok(pkg) => {
                    info!(
                        version = rad_core::MINECRAFT_VERSION,
                        dir = %versions_dir.display(),
                        "loaded RadLimbo version data package"
                    );
                    Some(Arc::new(LimboPackageProvider(Arc::new(pkg))))
                }
                Err(e) => {
                    error!(
                        error = %e,
                        dir = %versions_dir.display(),
                        "invalid RadLimbo version data package; refusing startup"
                    );
                    std::process::exit(1);
                }
            }
        } else {
            None
        };

    let bind_addr = cfg.bind_addr;
    let forwarding = cfg.forwarding_mode;
    let state = Arc::new(LimboState::with_registry_provider(cfg, registry_provider));

    let listener = match TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(%bind_addr, error = %e, "failed to bind RadLimbo listener");
            std::process::exit(1);
        }
    };

    info!(%bind_addr, ?forwarding, config = %cfg_path.display(), "RadLimbo listening");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down RadLimbo");
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer_addr)) => {
                        let st = Arc::clone(&state);
                        tokio::spawn(async move {
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
}
