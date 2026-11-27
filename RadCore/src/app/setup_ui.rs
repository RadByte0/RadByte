//! Boxed interactive first-run setup UI for `RadProxy` and `RadLimbo` (with Docker/Pelican env support).

use std::io::{Read, Write};
use std::path::Path;

use crate::auto_update::{
    AutoUpdateSettings, MAX_AUTO_UPDATE_INTERVAL_MINUTES, MIN_AUTO_UPDATE_INTERVAL_MINUTES,
    load_auto_update_settings, save_auto_update_settings,
};
use crate::console_ui::{RawTerminalGuard, is_interactive_tty, terminal_size};

const FORWARDING_MODES: [&str; 4] = ["none", "modern", "bungeeguard", "legacy"];
const INTERVAL_PRESETS: [u64; 8] = [1, 5, 10, 15, 30, 60, 120, 1440];

/// Reusable expandable arrow-key menu for granular auto-update settings.
pub struct AutoUpdateMenu {
    /// Whether the detailed submenu is expanded.
    pub open: bool,
    /// Whether cursor focus is currently inside the submenu rows.
    pub in_sub: bool,
    /// Active submenu row index.
    pub sub_idx: usize,
    /// Whether the interval input field is in digit-editing mode.
    pub editing_interval: bool,
    /// Draft buffer while editing interval minutes.
    pub interval_buf: String,
    /// Current auto-update settings state.
    pub settings: AutoUpdateSettings,
    /// Whether to show the `Update RadAPI` toggle.
    pub include_radapi: bool,
    /// Whether to show the `Update versions` toggle.
    pub include_versions: bool,
}

impl AutoUpdateMenu {
    /// Creates an `AutoUpdateMenu` initialized from `cfg_path`.
    #[must_use]
    pub fn load(cfg_path: &Path, include_radapi: bool, include_versions: bool) -> Self {
        let settings = load_auto_update_settings(cfg_path);
        Self {
            open: false,
            in_sub: false,
            sub_idx: 0,
            editing_interval: false,
            interval_buf: settings.interval_minutes.to_string(),
            settings,
            include_radapi,
            include_versions,
        }
    }

    fn item_count(&self) -> usize {
        3 + usize::from(self.include_radapi) + usize::from(self.include_versions)
    }

    /// Renders the main Auto-update row.
    #[must_use]
    pub fn main_row(&self, focused: bool) -> String {
        let arrow = if self.open { "▲" } else { "▼" };
        format!(
            "{} \x1b[1;37mAuto-update:\x1b[0m       {}  [\x1b[38;2;232;125;13m{arrow}\x1b[0m] \x1b[90m({}m, → details)\x1b[0m",
            marker(focused && !self.in_sub),
            checkbox(self.settings.enabled),
            self.settings.interval_minutes
        )
    }

    /// Appends the expanded sub-rows into `out` using `line_fn`.
    pub fn append_sub_rows(
        &self,
        out: &mut String,
        focused: bool,
        mut line_fn: impl FnMut(&str) -> String,
    ) {
        if !self.open {
            return;
        }
        let sub_mark = |idx: usize| marker(focused && self.in_sub && self.sub_idx == idx);
        let interval_disp = if self.editing_interval {
            format!("\x1b[38;2;232;125;13m{}_\x1b[0m min", self.interval_buf)
        } else {
            format!(
                "\x1b[38;2;232;125;13m{} min\x1b[0m \x1b[90m(←/→ step, Enter edit)\x1b[0m",
                self.settings.interval_minutes
            )
        };
        out.push_str(&line_fn(&format!(
            "   {} \x1b[1;37mCheck interval:\x1b[0m    {interval_disp}",
            sub_mark(0)
        )));
        out.push_str(&line_fn(&format!(
            "   {} \x1b[1;37mUpdate launcher:\x1b[0m   {}",
            sub_mark(1),
            checkbox(self.settings.launcher)
        )));
        out.push_str(&line_fn(&format!(
            "   {} \x1b[1;37mCheck Radons (GH):\x1b[0m {}",
            sub_mark(2),
            checkbox(self.settings.radons)
        )));
        let mut idx = 3usize;
        if self.include_radapi {
            out.push_str(&line_fn(&format!(
                "   {} \x1b[1;37mUpdate RadAPI:\x1b[0m     {}",
                sub_mark(idx),
                checkbox(self.settings.radapi)
            )));
            idx += 1;
        }
        if self.include_versions {
            out.push_str(&line_fn(&format!(
                "   {} \x1b[1;37mUpdate versions:\x1b[0m   {}",
                sub_mark(idx),
                checkbox(self.settings.versions)
            )));
        }
    }

    /// Handles a key while editing the numeric interval. Returns `true` if consumed.
    pub fn handle_interval_key(&mut self, b: u8) -> bool {
        if !self.editing_interval {
            return false;
        }
        match b {
            0x1b | b'\r' | b'\n' => {
                if let Ok(v) = self.interval_buf.parse::<u64>() {
                    self.settings.interval_minutes = v.clamp(
                        MIN_AUTO_UPDATE_INTERVAL_MINUTES,
                        MAX_AUTO_UPDATE_INTERVAL_MINUTES,
                    );
                }
                self.interval_buf = self.settings.interval_minutes.to_string();
                self.editing_interval = false;
            }
            0x08 | 0x7f => {
                self.interval_buf.pop();
            }
            c if c.is_ascii_digit() && self.interval_buf.len() < 5 => {
                self.interval_buf.push(c as char);
            }
            _ => {}
        }
        true
    }

