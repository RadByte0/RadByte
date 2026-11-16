//! Windows-native optional server control window. The server remains the same EXE.
mod version_picker;
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

type H = isize;
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
struct MinMaxInfo {
    reserved: Point,
    max_size: Point,
    max_position: Point,
    min_track_size: Point,
    max_track_size: Point,
}
#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
#[derive(Default)]
struct ScrollInfo {
    size: u32,
    mask: u32,
    min: i32,
    max: i32,
    page: u32,
    pos: i32,
    track_pos: i32,
}
#[repr(C)]
struct Msg {
    hwnd: H,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
    private: u32,
}
#[repr(C)]
struct Paint {
    dc: H,
    erase: i32,
    rect: Rect,
    restore: i32,
    inc: i32,
    reserved: [u8; 32],
}
#[repr(C)]
struct DrawItem {
    control_type: u32,
    control_id: u32,
    item_id: u32,
    item_action: u32,
    item_state: u32,
    hwnd: H,
    dc: H,
    rect: Rect,
    item_data: usize,
}
#[repr(C)]
struct WndClass {
    style: u32,
    proc: Option<unsafe extern "system" fn(H, u32, usize, isize) -> isize>,
    extra: i32,
    wnd_extra: i32,
    instance: H,
    icon: H,
    cursor: H,
    background: H,
    menu: *const u16,
    name: *const u16,
}
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[repr(C)]
#[derive(Default)]
struct Mem {
    cb: u32,
    faults: u32,
    peak: usize,
    working: usize,
    paged_peak: usize,
    paged: usize,
    nonpaged_peak: usize,
    nonpaged: usize,
    pagefile: usize,
    pagefile_peak: usize,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(c: *const WndClass) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: H,
        menu: H,
        inst: H,
        param: *const core::ffi::c_void,
    ) -> H;
    fn DefWindowProcW(h: H, m: u32, w: usize, l: isize) -> isize;
    fn ShowWindow(h: H, cmd: i32) -> i32;
    fn UpdateWindow(h: H) -> i32;
    fn GetMessageW(m: *mut Msg, h: H, min: u32, max: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn PostQuitMessage(code: i32);
    fn DestroyWindow(h: H) -> i32;
    fn SetTimer(h: H, id: usize, ms: u32, cb: usize) -> usize;
    fn InvalidateRect(h: H, r: *const Rect, erase: i32) -> i32;
    fn BeginPaint(h: H, p: *mut Paint) -> H;
    fn EndPaint(h: H, p: *const Paint) -> i32;
    fn GetClientRect(h: H, r: *mut Rect) -> i32;
    fn GetScrollInfo(h: H, bar: i32, info: *mut ScrollInfo) -> i32;
    fn MoveWindow(h: H, x: i32, y: i32, w: i32, ht: i32, repaint: i32) -> i32;
    fn SendMessageW(h: H, m: u32, w: usize, l: isize) -> isize;
    fn GetWindowTextW(h: H, buf: *mut u16, max: i32) -> i32;
    fn SetWindowTextW(h: H, s: *const u16) -> i32;
    fn LoadCursorW(h: H, name: *const u16) -> H;
    fn FillRect(dc: H, rect: *const Rect, brush: H) -> i32;
    fn DrawTextW(dc: H, text: *const u16, len: i32, rect: *mut Rect, format: u32) -> i32;
    fn GetParent(window: H) -> H;
}
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(
        window: H,
        callback: Option<unsafe extern "system" fn(H, u32, usize, isize, usize, usize) -> isize>,
        id: usize,
        data: usize,
    ) -> i32;
    fn DefSubclassProc(window: H, msg: u32, w: usize, l: isize) -> isize;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreatePen(style: i32, width: i32, color: u32) -> H;
    fn SelectObject(dc: H, obj: H) -> H;
    fn DeleteObject(obj: H) -> i32;
    fn DeleteDC(dc: H) -> i32;
    fn CreateCompatibleDC(dc: H) -> H;
    fn CreateCompatibleBitmap(dc: H, w: i32, h: i32) -> H;
    fn BitBlt(dst: H, x: i32, y: i32, w: i32, h: i32, src: H, sx: i32, sy: i32, rop: u32) -> i32;
    fn MoveToEx(dc: H, x: i32, y: i32, old: *mut Point) -> i32;
    fn LineTo(dc: H, x: i32, y: i32) -> i32;
    fn CreateSolidBrush(color: u32) -> H;
    fn RoundRect(
        dc: H,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        width: i32,
        height: i32,
    ) -> i32;
    fn GetStockObject(index: i32) -> H;
    fn SetTextColor(dc: H, color: u32) -> u32;
    fn SetBkColor(dc: H, color: u32) -> u32;
    fn SetBkMode(dc: H, mode: i32) -> i32;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> H;
}
#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        window: H,
        attribute: u32,
        value: *const core::ffi::c_void,
        size: u32,
    ) -> i32;
}
#[link(name = "uxtheme")]
unsafe extern "system" {
    fn SetWindowTheme(window: H, app_name: *const u16, id_list: *const u16) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> H;
    fn GetConsoleWindow() -> H;
    fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    fn FreeConsole() -> i32;
    fn GetProcessTimes(
        h: H,
        c: *mut FileTime,
        e: *mut FileTime,
        k: *mut FileTime,
        u: *mut FileTime,
    ) -> i32;
}
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(h: H, mem: *mut Mem, size: u32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
struct App {
    child: Child,
    logs: Arc<Mutex<VecDeque<String>>>,
    log: H,
    input: H,
    button: H,
    clear_button: H,
    detail_button: H,
    save_button: H,
    stop_button: H,
    info: H,
    history: VecDeque<String>,
    cmd_history: VecDeque<String>,
    cmd_index: Option<usize>,
    cpu: VecDeque<f32>,
    ram: VecDeque<f32>,
    last_cpu: u64,
    last_time: Instant,
    started: Instant,
    closing: bool,
    port_conflict: bool,
    configured_address: Option<String>,
    online_players: usize,
    game_version: Option<String>,
}
static APP: OnceLock<Mutex<App>> = OnceLock::new();
// Background, control background and monospaced font live until process exit.
static THEME: OnceLock<(H, H, H)> = OnceLock::new();
// Native UI typography is separate from the monospaced console.
static UI_FONTS: OnceLock<(H, H, H)> = OnceLock::new();
static FULL_LOG: AtomicBool = AtomicBool::new(false);

/// Compact only structured informational lines; warnings and errors retain all details.
fn display_log_line(line: &str, full: bool) -> Option<String> {
    if full {
        return Some(line.to_owned());
    }
    if line.starts_with("  ____") || line.starts_with(" |") || line.trim().is_empty() {
        return None;
    }
    let mut words = line.split_whitespace();
    let timestamp = words.next()?;
    let Some(level) = words.next() else {
        return Some(line.to_owned());
    };
    if level != "INFO" || !timestamp.ends_with('Z') {
        return Some(line.to_owned());
    }
    let Some(time) = timestamp.get(11..19) else {
        return Some(line.to_owned());
    };
    let Some((_, message)) = line.split_once(": ") else {
        return Some(line.to_owned());
    };
    let mut kept = Vec::new();
    let mut omitted = false;
    for word in message.split_whitespace() {
        if word.contains('=') {
            omitted = true;
            break;
        }
        kept.push(word);
    }
    if kept.is_empty() {
        return Some(line.to_owned());
    }
    let text = kept.join(" ");
    let address = if text.contains("listening") {
        message
            .split_whitespace()
            .find_map(|word| word.strip_prefix("bind_addr="))
    } else if text.contains("player joined") || text.contains("player left") {
        message
            .split_whitespace()
            .find_map(|word| word.strip_prefix("username="))
    } else {
        None
    };
    Some(format!(
        "[{time}]  INFO  {text}{}{}",
        address.map_or_else(String::new, |value| format!("  |  {value}")),
        if omitted { "  ..." } else { "" }
    ))
}

fn cpu_chart_scale(values: &VecDeque<f32>) -> f32 {
    let peak = values.iter().copied().fold(0.0_f32, f32::max);
    [1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0]
        .into_iter()
        .find(|&limit| peak <= limit)
        .unwrap_or(100.0)
}

struct Layout {
    main_width: i32,
    side_x: i32,
    side_width: i32,
    input_y: i32,
}

fn layout(width: i32, height: i32) -> Layout {
    let width = width.max(760);
    let side_width = (width / 3).clamp(230, 300);
    let side_x = width - 24 - side_width;
    Layout {
        main_width: (side_x - 44).max(300),
        side_x,
        side_width,
        input_y: (height - 56).max(350),
    }
}

struct ResourceLayout {
    chart_height: i32,
    details_top: Option<i32>,
}

fn resource_layout(input_y: i32) -> ResourceLayout {
    let available = (input_y - 12 - 132).max(282);
    let show_details = available >= 390;
    let details_and_gaps = if show_details { 120 } else { 12 };
    let chart_height = ((available - details_and_gaps) / 2).clamp(130, 200);
    ResourceLayout {
        chart_height,
        details_top: show_details.then_some(132 + chart_height * 2 + 24),
    }
}

fn configured_address(line: &str) -> Option<String> {
    if !line.contains("RadServer listening") {
        return None;
    }
    let value = line
        .split_whitespace()
        .find_map(|word| word.strip_prefix("bind_addr="))?;
    let value = value.trim_matches('"');
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_version_info(line: &str) -> Option<String> {
    if !line.contains("loaded native Minecraft version library") {
        return None;
    }
    let ver = line
        .split_whitespace()
        .find_map(|w| w.strip_prefix("game_version="))?
        .trim_matches('"');
    let proto = line
        .split_whitespace()
        .find_map(|w| w.strip_prefix("protocol="))?;
    Some(format!("MC {ver} (proto {proto})"))
}

unsafe extern "system" fn input_subclass(
    window: H,
    msg: u32,
    w: usize,
    l: isize,
    _id: usize,
    _data: usize,
) -> isize {
    // Enter submits; Up/Down arrows navigate command history.
    if msg == 0x0100 {
        if w == 13 {
            unsafe {
                SendMessageW(GetParent(window), 0x0111, 103, 0);
            }
            return 0;
        }
        if w == 38 || w == 40 {
            if let Some(app_lock) = APP.get()
                && let Ok(mut a) = app_lock.lock()
                && !a.cmd_history.is_empty()
            {
                let len = a.cmd_history.len();
                let next_idx = if w == 38 {
                    match a.cmd_index {
                        Some(idx) => Some(idx.saturating_sub(1)),
                        None => Some(len - 1),
                    }
                } else {
                    match a.cmd_index {
                        Some(idx) if idx + 1 < len => Some(idx + 1),
                        _ => None,
                    }
                };
                a.cmd_index = next_idx;
                let text = next_idx
                    .and_then(|i| a.cmd_history.get(i))
                    .map(String::as_str)
                    .unwrap_or("");
                unsafe {
                    SetWindowTextW(window, wide(text).as_ptr());
                    SendMessageW(window, 0x00b1, 0x7fffffff, -1);
                }
            }
            return 0;
        }
    }
    unsafe { DefSubclassProc(window, msg, w, l) }
}

/// Remove terminal control sequences from redirected output before displaying it.
fn strip_terminal_codes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break;
                }
            }
        } else if ch != '\u{1b}' {
            output.push(ch);
        }
    }
    output
}

