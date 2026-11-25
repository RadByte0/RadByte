//! First-run native version chooser.
use super::*;
use std::{
    path::Path,
    sync::atomic::{AtomicIsize, Ordering},
};

static RESULT: AtomicIsize = AtomicIsize::new(0);
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
struct State {
    versions: std::path::PathBuf,
    search: H,
    list: H,
    path: H,
    status: H,
    install: H,
    auto_install: H,
    browse: H,
    font: H,
    title_font: H,
    background: H,
    controls: H,
    busy: bool,
    error: String,
    heading: H,
    subtitle: H,
}

#[repr(C)]
struct BrowseInfo {
    owner: H,
    root: *const core::ffi::c_void,
    display: *mut u16,
    title: *const u16,
    flags: u32,
    callback: usize,
    data: isize,
    image: i32,
}
#[link(name = "shell32")]
unsafe extern "system" {
    fn SHBrowseForFolderW(info: *mut BrowseInfo) -> *mut core::ffi::c_void;
    fn SHGetPathFromIDListW(id: *const core::ffi::c_void, path: *mut u16) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
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
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(ptr: *mut core::ffi::c_void);
    fn CoInitializeEx(reserved: *mut core::ffi::c_void, flags: u32) -> i32;
    fn CoUninitialize();
}
#[link(name = "user32")]
unsafe extern "system" {
    fn PostMessageW(h: H, msg: u32, w: usize, l: isize) -> i32;
    fn EnableWindow(h: H, enable: i32) -> i32;
    fn MessageBoxW(owner: H, text: *const u16, caption: *const u16, flags: u32) -> i32;
}

fn matches_version(query: &str) -> bool {
    let q: String = query
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    q.is_empty()
        || [
            "263",
            "minecraft263",
            "minecraft",
            "latest",
            "newest",
            "777",
        ]
        .iter()
        .any(|s| s.contains(&q))
}

