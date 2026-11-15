//! Cross-platform WebAssembly (`wasm32-unknown-unknown`) guest SDK for v1 radons.
//!
//! When compiled to `wasm32`, these helpers call the `"radbyte"` host imports
//! provided by `RadProxy`'s `wasmi` sandbox.

/// Return code for `radon_on_handshake` / `radon_on_login` indicating normal continuation.
pub const ACTION_CONTINUE: i32 = 0;

/// Return code for `radon_on_handshake` / `radon_on_login` indicating connection rejection.
pub const ACTION_DENY: i32 = 1;

/// Return code for `RadServer` v1 hooks indicating the event is allowed.
pub const ACTION_ALLOW: i32 = 0;

/// Return code for `RadServer` v1 hooks indicating the event is cancelled.
pub const ACTION_CANCEL: i32 = 1;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "radbyte")]
unsafe extern "C" {
    fn rad_get_host(ptr: i32, max_len: i32) -> i32;
    fn rad_get_username(ptr: i32, max_len: i32) -> i32;
    fn rad_get_ip(ptr: i32, max_len: i32) -> i32;
    fn rad_get_config(ptr: i32, max_len: i32) -> i32;
    fn rad_set_deny_reason(ptr: i32, len: i32);
    fn rad_send_chat(ptr: i32, len: i32);
    fn rad_broadcast_chat(ptr: i32, len: i32);
    fn rad_teleport_player(x: f64, y: f64, z: f64);
    fn rad_set_block(x: i32, y: i32, z: i32, state_id: i32);
}

/// Reads the client's `Handshake.server_address` hostname (available in `on_handshake` and `on_login`).
#[must_use]
pub fn host() -> String {
    read_host_string(|ptr, cap| {
        #[cfg(target_arch = "wasm32")]
        // SAFETY: `ptr` points to a valid mutable buffer of `cap` bytes in WASM linear memory.
        unsafe {
            rad_get_host(ptr, cap)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (ptr, cap);
            0
        }
    })
}

/// Reads the player's username (available in `on_login`).
#[must_use]
pub fn username() -> String {
    read_host_string(|ptr, cap| {
        #[cfg(target_arch = "wasm32")]
        // SAFETY: `ptr` points to a valid mutable buffer of `cap` bytes in WASM linear memory.
        unsafe {
            rad_get_username(ptr, cap)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (ptr, cap);
            0
        }
    })
}

/// Reads the client's IP address string.
#[must_use]
pub fn ip() -> String {
    read_host_string(|ptr, cap| {
        #[cfg(target_arch = "wasm32")]
        // SAFETY: `ptr` points to a valid mutable buffer of `cap` bytes in WASM linear memory.
        unsafe {
            rad_get_ip(ptr, cap)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (ptr, cap);
            0
        }
    })
}

/// Reads the raw TOML config for this radon from `radonfigs/<name>.toml`.
#[must_use]
pub fn config() -> String {
    read_host_string(|ptr, cap| {
        #[cfg(target_arch = "wasm32")]
        // SAFETY: `ptr` points to a valid mutable buffer of `cap` bytes in WASM linear memory.
        unsafe {
            rad_get_config(ptr, cap)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (ptr, cap);
            0
        }
    })
}

/// Sets the disconnect reason message shown to the client when returning [`ACTION_DENY`].
pub fn set_deny_reason(reason: &str) {
    #[cfg(target_arch = "wasm32")]
    // SAFETY: `reason.as_ptr()` is valid for `reason.len()` bytes.
    unsafe {
        rad_set_deny_reason(reason.as_ptr() as i32, reason.len() as i32);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = reason;
    }
}

/// Sends a chat message to the triggering player (available in `RadServer` v1 hooks).
pub fn send_chat(message: &str) {
    #[cfg(target_arch = "wasm32")]
    // SAFETY: `message.as_ptr()` is valid for `message.len()` bytes.
    unsafe {
        rad_send_chat(message.as_ptr() as i32, message.len() as i32);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = message;
    }
}

/// Broadcasts a chat message to all connected players (available in `RadServer` v1 hooks).
pub fn broadcast_chat(message: &str) {
    #[cfg(target_arch = "wasm32")]
    // SAFETY: `message.as_ptr()` is valid for `message.len()` bytes.
    unsafe {
        rad_broadcast_chat(message.as_ptr() as i32, message.len() as i32);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = message;
    }
}

/// Teleports the triggering player to `(x, y, z)` (available in `RadServer` v1 hooks).
pub fn teleport_player(x: f64, y: f64, z: f64) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        rad_teleport_player(x, y, z);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (x, y, z);
    }
}

/// Sets the block state at `(x, y, z)` (available in `RadServer` v1 hooks).
pub fn set_block(x: i32, y: i32, z: i32, state_id: i32) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        rad_set_block(x, y, z, state_id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (x, y, z, state_id);
    }
}

fn read_host_string(reader: impl FnOnce(i32, i32) -> i32) -> String {
    let mut buf = [0u8; 512];
    let n = reader(buf.as_mut_ptr() as i32, buf.len() as i32);
    if n <= 0 {
        return String::new();
    }
    let len = (n as usize).min(buf.len());
    String::from_utf8_lossy(&buf[..len]).into_owned()
}
