//! BungeeCord plugin messaging (`bungeecord:main`), `minecraft:brand`, and
//! `DeclareCommands` helpers for `RadProxy`.

use bytes::{Buf, BufMut, BytesMut};
use rad_core::packet::play::{CommandNode, CommandNodeKind, DeclareCommands};
use rad_core::types::{Encode, VarInt};

/// Parsed BungeeCord plugin message subcommand received from a backend server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BungeeRequest {
    /// Switch the current player to `server`.
    Connect {
        /// Target server or group name.
        server: String,
    },
    /// Switch `player` to `server`.
    ConnectOther {
        /// Target player username.
        player: String,
        /// Target server or group name.
        server: String,
    },
    /// Query the current player's real IP and port.
    Ip,
    /// Query online player count for `server` (or `"ALL"`).
    PlayerCount {
        /// Target server name or `"ALL"`.
        server: String,
    },
    /// Query online player list for `server` (or `"ALL"`).
    PlayerList {
        /// Target server name or `"ALL"`.
        server: String,
    },
    /// Query configured server names.
    GetServers,
    /// Send a chat message to `player` (or `"ALL"`).
    Message {
        /// Target player username or `"ALL"`.
        player: String,
        /// Chat message text.
        message: String,
    },
    /// Kick `player` with `reason`.
    KickPlayer {
        /// Target player username.
        player: String,
        /// Disconnect reason.
        reason: String,
    },
}

fn read_bungee_utf(buf: &mut &[u8]) -> Option<String> {
    if buf.remaining() < 2 {
        return None;
    }
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    buf.advance(2);
    if buf.remaining() < len {
        return None;
    }
    let s = core::str::from_utf8(&buf[..len]).ok()?.to_owned();
    buf.advance(len);
    Some(s)
}

pub fn write_bungee_utf(buf: &mut BytesMut, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    buf.put_u16(len);
    buf.put_slice(&bytes[..len as usize]);
}

/// Parses a `bungeecord:main` / `BungeeCord` plugin message payload.
#[must_use]
pub fn parse_bungee_request(mut data: &[u8]) -> Option<BungeeRequest> {
    let subchannel = read_bungee_utf(&mut data)?;
    match subchannel.as_str() {
        "Connect" => Some(BungeeRequest::Connect {
            server: read_bungee_utf(&mut data)?,
        }),
        "ConnectOther" => Some(BungeeRequest::ConnectOther {
            player: read_bungee_utf(&mut data)?,
            server: read_bungee_utf(&mut data)?,
        }),
        "IP" => Some(BungeeRequest::Ip),
        "PlayerCount" => Some(BungeeRequest::PlayerCount {
            server: read_bungee_utf(&mut data)?,
        }),
        "PlayerList" => Some(BungeeRequest::PlayerList {
            server: read_bungee_utf(&mut data)?,
        }),
        "GetServers" => Some(BungeeRequest::GetServers),
        "Message" => Some(BungeeRequest::Message {
            player: read_bungee_utf(&mut data)?,
            message: read_bungee_utf(&mut data)?,
        }),
        "KickPlayer" => Some(BungeeRequest::KickPlayer {
            player: read_bungee_utf(&mut data)?,
            reason: read_bungee_utf(&mut data)?,
        }),
        _ => None,
    }
}

/// Encodes a `minecraft:brand` payload (VarInt length + UTF-8 brand string).
#[must_use]
pub fn encode_brand_payload(brand: &str) -> Vec<u8> {
    let bytes = brand.as_bytes();
    let mut out = BytesMut::with_capacity(bytes.len() + 5);
    let _ = VarInt(bytes.len() as i32).encode(&mut out);
    out.put_slice(bytes);
    out.to_vec()
}

/// Builds a [`DeclareCommands`] tree for `RadProxy`'s built-in commands.
#[must_use]
pub fn build_proxy_declare_commands(is_admin: bool) -> DeclareCommands {
    let mut nodes = vec![CommandNode::root(Vec::new())];

    // /server [name]
    let server_idx = nodes.len() as i32;
    let server_arg_idx = server_idx + 1;
    nodes[0].children.push(VarInt(server_idx));
    nodes.push(CommandNode::literal(
        "server",
        true,
        vec![VarInt(server_arg_idx)],
    ));
    nodes.push(CommandNode::word_arg("name", true, vec![], None));

    // /glist
    let glist_idx = nodes.len() as i32;
    nodes[0].children.push(VarInt(glist_idx));
    nodes.push(CommandNode::literal("glist", true, vec![]));

    // /lang [code]
    let lang_idx = nodes.len() as i32;
    let lang_arg_idx = lang_idx + 1;
    nodes[0].children.push(VarInt(lang_idx));
    nodes.push(CommandNode::literal(
        "lang",
        true,
        vec![VarInt(lang_arg_idx)],
    ));
    nodes.push(CommandNode::word_arg("code", true, vec![], None));

    if is_admin {
        for (cmd, arg) in [
            ("find", "player"),
            ("alert", "message"),
            ("kick", "player"),
            ("radproxy", "subcommand"),
        ] {
            let cmd_idx = nodes.len() as i32;
            let arg_idx = cmd_idx + 1;
            nodes[0].children.push(VarInt(cmd_idx));
            nodes.push(CommandNode::literal(
                cmd,
                cmd == "radproxy",
                vec![VarInt(arg_idx)],
            ));
            let node = if cmd == "alert" || cmd == "kick" {
                CommandNode::greedy_arg(arg, true, vec![], None)
            } else {
                CommandNode::word_arg(arg, true, vec![], None)
            };
            nodes.push(node);
        }

        // /send <player|all> <server>
        let send_idx = nodes.len() as i32;
        let send_target_idx = send_idx + 1;
        let send_srv_idx = send_idx + 2;
        nodes[0].children.push(VarInt(send_idx));
        nodes.push(CommandNode::literal(
            "send",
            false,
            vec![VarInt(send_target_idx)],
        ));
        nodes.push(CommandNode::word_arg(
            "target",
            false,
            vec![VarInt(send_srv_idx)],
            None,
        ));
        nodes.push(CommandNode::word_arg("server", true, vec![], None));
    }

    DeclareCommands {
        nodes,
        root_index: VarInt(0),
    }
}

