//! `RadProxy` — High-performance L7 Minecraft proxy and traffic filter.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod auto_update;
mod config;
mod metrics;
mod net;
mod proxy;
mod radon;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rad_core::cli::{exit_with_pause, print_startup_banner, resolve_config_path};
use rad_core::console_ui::{ConsoleUi, make_log_writer};
use rad_core::{MINECRAFT_VERSION, PROTOCOL_VERSION};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::config::ProxyConfig;
use crate::proxy::auth::ProxyKeypair;
use crate::proxy::{ProxyState, handle_connection};
use crate::radon::RadonManager;

const PROXY_BANNER: &str = r"
  ____           _ ____
 |  _ \ __ _  __| |  _ \ _ __ _____  ___   _
 | |_) / _` |/ _` | |_) | '__/ _ \ \/ / | | |
 |  _ < (_| | (_| |  __/| | | (_) >  <| |_| |
 |_| \_\__,_|\__,_|_|   |_|  \___/_/\_\\__, |
                                       |___/
";

fn main() {
    let _console_ui = ConsoleUi::start(
        "RadProxy",
        env!("CARGO_PKG_VERSION"),
        "reload, status, stop",
    );
    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(make_log_writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rad_proxy=info".into()),
        )
        .init();

    print_startup_banner("RadProxy", env!("CARGO_PKG_VERSION"), PROXY_BANNER);

    for dir in ["radons", "radonfigs"] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            error!(error = %e, dir, "failed to create proxy directory");
            exit_with_pause(1);
        }
    }

    let cfg_path = resolve_config_path("rad.properties");
    let work_dir = cfg_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    auto_update::run_startup_auto_update(&cfg_path, work_dir);
    let config = match ProxyConfig::load_or_create(&cfg_path) {
        Ok(cfg) => cfg,
        Err(err) => {
            error!(%err, path = %cfg_path.display(), "failed to load config");
            exit_with_pause(1);
        }
    };
    info!(config_version = config.config_version, config = %cfg_path.display(), "loaded config");

    let mut radons = RadonManager::new();
    if let Err(err) = radons.load_from_dirs(&config.radons_dir, &config.radonfigs_dir) {
        error!(%err, radons_dir = %config.radons_dir, radonfigs_dir = %config.radonfigs_dir, "failed to scan radons/radonfigs directories");
    }

    let keypair = if config.online_mode {
        info!("generating 1024-bit RSA keypair for online-mode");
        match ProxyKeypair::generate() {
            Ok(kp) => Some(kp),
            Err(err) => {
                error!(%err, "failed to generate RSA keypair");
                exit_with_pause(1);
            }
        }
    } else {
        None
    };

    let bind_addr = config.bind_addr;
    let online_mode = config.online_mode;
    let server_count = config.servers.len();
    let metrics_bind_addr = config.metrics_bind_addr;
    let radon_count = radons.len();
    let state = Arc::new(
        ProxyState::new(config, radons, keypair)
            .with_config_path(cfg_path.to_string_lossy().into_owned()),
    );

    if let Some(metrics_addr) = metrics_bind_addr {
        let metrics_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("rad-metrics".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build metrics runtime");
                rt.block_on(async move {
                    metrics::http::spawn_metrics_server(metrics_addr, metrics_state);
                    std::future::pending::<()>().await;
                });
            })
            .expect("failed to spawn metrics thread");
    }

    #[cfg(unix)]
    run_thread_per_core(bind_addr, online_mode, radon_count, server_count, state);

    #[cfg(not(unix))]
    run_multi_thread(bind_addr, online_mode, radon_count, server_count, state);
}

#[cfg(unix)]
fn run_thread_per_core(
    bind_addr: std::net::SocketAddr,
    online_mode: bool,
    radon_count: usize,
    server_count: usize,
    state: Arc<ProxyState>,
) {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .max(1);

    info!(
        %bind_addr,
        workers,
        mode = "thread-per-core",
        mc_version = MINECRAFT_VERSION,
        protocol = PROTOCOL_VERSION,
        online_mode,
        radons = radon_count,
        servers = server_count,
        "RadProxy listening (type 'help' for console commands)"
    );

    for idx in 0..workers {
        let state = Arc::clone(&state);
        std::thread::Builder::new()
            .name(format!("rad-worker-{idx}"))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build worker runtime");
                let local = tokio::task::LocalSet::new();
                rt.block_on(local.run_until(async move {
                    let listener = match bind_reuseport_listener(bind_addr) {
                        Ok(l) => l,
                        Err(err) => {
                            error!(%bind_addr, worker = idx, %err, "failed to bind TCP listener");
                            exit_with_pause(1);
                        }
                    };
                    loop {
                        match listener.accept().await {
                            Ok((stream, peer_addr)) => {
                                let state = Arc::clone(&state);
                                tokio::task::spawn_local(async move {
                                    handle_connection(stream, peer_addr, state).await;
                                });
                            }
                            Err(err) => {
                                error!(worker = idx, %err, "failed to accept connection");
                            }
                        }
                    }
                }));
            })
            .expect("failed to spawn worker thread");
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build control runtime");
    rt.block_on(run_control_loop(state));
}

