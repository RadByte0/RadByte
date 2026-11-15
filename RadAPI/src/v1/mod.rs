//! Stable **v1** RadAPI surface (`API_VERSION = 1`).

pub mod wasm;

/// Current `RadAPI` v1 version constant.
pub const API_VERSION: u32 = 1;

#[cfg(feature = "proxy")]
mod proxy_types {
    use std::net::SocketAddr;

    use rad_core::packet::handshake::Handshake;
    use rad_core::packet::login::LoginStart;
    use rad_core::text::TextComponent;

    pub use rad_core::radon::{RadonManifest, RadonTarget};

    /// Decision returned by a [`ProxyRadon`] hook.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum RadonAction {
        /// Allow processing to continue normally.
        Continue,
        /// Reject/disconnect the client with a formatted reason.
        Deny(TextComponent),
        /// Override the target backend address for this connection.
        RouteTo(SocketAddr),
    }

    /// Shared read-only context passed to radon hooks.
    #[derive(Debug, Clone, Copy)]
    pub struct RadonContext {
        /// Client socket address.
        pub peer_addr: SocketAddr,
        /// Current online player count on the proxy.
        pub online_players: i32,
    }

    /// Trait implemented by all `RadProxy` v1 radons.
    pub trait ProxyRadon: Send + Sync {
        /// Unique radon name.
        fn name(&self) -> &str;

        /// Radon version string.
        fn version(&self) -> &str;

        /// Called once when the radon is enabled.
        fn on_enable(&self) {}

        /// Called when a client sends a `Handshake` packet.
        fn on_handshake(&self, _ctx: RadonContext, _handshake: &Handshake) -> RadonAction {
            RadonAction::Continue
        }

        /// Called when a client sends a `LoginStart` packet.
        fn on_login(
            &self,
            _ctx: RadonContext,
            _handshake: &Handshake,
            _login: &LoginStart,
        ) -> RadonAction {
            RadonAction::Continue
        }

        /// Called once when the radon is disabled on shutdown or reload.
        fn on_disable(&self) {}
    }
}

#[cfg(feature = "proxy")]
pub use proxy_types::*;

/// Decision returned by a cancellable `RadServer` v1 event hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerEventResult {
    /// Allow the action to proceed normally.
    Allow,
    /// Cancel the action.
    Cancel,
}

/// Snapshot of a player passed to [`ServerRadon`] hooks.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerPlayerContext {
    /// Server-assigned entity ID.
    pub entity_id: i32,
    /// Player username.
    pub username: String,
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Current health (`0.0..=20.0`).
    pub hp: f32,
}

/// Imperative server manipulation action requested by a [`ServerRadon`].
#[derive(Debug, Clone, PartialEq)]
pub enum ServerCommandAction {
    /// Broadcast a system chat message to all players.
    BroadcastChat(String),
    /// Send a system chat message to a specific player.
    SendMessage {
        /// Target player entity ID.
        entity_id: i32,
        /// Message text.
        message: String,
    },
    /// Teleport a player to `(x, y, z)`.
    TeleportPlayer {
        /// Target player entity ID.
        entity_id: i32,
        /// Target X coordinate.
        x: f64,
        /// Target Y coordinate.
        y: f64,
        /// Target Z coordinate.
        z: f64,
    },
    /// Set a block in the world.
    SetBlock {
        /// Block X coordinate.
        x: i32,
        /// Block Y coordinate.
        y: i32,
        /// Block Z coordinate.
        z: i32,
        /// Block state ID.
        state_id: u16,
    },
    /// Disconnect a player with a reason.
    KickPlayer {
        /// Target player entity ID.
        entity_id: i32,
        /// Disconnect reason.
        reason: String,
    },
}

/// Trait implemented by native `RadServer` v1 extensions (Radons).
pub trait ServerRadon: Send + Sync {
    /// Unique radon name.
    fn name(&self) -> &str;

    /// Radon version string.
    fn version(&self) -> &str;

    /// Called when a player finishes joining `RadServer`.
    fn on_player_join(
        &self,
        _player: &ServerPlayerContext,
        _actions: &mut Vec<ServerCommandAction>,
    ) {
    }

    /// Called when a player disconnects from `RadServer`.
    fn on_player_quit(
        &self,
        _player: &ServerPlayerContext,
        _actions: &mut Vec<ServerCommandAction>,
    ) {
    }

    /// Called when a player sends a chat message.
    fn on_player_chat(
        &self,
        _player: &ServerPlayerContext,
        _message: &str,
        _actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        ServerEventResult::Allow
    }

    /// Called before a player breaks a block at `(x, y, z)`.
    fn on_block_break(
        &self,
        _player: &ServerPlayerContext,
        _x: i32,
        _y: i32,
        _z: i32,
        _state_id: u16,
        _actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        ServerEventResult::Allow
    }

    /// Called before a player places a block at `(x, y, z)`.
    fn on_block_place(
        &self,
        _player: &ServerPlayerContext,
        _x: i32,
        _y: i32,
        _z: i32,
        _state_id: u16,
        _actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        ServerEventResult::Allow
    }

    /// Called before a player takes damage.
    fn on_player_damage(
        &self,
        _player: &ServerPlayerContext,
        _amount: f32,
        _actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        ServerEventResult::Allow
    }

    /// Called once per server tick (`20 TPS`).
    fn on_server_tick(&self, _tick: u64, _actions: &mut Vec<ServerCommandAction>) {}
}
