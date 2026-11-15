use rad_api::v1::wasm;

#[unsafe(no_mangle)]
pub extern "C" fn radon_on_handshake(protocol: i32, _port: i32, _next_state: i32) -> i32 {
    let cfg = wasm::config();
    if cfg.contains("deny_proto=999") && protocol == 999 {
        wasm::set_deny_reason("blocked by compiled wasm radon");
        return wasm::ACTION_DENY;
    }
    wasm::ACTION_CONTINUE
}

#[unsafe(no_mangle)]
pub extern "C" fn radon_on_login(_online_players: i32) -> i32 {
    if wasm::username() == "BannedWasmUser" {
        wasm::set_deny_reason("banned by compiled wasm radon");
        return wasm::ACTION_DENY;
    }
    wasm::ACTION_CONTINUE
}
