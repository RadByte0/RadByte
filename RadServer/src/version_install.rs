//! Install the supported version using locally supplied data or Mojang's official data generator.
use rad_core::version_package::{
    LoadedVersionPackage, VersionLibraryMeta, VersionManifest, VersionPackageFile,
};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Read, Write},
    path::Path,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MINECRAFT_EULA_URL: &str = "https://aka.ms/MinecraftEULA";
const MOJANG_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn release_target() -> Result<(&'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok(("x86_64-pc-windows-msvc", "dll")),
        ("windows", "aarch64") => Ok(("aarch64-pc-windows-msvc", "dll")),
        ("linux", "x86_64") if cfg!(target_env = "musl") => Ok(("x86_64-unknown-linux-musl", "so")),
        ("linux", "aarch64") if cfg!(target_env = "musl") => {
            Ok(("aarch64-unknown-linux-musl", "so"))
        }
        ("linux", "x86_64") => Ok(("x86_64-unknown-linux-gnu", "so")),
        ("linux", "aarch64") => Ok(("aarch64-unknown-linux-gnu", "so")),
        ("macos", "x86_64") => Ok(("x86_64-apple-darwin", "dylib")),
        ("macos", "aarch64") => Ok(("aarch64-apple-darwin", "dylib")),
        (os, arch) => Err(format!("Unsupported platform: {os}/{arch}")),
    }
}

/// Release entry discovered on GitHub Releases for the current platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableRelease {
    /// GitHub release tag (e.g. `"v0.3.3"`).
    pub tag: String,
    /// Minecraft game version extracted from the matching `rad-version-<ver>-...` asset.
    pub game_version: String,
}

fn release_repo() -> Result<&'static str, String> {
    let repo = option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte");
    if !repo
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/.-_".contains(&b))
    {
        return Err("Invalid release repository".into());
    }
    Ok(repo)
}

/// Fetches recent GitHub Releases that ship a matching native `rad-version` library for this OS/arch.
pub fn fetch_available_releases() -> Result<Vec<AvailableRelease>, String> {
    let repo = release_repo()?;
    let (target, ext) = release_target()?;
    let suffix = format!("-{target}.{ext}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("RadServer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let releases: Vec<serde_json::Value> = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases?per_page=15"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot fetch GitHub releases: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for rel in releases {
        if rel["draft"].as_bool() == Some(true) {
            continue;
        }
        let Some(tag) = rel["tag_name"].as_str() else {
            continue;
        };
        let Some(assets) = rel["assets"].as_array() else {
            continue;
        };
        for asset in assets {
            let Some(name) = asset["name"].as_str() else {
                continue;
            };
            if let Some(rest) = name.strip_prefix("rad-version-")
                && let Some(ver) = rest.strip_suffix(&suffix)
                && !ver.is_empty()
            {
                out.push(AvailableRelease {
                    tag: tag.to_owned(),
                    game_version: ver.to_owned(),
                });
                break;
            }
        }
    }
    if out.is_empty() {
        return Err("No compatible releases found on GitHub.".into());
    }
    Ok(out)
}

pub fn install(versions: &Path, data: &Path) -> Result<(), String> {
    install_with_tag(versions, data, None)
}

pub fn install_with_tag(
    versions: &Path,
    data: &Path,
    tag_override: Option<&str>,
) -> Result<(), String> {
    install_with_source(
        versions,
        data,
        "local user-supplied data; provenance not verified",
        tag_override,
    )
}

