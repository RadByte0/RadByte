//! Antigravity-style cross-platform alternate-screen console TUI with internal scrolling and pinned input bar.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cli::enable_ansi_colors;

const TUI_LOG_ADDR_ENV: &str = "RADBYTE_TUI_LOG_ADDR";
const TUI_CMD_ADDR_ENV: &str = "RADBYTE_TUI_CMD_ADDR";
const MAX_LOG_LINES: usize = 1000;

fn is_truthy_env(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

fn is_container_or_panel() -> bool {
    if std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--nogui" || arg == "nogui")
    {
        return true;
    }
    for var in ["CI", "RADBYTE_NO_TUI", "RADBYTE_NOGUI", "NOGUI"] {
        if is_truthy_env(var) {
            return true;
        }
    }
    for panel_var in [
        "P_SERVER_UUID",
        "PELICAN_SERVER_UUID",
        "PTERODACTYL_SERVER_UUID",
        "P_SERVER_LOCATION",
    ] {
        if std::env::var_os(panel_var).is_some() {
            return true;
        }
    }
    #[cfg(unix)]
    {
        std::path::Path::new("/.dockerenv").exists()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Returns `true` when both stdin and stdout are interactive terminals and not inside a panel/container.
#[must_use]
pub fn is_interactive_tty() -> bool {
    if is_container_or_panel() {
        return false;
    }
    std::io::stdout().is_terminal() && std::io::stdin().is_terminal()
}

/// Returns `(cols, rows)` of the current terminal, defaulting to `(80, 24)`.
#[must_use]
pub fn terminal_size() -> (u16, u16) {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Coord {
            x: i16,
            y: i16,
        }
        #[repr(C)]
        #[derive(Default)]
        struct SmallRect {
            left: i16,
            top: i16,
            right: i16,
            bottom: i16,
        }
        #[repr(C)]
        #[derive(Default)]
        struct ConsoleScreenBufferInfo {
            size: Coord,
            cursor: Coord,
            attrs: u16,
            window: SmallRect,
            max_window_size: Coord,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetStdHandle(std_handle: u32) -> isize;
            fn GetConsoleScreenBufferInfo(
                console_output: isize,
                info: *mut ConsoleScreenBufferInfo,
            ) -> i32;
        }
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if handle != 0 && handle != -1 {
            let mut info = ConsoleScreenBufferInfo::default();
            if unsafe { GetConsoleScreenBufferInfo(handle, &raw mut info) } != 0 {
                let cols = (info.window.right - info.window.left + 1).max(40) as u16;
                let rows = (info.window.bottom - info.window.top + 1).max(12) as u16;
                return (cols, rows);
            }
        }
    }
    #[cfg(unix)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct WinSize {
            ws_row: u16,
            ws_col: u16,
            ws_xpixel: u16,
            ws_ypixel: u16,
        }
        unsafe extern "C" {
            fn ioctl(fd: i32, request: usize, ...) -> i32;
        }
        #[cfg(target_os = "linux")]
        const TIOCGWINSZ: usize = 0x5413;
        #[cfg(target_os = "macos")]
        const TIOCGWINSZ: usize = 0x4008_7468;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        const TIOCGWINSZ: usize = 0x5413;

        let mut ws = WinSize::default();
        if unsafe { ioctl(1, TIOCGWINSZ, &raw mut ws) } == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            return (ws.ws_col.max(40), ws.ws_row.max(12));
        }
    }
    (80, 24)
}

/// Cross-platform RAII guard for raw/non-canonical terminal input mode.
pub struct RawTerminalGuard {
    #[cfg(windows)]
    stdin_handle: isize,
    #[cfg(windows)]
    orig_mode: u32,
    #[cfg(unix)]
    saved_stty: Option<String>,
}

