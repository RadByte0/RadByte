//! Boxed interactive first-run setup UI for `RadProxy` and `RadLimbo` (with Docker/Pelican env support).

use std::io::{Read, Write};
use std::path::Path;

use crate::auto_update::{is_auto_update_enabled, set_auto_update_setting};
use crate::console_ui::{RawTerminalGuard, is_interactive_tty, terminal_size};

const FORWARDING_MODES: [&str; 4] = ["none", "modern", "bungeeguard", "legacy"];

/// Ensures `RadProxy` has an initial `rad.properties` via interactive TUI (or Docker/Pelican env vars).
pub fn ensure_proxy_setup(cfg_path: &Path) -> Result<(), String> {
    let force_setup = std::env::args_os().skip(1).any(|a| a == "--setup");
    if cfg_path.exists() && !force_setup {
        return Ok(());
    }
    if !is_interactive_tty() {
        apply_headless_env_defaults(cfg_path, "0.0.0.0", "25565", Some("127.0.0.1:25567"))?;
        return Ok(());
    }

    let mut ip = "0.0.0.0".to_owned();
    let mut port = "25565".to_owned();
    let mut backend = "127.0.0.1:25567".to_owned();
    let mut fwd_idx = 0usize;
    let mut fwd_open = false;
    let mut auto_update = is_auto_update_enabled(cfg_path);
    let mut focused = 0usize; // 0=ip, 1=port, 2=backend, 3=fwd, 4=auto_update, 5=save
    let mut editing: Option<usize> = None;

    let _screen = SetupScreen::enter();
    let mut stdin = std::io::stdin();
    let mut byte = [0u8; 1];
    let mut esc = Vec::new();
    let mut in_esc = false;
    let mut last_size = terminal_size();

    loop {
        let size = terminal_size();
        let clear = size != last_size;
        last_size = size;
        let width = usize::from(size.0).clamp(52, 92);
        let inner = width - 4;

        let mut out = String::with_capacity(2048);
        if clear {
            out.push_str("\x1b[2J\x1b[3J\x1b[H");
        } else {
            out.push_str("\x1b[H");
        }
        append_setup_banner(&mut out, "RadProxy Setup", env!("CARGO_PKG_VERSION"), width);
        out.push_str(&format!(
            "\x1b[2K  \x1b[38;2;56;56;56m╭{}╮\x1b[0m\n",
            "─".repeat(inner + 2)
        ));

        let rows: [String; 6] = [
            format_field_row(focused == 0, editing == Some(0), "Listen IP", &ip),
            format_field_row(focused == 1, editing == Some(1), "Listen Port", &port),
            format_field_row(focused == 2, editing == Some(2), "Backend Server", &backend),
            format!(
                "{} \x1b[1;37mForwarding mode:\x1b[0m   [{}] \x1b[38;2;232;125;13m{}\x1b[0m",
                marker(focused == 3),
                if fwd_open { "▲" } else { "▼" },
                FORWARDING_MODES[fwd_idx]
            ),
            format!(
                "{} \x1b[1;37mAuto-update all:\x1b[0m  {} \x1b[90m(launcher & RadAPI)\x1b[0m",
                marker(focused == 4),
                checkbox(auto_update)
            ),
            format!(
                "{} {}",
                marker(focused == 5),
                button(focused == 5, "Save & Start Proxy")
            ),
        ];

        for (i, line) in rows.iter().enumerate() {
            out.push_str(&box_line(line, inner));
            if i == 3 && fwd_open {
                for (m_i, mode) in FORWARDING_MODES.iter().enumerate() {
                    let dot = if m_i == fwd_idx {
                        "\x1b[38;2;232;125;13m●\x1b[0m"
                    } else {
                        "\x1b[90m○\x1b[0m"
                    };
                    out.push_str(&box_line(&format!("      {dot} {mode}"), inner));
                }
            }
        }
        out.push_str(&format!(
            "\x1b[2K  \x1b[38;2;56;56;56m╰{}╯\x1b[0m\n\x1b[J",
            "─".repeat(inner + 2)
        ));
        write_flush(&out);

        if stdin.read(&mut byte).unwrap_or(0) == 0 {
            return Err("Setup cancelled".into());
        }
        let b = byte[0];
        if in_esc {
            esc.push(b);
            if matches!(b, b'~' | b'A'..=b'Z' | b'a'..=b'z') || esc.len() >= 12 {
                in_esc = false;
                match esc.as_slice() {
                    b"[A" => {
                        editing = None;
                        if fwd_open && focused == 3 {
                            fwd_idx = fwd_idx.saturating_sub(1);
                        } else {
                            focused = focused.saturating_sub(1);
                        }
                    }
                    b"[B" => {
                        editing = None;
                        if fwd_open && focused == 3 {
                            fwd_idx = (fwd_idx + 1).min(FORWARDING_MODES.len() - 1);
                        } else if focused < 5 {
                            focused += 1;
                        }
                    }
                    b"[C" if focused == 3 => fwd_open = true,
                    b"[D" if focused == 3 => fwd_open = false,
                    _ => {}
                }
                esc.clear();
            }
            continue;
        }
        if let Some(ed) = editing {
            let target = match ed {
                0 => &mut ip,
                1 => &mut port,
                _ => &mut backend,
            };
            match b {
                0x1b | b'\r' | b'\n' => editing = None,
                0x08 | 0x7f => {
                    target.pop();
                }
                c if (0x20..0x7f).contains(&c) => target.push(c as char),
                _ => {}
            }
            continue;
        }
        match b {
            0x1b => {
                in_esc = true;
                esc.clear();
            }
            0x03 | b'q' | b'Q' => return Err("Setup cancelled by user".into()),
            b'\t' => focused = (focused + 1) % 6,
            b' ' | b'\r' | b'\n' => match focused {
                0..=2 => editing = Some(focused),
                3 => fwd_open = !fwd_open,
                4 => auto_update = !auto_update,
                5 => {
                    write_initial_properties(
                        cfg_path,
                        ip.trim(),
                        port.trim(),
                        Some(backend.trim()),
                        FORWARDING_MODES[fwd_idx],
                        auto_update,
                    )?;
                    return Ok(());
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Ensures `RadLimbo` has an initial `rad.properties` via interactive TUI (or Docker/Pelican env vars).
pub fn ensure_limbo_setup(cfg_path: &Path) -> Result<(), String> {
    let force_setup = std::env::args_os().skip(1).any(|a| a == "--setup");
    if cfg_path.exists() && !force_setup {
        return Ok(());
    }
    if !is_interactive_tty() {
        apply_headless_env_defaults(cfg_path, "127.0.0.1", "25566", None)?;
        return Ok(());
    }

    let mut ip = "127.0.0.1".to_owned();
    let mut port = "25566".to_owned();
    let mut fwd_idx = 0usize;
    let mut fwd_open = false;
    let mut auto_update = is_auto_update_enabled(cfg_path);
    let mut focused = 0usize; // 0=ip, 1=port, 2=fwd, 3=auto_update, 4=save
    let mut editing: Option<usize> = None;

    let _screen = SetupScreen::enter();
    let mut stdin = std::io::stdin();
    let mut byte = [0u8; 1];
    let mut esc = Vec::new();
    let mut in_esc = false;
    let mut last_size = terminal_size();

    loop {
        let size = terminal_size();
        let clear = size != last_size;
        last_size = size;
        let width = usize::from(size.0).clamp(52, 92);
        let inner = width - 4;

        let mut out = String::with_capacity(2048);
        if clear {
            out.push_str("\x1b[2J\x1b[3J\x1b[H");
        } else {
            out.push_str("\x1b[H");
        }
        append_setup_banner(&mut out, "RadLimbo Setup", env!("CARGO_PKG_VERSION"), width);
        out.push_str(&format!(
            "\x1b[2K  \x1b[38;2;56;56;56m╭{}╮\x1b[0m\n",
            "─".repeat(inner + 2)
        ));

        let rows: [String; 5] = [
            format_field_row(focused == 0, editing == Some(0), "Listen IP", &ip),
            format_field_row(focused == 1, editing == Some(1), "Listen Port", &port),
            format!(
                "{} \x1b[1;37mForwarding mode:\x1b[0m   [{}] \x1b[38;2;232;125;13m{}\x1b[0m",
                marker(focused == 2),
                if fwd_open { "▲" } else { "▼" },
                FORWARDING_MODES[fwd_idx]
            ),
            format!(
                "{} \x1b[1;37mAuto-update:\x1b[0m      {} \x1b[90m(launcher)\x1b[0m",
                marker(focused == 3),
                checkbox(auto_update)
            ),
            format!(
                "{} {}",
                marker(focused == 4),
                button(focused == 4, "Save & Start Limbo")
            ),
        ];

        for (i, line) in rows.iter().enumerate() {
            out.push_str(&box_line(line, inner));
            if i == 2 && fwd_open {
                for (m_i, mode) in FORWARDING_MODES.iter().enumerate() {
                    let dot = if m_i == fwd_idx {
                        "\x1b[38;2;232;125;13m●\x1b[0m"
                    } else {
                        "\x1b[90m○\x1b[0m"
                    };
                    out.push_str(&box_line(&format!("      {dot} {mode}"), inner));
                }
            }
        }
        out.push_str(&format!(
            "\x1b[2K  \x1b[38;2;56;56;56m╰{}╯\x1b[0m\n\x1b[J",
            "─".repeat(inner + 2)
        ));
        write_flush(&out);

        if stdin.read(&mut byte).unwrap_or(0) == 0 {
            return Err("Setup cancelled".into());
        }
        let b = byte[0];
        if in_esc {
            esc.push(b);
            if matches!(b, b'~' | b'A'..=b'Z' | b'a'..=b'z') || esc.len() >= 12 {
                in_esc = false;
                match esc.as_slice() {
                    b"[A" => {
                        editing = None;
                        if fwd_open && focused == 2 {
                            fwd_idx = fwd_idx.saturating_sub(1);
                        } else {
                            focused = focused.saturating_sub(1);
                        }
                    }
                    b"[B" => {
                        editing = None;
                        if fwd_open && focused == 2 {
                            fwd_idx = (fwd_idx + 1).min(FORWARDING_MODES.len() - 1);
                        } else if focused < 4 {
                            focused += 1;
                        }
                    }
                    b"[C" if focused == 2 => fwd_open = true,
                    b"[D" if focused == 2 => fwd_open = false,
                    _ => {}
                }
                esc.clear();
            }
            continue;
        }
        if let Some(ed) = editing {
            let target = if ed == 0 { &mut ip } else { &mut port };
            match b {
                0x1b | b'\r' | b'\n' => editing = None,
                0x08 | 0x7f => {
                    target.pop();
                }
                c if (0x20..0x7f).contains(&c) => target.push(c as char),
                _ => {}
            }
            continue;
        }
        match b {
            0x1b => {
                in_esc = true;
                esc.clear();
            }
            0x03 | b'q' | b'Q' => return Err("Setup cancelled by user".into()),
            b'\t' => focused = (focused + 1) % 5,
            b' ' | b'\r' | b'\n' => match focused {
                0 | 1 => editing = Some(focused),
                2 => fwd_open = !fwd_open,
                3 => auto_update = !auto_update,
                4 => {
                    write_initial_properties(
                        cfg_path,
                        ip.trim(),
                        port.trim(),
                        None,
                        FORWARDING_MODES[fwd_idx],
                        auto_update,
                    )?;
                    return Ok(());
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn apply_headless_env_defaults(
    cfg_path: &Path,
    default_ip: &str,
    default_port: &str,
    default_backend: Option<&str>,
) -> Result<(), String> {
    let has_env = std::env::var_os("SERVER_PORT").is_some()
        || std::env::var_os("SERVER_IP").is_some()
        || std::env::var_os("BACKEND_ADDR").is_some()
        || std::env::var_os("FORWARDING_MODE").is_some();
    if !has_env {
        return Ok(());
    }
    let ip = std::env::var("SERVER_IP").unwrap_or_else(|_| default_ip.to_owned());
    let port = std::env::var("SERVER_PORT").unwrap_or_else(|_| default_port.to_owned());
    let backend =
        default_backend.map(|d| std::env::var("BACKEND_ADDR").unwrap_or_else(|_| d.to_owned()));
    let fwd = std::env::var("FORWARDING_MODE").unwrap_or_else(|_| "none".to_owned());
    let auto_update = is_auto_update_enabled(cfg_path);
    write_initial_properties(cfg_path, &ip, &port, backend.as_deref(), &fwd, auto_update)
}

fn write_initial_properties(
    cfg_path: &Path,
    ip: &str,
    port: &str,
    backend: Option<&str>,
    forwarding: &str,
    auto_update: bool,
) -> Result<(), String> {
    if let Some(parent) = cfg_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut out = format!(
        "# RadByte configuration\nserver-ip={ip}\nserver-port={port}\nforwarding-mode={forwarding}\n"
    );
    if let Some(b) = backend {
        out.push_str(&format!("backend-addr={b}\n"));
    }
    std::fs::write(cfg_path, out).map_err(|e| e.to_string())?;
    set_auto_update_setting(cfg_path, auto_update).map_err(|e| e.to_string())
}

struct SetupScreen(Option<RawTerminalGuard>);
impl SetupScreen {
    fn enter() -> Self {
        write_flush("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        Self(Some(RawTerminalGuard::enter()))
    }
}
impl Drop for SetupScreen {
    fn drop(&mut self) {
        drop(self.0.take());
        write_flush("\x1b[?25h\x1b[?1049l\x1b[0m");
    }
}

fn append_setup_banner(out: &mut String, title: &str, version: &str, _width: usize) {
    out.push_str("\x1b[2K  \x1b[38;2;245;158;11m█▀▀█ ▄▀▀▄ █▀▀▄ █▀▀▄ █  █ ▀▀█▀▀ █▀▀▀\x1b[0m\n");
    out.push_str("\x1b[2K  \x1b[38;2;232;125;13m█▄▄▀ █▄▄█ █  █ █▀▀▄ █▄▄█   █   █▀▀ \x1b[0m\n");
    out.push_str("\x1b[2K  \x1b[38;2;180;83;9m█  █ █  █ █▄▄▀ █▄▄▀ ▄▄▄█   █   █▄▄▄\x1b[0m\n");
    out.push_str(&format!(
        "\x1b[2K  \x1b[1;37m{title}\x1b[0m \x1b[38;2;232;125;13mv{version}\x1b[0m  \x1b[90m•  ↑/↓ Navigate  •  Space/Enter Edit/Toggle  •  Esc Quit\x1b[0m\n"
    ));
}

fn box_line(content: &str, inner_width: usize) -> String {
    let vis = visible_len(content);
    let pad = inner_width.saturating_sub(vis);
    format!(
        "\x1b[2K  \x1b[38;2;56;56;56m│\x1b[0m {content}{} \x1b[38;2;56;56;56m│\x1b[0m\n",
        " ".repeat(pad)
    )
}

fn visible_len(s: &str) -> usize {
    let mut len = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for seq in chars.by_ref() {
                if ('@'..='~').contains(&seq) {
                    break;
                }
            }
        } else {
            len += 1;
        }
    }
    len
}

fn marker(active: bool) -> &'static str {
    if active {
        "\x1b[38;2;232;125;13m›\x1b[0m"
    } else {
        " "
    }
}

fn checkbox(on: bool) -> &'static str {
    if on {
        "\x1b[38;2;232;125;13m[x]\x1b[0m"
    } else {
        "\x1b[90m[ ]\x1b[0m"
    }
}

fn button(active: bool, label: &str) -> String {
    if active {
        format!("\x1b[48;2;232;125;13m\x1b[1;30m  {label}  \x1b[0m")
    } else {
        format!("\x1b[38;2;232;125;13m[ {label} ]\x1b[0m")
    }
}

fn format_field_row(active: bool, editing: bool, label: &str, value: &str) -> String {
    let val = if editing {
        format!("\x1b[38;2;232;125;13m{value}_\x1b[0m")
    } else {
        value.to_owned()
    };
    format!("{} \x1b[1;37m{label:<17}\x1b[0m {val}", marker(active))
}

fn write_flush(s: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(s.as_bytes());
    let _ = stdout.flush();
}
