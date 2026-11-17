use rad_core::version_package::{
    LoadedVersionPackage, VersionLibraryMeta, VersionManifest, VersionPackageFile,
};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn verify_selection(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let parsed: toml::Value = toml::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Invalid {}: {e}", path.display()))?;
    if parsed.get("version").and_then(toml::Value::as_str) != Some("26.3") {
        return Err("Existing version.toml does not select 26.3; it was not changed".into());
    }
    Ok(())
}

fn utc_now() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let days = (seconds / 86400) as i64;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let day_seconds = seconds % 86400;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3600,
        day_seconds / 60 % 60,
        day_seconds % 60
    ))
}

pub fn create(data: &Path, library: &Path, versions: &Path) -> Result<(), String> {
    let dest = versions.join("26.3");
    if dest.exists() {
        return Err(format!(
            "{} already exists; it was not changed",
            dest.display()
        ));
    }
    let name = library
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid library filename")?;
    let lib_bytes =
        std::fs::read(library).map_err(|e| format!("Cannot read {}: {e}", library.display()))?;
    if lib_bytes.is_empty() || lib_bytes.len() > 256 * 1024 * 1024 {
        return Err("Invalid library size".into());
    }
    let mut input = Vec::new();
    for (id, key) in [
        ("items", "entries"),
        ("blocks", "blocks"),
        ("entities", "entries"),
        ("menus", "entries"),
        ("tags", ""),
    ] {
        let path = data.join(format!("{id}.json"));
        if id == "tags" && !path.exists() {
            continue;
        }
        let bytes =
            std::fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(format!("{} is too large", path.display()));
        }
        let json: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid {}: {e}", path.display()))?;
        let count = if id == "tags" {
            ["block_tags", "item_tags"]
                .iter()
                .map(|k| json[*k].as_array().map_or(0, Vec::len))
                .sum::<usize>()
                + json["core_registry_tags"].as_array().map_or(0, |sets| {
                    sets.iter()
                        .map(|set| set["tags"].as_array().map_or(0, Vec::len))
                        .sum()
                })
        } else {
            json[key].as_array().map_or(0, Vec::len)
        };
        if id != "tags" && count == 0 {
            return Err(format!("{} has no entries", path.display()));
        }
        input.push((id, bytes, count));
    }
    std::fs::create_dir_all(versions).map_err(|e| e.to_string())?;
    let selection = versions.join("version.toml");
    if selection.exists() {
        verify_selection(&selection)?;
    }
    let stage = versions.join(format!(".rad-install-{}", std::process::id()));
    std::fs::create_dir(&stage).map_err(|e| format!("Cannot create {}: {e}", stage.display()))?;
    let result = (|| -> Result<(), String> {
        std::fs::create_dir(stage.join("data")).map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for (id, bytes, count) in input {
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
        std::fs::write(stage.join(name), &lib_bytes).map_err(|e| e.to_string())?;
        let manifest = VersionManifest {
            schema_version: 1,
            game_version: "26.3".into(),
            protocol_version: 777,
            package_format_version: 1,
            world_data_version: 4790,
            source: "local user-supplied data; provenance not verified".into(),
            generated_utc: utc_now()?,
            notes: None,
            library: Some(VersionLibraryMeta {
                path: name.into(),
                abi_version: 1,
                target_os: std::env::consts::OS.into(),
                target_arch: std::env::consts::ARCH.into(),
                size_bytes: lib_bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&lib_bytes)),
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
                if let Err(e) = file.write_all(b"version = \"26.3\"\n") {
                    drop(file);
                    let _ = std::fs::remove_file(&selection);
                    return Err(e.to_string());
                }
                created_selection = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                verify_selection(&selection)?
            }
            Err(e) => return Err(e.to_string()),
        }
        if let Err(e) = std::fs::rename(&stage, &dest) {
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
    result?;
    println!("Installed version package: {}", dest.display());
    Ok(())
}