impl RawTerminalGuard {
    /// Enters raw/non-canonical input mode and restores the original mode on drop.
    #[must_use]
    pub fn enter() -> Self {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetStdHandle(std_handle: u32) -> isize;
                fn GetConsoleMode(console_handle: isize, mode: *mut u32) -> i32;
                fn SetConsoleMode(console_handle: isize, mode: u32) -> i32;
            }
            const STD_INPUT_HANDLE: u32 = -10i32 as u32;
            const ENABLE_ECHO_INPUT: u32 = 0x0004;
            const ENABLE_LINE_INPUT: u32 = 0x0002;
            const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
            let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
            let mut orig_mode = 0u32;
            if handle != 0
                && handle != -1
                && unsafe { GetConsoleMode(handle, &raw mut orig_mode) } != 0
            {
                let raw = (orig_mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT;
                unsafe {
                    let _ = SetConsoleMode(handle, raw);
                }
            }
            Self {
                stdin_handle: handle,
                orig_mode,
            }
        }
        #[cfg(unix)]
        {
            let saved_stty = std::process::Command::new("stty")
                .arg("-g")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|s| !s.is_empty());
            let _ = std::process::Command::new("stty")
                .args(["-icanon", "-echo", "min", "1", "time", "0"])
                .status();
            Self { saved_stty }
        }
    }
}

impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.stdin_handle != 0 && self.stdin_handle != -1 {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn SetConsoleMode(console_handle: isize, mode: u32) -> i32;
            }
            unsafe {
                let _ = SetConsoleMode(self.stdin_handle, self.orig_mode);
            }
        }
        #[cfg(unix)]
        if let Some(ref mode) = self.saved_stty {
            let _ = std::process::Command::new("stty").arg(mode).status();
        } else {
            let _ = std::process::Command::new("stty")
                .args(["icanon", "echo"])
                .status();
        }
    }
}

struct ProcessMetricsSampler {
    last_wall: std::time::Instant,
    last_cpu_ns: u64,
}

impl ProcessMetricsSampler {
    fn new() -> Self {
        Self {
            last_wall: std::time::Instant::now(),
            last_cpu_ns: read_process_cpu_ns(),
        }
    }

    /// Returns `(cpu_percent_per_thread, ram_megabits)`.
    /// CPU percentage is 100.0% per fully utilized logical thread (can exceed 100% on multi-core).
    fn sample(&mut self) -> (f64, f64) {
        let now = std::time::Instant::now();
        let cpu_ns = read_process_cpu_ns();
        let wall_ns = now.duration_since(self.last_wall).as_nanos();
        let delta_cpu = cpu_ns.saturating_sub(self.last_cpu_ns);
        let cpu_pct = if wall_ns > 0 {
            (delta_cpu as f64 / wall_ns as f64) * 100.0
        } else {
            0.0
        };
        self.last_wall = now;
        self.last_cpu_ns = cpu_ns;
        let ram_mbit = (read_process_rss_bytes() as f64 * 8.0) / 1_000_000.0;
        (cpu_pct, ram_mbit)
    }
}

fn read_process_cpu_ns() -> u64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> isize;
            fn GetProcessTimes(
                process: isize,
                creation: *mut FileTime,
                exit: *mut FileTime,
                kernel: *mut FileTime,
                user: *mut FileTime,
            ) -> i32;
        }
        let mut c = FileTime::default();
        let mut e = FileTime::default();
        let mut k = FileTime::default();
        let mut u = FileTime::default();
        if unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &raw mut c,
                &raw mut e,
                &raw mut k,
                &raw mut u,
            )
        } != 0
        {
            let kt = (u64::from(k.high) << 32) | u64::from(k.low);
            let ut = (u64::from(u.high) << 32) | u64::from(u.low);
            return kt.saturating_add(ut).saturating_mul(100);
        }
        0
    }
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn sysconf(name: i32) -> isize;
        }
        const SC_CLK_TCK: i32 = 2;
        let Ok(raw) = std::fs::read_to_string("/proc/self/stat") else {
            return 0;
        };
        let Some(after_comm) = raw.rfind(')').map(|i| &raw[i + 2..]) else {
            return 0;
        };
        let mut fields = after_comm.split_whitespace();
        let utime: u64 = fields.nth(11).and_then(|s| s.parse().ok()).unwrap_or(0);
        let stime: u64 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let clk_tck = unsafe { sysconf(SC_CLK_TCK) }.max(1) as u64;
        utime.saturating_add(stime).saturating_mul(1_000_000_000) / clk_tck
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        #[repr(C)]
        #[derive(Default)]
        struct TimeVal {
            tv_sec: i64,
            tv_usec: i64,
        }
        #[repr(C)]
        struct RUsage {
            ru_utime: TimeVal,
            ru_stime: TimeVal,
            _rest: [i64; 14],
        }
        unsafe extern "C" {
            fn getrusage(who: i32, usage: *mut RUsage) -> i32;
        }
        let mut ru = unsafe { std::mem::zeroed::<RUsage>() };
        if unsafe { getrusage(0, &raw mut ru) } == 0 {
            let u_ns = (ru.ru_utime.tv_sec.max(0) as u64) * 1_000_000_000
                + (ru.ru_utime.tv_usec.max(0) as u64) * 1_000;
            let s_ns = (ru.ru_stime.tv_sec.max(0) as u64) * 1_000_000_000
                + (ru.ru_stime.tv_usec.max(0) as u64) * 1_000;
            return u_ns.saturating_add(s_ns);
        }
        0
    }
}