#[cfg(not(unix))]
fn run_multi_thread(
    bind_addr: std::net::SocketAddr,
    online_mode: bool,
    radon_count: usize,
    server_count: usize,
    state: Arc<ProxyState>,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async move {
        let listener = match bind_single_listener(bind_addr) {
            Ok(l) => l,
            Err(err) => {
                error!(%bind_addr, %err, "failed to bind TCP listener");
                exit_with_pause(1);
            }
        };

        info!(
            %bind_addr,
            mode = "multi-thread",
            mc_version = MINECRAFT_VERSION,
            protocol = PROTOCOL_VERSION,
            online_mode,
            radons = radon_count,
            servers = server_count,
            "RadProxy listening (type 'help' for console commands)"
        );

        let accept_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer_addr)) => {
                        let state = Arc::clone(&accept_state);
                        tokio::spawn(async move {
                            handle_connection(stream, peer_addr, state).await;
                        });
                    }
                    Err(err) => {
                        error!(%err, "failed to accept connection");
                    }
                }
            }
        });

        run_control_loop(state).await;
    });
}

async fn run_control_loop(state: Arc<ProxyState>) {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    rad_core::console_ui::spawn_command_reader(move |line| cmd_tx.send(line).is_ok());

    loop {
        tokio::select! {
            Some(line) = cmd_rx.recv() => {
                ConsoleUi::echo_command(line.trim());
                if handle_console_command(line.trim(), &state) {
                    state.kick_all("Proxy is shutting down.");
                    state.current_radons().disable_all();
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down RadProxy");
                state.kick_all("Proxy is shutting down.");
                state.current_radons().disable_all();
                break;
            }
        }
    }
}

#[cfg(unix)]
fn bind_reuseport_listener(bind_addr: std::net::SocketAddr) -> std::io::Result<TcpListener> {
    let domain = if bind_addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };
    let sock = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    sock.set_reuse_address(true)?;
    sock.set_reuse_port(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&bind_addr.into())?;
    sock.listen(4096)?;
    let std_listener: std::net::TcpListener = sock.into();
    TcpListener::from_std(std_listener)
}

#[cfg(not(unix))]
fn bind_single_listener(bind_addr: std::net::SocketAddr) -> std::io::Result<TcpListener> {
    let domain = if bind_addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };
    let sock = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    sock.set_reuse_address(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&bind_addr.into())?;
    sock.listen(4096)?;
    let std_listener: std::net::TcpListener = sock.into();
    TcpListener::from_std(std_listener)
}

fn handle_console_command(cmd: &str, state: &ProxyState) -> bool {
    match cmd {
        "" => false,
        "help" => {
            info!(
                "commands: status, glist, servers, radons, alert <msg>, send <player|all> <server>, kick <player> [reason], reload, stop"
            );
            false
        }
        "status" | "glist" => {
            let snap = state.metrics.snapshot();
            let online = state.online_players.load(Ordering::Relaxed);
            info!(
                online,
                total_connections = snap.total_connections,
                status_pings = snap.status_pings,
                blocked_by_filter = snap.blocked_by_filter,
                blocked_by_radon = snap.blocked_by_radon,
                server_switches = snap.server_switches,
                "RadProxy status"
            );
            false
        }
        "radons" => {
            let cfg = state.current_config();
            let radons = state.current_radons();
            let list = radons.list();
            if list.is_empty() {
                info!("no radons loaded in {}", cfg.radons_dir);
            } else {
                for (name, version) in list {
                    info!(name, version, "loaded radon");
                }
            }
            false
        }
        "servers" => {
            let cfg = state.current_config();
            if cfg.servers.is_empty() {
                info!("no named [servers] configured in rad.toml");
            } else {
                for (name, addr) in &cfg.servers {
                    info!(name = %name, %addr, "configured backend server");
                }
            }
            false
        }
        "reload" => {
            match state.reload_current() {
                Ok(radon_count) => {
                    info!(radons = radon_count, "reloaded proxy config and radons");
                }
                Err(err) => {
                    error!(%err, "failed to reload proxy config");
                }
            }
            false
        }
        "stop" | "exit" => {
            info!("stopping RadProxy via console");
            true
        }
        other => {
            if let Some(msg) = other.strip_prefix("alert ") {
                let count = state.alert_all(&format!("[Alert] {}", msg.trim()));
                info!(recipients = count, "sent global alert");
                return false;
            }
            if let Some(rest) = other.strip_prefix("send ") {
                let mut parts = rest.split_whitespace();
                if let (Some(target), Some(server)) = (parts.next(), parts.next()) {
                    let cfg = state.current_config();
                    if let Some(&addr) = cfg.servers.get(server) {
                        if target.eq_ignore_ascii_case("all") {
                            let count = state.send_all(server, addr);
                            info!(count, server, "sent all players");
                        } else if state.send_player(target, server, addr) {
                            info!(player = target, server, "sent player");
                        } else {
                            info!(player = target, "player not online");
                        }
                    } else {
                        info!(server, "unknown server in rad.toml");
                    }
                    return false;
                }
            }
            if let Some(rest) = other.strip_prefix("kick ") {
                let rest = rest.trim();
                let (target, reason) = rest
                    .split_once(char::is_whitespace)
                    .map(|(t, r)| (t, r.trim()))
                    .unwrap_or((rest, "Kicked by console."));
                if state.kick_player(target, reason) {
                    info!(player = target, reason, "kicked player");
                } else {
                    info!(player = target, "player not online");
                }
                return false;
            }
            info!(command = other, "unknown command (type 'help')");
            false
        }
    }
}