fn install_with_source(
    versions: &Path,
    data: &Path,
    source_label: &str,
    tag_override: Option<&str>,
) -> Result<(), String> {
    let selection = versions.join("version.toml");
    if selection.exists() {
        let selected = crate::version_selection::load(versions)?;
        if selected != "26.3" {
            return Err(format!(
                "The existing version.toml selects {selected}. Change it to 26.3 before installing."
            ));
        }
    }
    let tag = tag_override
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .or(option_env!("RADBYTE_RELEASE_TAG").filter(|tag| !tag.is_empty()))
        .or(Some(concat!("v", env!("CARGO_PKG_VERSION"))))
        .filter(|tag| {
            tag.starts_with('v')
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        })
        .ok_or("Installation is available in GitHub release builds only.")?;
    let repo = release_repo()?;
    let (target, ext) = release_target()?;
    let preferred_name = format!("rad-version-26.3-{target}.{ext}");
    let legacy_name = format!("rad_version_26_3-{target}.{ext}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("RadServer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let release: serde_json::Value = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/tags/{tag}"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot find release: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;
    let assets = release["assets"]
        .as_array()
        .ok_or("Release has no assets.")?;
    let asset = [&preferred_name, &legacy_name]
        .into_iter()
        .find_map(|name| assets.iter().find(|item| item["name"] == *name))
        .ok_or("This release does not contain a matching version library.")?;
    let name = asset["name"]
        .as_str()
        .ok_or("Invalid library name in release.")?
        .to_owned();
    let expected = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .ok_or("Release has no SHA-256 for the library.")?;
    let response = client
        .get(format!(
            "https://github.com/{repo}/releases/download/{tag}/{name}"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot download version library: {e}"))?;
    let mut library = Vec::new();
    response
        .take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut library)
        .map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&library));
    if library.is_empty() || library.len() > 256 * 1024 * 1024 || hash != expected {
        return Err("Downloaded library failed SHA-256 verification.".into());
    }
    std::fs::create_dir_all(versions).map_err(|e| e.to_string())?;
    let destination = versions.join("26.3");
    if destination.exists() {
        return Err(format!(
            "{} already exists; it was not changed.",
            destination.display()
        ));
    }
    let stage = versions.join(format!(".rad-install-{}", std::process::id()));
    std::fs::create_dir(&stage).map_err(|e| format!("Cannot create staging directory: {e}"))?;
    let result = (|| -> Result<(), String> {
        std::fs::create_dir(stage.join("data")).map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for (id, key) in [
            ("items", "entries"),
            ("blocks", "blocks"),
            ("entities", "entries"),
            ("menus", "entries"),
        ] {
            let bytes = std::fs::read(data.join(format!("{id}.json")))
                .map_err(|e| format!("Cannot read {id}.json: {e}"))?;
            if bytes.len() > 32 * 1024 * 1024 {
                return Err(format!("{id}.json is too large"));
            }
            let parsed: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| format!("Invalid {id}.json: {e}"))?;
            let count = parsed[key].as_array().map_or(0, Vec::len);
            if count == 0 {
                return Err(format!("{id}.json has no entries"));
            }
            std::fs::write(stage.join("data").join(format!("{id}.json")), &bytes)
                .map_err(|e| e.to_string())?;
            files.push(VersionPackageFile {
                id: id.into(),
                path: format!("data/{id}.json"),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                entry_count: count,
            });
        }
        let tags_path = data.join("tags.json");
        if tags_path.exists() {
            let bytes = std::fs::read(tags_path).map_err(|e| e.to_string())?;
            if bytes.len() > 32 * 1024 * 1024 {
                return Err("tags.json is too large".into());
            }
            let tags: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| format!("Invalid tags.json: {e}"))?;
            let count = ["block_tags", "item_tags"]
                .iter()
                .map(|key| tags[*key].as_array().map_or(0, Vec::len))
                .sum::<usize>()
                + tags["core_registry_tags"].as_array().map_or(0, |sets| {
                    sets.iter()
                        .map(|set| set["tags"].as_array().map_or(0, Vec::len))
                        .sum()
                });
            std::fs::write(stage.join("data/tags.json"), &bytes).map_err(|e| e.to_string())?;
            files.push(VersionPackageFile {
                id: "tags".into(),
                path: "data/tags.json".into(),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                entry_count: count,
            });
        }
        std::fs::write(stage.join(&name), &library).map_err(|e| e.to_string())?;
        let manifest = VersionManifest {
            schema_version: 1,
            game_version: "26.3".into(),
            protocol_version: 777,
            package_format_version: 1,
            world_data_version: 4790,
            source: source_label.into(),
            generated_utc: utc_now(),
            notes: None,
            library: Some(VersionLibraryMeta {
                path: name,
                abi_version: 1,
                target_os: std::env::consts::OS.into(),
                target_arch: std::env::consts::ARCH.into(),
                size_bytes: library.len() as u64,
                sha256: hash,
            }),
            files,
        };
        std::fs::write(
            stage.join("version.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let package =
            LoadedVersionPackage::load_from_package_dir(&stage).map_err(|e| e.to_string())?;
        package
            .verify_and_resolve_library_path()
            .map_err(|e| e.to_string())?;
        let mut created_selection = false;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&selection)
        {
            Ok(mut file) => {
                use std::io::Write;
                if let Err(e) = file.write_all(b"version = \"26.3\"\n") {
                    drop(file);
                    let _ = std::fs::remove_file(&selection);
                    return Err(e.to_string());
                }
                created_selection = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if crate::version_selection::load(versions)? != "26.3" {
                    return Err(
                        "version.toml changed while installing; no package was installed".into(),
                    );
                }
            }
            Err(e) => return Err(e.to_string()),
        }
        if let Err(e) = std::fs::rename(&stage, &destination) {
            if created_selection {
                let _ = std::fs::remove_file(&selection);
            }
            return Err(e.to_string());
        }
        Ok(())
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

/// Ensures version `26.3` is installed (via `import/`, interactive console setup, or Mojang `--reports` after EULA).
pub fn ensure_installed_headless(work_dir: &Path, versions_dir: &Path) -> Result<(), String> {
    if versions_dir.join("26.3").join("version.json").is_file() {
        return Ok(());
    }

    let import_dir = work_dir.join("import");
    let has_import_data = ["items.json", "blocks.json", "entities.json", "menus.json"]
        .iter()
        .all(|name| import_dir.join(name).is_file());
    if has_import_data {
        tracing::info!(dir = %import_dir.display(), "found version data in import/; installing version 26.3");
        return install(versions_dir, &import_dir);
    }

    let _ = std::fs::create_dir_all(&import_dir);
    if std::io::stdin().is_terminal() {
        match prompt_console_setup(work_dir)? {
            ConsoleSetupChoice::Mojang(tag) => {
                ensure_eula_accepted(work_dir, &import_dir)?;
                return install_from_mojang_with_progress(
                    versions_dir,
                    tag.as_deref(),
                    &mut |msg| println!("  \x1b[38;2;232;125;13m>\x1b[0m {msg}"),
                );
            }
            ConsoleSetupChoice::Local { path, tag } => {
                return install_with_tag(versions_dir, Path::new(&path), tag.as_deref());
            }
        }
    }

    ensure_eula_accepted(work_dir, &import_dir)?;
    install_from_mojang_with_progress(versions_dir, None, &mut |msg| tracing::info!("{msg}"))
}

enum ConsoleSetupChoice {
    Mojang(Option<String>),
    Local { path: String, tag: Option<String> },
}

fn filter_console_releases(
    all: &[AvailableRelease],
    query: &str,
    show_older: bool,
) -> Vec<AvailableRelease> {
    let q = query.trim().to_ascii_lowercase();
    let mut seen_versions = std::collections::HashSet::new();
    let mut out = Vec::new();
    for rel in all {
        if !show_older && !seen_versions.insert(rel.game_version.clone()) {
            continue;
        }
        if q.is_empty()
            || rel.game_version.to_ascii_lowercase().contains(&q)
            || rel.tag.to_ascii_lowercase().contains(&q)
        {
            out.push(rel.clone());
        }
    }
    out
}

fn prompt_console_setup(work_dir: &Path) -> Result<ConsoleSetupChoice, String> {
    if rad_core::console_ui::is_interactive_tty() {
        return prompt_interactive_tui_setup(work_dir);
    }
    prompt_console_setup_plain(work_dir)
}

fn prompt_interactive_tui_setup(work_dir: &Path) -> Result<ConsoleSetupChoice, String> {
    let all = match fetch_available_releases() {
        Ok(list) if !list.is_empty() => list,
        _ => vec![AvailableRelease {
            tag: option_env!("RADBYTE_RELEASE_TAG")
                .filter(|s| !s.is_empty())
                .unwrap_or("latest")
                .to_owned(),
            game_version: "26.3".to_owned(),
        }],
    };
    let cfg_path = work_dir.join("rad.properties");
    let eula_path = work_dir.join("eula.txt");
    let mut auto_update = rad_core::auto_update::is_auto_update_enabled(&cfg_path);
    let mut eula_ok =
        is_eula_file_accepted(&eula_path) || std::env::args().skip(1).any(|a| a == "--accept-eula");
    let mut show_older = false;
    let mut dropdown_open = false;
    let mut editing_search = false;
    let mut editing_local = false;
    let mut query = String::new();
    let mut local_path = String::new();
    let mut selected_rel_idx = 0usize;
    let mut focused_row = 0usize; // 0=version dropdown, 1=older toggle, 2=auto-update, 3=EULA, 4=local folder, 5=Install&Start
    let mut notice = String::new();

    {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        let _ = stdout.flush();
    }
    struct SetupScreenGuard(Option<rad_core::console_ui::RawTerminalGuard>);
    impl Drop for SetupScreenGuard {
        fn drop(&mut self) {
            drop(self.0.take());
            let mut stdout = std::io::stdout().lock();
            let _ = write!(stdout, "\x1b[?25h\x1b[?1049l\x1b[0m");
            let _ = stdout.flush();
        }
    }
    let _guard = SetupScreenGuard(Some(rad_core::console_ui::RawTerminalGuard::enter()));
    let mut stdin = std::io::stdin();
    let mut byte = [0u8; 1];
    let mut esc_seq = Vec::new();
    let mut in_esc = false;

    loop {
        let filtered = filter_console_releases(&all, &query, show_older);
        if selected_rel_idx >= filtered.len() {
            selected_rel_idx = 0;
        }
        let (cols, _) = rad_core::console_ui::terminal_size();
        let width = usize::from(cols);
        let divider = "─".repeat(width);
        let selected_label = filtered.get(selected_rel_idx).map_or_else(
            || "Minecraft 26.3 (latest)".to_owned(),
            |r| format!("Minecraft {}  ({})", r.game_version, r.tag),
        );

        let mut out = String::with_capacity(2048);
        out.push_str("\x1b[H");
        out.push_str("\x1b[2K  \x1b[38;2;245;158;11m██████╗ \x1b[0m  \x1b[1;37mRadServer Setup\x1b[0m \x1b[38;2;232;125;13mv");
        out.push_str(env!("CARGO_PKG_VERSION"));
        out.push_str("\x1b[0m\n");
        out.push_str("\x1b[2K  \x1b[38;2;232;125;13m██╔══██╗\x1b[0m  \x1b[90mFirst-Run Configuration\x1b[0m\n");
        out.push_str("\x1b[2K  \x1b[38;2;232;125;13m██████╔╝\x1b[0m  \x1b[90m↑/↓ Navigate  •  Space/Enter Toggle  •  / Search  •  Esc Quit\x1b[0m\n");
        out.push_str(&format!("\x1b[2K\x1b[38;2;56;56;56m{divider}\x1b[0m\n"));

        let row_marker = |idx: usize| {
            if focused_row == idx {
                "\x1b[38;2;232;125;13m›\x1b[0m"
            } else {
                " "
            }
        };
        let arrow = if dropdown_open { "▲" } else { "▼" };
        out.push_str(&format!(
            "\x1b[2K  {} \x1b[1;37mVersion:\x1b[0m               [\x1b[38;2;232;125;13m{arrow}\x1b[0m] {selected_label}\n",
            row_marker(0)
        ));

        if dropdown_open {
            let search_hint = if editing_search {
                format!("\x1b[38;2;232;125;13mSearch: {query}_\x1b[0m")
            } else if query.is_empty() {
                "\x1b[90m(press / to search)\x1b[0m".to_owned()
            } else {
                format!("\x1b[90mFilter: {query} (press / to edit)\x1b[0m")
            };
            out.push_str(&format!("\x1b[2K       {search_hint}\n"));
            if filtered.is_empty() {
                out.push_str("\x1b[2K       \x1b[90m(no matching releases)\x1b[0m\n");
            } else {
                let win_start = selected_rel_idx.saturating_sub(4);
                for (i, rel) in filtered.iter().enumerate().skip(win_start).take(7) {
                    let mark = if i == selected_rel_idx {
                        "\x1b[38;2;232;125;13m●\x1b[0m"
                    } else {
                        "\x1b[90m○\x1b[0m"
                    };
                    out.push_str(&format!(
                        "\x1b[2K       {mark} Minecraft {}  \x1b[90m({})\x1b[0m\n",
                        rel.game_version, rel.tag
                    ));
                }
            }
        }

        let older_box = if show_older {
            "\x1b[38;2;232;125;13m[x]\x1b[0m"
        } else {
            "\x1b[90m[ ]\x1b[0m"
        };
        let auto_box = if auto_update {
            "\x1b[38;2;232;125;13m[x]\x1b[0m"
        } else {
            "\x1b[90m[ ]\x1b[0m"
        };
        let eula_box = if eula_ok {
            "\x1b[32m[x]\x1b[0m"
        } else {
            "\x1b[90m[ ]\x1b[0m"
        };
        let local_disp = if editing_local {
            format!("\x1b[38;2;232;125;13m{local_path}_\x1b[0m")
        } else if local_path.is_empty() {
            "\x1b[90m(auto-install from Mojang)\x1b[0m".to_owned()
        } else {
            local_path.clone()
        };

        out.push_str(&format!(
            "\x1b[2K  {} \x1b[1;37mShow older releases:\x1b[0m   {older_box}\n",
            row_marker(1)
        ));
        out.push_str(&format!(
            "\x1b[2K  {} \x1b[1;37mAuto-update all:\x1b[0m       {auto_box} \x1b[90m(launcher, versions, API)\x1b[0m\n",
            row_marker(2)
        ));
        out.push_str(&format!(
            "\x1b[2K  {} \x1b[1;37mMinecraft EULA:\x1b[0m        {eula_box} \x1b[90m(aka.ms/MinecraftEULA)\x1b[0m\n",
            row_marker(3)
        ));
        out.push_str(&format!(
            "\x1b[2K  {} \x1b[1;37mLocal data folder:\x1b[0m     {local_disp}\n",
            row_marker(4)
        ));
        out.push_str("\x1b[2K\n");
        let btn_style = if focused_row == 5 {
            "\x1b[48;2;232;125;13m\x1b[1;30m  Install & Start Server  \x1b[0m"
        } else {
            "\x1b[38;2;232;125;13m[ Install & Start Server ]\x1b[0m"
        };
        out.push_str(&format!("\x1b[2K  {} {btn_style}\n", row_marker(5)));
        if !notice.is_empty() {
            out.push_str(&format!(
                "\x1b[2K  \x1b[38;2;232;125;13m! {notice}\x1b[0m\n"
            ));
        }
        out.push_str("\x1b[J");

        {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.flush();
        }

        if stdin.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        let b = byte[0];
        if in_esc {
            esc_seq.push(b);
            let done = matches!(b, b'~' | b'A'..=b'Z' | b'a'..=b'z') || esc_seq.len() >= 12;
            if done {
                in_esc = false;
                match esc_seq.as_slice() {
                    b"[A" => {
                        editing_search = false;
                        editing_local = false;
                        if dropdown_open && focused_row == 0 {
                            selected_rel_idx = selected_rel_idx.saturating_sub(1);
                        } else {
                            focused_row = focused_row.saturating_sub(1);
                        }
                    }
                    b"[B" => {
                        editing_search = false;
                        editing_local = false;
                        if dropdown_open
                            && focused_row == 0
                            && selected_rel_idx + 1 < filtered.len()
                        {
                            selected_rel_idx += 1;
                        } else if focused_row < 5 {
                            focused_row += 1;
                        }
                    }
                    b"[C" if focused_row == 0 => dropdown_open = true,
                    b"[D" if focused_row == 0 => dropdown_open = false,
                    _ => {}
                }
                esc_seq.clear();
            }
            continue;
        }

        if editing_search {
            match b {
                0x1b => editing_search = false,
                b'\r' | b'\n' => editing_search = false,
                0x08 | 0x7f => {
                    query.pop();
                    selected_rel_idx = 0;
                }
                c if (0x20..0x7f).contains(&c) => {
                    query.push(c as char);
                    selected_rel_idx = 0;
                }
                _ => {}
            }
            continue;
        }
        if editing_local {
            match b {
                0x1b | b'\r' | b'\n' => editing_local = false,
                0x08 | 0x7f => {
                    local_path.pop();
                }
                c if (0x20..0x7f).contains(&c) => {
                    local_path.push(c as char);
                }
                _ => {}
            }
            continue;
        }

        match b {
            0x1b => {
                in_esc = true;
                esc_seq.clear();
            }
            0x03 | b'q' | b'Q' => return Err("Setup cancelled by user".into()),
            b'/' => {
                focused_row = 0;
                dropdown_open = true;
                editing_search = true;
            }
            b'\t' => focused_row = (focused_row + 1) % 6,
            b' ' | b'\r' | b'\n' => {
                notice.clear();
                match focused_row {
                    0 => dropdown_open = !dropdown_open,
                    1 => show_older = !show_older,
                    2 => {
                        auto_update = !auto_update;
                        let _ =
                            rad_core::auto_update::set_auto_update_setting(&cfg_path, auto_update);
                    }
                    3 => eula_ok = !eula_ok,
                    4 => editing_local = true,
                    5 => {
                        let _ =
                            rad_core::auto_update::set_auto_update_setting(&cfg_path, auto_update);
                        let tag = filtered
                            .get(selected_rel_idx)
                            .map(|r| r.tag.clone())
                            .filter(|t| t != "latest");
                        if !local_path.trim().is_empty() {
                            return Ok(ConsoleSetupChoice::Local {
                                path: local_path.trim().trim_matches('"').to_owned(),
                                tag,
                            });
                        }
                        if !eula_ok {
                            notice = "Accept the Minecraft EULA first (Space on EULA row).".into();
                            focused_row = 3;
                            continue;
                        }
                        let _ = write_eula_file(&eula_path, true);
                        return Ok(ConsoleSetupChoice::Mojang(tag));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Err("Setup cancelled".into())
}

fn prompt_console_setup_plain(work_dir: &Path) -> Result<ConsoleSetupChoice, String> {
    println!(
        "  \x1b[38;2;232;125;13m[FIRST-RUN SETUP]\x1b[0m Fetching available RadByte releases..."
    );
    let all = match fetch_available_releases() {
        Ok(list) if !list.is_empty() => list,
        _ => vec![AvailableRelease {
            tag: option_env!("RADBYTE_RELEASE_TAG")
                .filter(|s| !s.is_empty())
                .unwrap_or("latest")
                .to_owned(),
            game_version: "26.3".to_owned(),
        }],
    };
    let cfg_path = work_dir.join("rad.properties");
    let mut auto_update = rad_core::auto_update::is_auto_update_enabled(&cfg_path);
    let mut show_older = false;
    let mut query = String::new();
    let stdin = std::io::stdin();
    loop {
        let filtered = filter_console_releases(&all, &query, show_older);
        let auto_mark = if auto_update { "[x] ON" } else { "[ ] OFF" };
        println!();
        println!(
            "  \x1b[1;37mAvailable versions\x1b[0m (older: {}, filter: {}, auto-update: \x1b[38;2;232;125;13m{auto_mark}\x1b[0m):",
            if show_older { "shown" } else { "hidden" },
            if query.is_empty() { "none" } else { &query }
        );
        if filtered.is_empty() {
            println!("    \x1b[90m(no releases match filter)\x1b[0m");
        } else {
            for (idx, rel) in filtered.iter().take(20).enumerate() {
                let marker = if idx == 0 {
                    "\x1b[38;2;232;125;13m*\x1b[0m"
                } else {
                    " "
                };
                println!(
                    "   {marker} \x1b[1;37m[{}]\x1b[0m Minecraft {}  \x1b[90m({})\x1b[0m",
                    idx + 1,
                    rel.game_version,
                    rel.tag
                );
            }
        }
        println!(
            "  \x1b[90mCommands: [Enter]=#1, [1..N]=select, [a]=toggle older, [u]=toggle auto-update, [s <text>]=search, [local <dir>]=local data\x1b[0m"
        );
        print!("  \x1b[38;2;232;125;13m>\x1b[0m Select version [1]: ");
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        stdin
            .lock()
            .read_line(&mut input)
            .map_err(|e| format!("Failed to read console input: {e}"))?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            let _ = rad_core::auto_update::set_auto_update_setting(&cfg_path, auto_update);
            let tag = filtered
                .first()
                .map(|r| r.tag.clone())
                .filter(|t| t != "latest");
            return Ok(ConsoleSetupChoice::Mojang(tag));
        }
        if trimmed.eq_ignore_ascii_case("a") || trimmed.eq_ignore_ascii_case("all") {
            show_older = !show_older;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("u") || trimmed.eq_ignore_ascii_case("update") {
            auto_update = !auto_update;
            let _ = rad_core::auto_update::set_auto_update_setting(&cfg_path, auto_update);
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("s ")
            .or_else(|| trimmed.strip_prefix("search "))
        {
            query = rest.trim().to_owned();
            continue;
        }
        if trimmed.eq_ignore_ascii_case("s") || trimmed.eq_ignore_ascii_case("search") {
            query.clear();
            continue;
        }
        if let Some(local_dir) = trimmed.strip_prefix("local ") {
            let path = local_dir.trim().trim_matches('"').to_owned();
            let tag = filtered
                .first()
                .map(|r| r.tag.clone())
                .filter(|t| t != "latest");
            return Ok(ConsoleSetupChoice::Local { path, tag });
        }
        if let Ok(num) = trimmed.parse::<usize>()
            && let Some(rel) = num.checked_sub(1).and_then(|i| filtered.get(i))
        {
            let tag = (rel.tag != "latest").then(|| rel.tag.clone());
            return Ok(ConsoleSetupChoice::Mojang(tag));
        }
        println!("  \x1b[31mInvalid selection.\x1b[0m");
    }
}

fn is_eula_file_accepted(eula_path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(eula_path) else {
        return false;
    };
    raw.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.starts_with('#')
            && trimmed.split_once('=').is_some_and(|(k, v)| {
                k.trim().eq_ignore_ascii_case("eula") && v.trim().eq_ignore_ascii_case("true")
            })
    })
}

fn write_eula_file(eula_path: &Path, accepted: bool) -> Result<(), String> {
    let content = format!(
        "# By changing the setting below to TRUE you are indicating your agreement to Mojang's EULA ({MINECRAFT_EULA_URL}).\neula={}\n",
        if accepted { "true" } else { "false" }
    );
    std::fs::write(eula_path, content)
        .map_err(|e| format!("failed to write {}: {e}", eula_path.display()))
}

fn ensure_eula_accepted(work_dir: &Path, import_dir: &Path) -> Result<(), String> {
    if std::env::args().skip(1).any(|a| a == "--accept-eula") {
        return Ok(());
    }
    for var in ["EULA", "RADBYTE_EULA"] {
        if let Ok(val) = std::env::var(var)
            && matches!(
                val.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes"
            )
        {
            return Ok(());
        }
    }
    let eula_path = work_dir.join("eula.txt");
    if is_eula_file_accepted(&eula_path) {
        return Ok(());
    }

    if std::io::stdin().is_terminal() {
        println!();
        println!("RadServer version 26.3 is not installed yet.");
        println!(
            "To automatically download the official Minecraft 26.3 server.jar and generate registry data,"
        );
        println!("you must agree to the Minecraft EULA: {MINECRAFT_EULA_URL}");
        print!("Do you agree to the Minecraft EULA? [yes/no]: ");
        let _ = std::io::stdout().flush();

        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_ok()
            && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        {
            write_eula_file(&eula_path, true)?;
            return Ok(());
        }
    }

    if !eula_path.exists() {
        let _ = write_eula_file(&eula_path, false);
    }
    Err(format!(
        "Minecraft 26.3 is not installed. Either:\n  1) Agree to {MINECRAFT_EULA_URL} by setting eula=true in {} (or EULA=TRUE in panel variables) to auto-install, or\n  2) Place blocks.json, items.json, entities.json, and menus.json into {}",
        eula_path.display(),
        import_dir.display()
    ))
}

fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Downloads official Minecraft `server.jar`, runs `--reports`, converts outputs, and streams progress lines.
pub fn install_from_mojang_with_progress(
    versions: &Path,
    tag_override: Option<&str>,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("RadServer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    tracing::info!("fetching Mojang version manifest for Minecraft 26.3");
    on_progress("Fetching Mojang version manifest for Minecraft 26.3...");
    let manifest: serde_json::Value = client
        .get(MOJANG_MANIFEST_URL)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot fetch Mojang version manifest: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;

    let version_url = manifest["versions"]
        .as_array()
        .and_then(|arr| arr.iter().find(|v| v["id"] == "26.3"))
        .and_then(|v| v["url"].as_str())
        .ok_or("Minecraft 26.3 was not found in Mojang's version manifest.")?;

    let version_meta: serde_json::Value = client
        .get(version_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot fetch Minecraft 26.3 metadata: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;

    let server_url = version_meta["downloads"]["server"]["url"]
        .as_str()
        .ok_or("Mojang metadata for 26.3 is missing downloads.server.url")?;
    let expected_sha1 = version_meta["downloads"]["server"]["sha1"]
        .as_str()
        .ok_or("Mojang metadata for 26.3 is missing downloads.server.sha1")?;

    std::fs::create_dir_all(versions).map_err(|e| e.to_string())?;
    let work = versions.join(format!(".rad-mojang-{}", std::process::id()));
    std::fs::create_dir(&work).map_err(|e| format!("Cannot create temporary dir: {e}"))?;

    let result = (|| -> Result<(), String> {
        let java_bin = resolve_or_fetch_portable_java(&client, &work, on_progress)?;

        tracing::info!("downloading official Minecraft 26.3 server.jar");
        on_progress("Downloading official Minecraft 26.3 server.jar...");
        let resp = client
            .get(server_url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Cannot download Minecraft server.jar: {e}"))?;
        let mut jar_bytes = Vec::new();
        resp.take(256 * 1024 * 1024 + 1)
            .read_to_end(&mut jar_bytes)
            .map_err(|e| e.to_string())?;

        let actual_sha1 = format!("{:x}", Sha1::digest(&jar_bytes));
        if jar_bytes.is_empty()
            || jar_bytes.len() > 256 * 1024 * 1024
            || actual_sha1 != expected_sha1
        {
            return Err(format!(
                "Downloaded server.jar failed SHA-1 verification (expected {expected_sha1}, got {actual_sha1})"
            ));
        }

        let jar_path = work.join("server.jar");
        std::fs::write(&jar_path, &jar_bytes).map_err(|e| e.to_string())?;
        drop(jar_bytes);

        let reports_out = work.join("gen");
        tracing::info!("running Mojang data generator (--reports) to build local registry tables");
        on_progress("Running Mojang data generator (--reports)...");
        let mut java_cmd = Command::new(&java_bin);
        hide_console_window(&mut java_cmd);
        let mut child = java_cmd
            .current_dir(&work)
            .arg("-DbundlerMainClass=net.minecraft.data.Main")
            .arg("-jar")
            .arg("server.jar")
            .arg("--reports")
            .arg("--output")
            .arg("gen")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to launch Java data generator: {e}"))?;

        let stderr_handle = child.stderr.take().map(|err| {
            std::thread::spawn(move || {
                let mut buf = String::new();
                let mut reader = std::io::BufReader::new(err);
                let _ = reader.read_to_string(&mut buf);
                buf
            })
        });

        if let Some(out) = child.stdout.take() {
            let reader = std::io::BufReader::new(out);
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    on_progress(trimmed);
                }
            }
        }

        let status = child
            .wait()
            .map_err(|e| format!("Failed to wait for Java data generator: {e}"))?;
        let stderr_text = stderr_handle
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        if !status.success() {
            let detail = stderr_text.lines().last().unwrap_or("").trim();
            return Err(if detail.is_empty() {
                format!("Minecraft data generator exited with status {status}")
            } else {
                format!("Minecraft data generator exited with status {status}: {detail}")
            });
        }

        on_progress("Converting Mojang registry reports...");
        let data_dir = work.join("data");
        convert_mojang_reports(&reports_out.join("reports"), &data_dir)?;
        on_progress("Downloading and installing RadByte version library...");
        install_with_source(
            versions,
            &data_dir,
            "generated locally via Mojang server.jar --reports after EULA acceptance",
            tag_override,
        )
    })();

    if work.exists() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result
}

fn adoptium_os_arch() -> Result<(&'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok(("windows", "x64")),
        ("windows", "aarch64") => Ok(("windows", "aarch64")),
        ("linux", "x86_64") if cfg!(target_env = "musl") => Ok(("alpine-linux", "x64")),
        ("linux", "aarch64") if cfg!(target_env = "musl") => Ok(("alpine-linux", "aarch64")),
        ("linux", "x86_64") => Ok(("linux", "x64")),
        ("linux", "aarch64") => Ok(("linux", "aarch64")),
        ("macos", "x86_64") => Ok(("mac", "x64")),
        ("macos", "aarch64") => Ok(("mac", "aarch64")),
        (os, arch) => Err(format!("Portable JRE unsupported on {os}/{arch}")),
    }
}

fn find_java_in_dir(dir: &Path) -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    for rel in [
        Path::new("bin").join(exe),
        Path::new("Contents").join("Home").join("bin").join(exe),
    ] {
        let p = dir.join(&rel);
        if p.is_file() {
            return Some(p);
        }
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let sub = entry.path();
        if sub.is_dir() {
            for rel in [
                Path::new("bin").join(exe),
                Path::new("Contents").join("Home").join("bin").join(exe),
            ] {
                let p = sub.join(&rel);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn resolve_or_fetch_portable_java(
    client: &reqwest::blocking::Client,
    work: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<std::path::PathBuf, String> {
    on_progress("Checking Java runtime...");
    let mut sys_cmd = Command::new("java");
    hide_console_window(&mut sys_cmd);
    let sys_ok = sys_cmd
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if sys_ok {
        return Ok(std::path::PathBuf::from("java"));
    }

    let (os, arch) = adoptium_os_arch()?;
    tracing::info!(
        os,
        arch,
        "system Java not found; querying Adoptium for newest portable Temurin JRE"
    );
    on_progress("Java not found; downloading portable Temurin JRE...");
    let releases: serde_json::Value = client
        .get("https://api.adoptium.net/v3/info/available_releases")
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot query Adoptium available_releases: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    for key in ["most_recent_feature_release", "most_recent_lts"] {
        if let Some(ver) = releases[key].as_u64()
            && !candidates.contains(&ver)
        {
            candidates.push(ver);
        }
    }
    if candidates.is_empty() {
        candidates.push(25);
    }

    let mut chosen_pkg = None;
    for feature_ver in candidates {
        let url = format!(
            "https://api.adoptium.net/v3/assets/latest/{feature_ver}/hotspot?image_type=jre&os={os}&arch={arch}&vendor=eclipse"
        );
        if let Ok(resp) = client.get(&url).send().and_then(|r| r.error_for_status())
            && let Ok(assets) = resp.json::<serde_json::Value>()
            && let Some(first) = assets.as_array().and_then(|a| a.first())
            && first["binary"]["package"]["link"].as_str().is_some()
        {
            tracing::info!(
                java_version = feature_ver,
                "selected newest portable Temurin JRE"
            );
            chosen_pkg = Some(first["binary"]["package"].clone());
            break;
        }
    }
    let pkg =
        chosen_pkg.ok_or("Adoptium API returned no portable JRE package for this platform")?;
    let download_url = pkg["link"]
        .as_str()
        .ok_or("Adoptium JRE package missing download link")?;
    let expected_sha256 = pkg["checksum"]
        .as_str()
        .ok_or("Adoptium JRE package missing SHA-256 checksum")?;
    let archive_name = pkg["name"].as_str().unwrap_or("jre.tar.gz");

    let resp = client
        .get(download_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot download portable JRE: {e}"))?;
    let mut bytes = Vec::new();
    resp.take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if bytes.is_empty() || bytes.len() > 256 * 1024 * 1024 || actual_sha256 != expected_sha256 {
        return Err("Downloaded portable JRE failed SHA-256 verification".into());
    }

    let archive_path = work.join(archive_name);
    std::fs::write(&archive_path, &bytes).map_err(|e| e.to_string())?;
    drop(bytes);

    let jre_dir = work.join("jre");
    std::fs::create_dir_all(&jre_dir).map_err(|e| e.to_string())?;
    on_progress("Extracting portable Temurin JRE...");
    let mut tar_cmd = Command::new("tar");
    hide_console_window(&mut tar_cmd);
    let extract = tar_cmd
        .arg("-xf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&jre_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Failed to run tar to extract portable JRE: {e}"))?;
    let _ = std::fs::remove_file(&archive_path);
    if !extract.success() {
        return Err(format!("Failed to extract portable JRE (exit {extract})"));
    }

    find_java_in_dir(&jre_dir)
        .ok_or_else(|| "Extracted portable JRE does not contain a java executable".into())
}

fn convert_mojang_reports(reports_dir: &Path, out_data_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out_data_dir).map_err(|e| e.to_string())?;
    let registries_raw = std::fs::read(reports_dir.join("registries.json"))
        .map_err(|e| format!("Cannot read generated registries.json: {e}"))?;
    let registries: serde_json::Value = serde_json::from_slice(&registries_raw)
        .map_err(|e| format!("Invalid generated registries.json: {e}"))?;

    let blocks_raw = std::fs::read(reports_dir.join("blocks.json"))
        .map_err(|e| format!("Cannot read generated blocks.json: {e}"))?;
    let blocks_report: serde_json::Value = serde_json::from_slice(&blocks_raw)
        .map_err(|e| format!("Invalid generated blocks.json: {e}"))?;

    let collect_registry = |reg_key: &str| -> Result<Vec<(String, i32)>, String> {
        let entries = registries[reg_key]["entries"]
            .as_object()
            .ok_or_else(|| format!("registries.json is missing {reg_key}.entries"))?;
        let mut out: Vec<(String, i32)> = entries
            .iter()
            .filter_map(|(name, v)| {
                v["protocol_id"]
                    .as_i64()
                    .and_then(|id| i32::try_from(id).ok())
                    .map(|id| (name.clone(), id))
            })
            .collect();
        out.sort_by_key(|(_, id)| *id);
        Ok(out)
    };

    let items = collect_registry("minecraft:item")?;
    let items_json = serde_json::json!({
        "entries": items.iter().map(|(name, id)| serde_json::json!({ "name": name, "id": id })).collect::<Vec<_>>()
    });
    std::fs::write(
        out_data_dir.join("items.json"),
        serde_json::to_vec(&items_json).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    for (reg_key, file_name) in [
        ("minecraft:entity_type", "entities.json"),
        ("minecraft:menu", "menus.json"),
    ] {
        let entries = collect_registry(reg_key)?;
        let json = serde_json::json!({
            "entries": entries.iter().map(|(name, id)| serde_json::json!({ "id": name, "id_num": id })).collect::<Vec<_>>()
        });
        std::fs::write(
            out_data_dir.join(file_name),
            serde_json::to_vec(&json).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }

    let block_ids: BTreeMap<String, u16> = collect_registry("minecraft:block")?
        .into_iter()
        .filter_map(|(name, id)| u16::try_from(id).ok().map(|u| (name, u)))
        .collect();
    let blocks_obj = blocks_report
        .as_object()
        .ok_or("generated blocks.json must be a JSON object")?;
    let mut converted_blocks = Vec::with_capacity(blocks_obj.len());
    for (name, def) in blocks_obj {
        let block_id = *block_ids
            .get(name)
            .ok_or_else(|| format!("block '{name}' missing from minecraft:block registry"))?;
        let states = def["states"]
            .as_array()
            .ok_or_else(|| format!("block '{name}' has no states array"))?;
        let mut min_state = u16::MAX;
        let mut max_state = 0u16;
        let mut default_state = None;
        for st in states {
            let sid = st["id"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| format!("invalid state id for block '{name}'"))?;
            min_state = min_state.min(sid);
            max_state = max_state.max(sid);
            if st["default"].as_bool() == Some(true) {
                default_state = Some(sid);
            }
        }
        if min_state > max_state {
            return Err(format!("block '{name}' has empty states"));
        }
        converted_blocks.push((
            block_id,
            serde_json::json!({
                "block_id": block_id,
                "name": name,
                "default_state": default_state.unwrap_or(min_state),
                "min_state": min_state,
                "max_state": max_state,
            }),
        ));
    }
    converted_blocks.sort_by_key(|(id, _)| *id);
    let blocks_json = serde_json::json!({
        "blocks": converted_blocks.into_iter().map(|(_, v)| v).collect::<Vec<_>>()
    });
    std::fs::write(
        out_data_dir.join("blocks.json"),
        serde_json::to_vec(&blocks_json).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_mojang_reports_to_radbyte_schema() {
        let root = std::env::temp_dir().join(format!("rad-reports-test-{}", std::process::id()));
        let reports = root.join("reports");
        let out = root.join("out");
        std::fs::create_dir_all(&reports).unwrap();

        std::fs::write(
            reports.join("registries.json"),
            r#"{
              "minecraft:block": { "entries": { "minecraft:air": { "protocol_id": 0 }, "minecraft:stone": { "protocol_id": 1 } } },
              "minecraft:item": { "entries": { "minecraft:air": { "protocol_id": 0 }, "minecraft:stone": { "protocol_id": 1 } } },
              "minecraft:entity_type": { "entries": { "minecraft:pig": { "protocol_id": 94 } } },
              "minecraft:menu": { "entries": { "minecraft:generic_9x3": { "protocol_id": 2 } } }
            }"#,
        )
        .unwrap();
        std::fs::write(
            reports.join("blocks.json"),
            r#"{
              "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
              "minecraft:stone": { "states": [ { "id": 1, "default": true } ] }
            }"#,
        )
        .unwrap();

        convert_mojang_reports(&reports, &out).unwrap();
        let blocks: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("blocks.json")).unwrap()).unwrap();
        assert_eq!(blocks["blocks"][1]["name"], "minecraft:stone");
        assert_eq!(blocks["blocks"][1]["default_state"], 1);
        let entities: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("entities.json")).unwrap()).unwrap();
        assert_eq!(entities["entries"][0]["id"], "minecraft:pig");
        assert_eq!(entities["entries"][0]["id_num"], 94);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn eula_file_parsing_respects_comments_and_case() {
        let root = std::env::temp_dir().join(format!("rad-eula-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let p = root.join("eula.txt");
        write_eula_file(&p, false).unwrap();
        assert!(!is_eula_file_accepted(&p));
        write_eula_file(&p, true).unwrap();
        assert!(is_eula_file_accepted(&p));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "requires a tagged release and locally supplied version data"]
    fn installs_and_validates_a_real_release_package() {
        let source = std::env::var_os("RADBYTE_TEST_DATA_DIR").expect("set RADBYTE_TEST_DATA_DIR");
        let root = std::env::temp_dir().join(format!("rad-install-test-{}", std::process::id()));
        assert!(!root.exists());
        let result = install(&root, Path::new(&source));
        if result.is_ok() {
            let pkg = LoadedVersionPackage::load_from_versions_dir(&root, "26.3").unwrap();
            pkg.verify_and_resolve_library_path().unwrap();
            assert!(install(&root, Path::new(&source)).is_err());
        }
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        result.unwrap();
    }
}