fn read_process_rss_bytes() -> u64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(
                process: isize,
                ppsmem_counters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }
        let mut pmc = ProcessMemoryCounters {
            cb: core::mem::size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &raw mut pmc, pmc.cb) } != 0 {
            return pmc.working_set_size as u64;
        }
        0
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(raw) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:")
                && let Some(kb) = rest
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
            {
                return kb.saturating_mul(1024);
            }
        }
        0
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        #[repr(C)]
        #[derive(Default)]
        struct TimeVal {
            tv_sec: i64,
            tv_usec: i64,
        }
        #[repr(C)]
        struct RUsage {
            ru_utime: TimeVal,
            ru_stime: TimeVal,
            ru_maxrss: i64,
            _rest: [i64; 13],
        }
        unsafe extern "C" {
            fn getrusage(who: i32, usage: *mut RUsage) -> i32;
        }
        let mut ru = unsafe { std::mem::zeroed::<RUsage>() };
        if unsafe { getrusage(0, &raw mut ru) } == 0 {
            return ru.ru_maxrss.max(0) as u64;
        }
        0
    }
}

struct TuiState {
    product: String,
    version: String,
    hint: String,
    cpu_pct: f64,
    ram_mbit: f64,
    logs: VecDeque<String>,
    input: String,
    scroll_from_bottom: usize,
    last_size: (u16, u16),
}

impl TuiState {
    fn push_log(&mut self, line: String) {
        if self.logs.len() >= MAX_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
        if self.scroll_from_bottom > 0 {
            self.scroll_from_bottom = self
                .scroll_from_bottom
                .saturating_add(1)
                .min(self.logs.len());
        }
    }