/// Append without dragging a reader away from older console output.
fn append_console(log: H, text: &str) {
    if text.is_empty() {
        return;
    }
    unsafe {
        let mut scroll = ScrollInfo {
            size: std::mem::size_of::<ScrollInfo>() as u32,
            mask: 0x0001 | 0x0002 | 0x0004,
            ..Default::default()
        };
        let has_scroll = GetScrollInfo(log, 1, &mut scroll) != 0;
        let at_bottom = !has_scroll
            || scroll.page == 0
            || i64::from(scroll.pos) + i64::from(scroll.page) > i64::from(scroll.max);
        let first_line = SendMessageW(log, 0x00ce, 0, 0);
        let mut selection_start = 0_u32;
        let mut selection_end = 0_u32;
        SendMessageW(
            log,
            0x00b0,
            (&mut selection_start as *mut u32) as usize,
            (&mut selection_end as *mut u32) as isize,
        );
        SendMessageW(log, 0x00cf, 0, 0);
        // Keep the log bounded before the control's limit blocks new lines.
        let length = SendMessageW(log, 0x000e, 0, 0).max(0) as usize;
        let incoming = text.encode_utf16().count();
        let mut removed_lines = 0_isize;
        if length.saturating_add(incoming) > 480_000 {
            let cut = SendMessageW(log, 0x00c9, 150_000, 0).max(0) as usize; // EM_LINEFROMCHAR
            let cut_char = SendMessageW(log, 0x00bb, cut, 0); // EM_LINEINDEX
            let cut_char = if cut_char > 0 {
                cut_char as usize
            } else {
                150_000.min(length)
            };
            removed_lines = cut as isize;
            SendMessageW(log, 0x00b1, 0, cut_char as isize);
            SendMessageW(log, 0x00c2, 0, wide("").as_ptr() as isize);
            selection_start = selection_start.saturating_sub(cut_char as u32);
            selection_end = selection_end.saturating_sub(cut_char as u32);
        }
        SendMessageW(log, 0x00b1, 0x7fffffff, -1);
        SendMessageW(log, 0x00c2, 0, wide(text).as_ptr() as isize);
        SendMessageW(log, 0x00cf, 1, 0);
        if at_bottom {
            SendMessageW(log, 0x00b6, 0, 32_767); // EM_LINESCROLL towards end
        } else {
            SendMessageW(
                log,
                0x00b1,
                selection_start as usize,
                selection_end as isize,
            );
            let current = SendMessageW(log, 0x00ce, 0, 0);
            let desired = first_line.saturating_sub(removed_lines).max(0);
            SendMessageW(log, 0x00b6, 0, desired - current); // EM_LINESCROLL
        }
    }
}