unsafe extern "system" fn proc(h: H, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        0x0001 => {
            let mut s = STATE.get().unwrap().lock().unwrap();
            let inst = unsafe { GetModuleHandleW(std::ptr::null()) };
            let heading = ctrl(h, "STATIC", "RadServer", 0, 0, inst);
            let description = ctrl(h, "STATIC", "VERSION SETUP", 0, 0, inst);
            s.heading = heading;
            s.subtitle = description;
            let search_label = ctrl(h, "STATIC", "Search", 0, 0, inst);
            s.search = ctrl(h, "EDIT", "", 0x00800000 | 0x0080, 201, inst);
            s.list = ctrl(
                h,
                "LISTBOX",
                "",
                0x00800000 | 0x0001 | 0x01000000,
                202,
                inst,
            );
            let label = ctrl(h, "STATIC", "Version data", 0, 0, inst);
            s.path = ctrl(h, "EDIT", "", 0x00800000 | 0x0080, 203, inst);
            s.browse = ctrl(h, "BUTTON", "Browse", 0x0000000b, 204, inst);
            s.install = ctrl(h, "BUTTON", "Install version", 0x0000000b, 205, inst);
            s.auto_install = ctrl(h, "BUTTON", "Auto install (EULA)", 0x0000000b, 206, inst);
            s.status = ctrl(
                h,
                "STATIC",
                "Select a local data folder, or click Auto install (EULA).",
                0,
                0,
                inst,
            );
            for (item, x, y, width, height) in [
                (heading, 26, 18, 260, 34),
                (description, 26, 57, 260, 22),
                (search_label, 40, 111, 450, 20),
                (s.search, 40, 137, 488, 30),
                (s.list, 40, 180, 488, 70),
                (label, 40, 271, 450, 22),
                (s.path, 40, 297, 386, 30),
                (s.browse, 436, 297, 92, 30),
                (s.status, 40, 344, 488, 45),
                (s.auto_install, 40, 405, 190, 38),
                (s.install, 348, 405, 180, 38),
            ] {
                unsafe {
                    MoveWindow(item, x, y, width, height, 1);
                    SendMessageW(item, 0x0030, s.font as usize, 1);
                }
            }
            unsafe {
                SendMessageW(heading, 0x0030, s.title_font as usize, 1);
            }
            unsafe {
                SendMessageW(
                    s.search,
                    0x1501,
                    0,
                    wide("Search version, protocol, or latest...").as_ptr() as isize,
                );
                SendMessageW(
                    s.list,
                    0x0180,
                    0,
                    wide("26.3    •    Protocol 777    •    Latest supported").as_ptr() as isize,
                );
                SendMessageW(s.list, 0x0186, 0, 0);
            }
            0
        }
        0x0111 => {
            let id = w & 0xffff;
            let notification = w >> 16;
            if id == 201 && notification == 0x0300 {
                let s = STATE.get().unwrap().lock().unwrap();
                let (search, list) = (s.search, s.list);
                drop(s);
                let mut text = [0u16; 256];
                unsafe {
                    GetWindowTextW(search, text.as_mut_ptr(), 256);
                    SendMessageW(list, 0x0184, 0, 0);
                }
                if matches_version(String::from_utf16_lossy(&text).trim_end_matches('\0')) {
                    unsafe {
                        SendMessageW(
                            list,
                            0x0180,
                            0,
                            wide("26.3    •    Protocol 777    •    Latest supported").as_ptr()
                                as isize,
                        );
                        SendMessageW(list, 0x0186, 0, 0);
                    }
                }
            } else if id == 204 {
                let initialized = unsafe { CoInitializeEx(std::ptr::null_mut(), 2) };
                if initialized < 0 {
                    let s = STATE.get().unwrap().lock().unwrap();
                    let status = s.status;
                    drop(s);
                    unsafe {
                        SetWindowTextW(
                            status,
                            wide("Could not open the folder picker. Enter a folder path instead.")
                                .as_ptr(),
                        );
                    }
                    return 0;
                }
                let mut display = [0u16; 260];
                let title = wide("Choose the folder containing the version data");
                let mut info = BrowseInfo {
                    owner: h,
                    root: std::ptr::null(),
                    display: display.as_mut_ptr(),
                    title: title.as_ptr(),
                    flags: 0x41,
                    callback: 0,
                    data: 0,
                    image: 0,
                };
                let ptr = unsafe { SHBrowseForFolderW(&mut info) };
                if !ptr.is_null() {
                    let mut path = [0u16; 260];
                    unsafe {
                        if SHGetPathFromIDListW(ptr, path.as_mut_ptr()) != 0 {
                            let path_field = STATE.get().unwrap().lock().unwrap().path;
                            SetWindowTextW(path_field, path.as_ptr());
                        }
                        CoTaskMemFree(ptr);
                    }
                }
                unsafe {
                    CoUninitialize();
                }
            } else if id == 205 {
                let s = STATE.get().unwrap().lock().unwrap();
                if s.busy {
                    return 0;
                }
                let (list, status, path_field) = (s.list, s.status, s.path);
                drop(s);
                if unsafe { SendMessageW(list, 0x0188, 0, 0) } < 0 {
                    unsafe {
                        SetWindowTextW(status, wide("Select version 26.3 first.").as_ptr());
                    }
                    return 0;
                }
                let mut path = [0u16; 32768];
                unsafe {
                    GetWindowTextW(path_field, path.as_mut_ptr(), path.len() as i32);
                }
                let path = String::from_utf16_lossy(&path)
                    .trim_end_matches('\0')
                    .trim()
                    .to_owned();
                if path.is_empty() {
                    unsafe {
                        SetWindowTextW(
                            status,
                            wide("Choose the folder containing the four data files.").as_ptr(),
                        );
                    }
                    return 0;
                }
                let mut s = STATE.get().unwrap().lock().unwrap();
                s.busy = true;
                let versions = s.versions.clone();
                let (install, auto_install, browse, status) =
                    (s.install, s.auto_install, s.browse, s.status);
                drop(s);
                unsafe {
                    EnableWindow(install, 0);
                    EnableWindow(auto_install, 0);
                    EnableWindow(browse, 0);
                    SetWindowTextW(
                        status,
                        wide("Installing... Downloading and verifying the version library.")
                            .as_ptr(),
                    );
                }
                std::thread::spawn(move || {
                    let result = crate::version_install::install(&versions, Path::new(&path));
                    let mut state = STATE.get().unwrap().lock().unwrap();
                    state.error = result.err().unwrap_or_default();
                    state.busy = false;
                    unsafe {
                        PostMessageW(h, 0x8001, 0, 0);
                    }
                });
            } else if id == 206 {
                let s = STATE.get().unwrap().lock().unwrap();
                if s.busy {
                    return 0;
                }
                let (list, status) = (s.list, s.status);
                drop(s);
                if unsafe { SendMessageW(list, 0x0188, 0, 0) } < 0 {
                    unsafe {
                        SetWindowTextW(status, wide("Select version 26.3 first.").as_ptr());
                    }
                    return 0;
                }
                let agreed = unsafe {
                    MessageBoxW(
                        h,
                        wide("RadServer can automatically download the official Minecraft 26.3 server.jar (and temporary portable Java if needed) to generate the 4 registry files locally.\n\nDo you agree to the Minecraft EULA?\nhttps://aka.ms/MinecraftEULA").as_ptr(),
                        wide("Minecraft EULA").as_ptr(),
                        0x04 | 0x20,
                    )
                };
                if agreed != 6 {
                    unsafe {
                        SetWindowTextW(
                            status,
                            wide("Minecraft EULA must be accepted for automatic installation.")
                                .as_ptr(),
                        );
                    }
                    return 0;
                }
                let mut s = STATE.get().unwrap().lock().unwrap();
                s.busy = true;
                let versions = s.versions.clone();
                let (install, auto_install, browse, status) =
                    (s.install, s.auto_install, s.browse, s.status);
                drop(s);
                if let Some(work_dir) = versions.parent() {
                    let _ = std::fs::write(
                        work_dir.join("eula.txt"),
                        "# By changing the setting below to TRUE you are indicating your agreement to Mojang's EULA (https://aka.ms/MinecraftEULA).\neula=true\n",
                    );
                }
                unsafe {
                    EnableWindow(install, 0);
                    EnableWindow(auto_install, 0);
                    EnableWindow(browse, 0);
                    SetWindowTextW(
                        status,
                        wide("Auto-installing... Fetching Java, Mojang server.jar, and RadByte library.").as_ptr(),
                    );
                }
                std::thread::spawn(move || {
                    let result = crate::version_install::install_from_mojang(&versions);
                    let mut state = STATE.get().unwrap().lock().unwrap();
                    state.error = result.err().unwrap_or_default();
                    state.busy = false;
                    unsafe {
                        PostMessageW(h, 0x8001, 0, 0);
                    }
                });
            }
            0
        }
        0x8001 => {
            let s = STATE.get().unwrap().lock().unwrap();
            let (error, status, install, auto_install, browse) = (
                s.error.clone(),
                s.status,
                s.install,
                s.auto_install,
                s.browse,
            );
            drop(s);
            if error.is_empty() {
                RESULT.store(1, Ordering::Relaxed);
                unsafe {
                    DestroyWindow(h);
                }
            } else {
                unsafe {
                    SetWindowTextW(status, wide(&error).as_ptr());
                    EnableWindow(install, 1);
                    EnableWindow(auto_install, 1);
                    EnableWindow(browse, 1);
                }
            }
            0
        }
        0x0014 => 1,
        0x000f => {
            let mut paint: Paint = unsafe { std::mem::zeroed() };
            unsafe {
                let dc = BeginPaint(h, &mut paint);
                let mut rect: Rect = std::mem::zeroed();
                GetClientRect(h, &mut rect);
                FillRect(dc, &rect, STATE.get().unwrap().lock().unwrap().background);
                let s = STATE.get().unwrap().lock().unwrap();
                rounded_rect(
                    dc,
                    &Rect {
                        left: 24,
                        top: 94,
                        right: 544,
                        bottom: 459,
                    },
                    s.controls,
                    12,
                );
                rounded_rect(
                    dc,
                    &Rect {
                        left: 558,
                        top: 94,
                        right: 816,
                        bottom: 273,
                    },
                    s.controls,
                    12,
                );
                let old = SelectObject(dc, s.font);
                draw_text(dc, "VERSION", 575, 111, 210, 0x00aaa6a2);
                SelectObject(dc, s.title_font);
                draw_text(dc, "26.3", 575, 143, 210, 0x00eeeae7);
                SelectObject(dc, s.font);
                draw_text(dc, "Java Edition", 575, 185, 210, 0x00c8c4c0);
                draw_text(dc, "Protocol 777", 575, 213, 210, 0x00aaa6a2);
                SelectObject(dc, old);
                EndPaint(h, &paint);
            }
            0
        }
        0x002b if (204..=206).contains(&w) && l != 0 => {
            let item = unsafe { &mut *(l as *mut DrawItem) };
            let s = STATE.get().unwrap().lock().unwrap();
            unsafe {
                FillRect(item.dc, &item.rect, s.controls);
                let pressed = item.item_state & 1 != 0;
                let brush = CreateSolidBrush(match (w, pressed) {
                    (205, true) => 0x00804720,
                    (205, false) => 0x00945426,
                    (_, true) => 0x004a4540,
                    _ => 0x00332f2c,
                });
                let previous_brush = SelectObject(item.dc, brush);
                let previous_pen = SelectObject(item.dc, GetStockObject(8));
                RoundRect(
                    item.dc,
                    item.rect.left,
                    item.rect.top,
                    item.rect.right,
                    item.rect.bottom,
                    10,
                    10,
                );
                SelectObject(item.dc, previous_pen);
                SelectObject(item.dc, previous_brush);
                DeleteObject(brush);
                let old_font = SelectObject(item.dc, s.font);
                SetTextColor(
                    item.dc,
                    if s.busy && (w == 205 || w == 206) {
                        0x00888888
                    } else {
                        0x00ffffff
                    },
                );
                SetBkMode(item.dc, 1);
                DrawTextW(
                    item.dc,
                    wide(match w {
                        205 => "Install version",
                        206 => "Auto install (EULA)",
                        _ => "Browse",
                    })
                    .as_ptr(),
                    -1,
                    &mut item.rect,
                    0x25,
                );
                SelectObject(item.dc, old_font);
            }
            1
        }
        0x0133 | 0x0134 | 0x0138 => {
            let s = STATE.get().unwrap().lock().unwrap();
            unsafe {
                SetTextColor(w as H, 0x00e5e7eb);
                let header = l as H == s.heading || l as H == s.subtitle;
                SetBkColor(w as H, if header { 0x001b1818 } else { 0x00242120 });
            }
            if msg == 0x0138 && (l as H == s.heading || l as H == s.subtitle) {
                s.background
            } else {
                s.controls
            }
        }
        0x0002 => {
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(h, msg, w, l) },
    }
}