    fn render(&self, clear_screen: bool) {
        let (cols, rows) = self.last_size;
        let width = usize::from(cols);
        let height = usize::from(rows);
        // Layout:
        // rows 1..=6: pixel-art header + divider
        // rows 7..=(height - 4): internal scrollable log viewport
        // rows (height - 3)..=height: top divider, `> input`, bottom divider, hint/label
        let header_rows = 6usize;
        let footer_rows = 4usize;
        let log_height = height.saturating_sub(header_rows + footer_rows).max(1);

        let mut wrapped_logs = Vec::new();
        for entry in &self.logs {
            wrap_ansi_or_plain(entry, width.saturating_sub(1).max(20), &mut wrapped_logs);
        }

        let total_wrapped = wrapped_logs.len();
        let max_scroll = total_wrapped.saturating_sub(log_height);
        let scroll = self.scroll_from_bottom.min(max_scroll);
        let end = total_wrapped.saturating_sub(scroll);
        let start = end.saturating_sub(log_height);
        let visible = &wrapped_logs[start..end];

        let divider = "─".repeat(width);
        let mut out = String::with_capacity(4096);
        if clear_screen {
            out.push_str("\x1b[?25l\x1b[2J\x1b[3J\x1b[H");
        } else {
            out.push_str("\x1b[?25l\x1b[H");
        }

        // Header (6 lines)
        append_pixel_header(
            &mut out,
            &self.product,
            &self.version,
            &self.hint,
            self.cpu_pct,
            self.ram_mbit,
            width,
        );
        out.push_str(&format!("\x1b[2K\x1b[38;2;56;56;56m{divider}\x1b[0m\n"));

        // Log viewport (exact log_height lines)
        for i in 0..log_height {
            out.push_str("\x1b[2K");
            if let Some(line) = visible.get(i) {
                out.push_str(line);
            }
            out.push_str("\x1b[0m\n");
        }

        // Footer (4 lines)
        let label = format!("{} v{}", self.product, self.version);
        let scroll_badge = if scroll > 0 {
            format!("{}  •  ↑{scroll}", self.hint)
        } else {
            self.hint.clone()
        };
        let left_len = scroll_badge.chars().count();
        let right_len = label.chars().count();
        let gap = width.saturating_sub(left_len + right_len + 2).max(1);
        let spaces = " ".repeat(gap);

        out.push_str(&format!("\x1b[2K\x1b[38;2;56;56;56m{divider}\x1b[0m\n"));
        out.push_str(&format!(
            "\x1b[2K\x1b[38;2;232;125;13m>\x1b[0m \x1b[1;37m{}\x1b[0m\n",
            self.input
        ));
        out.push_str(&format!("\x1b[2K\x1b[38;2;56;56;56m{divider}\x1b[0m\n"));
        let status_line = truncate_plain(
            &format!("{scroll_badge}{spaces}"),
            width.saturating_sub(right_len + 1),
        );
        out.push_str(&format!(
            "\x1b[2K \x1b[90m{status_line}\x1b[38;2;160;160;160m{label}\x1b[0m"
        ));

        // Position cursor at `> <input>`
        let input_row = header_rows + log_height + 2;
        let input_col = (3 + self.input.chars().count()).min(width);
        out.push_str(&format!("\x1b[{input_row};{input_col}H\x1b[?25h"));

        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }
}

fn truncate_plain(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn wrap_ansi_or_plain(line: &str, max_cols: usize, out: &mut Vec<String>) {
    let clean = line.trim_end_matches(['\r', '\n']);
    if clean.is_empty() {
        out.push(String::new());
        return;
    }
    // Count visible characters (excluding ANSI CSI sequences) and split cleanly when exceeding max_cols.
    let mut current = String::new();
    let mut vis = 0usize;
    let mut chars = clean.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            current.push(ch);
            if let Some(bracket) = chars.next() {
                current.push(bracket);
            }
            for seq_ch in chars.by_ref() {
                current.push(seq_ch);
                if ('@'..='~').contains(&seq_ch) {
                    break;
                }
            }
            continue;
        }
        if vis >= max_cols {
            current.push_str("\x1b[0m");
            out.push(std::mem::take(&mut current));
            vis = 0;
        }
        current.push(ch);
        vis += 1;
    }
    if !current.is_empty() {
        out.push(current);
    }
}

fn append_pixel_header(
    out: &mut String,
    product: &str,
    version: &str,
    hint: &str,
    cpu_pct: f64,
    ram_mbit: f64,
    _width: usize,
) {
    out.push_str("\x1b[2K  \x1b[38;2;245;158;11m█▀▀█ ▄▀▀▄ █▀▀▄ █▀▀▄ █  █ ▀▀█▀▀ █▀▀▀\x1b[0m\n");
    out.push_str("\x1b[2K  \x1b[38;2;232;125;13m█▄▄▀ █▄▄█ █  █ █▀▀▄ █▄▄█   █   █▀▀ \x1b[0m\n");
    out.push_str("\x1b[2K  \x1b[38;2;180;83;9m█  █ █  █ █▄▄▀ █▄▄▀ ▄▄▄█   █   █▄▄▄\x1b[0m\n");
    out.push_str(&format!(
        "\x1b[2K  \x1b[1;37m{product} CLI\x1b[0m \x1b[38;2;232;125;13mv{version}\x1b[0m  \x1b[90m•  Minecraft {} (protocol {})\x1b[0m\n",
        crate::MINECRAFT_VERSION,
        crate::PROTOCOL_VERSION
    ));
    out.push_str(&format!(
        "\x1b[2K  \x1b[32m●\x1b[0m \x1b[37mOnline\x1b[0m  \x1b[90m•\x1b[0m  \x1b[38;2;232;125;13mCPU {cpu_pct:.1}%\x1b[0m  \x1b[90m•\x1b[0m  \x1b[38;2;245;158;11mRAM {ram_mbit:.1} Mbit\x1b[0m  \x1b[90m•  {hint}  •  Scroll: Mouse/PgUp/PgDn\x1b[0m\n"
    ));
}

