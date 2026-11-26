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

/// Returns `true` when both stdin and stdout are interactive terminals and TUI is not disabled.
#[must_use]
pub fn is_interactive_tty() -> bool {
    if std::env::var("CI").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        || std::env::var("RADBYTE_NO_TUI").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
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

struct RawTerminalGuard {
    #[cfg(windows)]
    stdin_handle: isize,
    #[cfg(windows)]
    orig_mode: u32,
    #[cfg(unix)]
    orig_termios: [u8; 64],
    #[cfg(unix)]
    termios_saved: bool,
}

impl RawTerminalGuard {
    fn enter() -> Self {
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
            unsafe extern "C" {
                fn tcgetattr(fd: i32, termios: *mut u8) -> i32;
                fn tcsetattr(fd: i32, optional_actions: i32, termios: *const u8) -> i32;
            }
            let mut orig = [0u8; 64];
            let ok = unsafe { tcgetattr(0, orig.as_mut_ptr()) } == 0;
            if ok {
                // Disable ICANON and ECHO cleanly via stty-compatible raw flags or c_lflag offset is platform-specific;
                // use `stty -icanon -echo` fallback for portability across Linux/macOS/ARM64.
                let _ = std::process::Command::new("stty")
                    .args(["-icanon", "-echo", "min", "1", "time", "0"])
                    .status();
            }
            Self {
                orig_termios: orig,
                termios_saved: ok,
            }
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
        if self.termios_saved {
            unsafe extern "C" {
                fn tcsetattr(fd: i32, optional_actions: i32, termios: *const u8) -> i32;
            }
            unsafe {
                let _ = tcsetattr(0, 0, self.orig_termios.as_ptr());
            }
        }
    }
}

struct TuiState {
    product: String,
    version: String,
    hint: String,
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

    fn render(&self) {
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
        out.push_str("\x1b[?25l\x1b[H");

        // Header (6 lines)
        append_pixel_header(&mut out, &self.product, &self.version, &self.hint, width);
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
        out.push_str(&format!(
            "\x1b[2K \x1b[90m{scroll_badge}{spaces}\x1b[38;2;160;160;160m{label}\x1b[0m"
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

fn append_pixel_header(out: &mut String, product: &str, version: &str, hint: &str, _width: usize) {
    let rows = [
        (
            "\x1b[38;2;232;125;13m██████╗ \x1b[0m",
            format!("\x1b[1;37m{product} CLI\x1b[0m \x1b[38;2;232;125;13mv{version}\x1b[0m"),
        ),
        (
            "\x1b[38;2;245;158;11m██╔══██╗\x1b[0m",
            format!(
                "\x1b[90mMinecraft {} (protocol {})\x1b[0m",
                crate::MINECRAFT_VERSION,
                crate::PROTOCOL_VERSION
            ),
        ),
        (
            "\x1b[38;2;59;130;246m██████╔╝\x1b[0m",
            format!("\x1b[32m●\x1b[0m \x1b[37mOnline\x1b[0m  \x1b[90m•  {hint}\x1b[0m"),
        ),
        (
            "\x1b[38;2;99;102;241m██╔══██╗\x1b[0m",
            "\x1b[90mPgUp/PgDn: scroll logs  •  Enter: run command\x1b[0m".to_owned(),
        ),
        ("\x1b[38;2;139;92;246m██║  ██║\x1b[0m", String::new()),
    ];
    for (art, info) in rows {
        if info.is_empty() {
            out.push_str(&format!("\x1b[2K  {art}\n"));
        } else {
            out.push_str(&format!("\x1b[2K  {art}   {info}\n"));
        }
    }
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

        // Enter alternate screen buffer (disables outer terminal scrollbar).
        {
            let mut stdout = std::io::stdout().lock();
            let _ = write!(stdout, "\x1b[?1049h\x1b[2J\x1b[H");
            let _ = stdout.flush();
        }

        let raw_guard = Some(RawTerminalGuard::enter());
        let state = Arc::new(Mutex::new(TuiState {
            product: product.to_owned(),
            version: version.to_owned(),
            hint: hint.to_owned(),
            logs: VecDeque::new(),
            input: String::new(),
            scroll_from_bottom: 0,
            last_size: terminal_size(),
        }));
        state.lock().unwrap().render();

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

        // Thread 2: receive log lines from any in-process or cdylib writer + detect resize.
        let log_stop = Arc::clone(&stop_flag);
        let log_state = Arc::clone(&state);
        let _ = log_sock.set_read_timeout(Some(Duration::from_millis(200)));
        let log_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while !log_stop.load(Ordering::Relaxed) {
                let mut changed = false;
                if let Ok((n, _)) = log_sock.recv_from(&mut buf) {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let mut st = log_state.lock().unwrap();
                    for line in text.lines() {
                        st.push_log(line.to_owned());
                    }
                    changed = true;
                }
                let size = terminal_size();
                let mut st = log_state.lock().unwrap();
                if st.last_size != size {
                    st.last_size = size;
                    changed = true;
                }
                if changed {
                    st.render();
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
                            let done =
                                matches!(b, b'~' | b'A'..=b'Z' | b'a'..=b'z') || esc_seq.len() >= 6;
                            if done {
                                in_esc = false;
                                let mut st = key_state.lock().unwrap();
                                match esc_seq.as_slice() {
                                    b"[5~" => {
                                        st.scroll_from_bottom =
                                            st.scroll_from_bottom.saturating_add(5);
                                        st.render();
                                    }
                                    b"[6~" => {
                                        st.scroll_from_bottom =
                                            st.scroll_from_bottom.saturating_sub(5);
                                        st.render();
                                    }
                                    b"[A" if st.input.is_empty() => {
                                        st.scroll_from_bottom =
                                            st.scroll_from_bottom.saturating_add(1);
                                        st.render();
                                    }
                                    b"[B" if st.input.is_empty() => {
                                        st.scroll_from_bottom =
                                            st.scroll_from_bottom.saturating_sub(1);
                                        st.render();
                                    }
                                    _ => {}
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
                                    st.render();
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
                                st.render();
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
                                st.render();
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
            let _ = write!(stdout, "\x1b[?25h\x1b[?1049l\x1b[0m");
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

impl Write for ConsoleLogWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let (Some(sock), Some(target)) = (&self.sock, &self.target) else {
            return std::io::stdout().write(data);
        };
        self.buf.extend_from_slice(data);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let _ = sock.send_to(&line, target);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let (Some(sock), Some(target)) = (&self.sock, &self.target) else {
            return std::io::stdout().flush();
        };
        if !self.buf.is_empty() {
            let _ = sock.send_to(&self.buf, target);
            self.buf.clear();
        }
        Ok(())
    }
}

impl Drop for ConsoleLogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