/// Merges `RadProxy`'s built-in commands into a backend server's [`DeclareCommands`] tree.
///
/// Preserves all backend nodes, indices, and subtrees; on root literal name collision,
/// `RadProxy`'s command takes priority because the proxy intercepts those commands first.
#[must_use]
pub fn merge_declare_commands(mut backend: DeclareCommands, is_admin: bool) -> DeclareCommands {
    let proxy = build_proxy_declare_commands(is_admin);
    let Ok(backend_root) = usize::try_from(backend.root_index.0) else {
        return proxy;
    };
    if backend_root >= backend.nodes.len() || proxy.nodes.is_empty() {
        return proxy;
    }

    let proxy_root_names: Vec<String> = proxy.nodes[0]
        .children
        .iter()
        .filter_map(|idx| usize::try_from(idx.0).ok())
        .filter_map(|idx| proxy.nodes.get(idx))
        .filter_map(|node| match &node.kind {
            CommandNodeKind::Literal { name } => Some(name.clone()),
            _ => None,
        })
        .collect();

    // Remove colliding root literals from backend root so proxy commands take priority.
    let kept_children: Vec<VarInt> = backend.nodes[backend_root]
        .children
        .iter()
        .copied()
        .filter(|idx| {
            let Some(node_idx) = usize::try_from(idx.0).ok() else {
                return false;
            };
            let Some(node) = backend.nodes.get(node_idx) else {
                return false;
            };
            !matches!(
                &node.kind,
                CommandNodeKind::Literal { name }
                    if proxy_root_names.iter().any(|p| p.eq_ignore_ascii_case(name))
            )
        })
        .collect();
    backend.nodes[backend_root].children = kept_children;

    // Proxy index 0 is skipped; proxy index `i >= 1` maps to `base + i - 1`.
    let offset = backend.nodes.len() as i32 - 1;
    for mut node in proxy.nodes.into_iter().skip(1) {
        for child in &mut node.children {
            child.0 += offset;
        }
        backend.nodes.push(node);
    }
    let proxy_tree = build_proxy_declare_commands(is_admin);
    for child in &proxy_tree.nodes[0].children {
        backend.nodes[backend_root]
            .children
            .push(VarInt(child.0 + offset));
    }

    backend
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bungee_connect_and_playercount() {
        let mut buf = BytesMut::new();
        write_bungee_utf(&mut buf, "Connect");
        write_bungee_utf(&mut buf, "survival");
        assert_eq!(
            parse_bungee_request(&buf),
            Some(BungeeRequest::Connect {
                server: "survival".into()
            })
        );

        buf.clear();
        write_bungee_utf(&mut buf, "PlayerCount");
        write_bungee_utf(&mut buf, "ALL");
        assert_eq!(
            parse_bungee_request(&buf),
            Some(BungeeRequest::PlayerCount {
                server: "ALL".into()
            })
        );
    }

    #[test]
    fn builds_admin_and_player_command_trees() {
        let player_tree = build_proxy_declare_commands(false);
        assert_eq!(player_tree.nodes[0].children.len(), 3);

        let admin_tree = build_proxy_declare_commands(true);
        assert_eq!(admin_tree.nodes[0].children.len(), 8);
    }

    #[test]
    fn merges_backend_and_proxy_declare_commands_preserving_non_colliding() {
        let backend = DeclareCommands {
            nodes: vec![
                CommandNode::root(vec![VarInt(1), VarInt(2)]),
                CommandNode::literal("help", true, vec![]),
                CommandNode::literal("server", true, vec![]),
            ],
            root_index: VarInt(0),
        };

        let merged = merge_declare_commands(backend, false);
        let root_names: Vec<&str> = merged.nodes[0]
            .children
            .iter()
            .filter_map(|i| merged.nodes.get(i.0 as usize))
            .filter_map(|n| match &n.kind {
                CommandNodeKind::Literal { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(root_names, vec!["help", "server", "glist", "lang"]);
        // Colliding backend "server" was replaced by proxy's "server" with its `name` argument child.
        let server_idx = merged.nodes[0].children[1].0 as usize;
        assert_eq!(merged.nodes[server_idx].children.len(), 1);
    }
}