fn draw_text(dc: H, text: &str, x: i32, y: i32, width: i32, color: u32) {
    let mut rect = Rect {
        left: x,
        top: y,
        right: x + width.max(1),
        bottom: y + 28,
    };
    unsafe {
        SetTextColor(dc, color);
        SetBkMode(dc, 1);
        DrawTextW(dc, wide(text).as_ptr(), -1, &mut rect, 0x20 | 0x8000);
    }
}

fn rounded_rect(dc: H, bounds: &Rect, brush: H, radius: i32) {
    unsafe {
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, GetStockObject(8));
        RoundRect(
            dc,
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.bottom,
            radius,
            radius,
        );
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
    }
}

fn chart(
    dc: H,
    bounds: Rect,
    title: &str,
    unit: &str,
    values: &VecDeque<f32>,
    scale: f32,
    color: u32,
) {
    let (_, card, _) = *THEME.get().unwrap();
    let (body, _, value_font) = *UI_FONTS.get().unwrap();
    unsafe {
        rounded_rect(dc, &bounds, card, 12);
        let old_font = SelectObject(dc, body);
        let width = bounds.right - bounds.left;
        let current = values.back().copied().unwrap_or_default();
        let peak = values.iter().copied().fold(0.0_f32, f32::max);
        draw_text(
            dc,
            title,
            bounds.left + 14,
            bounds.top + 10,
            width - 28,
            0x00a6a4a0,
        );
        SelectObject(dc, value_font);
        draw_text(
            dc,
            &format!("{current:.1} {unit}"),
            bounds.left + 14,
            bounds.top + 32,
            width / 2,
            color,
        );
        SelectObject(dc, body);
        draw_text(
            dc,
            &format!("Peak {peak:.1} {unit}"),
            bounds.left + width / 2,
            bounds.top + 35,
            width / 2 - 12,
            0x00b2aaa6,
        );
        let plot_left = bounds.left + 56;
        let plot_right = (bounds.right - 14).max(plot_left + 1);
        let plot_top = bounds.top + 73;
        let plot_bottom = bounds.bottom - 31;
        let grid = CreatePen(0, 1, 0x0034302c);
        let old_pen = SelectObject(dc, grid);
        for part in 0..=2 {
            let y = plot_top + (plot_bottom - plot_top) * part / 2;
            MoveToEx(dc, plot_left, y, std::ptr::null_mut());
            LineTo(dc, plot_right, y);
            let label = scale * (2 - part) as f32 / 2.0;
            draw_text(
                dc,
                &if scale <= 2.0 {
                    format!("{label:.1}")
                } else {
                    format!("{label:.0}")
                },
                bounds.left + 8,
                y - 8,
                44,
                0x008b8985,
            );
        }
        SelectObject(dc, old_pen);
        DeleteObject(grid);
        let line = CreatePen(0, 2, color);
        let old_pen = SelectObject(dc, line);
        let start = 120_usize.saturating_sub(values.len());
        for (index, value) in values.iter().enumerate() {
            let x = plot_left + ((start + index) as i32 * (plot_right - plot_left) / 119);
            let y = plot_bottom
                - ((*value / scale).clamp(0.0, 1.0) * (plot_bottom - plot_top) as f32) as i32;
            if index == 0 {
                MoveToEx(dc, x, y, std::ptr::null_mut());
            } else {
                LineTo(dc, x, y);
            }
        }
        SelectObject(dc, old_pen);
        DeleteObject(line);
        draw_text(dc, "-120 s", plot_left, bounds.bottom - 23, 76, 0x008b8985);
        draw_text(
            dc,
            "now",
            (plot_right - 60).max(plot_left + 78),
            bounds.bottom - 23,
            60,
            0x008b8985,
        );
        SelectObject(dc, old_font);
    }
}
fn add_pipe<R: std::io::Read + Send + 'static>(pipe: R, q: Arc<Mutex<VecDeque<String>>>) {
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            let line = strip_terminal_codes(&line);
            if let Ok(mut q) = q.lock() {
                if q.len() >= 1000 {
                    q.pop_front();
                }
                q.push_back(line);
            }
        }
    });
}
fn sample(p: &Child) -> Option<(u64, usize)> {
    unsafe {
        let handle = p.as_raw_handle() as H;
        let (mut c, mut e, mut k, mut u) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        let mut m = Mem {
            cb: std::mem::size_of::<Mem>() as u32,
            ..Default::default()
        };
        if GetProcessTimes(handle, &mut c, &mut e, &mut k, &mut u) == 0
            || GetProcessMemoryInfo(handle, &mut m, m.cb) == 0
        {
            return None;
        }
        let ticks = |v: FileTime| (u64::from(v.high) << 32) | u64::from(v.low);
        Some((ticks(k) + ticks(u), m.working))
    }
}
fn ctrl(parent: H, class: &str, text: &str, style: u32, id: usize, inst: H) -> H {
    unsafe {
        CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            0x40000000 | 0x10000000 | style,
            0,
            0,
            1,
            1,
            parent,
            id as H,
            inst,
            std::ptr::null(),
        )
    }
}
unsafe extern "system" fn window(h: H, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        0x0001 => {
            let inst = unsafe { GetModuleHandleW(std::ptr::null()) };
            let mut a = APP.get().unwrap().lock().unwrap();
            a.info = ctrl(h, "STATIC", "Starting server...", 0, 0, inst);
            a.log = ctrl(
                h,
                "EDIT",
                "",
                0x00200000 | 0x0800 | 0x0004 | 0x1000 | 0x0040,
                101,
                inst,
            );
            // Ask Windows to match the scrollbar to the dark control when supported.
            unsafe {
                SetWindowTheme(a.log, wide("DarkMode_Explorer").as_ptr(), std::ptr::null());
            }
            a.input = ctrl(h, "EDIT", "", 0x00800000 | 0x0080, 102, inst);
            a.button = ctrl(h, "BUTTON", "Send", 0x0000000b, 103, inst);
            a.clear_button = ctrl(h, "BUTTON", "Clear", 0x0000000b, 104, inst);
            a.detail_button = ctrl(h, "BUTTON", "Details", 0x0000000b, 105, inst);
            a.save_button = ctrl(h, "BUTTON", "Save", 0x0000000b, 106, inst);
            a.stop_button = ctrl(h, "BUTTON", "Stop", 0x0000000b, 107, inst);
            let mono = THEME.get().unwrap().2;
            let body = UI_FONTS.get().unwrap().0;
            for control in [a.log, a.input] {
                unsafe {
                    SendMessageW(control, 0x0030, mono as usize, 1);
                }
            }
            for control in [
                a.info,
                a.button,
                a.clear_button,
                a.detail_button,
                a.save_button,
                a.stop_button,
            ] {
                unsafe {
                    SendMessageW(control, 0x0030, body as usize, 1);
                }
            }
            unsafe {
                SetWindowSubclass(a.input, Some(input_subclass), 1, 0);
                SendMessageW(
                    a.input,
                    0x1501,
                    0,
                    wide("Enter a command... (Up/Down for history)").as_ptr() as isize,
                );
            }
            unsafe {
                SendMessageW(a.log, 0x00c5, 500_000, 0);
            }
            unsafe { SetTimer(h, 1, 1000, 0) };
            0
        }
        // Prevent background erase flicker; WM_PAINT double-buffers the entire client area.
        0x0014 => 1,
        // WM_CTLCOLORSTATIC / WM_CTLCOLOREDIT
        0x0138 | 0x0133 => {
            let (background, controls, _) = *THEME.get().unwrap();
            unsafe {
                SetTextColor(w as H, 0x00e5e7eb);
                SetBkColor(
                    w as H,
                    if msg == 0x0133 {
                        0x00242120
                    } else {
                        0x001b1818
                    },
                );
            }
            if msg == 0x0133 { controls } else { background }
        }
        // Owner-drawn dark button.
        0x002b if (103..=107).contains(&w) && l != 0 => {
            let item = unsafe { &mut *(l as *mut DrawItem) };
            let (_, brush, _) = *THEME.get().unwrap();
            let font = UI_FONTS.get().unwrap().0;
            unsafe {
                FillRect(item.dc, &item.rect, THEME.get().unwrap().0);
                rounded_rect(item.dc, &item.rect, brush, 10);
                let old = SelectObject(item.dc, font);
                SetTextColor(item.dc, if w == 107 { 0x008080f5 } else { 0x00e5e7eb });
                SetBkMode(item.dc, 1);
                DrawTextW(
                    item.dc,
                    wide(match w {
                        103 => "Send",
                        104 => "Clear",
                        106 => "Save",
                        107 => "Stop",
                        _ if FULL_LOG.load(Ordering::Relaxed) => "Compact",
                        _ => "Details",
                    })
                    .as_ptr(),
                    -1,
                    &mut item.rect,
                    0x00000001 | 0x00000004 | 0x00000020,
                );
                SelectObject(item.dc, old);
            }
            1
        }
        0x0005 => {
            let a = APP.get().unwrap().lock().unwrap();
            let (width, height) = ((l as u32 & 65535) as i32, ((l as u32 >> 16) & 65535) as i32);
            let pos = layout(width, height);
            unsafe {
                MoveWindow(a.info, (width - 440).max(290), 22, 416, 22, 1);
                MoveWindow(
                    a.log,
                    24,
                    132,
                    pos.main_width,
                    (pos.input_y - 142).max(40),
                    1,
                );
                MoveWindow(
                    a.input,
                    24,
                    pos.input_y,
                    (pos.main_width - 100).max(30),
                    32,
                    1,
                );
                MoveWindow(a.button, 24 + pos.main_width - 88, pos.input_y, 88, 32, 1);
                MoveWindow(a.save_button, 24 + pos.main_width - 324, 96, 66, 27, 1);
                MoveWindow(a.stop_button, 24 + pos.main_width - 252, 96, 66, 27, 1);
                MoveWindow(a.detail_button, 24 + pos.main_width - 180, 96, 84, 27, 1);
                MoveWindow(a.clear_button, 24 + pos.main_width - 90, 96, 90, 27, 1);
            };
            0
        }
        // Keep the charts and console usable when resizing the window.
        0x0024 if l != 0 => {
            let limits = unsafe { &mut *(l as *mut MinMaxInfo) };
            limits.min_track_size.x = 760;
            limits.min_track_size.y = 540;
            0
        }
        0x0111 if (w & 65535) == 103 => {
            let mut a = APP.get().unwrap().lock().unwrap();
            let mut b = [0u16; 2048];
            let n = unsafe { GetWindowTextW(a.input, b.as_mut_ptr(), b.len() as i32) };
            if n > 0 {
                let text = String::from_utf16_lossy(&b[..n as usize]);
                let trimmed = text.trim().to_owned();
                if !trimmed.is_empty() {
                    if a.cmd_history.back() != Some(&trimmed) {
                        if a.cmd_history.len() >= 100 {
                            a.cmd_history.pop_front();
                        }
                        a.cmd_history.push_back(trimmed.clone());
                    }
                    a.cmd_index = None;
                    if let Some(stdin) = a.child.stdin.as_mut() {
                        let _ = writeln!(stdin, "{trimmed}");
                        let _ = stdin.flush();
                    }
                }
                unsafe {
                    SetWindowTextW(a.input, wide("").as_ptr());
                }
            }
            0
        }
        0x0111 if (w & 65535) == 106 => {
            let mut a = APP.get().unwrap().lock().unwrap();
            if let Some(stdin) = a.child.stdin.as_mut() {
                let _ = writeln!(stdin, "save-all");
                let _ = stdin.flush();
            }
            0
        }
        0x0111 if (w & 65535) == 107 => {
            unsafe {
                SendMessageW(h, 0x0010, 0, 0);
            }
            0
        }
        0x0111 if (w & 65535) == 104 => {
            let mut a = APP.get().unwrap().lock().unwrap();
            a.history.clear();
            if let Ok(mut pending) = a.logs.lock() {
                pending.clear();
            }
            unsafe {
                SetWindowTextW(a.log, wide("").as_ptr());
            }
            0
        }
        0x0111 if (w & 65535) == 105 => {
            let a = APP.get().unwrap().lock().unwrap();
            let full = !FULL_LOG.load(Ordering::Relaxed);
            FULL_LOG.store(full, Ordering::Relaxed);
            let display: Vec<_> = a
                .history
                .iter()
                .filter_map(|line| display_log_line(line, full))
                .collect();
            unsafe {
                SetWindowTextW(a.log, wide("").as_ptr());
                InvalidateRect(a.detail_button, std::ptr::null(), 1);
            }
            if !display.is_empty() {
                append_console(a.log, &(display.join("\r\n") + "\r\n"));
            }
            0
        }
        0x0113 => {
            let mut a = APP.get().unwrap().lock().unwrap();
            let logs: Vec<_> = {
                let mut q = a.logs.lock().unwrap();
                q.drain(..).collect()
            };
            let mut display = Vec::with_capacity(logs.len());
            let full = FULL_LOG.load(Ordering::Relaxed);
            for line in logs {
                if line.contains("os error 10048") || line.contains("Address already in use") {
                    a.port_conflict = true;
                }
                if let Some(address) = configured_address(&line) {
                    a.configured_address = Some(address);
                }
                if let Some(ver) = parse_version_info(&line) {
                    a.game_version = Some(ver);
                }
                if line.contains("player joined RadServer") {
                    a.online_players = a.online_players.saturating_add(1);
                } else if line.contains("player left RadServer") {
                    a.online_players = a.online_players.saturating_sub(1);
                }
                if let Some(formatted) = display_log_line(&line, full) {
                    display.push(formatted);
                }
                if a.history.len() >= 5_000 {
                    a.history.pop_front();
                }
                a.history.push_back(line);
            }
            if !display.is_empty() {
                append_console(a.log, &(display.join("\r\n") + "\r\n"));
            }
            if let Some((ticks, bytes)) = sample(&a.child) {
                let now = Instant::now();
                let dt = now.duration_since(a.last_time).as_secs_f64();
                let cpu = if dt > 0.0 && a.last_cpu > 0 {
                    (ticks.saturating_sub(a.last_cpu) as f64
                        / 10_000_000.0
                        / dt
                        / std::thread::available_parallelism().map_or(1, |n| n.get()) as f64
                        * 100.0) as f32
                } else {
                    0.0
                };
                a.last_cpu = ticks;
                a.last_time = now;
                let ram = bytes as f32 / 1048576.0;
                if a.cpu.len() >= 120 {
                    a.cpu.pop_front();
                    a.ram.pop_front();
                }
                a.cpu.push_back(cpu);
                a.ram.push_back(ram);
                let elapsed = a.started.elapsed().as_secs();
                let t = if a.closing {
                    "STOPPING  |  saving world...".to_owned()
                } else {
                    format!(
                        "{} online    ·    {:02}:{:02}:{:02}",
                        a.online_players,
                        elapsed / 3600,
                        (elapsed / 60) % 60,
                        elapsed % 60
                    )
                };
                unsafe {
                    SetWindowTextW(a.info, wide(&t).as_ptr());
                    InvalidateRect(h, std::ptr::null(), 0);
                }
            }
            if let Ok(Some(status)) = a.child.try_wait() {
                let text = if a.port_conflict {
                    "PORT IN USE: another server or application is already using this port"
                        .to_owned()
                } else {
                    format!("Server stopped ({status}). Close this window.")
                };
                unsafe {
                    SetWindowTextW(a.info, wide(&text).as_ptr());
                }
                if a.closing {
                    drop(a);
                    unsafe { DestroyWindow(h) };
                }
            }
            0
        }
        0x000F => {
            let a = APP.get().unwrap().lock().unwrap();
            let mut p: Paint = unsafe { std::mem::zeroed() };
            let screen_dc = unsafe { BeginPaint(h, &mut p) };
            let mut r = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetClientRect(h, &mut r) };
            let w_px = (r.right - r.left).max(1);
            let h_px = (r.bottom - r.top).max(1);
            let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };
            let mem_bmp = if mem_dc != 0 {
                unsafe { CreateCompatibleBitmap(screen_dc, w_px, h_px) }
            } else {
                0
            };
            let (dc, old_bmp) = if mem_dc != 0 && mem_bmp != 0 {
                (mem_dc, unsafe { SelectObject(mem_dc, mem_bmp) })
            } else {
                (screen_dc, 0)
            };

            unsafe {
                FillRect(dc, &r, THEME.get().unwrap().0);
            }
            let pos = layout(r.right, r.bottom);
            let (_, surface, _) = *THEME.get().unwrap();
            let console_frame = Rect {
                left: 18,
                top: 126,
                right: 30 + pos.main_width,
                bottom: pos.input_y - 8,
            };
            rounded_rect(dc, &console_frame, surface, 12);
            let (body, title, _) = *UI_FONTS.get().unwrap();
            unsafe {
                let old = SelectObject(dc, title);
                draw_text(dc, "RadServer", 24, 15, 270, 0x00efedeb);
                SelectObject(dc, body);

                draw_text(dc, "Console", 24, 101, 92, 0x00c4beba);
                draw_text(dc, "Resources", pos.side_x, 101, pos.side_width, 0x00c4beba);
                SelectObject(dc, old);
            }
            let resources = resource_layout(pos.input_y);
            let cpu_bottom = 132 + resources.chart_height;
            let ram_top = cpu_bottom + 12;
            let max_ram = a.ram.iter().copied().fold(16.0_f32, f32::max);
            let ram_scale = max_ram.log2().ceil().exp2();
            chart(
                dc,
                Rect {
                    left: pos.side_x,
                    top: 132,
                    right: pos.side_x + pos.side_width,
                    bottom: cpu_bottom,
                },
                "CPU",
                "%",
                &a.cpu,
                cpu_chart_scale(&a.cpu),
                0x00f2b66d,
            );
            chart(
                dc,
                Rect {
                    left: pos.side_x,
                    top: ram_top,
                    right: pos.side_x + pos.side_width,
                    bottom: ram_top + resources.chart_height,
                },
                "Memory",
                "MiB",
                &a.ram,
                ram_scale,
                0x00a8df9b,
            );
            if let Some(top) = resources.details_top {
                let card = Rect {
                    left: pos.side_x,
                    top,
                    right: pos.side_x + pos.side_width,
                    bottom: top + 96,
                };
                let (_, brush, _) = *THEME.get().unwrap();
                let (body, _, _) = *UI_FONTS.get().unwrap();
                let elapsed = a.started.elapsed().as_secs();
                let players_ver = format!(
                    "{} players  ·  {}",
                    a.online_players,
                    a.game_version.as_deref().unwrap_or("MC 26.3")
                );
                let uptime = format!(
                    "Uptime  {:02}:{:02}:{:02}",
                    elapsed / 3600,
                    (elapsed / 60) % 60,
                    elapsed % 60
                );
                let address = format!(
                    "Address  {}",
                    a.configured_address.as_deref().unwrap_or("not reported")
                );
                unsafe {
                    rounded_rect(dc, &card, brush, 12);
                    let old = SelectObject(dc, body);
                    draw_text(
                        dc,
                        &players_ver,
                        card.left + 14,
                        top + 10,
                        pos.side_width - 28,
                        0x00a8df9b,
                    );
                    draw_text(
                        dc,
                        &uptime,
                        card.left + 14,
                        top + 37,
                        pos.side_width - 28,
                        0x00c4beba,
                    );
                    draw_text(
                        dc,
                        &address,
                        card.left + 14,
                        top + 64,
                        pos.side_width - 28,
                        0x00c4beba,
                    );
                    SelectObject(dc, old);
                }
            }
            if dc == mem_dc {
                unsafe {
                    BitBlt(screen_dc, 0, 0, w_px, h_px, mem_dc, 0, 0, 0x00CC0020);
                    SelectObject(mem_dc, old_bmp);
                    DeleteObject(mem_bmp);
                    DeleteDC(mem_dc);
                }
            }
            unsafe { EndPaint(h, &p) };
            0
        }
        0x0010 => {
            let mut a = APP.get().unwrap().lock().unwrap();
            if a.child.try_wait().ok().flatten().is_some() {
                drop(a);
                unsafe { DestroyWindow(h) };
            } else if !a.closing {
                a.closing = true;
                if let Some(stdin) = a.child.stdin.as_mut() {
                    let _ = writeln!(stdin, "stop");
                    let _ = stdin.flush();
                }
                unsafe {
                    SetWindowTextW(a.info, wide("Stopping server and saving world...").as_ptr());
                }
            }
            0
        }
        0x0002 => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(h, msg, w, l) },
    }
}

