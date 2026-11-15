//! Backend selection, forced hosts, load balancing, and fallback routing.

use super::*;

pub(super) fn resolve_forced_host(
    server_address: &str,
    cfg: &ProxyConfig,
) -> Option<(String, SocketAddr)> {
    if cfg.forced_hosts.is_empty() {
        return None;
    }
    let clean_host = server_address
        .split('\0')
        .next()
        .unwrap_or(server_address)
        .split(':')
        .next()
        .unwrap_or(server_address)
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let server_name = cfg.forced_hosts.get(&clean_host)?;
    let addr = *cfg.servers.get(server_name)?;
    Some((server_name.clone(), addr))
}

pub(super) fn format_forwarded_handshake(
    handshake: &Handshake,
    peer_addr: SocketAddr,
    identity: &PlayerIdentity,
    cfg: &ProxyConfig,
) -> Handshake {
    let mut out = handshake.clone();
    match cfg.forwarding_mode {
        ForwardingMode::Legacy | ForwardingMode::BungeeGuard => {
            let base_host = handshake
                .server_address
                .split('\0')
                .next()
                .unwrap_or(&handshake.server_address);
            let uuid_simple = identity.uuid.simple().to_string();
            if cfg.forwarding_mode == ForwardingMode::BungeeGuard {
                let token = String::from_utf8_lossy(&cfg.forwarding_secret);
                let escaped = token.replace('\\', "\\\\").replace('"', "\\\"");
                out.server_address = format!(
                    "{base_host}\0{}\0{uuid_simple}\0[{{\"name\":\"bungeeguard-token\",\"value\":\"{escaped}\"}}]",
                    peer_addr.ip()
                );
            } else {
                out.server_address = format!("{base_host}\0{}\0{uuid_simple}", peer_addr.ip());
            }
        }
        ForwardingMode::None | ForwardingMode::Modern => {}
    }
    out
}

pub(super) fn can_access_server(username: &str, server_name: &str, cfg: &ProxyConfig) -> bool {
    let lower = username.to_ascii_lowercase();
    if cfg.admins.contains(&lower) {
        return true;
    }
    match cfg.restricted_servers.get(server_name) {
        Some(allowed) => allowed.contains(&lower),
        None => true,
    }
}

pub(super) fn resolve_initial_server(
    username: &str,
    target_backend: Option<SocketAddr>,
    cfg: &ProxyConfig,
) -> Option<(Option<String>, SocketAddr)> {
    if let Some(addr) = target_backend {
        let name = cfg
            .servers
            .iter()
            .find(|(_, server_addr)| **server_addr == addr)
            .map(|(name, _)| name.clone());
        if let Some(ref server_name) = name
            && !can_access_server(username, server_name, cfg)
        {
            return None;
        }
        return Some((name, addr));
    }

    for name in &cfg.try_servers {
        if can_access_server(username, name, cfg)
            && let Some(&addr) = cfg.servers.get(name)
        {
            return Some((Some(name.clone()), addr));
        }
    }
    None
}

pub(super) fn find_fallback_server(
    username: &str,
    current_name: Option<&str>,
    current_addr: SocketAddr,
    cfg: &ProxyConfig,
) -> Option<(String, SocketAddr)> {
    for name in &cfg.try_servers {
        if Some(name.as_str()) == current_name || !can_access_server(username, name, cfg) {
            continue;
        }
        if let Some(&addr) = cfg.servers.get(name)
            && addr != current_addr
        {
            return Some((name.clone(), addr));
        }
    }
    None
}

pub(super) async fn connect_with_fallback(
    username: &str,
    initial_name: Option<&str>,
    initial_addr: SocketAddr,
    cfg: &ProxyConfig,
) -> Option<(Option<String>, SocketAddr, TcpStream)> {
    let initial_allowed = initial_name.is_none_or(|name| can_access_server(username, name, cfg));
    if initial_allowed
        && let Ok(Ok(stream)) = timeout(cfg.connect_timeout, TcpStream::connect(initial_addr)).await
    {
        return Some((initial_name.map(str::to_owned), initial_addr, stream));
    }

    for name in &cfg.try_servers {
        if Some(name.as_str()) == initial_name || !can_access_server(username, name, cfg) {
            continue;
        }
        let Some(&addr) = cfg.servers.get(name) else {
            continue;
        };
        if addr == initial_addr {
            continue;
        }
        if let Ok(Ok(stream)) = timeout(cfg.connect_timeout, TcpStream::connect(addr)).await {
            return Some((Some(name.clone()), addr, stream));
        }
    }
    None
}
