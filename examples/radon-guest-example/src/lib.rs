use rad_api::v1::wasm;

#[unsafe(no_mangle)]
pub extern "C" fn radon_on_player_join(_entity_id: i32) {
    let cfg = wasm::config();
    let user = wasm::username();
    let msg = if cfg.is_empty() {
        format!("Welcome {user} from compiled wasm radon")
    } else {
        format!("Welcome {user} [{cfg}]")
    };
    wasm::send_chat(&msg);
}

#[unsafe(no_mangle)]
pub extern "C" fn radon_on_block_break(x: i32, _y: i32, _z: i32, _state_id: i32) -> i32 {
    let cfg = wasm::config();
    if x == 42 || (cfg.contains("protect_x0=true") && x == 0) {
        wasm::send_chat("protected by compiled wasm radon");
        return wasm::ACTION_CANCEL;
    }
    wasm::ACTION_ALLOW
}