pub fn run() -> Result<i32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if std::env::args_os().len() == 1
        && let Some(dir) = exe.parent()
    {
        std::env::set_current_dir(dir).map_err(|e| e.to_string())?;
    }
    let cfg_path = rad_core::cli::resolve_config_path("rad.properties");
    let versions = crate::resolve_versions_dir(&cfg_path);
    let installed = crate::version_selection::load(&versions)
        .ok()
        .is_some_and(|version| {
            rad_core::version_package::LoadedVersionPackage::load_from_versions_dir(
                &versions, &version,
            )
            .is_ok()
        });
    if !installed && !version_picker::choose(&versions)? {
        return Ok(0);
    }
    let mut command = Command::new(&exe);
    command
        .arg("--nogui")
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000);
    // A double-click commonly starts in an unrelated current directory.
    if std::env::args_os().len() == 1
        && let Some(dir) = exe.parent()
    {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Unable to start server: {e}"))?;
    let logs = Arc::new(Mutex::new(VecDeque::new()));
    if let Some(out) = child.stdout.take() {
        add_pipe(out, logs.clone())
    }
    if let Some(err) = child.stderr.take() {
        add_pipe(err, logs.clone())
    }
    let base = sample(&child).map_or(0, |x| x.0);
    APP.set(Mutex::new(App {
        child,
        logs,
        log: 0,
        input: 0,
        button: 0,
        clear_button: 0,
        detail_button: 0,
        save_button: 0,
        stop_button: 0,
        info: 0,
        history: VecDeque::new(),
        cmd_history: VecDeque::new(),
        cmd_index: None,
        cpu: VecDeque::new(),
        ram: VecDeque::new(),
        last_cpu: base,
        last_time: Instant::now(),
        started: Instant::now(),
        closing: false,
        port_conflict: false,
        configured_address: None,
        online_players: 0,
        game_version: None,
    }))
    .map_err(|_| "GUI already initialized")?;
    unsafe {
        let background = CreateSolidBrush(0x001b1818);
        let controls = CreateSolidBrush(0x00242120);
        let font = CreateFontW(
            -16,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0x31,
            wide("Cascadia Mono").as_ptr(),
        );
        if background == 0 || controls == 0 || font == 0 {
            return Err("Unable to initialize window theme".into());
        }
        THEME
            .set((background, controls, font))
            .map_err(|_| "Theme already initialized")?;
        let body = CreateFontW(
            -15,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let title = CreateFontW(
            -24,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let value_font = CreateFontW(
            -21,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            wide("Segoe UI").as_ptr(),
        );
        if body == 0 || title == 0 || value_font == 0 {
            return Err("Unable to initialize interface fonts".into());
        }
        UI_FONTS
            .set((body, title, value_font))
            .map_err(|_| "Interface fonts already initialized")?;
        let mut ids = [0u32; 2];
        if GetConsoleProcessList(ids.as_mut_ptr(), 2) == 1 {
            let c = GetConsoleWindow();
            if c != 0 {
                ShowWindow(c, 0);
                FreeConsole();
            }
        }
        let instance = GetModuleHandleW(std::ptr::null());
        let name = wide("RadServerNativeWindow");
        let class = WndClass {
            style: 3,
            proc: Some(window),
            extra: 0,
            wnd_extra: 0,
            instance,
            icon: 0,
            cursor: LoadCursorW(0, 32512usize as *const u16),
            background,
            menu: std::ptr::null(),
            name: name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            return Err("Unable to register window class".into());
        }
        let h = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("RadServer — console and resources").as_ptr(),
            0x00CF0000 | 0x10000000 | 0x02000000,
            0x80000000u32 as i32,
            0x80000000u32 as i32,
            1050,
            720,
            0,
            0,
            instance,
            std::ptr::null(),
        );
        if h == 0 {
            return Err("Unable to create server window".into());
        }
        let dark_title: i32 = 1;
        let _ = DwmSetWindowAttribute(
            h,
            20,
            (&dark_title as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        );
        let rounded: i32 = 2;
        let _ = DwmSetWindowAttribute(h, 33, (&rounded as *const i32).cast(), 4);
        ShowWindow(h, 1);
        UpdateWindow(h);
        let mut m: Msg = std::mem::zeroed();
        loop {
            let r = GetMessageW(&mut m, 0, 0, 0);
            if r <= 0 {
                return if r == 0 {
                    Ok(m.wparam as i32)
                } else {
                    Err("Window message loop failed".into())
                };
            }
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_log_keeps_messages_and_preserves_errors() {
        let info =
            "2026-11-16T14:55:17.000Z  INFO rad_server: loaded library path=C:\\test.dll version=1";
        assert_eq!(
            display_log_line(info, false).as_deref(),
            Some("[14:55:17]  INFO  loaded library  ...")
        );
        assert_eq!(display_log_line(info, true).as_deref(), Some(info));
        let error =
            "2026-11-16T14:55:17.000Z ERROR rad_server: failed bind address=127.0.0.1:25567";
        assert_eq!(display_log_line(error, false).as_deref(), Some(error));
        assert!(display_log_line(" |  _ \\ __ _", false).is_none());
        assert_eq!(
            display_log_line(" |  _ \\ __ _", true).as_deref(),
            Some(" |  _ \\ __ _")
        );
        assert_eq!(
            display_log_line("plain output", false).as_deref(),
            Some("plain output")
        );
        let listening = "2026-11-16T14:55:17.000Z  INFO rad_server: RadServer listening bind_addr=127.0.0.1:25567 tps=20";
        assert_eq!(
            display_log_line(listening, false).as_deref(),
            Some("[14:55:17]  INFO  RadServer listening  |  127.0.0.1:25567  ...")
        );
    }

    #[test]
    fn cpu_graph_scale_adapts_to_low_and_high_load() {
        assert_eq!(cpu_chart_scale(&VecDeque::from([0.1, 0.2])), 1.0);
        assert_eq!(cpu_chart_scale(&VecDeque::from([2.1])), 5.0);
        assert_eq!(cpu_chart_scale(&VecDeque::from([51.0])), 100.0);
    }

    #[test]
    fn dashboard_keeps_console_and_resources_separate() {
        for (width, height) in [(760, 500), (1050, 680), (1920, 1040)] {
            let pos = layout(width, height);
            assert!(24 + pos.main_width < pos.side_x);
            assert!(pos.side_x + pos.side_width <= width - 24);
            assert!(pos.input_y > 132);
            assert!(pos.input_y + 32 <= height);
            let resources = resource_layout(pos.input_y);
            let charts_bottom = 132 + resources.chart_height * 2 + 12;
            assert!(charts_bottom <= pos.input_y - 12);
            if let Some(top) = resources.details_top {
                assert!(top >= charts_bottom + 12);
                assert!(top + 96 <= pos.input_y - 12);
            }
        }
    }

    #[test]
    fn listening_address_is_only_taken_from_the_server_listening_event() {
        assert_eq!(
            configured_address(
                "INFO rad_server: RadServer listening bind_addr=127.0.0.1:25567 tps=20"
            )
            .as_deref(),
            Some("127.0.0.1:25567")
        );
        assert_eq!(
            configured_address("WARN unrelated bind_addr=0.0.0.0:42"),
            None
        );
        assert_eq!(configured_address("INFO RadServer listening"), None);
    }

    #[test]
    fn strips_ansi_and_preserves_unicode() {
        assert_eq!(
            strip_terminal_codes("\u{1b}[2m2026\u{1b}[0m \u{1b}[31mпомилка\u{1b}[0m"),
            "2026 помилка"
        );
        assert_eq!(strip_terminal_codes("ordinary text"), "ordinary text");
    }

    #[test]
    fn console_preserves_scroll_position_when_new_lines_arrive() {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                0,
                0,
                0,
                420,
                140,
                0,
                0,
                instance,
                std::ptr::null(),
            );
            assert_ne!(parent, 0, "failed to create isolated parent");
            let edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide("").as_ptr(),
                0x40000000 | 0x10000000 | 0x00200000 | 0x0800 | 0x0004 | 0x0040,
                0,
                0,
                400,
                120,
                parent,
                0,
                instance,
                std::ptr::null(),
            );
            assert_ne!(edit, 0, "failed to create isolated edit control");
            let initial = (0..200)
                .map(|i| format!("line {i}\r\n"))
                .collect::<String>();
            append_console(edit, &initial);
            let count = SendMessageW(edit, 0x00ba, 0, 0);
            let length = SendMessageW(edit, 0x000e, 0, 0);
            assert!(
                count > 100,
                "expected multiline content: lines={count}, length={length}"
            );
            SendMessageW(edit, 0x00b6, 0, -10_000); // EM_LINESCROLL up
            let top = SendMessageW(edit, 0x00ce, 0, 0);
            append_console(edit, "new line while reading\r\n");
            assert_eq!(SendMessageW(edit, 0x00ce, 0, 0), top);
            SendMessageW(edit, 0x00b6, 0, 10_000); // EM_LINESCROLL down
            let bottom_before = SendMessageW(edit, 0x00ce, 0, 0);
            assert!(bottom_before > top, "test content must actually scroll");
            append_console(edit, "latest line\r\n");
            let bottom_after = SendMessageW(edit, 0x00ce, 0, 0);
            let mut si = ScrollInfo {
                size: std::mem::size_of::<ScrollInfo>() as u32,
                mask: 7,
                ..Default::default()
            };
            GetScrollInfo(edit, 1, &mut si);
            assert!(
                bottom_after > bottom_before,
                "before={bottom_before}, after={bottom_after}, pos={}, page={}, max={}",
                si.pos,
                si.page,
                si.max
            );
            DestroyWindow(edit);
            DestroyWindow(parent);
        }
    }
}