pub fn choose(versions: &Path) -> Result<bool, String> {
    unsafe {
        let mut ids = [0u32; 2];
        if GetConsoleProcessList(ids.as_mut_ptr(), 2) == 1 {
            let c = GetConsoleWindow();
            if c != 0 {
                ShowWindow(c, 0);
                FreeConsole();
            }
        }
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
            0,
            wide("Segoe UI").as_ptr(),
        );
        let title_font = CreateFontW(
            -23,
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
        STATE
            .set(Mutex::new(State {
                versions: versions.to_path_buf(),
                search: 0,
                list: 0,
                path: 0,
                status: 0,
                install: 0,
                auto_install: 0,
                browse: 0,
                font,
                title_font,
                background,
                controls,
                busy: false,
                error: String::new(),
                heading: 0,
                subtitle: 0,
            }))
            .map_err(|_| "Version chooser already initialized")?;
        let instance = GetModuleHandleW(std::ptr::null());
        let name = wide("RadServerVersionPicker");
        let class = WndClass {
            style: 3,
            proc: Some(proc),
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
            return Err("Cannot register version chooser".into());
        }
        let h = CreateWindowExW(
            0,
            name.as_ptr(),
            wide("RadServer — Choose version").as_ptr(),
            0x00ca0000 | 0x10000000,
            0x80000000u32 as i32,
            0x80000000u32 as i32,
            855,
            520,
            0,
            0,
            instance,
            std::ptr::null(),
        );
        if h == 0 {
            return Err("Cannot create version chooser".into());
        }
        let dark: i32 = 1;
        let rounded: i32 = 2;
        let _ = DwmSetWindowAttribute(h, 20, (&dark as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(h, 33, (&rounded as *const i32).cast(), 4);
        ShowWindow(h, 1);
        UpdateWindow(h);
        let mut m: Msg = std::mem::zeroed();
        loop {
            let r = GetMessageW(&mut m, 0, 0, 0);
            if r == 0 {
                break;
            }
            if r < 0 {
                return Err("Version chooser message loop failed".into());
            }
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
        Ok(RESULT.load(Ordering::Relaxed) == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::matches_version;

    #[test]
    fn search_matches_version_protocol_and_aliases() {
        for query in [
            "",
            "26.3",
            "26 3",
            "Minecraft 26.3",
            "777",
            "latest",
            "LATEST",
        ] {
            assert!(matches_version(query), "{query}");
        }
        assert!(!matches_version("1.20"));
    }
}