/// Active Antigravity-style alternate-screen console session.
pub struct ConsoleUi {
    active: bool,
    stop_flag: Arc<AtomicBool>,
    raw_guard: Option<RawTerminalGuard>,
    handles: Vec<std::thread::JoinHandle<()>>,
}

impl ConsoleUi {
    /// Starts the Antigravity-style alternate-screen TUI in interactive terminals.
    #[must_use]
    pub fn start(product: &str, version: &str, hint: &str) -> Self {
        enable_ansi_colors();
        if !is_interactive_tty() {
            return Self {
                active: false,
                stop_flag: Arc::new(AtomicBool::new(true)),
                raw_guard: None,
                handles: Vec::new(),
            };
        }

        let Ok(log_sock) = UdpSocket::bind("127.0.0.1:0") else {
            return Self {
                active: false,
                stop_flag: Arc::new(AtomicBool::new(true)),
                raw_guard: None,
                handles: Vec::new(),
            };
        };
        let Ok(cmd_listener) = TcpListener::bind("127.0.0.1:0") else {
            return Self {
                active: false,
                stop_flag: Arc::new(AtomicBool::new(true)),
                raw_guard: None,
                handles: Vec::new(),
            };
        };
        let log_addr = log_sock
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();
        let cmd_addr = cmd_listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();

        unsafe {
            std::env::set_var(TUI_LOG_ADDR_ENV, &log_addr);
            std::env::set_var(TUI_CMD_ADDR_ENV, &cmd_addr);
        }

        // Enter alternate screen buffer (disables outer terminal scrollbar) and enable SGR mouse wheel.
        {
            let mut stdout = std::io::stdout().lock();
            let _ = write!(stdout, "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[2J\x1b[H");
            let _ = stdout.flush();
        }

        let raw_guard = Some(RawTerminalGuard::enter());
        let mut sampler = ProcessMetricsSampler::new();
        let (_, init_ram) = sampler.sample();
        let state = Arc::new(Mutex::new(TuiState {
            product: product.to_owned(),
            version: version.to_owned(),
            hint: hint.to_owned(),
            cpu_pct: 0.0,
            ram_mbit: init_ram,
            logs: VecDeque::new(),
            input: String::new(),
            scroll_from_bottom: 0,
            last_size: terminal_size(),
        }));
        state.lock().unwrap().render(true);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let cmd_clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));

        // Thread 1: accept command stream subscribers (RadProxy, RadLimbo, or rad_version_26_3.dll).
        let accept_stop = Arc::clone(&stop_flag);
        let accept_clients = Arc::clone(&cmd_clients);
        let _ = cmd_listener.set_nonblocking(true);
        let accept_handle = std::thread::spawn(move || {
            while !accept_stop.load(Ordering::Relaxed) {
                match cmd_listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nodelay(true);
                        accept_clients.lock().unwrap().push(stream);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });

        // Thread 2: receive log lines from any in-process or cdylib writer + refresh metrics/resize.
        let log_stop = Arc::clone(&stop_flag);
        let log_state = Arc::clone(&state);
        let _ = log_sock.set_read_timeout(Some(Duration::from_millis(200)));
        let log_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut ticks = 0u32;
            while !log_stop.load(Ordering::Relaxed) {
                let mut changed = false;
                let mut resized = false;
                if let Ok((n, _)) = log_sock.recv_from(&mut buf) {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let mut st = log_state.lock().unwrap();
                    for line in text.lines() {
                        st.push_log(line.to_owned());
                    }
                    changed = true;
                }
                ticks = ticks.wrapping_add(1);
                let size = terminal_size();
                let mut st = log_state.lock().unwrap();
                if st.last_size != size {
                    st.last_size = size;
                    changed = true;
                    resized = true;
                }
                if ticks.is_multiple_of(5) {
                    let (cpu_pct, ram_mbit) = sampler.sample();
                    st.cpu_pct = cpu_pct;
                    st.ram_mbit = ram_mbit;
                    changed = true;
                }
                if changed {
                    st.render(resized);
                }
            }
        });

        // Thread 3: read raw keyboard input (`> `, Backspace, Enter, PgUp/PgDn, Up/Down).
        let key_stop = Arc::clone(&stop_flag);
        let key_state = Arc::clone(&state);
        let key_clients = Arc::clone(&cmd_clients);
        let key_handle = std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut byte = [0u8; 1];
            let mut esc_seq = Vec::new();
            let mut in_esc = false;
            while !key_stop.load(Ordering::Relaxed) {
                match stdin.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let b = byte[0];
                        if in_esc {
                            esc_seq.push(b);
                            let done = matches!(b, b'~' | b'A'..=b'Z' | b'a'..=b'z')
                                || esc_seq.len() >= 24;
                            if done {
                                in_esc = false;
                                let mut st = key_state.lock().unwrap();
                                if esc_seq.starts_with(b"[<64;") {
                                    st.scroll_from_bottom = st.scroll_from_bottom.saturating_add(3);
                                    st.render(false);
                                } else if esc_seq.starts_with(b"[<65;") {
                                    st.scroll_from_bottom = st.scroll_from_bottom.saturating_sub(3);
                                    st.render(false);
                                } else {
                                    match esc_seq.as_slice() {
                                        b"[5~" => {
                                            st.scroll_from_bottom =
                                                st.scroll_from_bottom.saturating_add(5);
                                            st.render(false);
                                        }
                                        b"[6~" => {
                                            st.scroll_from_bottom =
                                                st.scroll_from_bottom.saturating_sub(5);
                                            st.render(false);
                                        }
                                        b"[A" if st.input.is_empty() => {
                                            st.scroll_from_bottom =
                                                st.scroll_from_bottom.saturating_add(1);
                                            st.render(false);
                                        }
                                        b"[B" if st.input.is_empty() => {
                                            st.scroll_from_bottom =
                                                st.scroll_from_bottom.saturating_sub(1);
                                            st.render(false);
                                        }
                                        _ => {}
                                    }
                                }
                                esc_seq.clear();
                            }
                            continue;
                        }
                        match b {
                            0x1b => {
                                in_esc = true;
                                esc_seq.clear();
                            }
                            b'\r' | b'\n' => {
                                let cmd = {
                                    let mut st = key_state.lock().unwrap();
                                    let cmd = std::mem::take(&mut st.input);
                                    let trimmed = cmd.trim().to_owned();
                                    if !trimmed.is_empty() {
                                        st.scroll_from_bottom = 0;
                                        st.push_log(format!(
                                            "\x1b[38;2;232;125;13m>\x1b[0m \x1b[1;37m{trimmed}\x1b[0m"
                                        ));
                                    }
                                    st.render(false);
                                    trimmed
                                };
                                if !cmd.is_empty() {
                                    let mut clients = key_clients.lock().unwrap();
                                    clients.retain_mut(|c| {
                                        writeln!(c, "{cmd}").is_ok() && c.flush().is_ok()
                                    });
                                }
                            }
                            0x08 | 0x7f => {
                                let mut st = key_state.lock().unwrap();
                                st.input.pop();
                                st.render(false);
                            }
                            0x03 => {
                                // Ctrl+C: forward `stop` to listeners.
                                let mut clients = key_clients.lock().unwrap();
                                clients.retain_mut(|c| {
                                    writeln!(c, "stop").is_ok() && c.flush().is_ok()
                                });
                            }
                            b if (0x20..0x7f).contains(&b) => {
                                let mut st = key_state.lock().unwrap();
                                st.input.push(b as char);
                                st.render(false);
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Self {
            active: true,
            stop_flag,
            raw_guard,
            handles: vec![accept_handle, log_handle, key_handle],
        }
    }

    /// No-op in alternate-screen mode (command echoing is handled directly on Enter).
    pub fn echo_command(_cmd: &str) {}
}

impl Drop for ConsoleUi {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        drop(self.raw_guard.take());
        if self.active {
            unsafe {
                std::env::remove_var(TUI_LOG_ADDR_ENV);
                std::env::remove_var(TUI_CMD_ADDR_ENV);
            }
            let mut stdout = std::io::stdout().lock();
            let _ = write!(stdout, "\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l\x1b[0m");
            let _ = stdout.flush();
        }
        // Only join background UDP/accept threads; stdin reader may be blocked on ReadConsole.
        for h in self.handles.drain(..self.handles.len().min(2)) {
            let _ = h.join();
        }
    }
}