    /// Handles an escape sequence (`[A`, `[B`, `[C`, `[D`) when focused on this row.
    /// Returns `Some(delta)` if focus should move to previous (`-1`) or next (`1`) main row.
    pub fn handle_esc(&mut self, seq: &[u8]) -> Option<isize> {
        self.editing_interval = false;
        match seq {
            b"[A" => {
                if self.open && self.in_sub {
                    if self.sub_idx == 0 {
                        self.in_sub = false;
                    } else {
                        self.sub_idx -= 1;
                    }
                    None
                } else {
                    Some(-1)
                }
            }
            b"[B" => {
                if self.open && !self.in_sub {
                    self.in_sub = true;
                    self.sub_idx = 0;
                    None
                } else if self.open && self.in_sub {
                    if self.sub_idx + 1 < self.item_count() {
                        self.sub_idx += 1;
                        None
                    } else {
                        self.in_sub = false;
                        Some(1)
                    }
                } else {
                    Some(1)
                }
            }
            b"[C" => {
                if self.in_sub && self.sub_idx == 0 {
                    self.step_interval(1);
                } else {
                    self.open = true;
                    self.in_sub = true;
                    self.sub_idx = 0;
                }
                None
            }
            b"[D" => {
                if self.in_sub && self.sub_idx == 0 {
                    self.step_interval(-1);
                } else {
                    self.in_sub = false;
                    self.open = false;
                }
                None
            }
            _ => None,
        }
    }

    /// Handles Space/Enter when focused on this row or one of its sub-rows.
    pub fn handle_activate(&mut self) {
        if !self.in_sub {
            self.settings.enabled = !self.settings.enabled;
            return;
        }
        match self.sub_idx {
            0 => {
                self.interval_buf = self.settings.interval_minutes.to_string();
                self.editing_interval = true;
            }
            1 => self.settings.launcher = !self.settings.launcher,
            2 => self.settings.radons = !self.settings.radons,
            3 if self.include_radapi => self.settings.radapi = !self.settings.radapi,
            idx if self.include_versions && idx == 3 + usize::from(self.include_radapi) => {
                self.settings.versions = !self.settings.versions;
            }
            _ => {}
        }
    }

    fn step_interval(&mut self, dir: isize) {
        let cur = self.settings.interval_minutes;
        let next = if dir > 0 {
            INTERVAL_PRESETS
                .iter()
                .copied()
                .find(|&p| p > cur)
                .unwrap_or(MAX_AUTO_UPDATE_INTERVAL_MINUTES)
        } else {
            INTERVAL_PRESETS
                .iter()
                .rev()
                .copied()
                .find(|&p| p < cur)
                .unwrap_or(MIN_AUTO_UPDATE_INTERVAL_MINUTES)
        };
        self.settings.interval_minutes = next;
        self.interval_buf = next.to_string();
    }
}

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
    let mut auto_menu = AutoUpdateMenu::load(cfg_path, true, false);
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
        let width = usize::from(size.0).clamp(56, 92);
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
            auto_menu.main_row(focused == 4),
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
            if i == 4 {
                auto_menu.append_sub_rows(&mut out, focused == 4, |l| box_line(l, inner));
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
                if focused == 4 {
                    if let Some(delta) = auto_menu.handle_esc(esc.as_slice()) {
                        focused = if delta < 0 { 3 } else { 5 };
                    }
                } else {
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
                }
                esc.clear();
            }
            continue;
        }
        if auto_menu.handle_interval_key(b) {
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
            b'\t' => {
                auto_menu.in_sub = false;
                focused = (focused + 1) % 6;
            }
            b' ' | b'\r' | b'\n' => match focused {
                0..=2 => editing = Some(focused),
                3 => fwd_open = !fwd_open,
                4 => auto_menu.handle_activate(),
                5 => {
                    write_initial_properties(
                        cfg_path,
                        ip.trim(),
                        port.trim(),
                        Some(backend.trim()),
                        FORWARDING_MODES[fwd_idx],
                        &auto_menu.settings,
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
    let mut auto_menu = AutoUpdateMenu::load(cfg_path, false, false);
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
        let width = usize::from(size.0).clamp(56, 92);
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
            auto_menu.main_row(focused == 3),
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
            if i == 3 {
                auto_menu.append_sub_rows(&mut out, focused == 3, |l| box_line(l, inner));
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
                if focused == 3 {
                    if let Some(delta) = auto_menu.handle_esc(esc.as_slice()) {
                        focused = if delta < 0 { 2 } else { 4 };
                    }
                } else {
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
                }
                esc.clear();
            }
            continue;
        }
        if auto_menu.handle_interval_key(b) {
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
            b'\t' => {
                auto_menu.in_sub = false;
                focused = (focused + 1) % 5;
            }
            b' ' | b'\r' | b'\n' => match focused {
                0 | 1 => editing = Some(focused),
                2 => fwd_open = !fwd_open,
                3 => auto_menu.handle_activate(),
                4 => {
                    write_initial_properties(
                        cfg_path,
                        ip.trim(),
                        port.trim(),
                        None,
                        FORWARDING_MODES[fwd_idx],
                        &auto_menu.settings,
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
    let auto_settings = load_auto_update_settings(cfg_path);
    write_initial_properties(
        cfg_path,
        &ip,
        &port,
        backend.as_deref(),
        &fwd,
        &auto_settings,
    )
}

fn write_initial_properties(
    cfg_path: &Path,
    ip: &str,
    port: &str,
    backend: Option<&str>,
    forwarding: &str,
    auto_settings: &AutoUpdateSettings,
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
    save_auto_update_settings(cfg_path, auto_settings).map_err(|e| e.to_string())
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