/// Reads console command lines from the TUI input bar when active, or `stdin` otherwise.
pub fn spawn_command_reader<F>(mut on_line: F)
where
    F: FnMut(String) -> bool + Send + 'static,
{
    if let Ok(addr) = std::env::var(TUI_CMD_ADDR_ENV)
        && let Ok(stream) = TcpStream::connect(addr)
    {
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if !on_line(line) {
                    break;
                }
            }
        });
        return;
    }
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if !on_line(line) {
                break;
            }
        }
    });
}

/// `std::io::Write` adapter for `tracing_subscriber`.
pub struct ConsoleLogWriter {
    buf: Vec<u8>,
    sock: Option<UdpSocket>,
    target: Option<String>,
}

/// Factory function for `tracing_subscriber::fmt().with_writer(make_log_writer)`.
#[must_use]
pub fn make_log_writer() -> ConsoleLogWriter {
    let target = std::env::var(TUI_LOG_ADDR_ENV).ok();
    let sock = target
        .as_ref()
        .and_then(|_| UdpSocket::bind("127.0.0.1:0").ok());
    ConsoleLogWriter {
        buf: Vec::new(),
        sock,
        target,
    }
}

fn compact_log_line(raw: &str) -> String {
    // Shortens `YYYY-MM-DDTHH:MM:SS.xxxxxxZ  INFO ...` into `[HH:MM:SS INFO] ...`.
    if let Some(t_pos) = raw.find('T')
        && t_pos >= 10
        && let Some(z_rel) = raw[t_pos..].find('Z')
        && z_rel >= 9
    {
        let date_slice = &raw[t_pos - 10..t_pos];
        let time_slice = &raw[t_pos + 1..t_pos + 9];
        if date_slice.as_bytes().get(4) == Some(&b'-')
            && date_slice.as_bytes().get(7) == Some(&b'-')
            && time_slice.as_bytes().get(2) == Some(&b':')
            && time_slice.as_bytes().get(5) == Some(&b':')
        {
            let prefix = &raw[..t_pos - 10];
            let suffix = raw[t_pos + z_rel + 1..].trim_start_matches(' ');
            // Collapse `tracing`'s padded ` INFO` / ` WARN` / `ERROR` spacing.
            let cleaned_suffix = suffix.replace("\x1b[0m  ", "\x1b[0m ").replace("  ", " ");
            return format!("{prefix}[{time_slice}]\x1b[0m {cleaned_suffix}");
        }
    }
    raw.to_owned()
}

impl Write for ConsoleLogWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            let compact = compact_log_line(&text);
            if let (Some(sock), Some(target)) = (&self.sock, &self.target) {
                let _ = sock.send_to(compact.as_bytes(), target);
            } else {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(compact.as_bytes());
                let _ = stdout.flush();
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let text = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            let compact = compact_log_line(&text);
            if let (Some(sock), Some(target)) = (&self.sock, &self.target) {
                let _ = sock.send_to(compact.as_bytes(), target);
            } else {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(compact.as_bytes());
                let _ = stdout.flush();
            }
        }
        Ok(())
    }
}

impl Drop for ConsoleLogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
