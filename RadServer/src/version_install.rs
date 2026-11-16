//! Install the supported version using locally supplied data and a release library.
use rad_core::version_package::{
    LoadedVersionPackage, VersionLibraryMeta, VersionManifest, VersionPackageFile,
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, time::Duration};

#[repr(C)]
#[derive(Default)]
struct SystemTime {
    year: u16,
    month: u16,
    weekday: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millisecond: u16,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemTime(time: *mut SystemTime);
}

fn utc_now() -> String {
    let mut time = SystemTime::default();
    unsafe {
        GetSystemTime(&mut time);
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

pub fn install(versions: &Path, data: &Path) -> Result<(), String> {
    let selection = versions.join("version.toml");
    if selection.exists() {
        let selected = crate::version_selection::load(versions)?;
        if selected != "26.3" {
            return Err(format!(
                "The existing version.toml selects {selected}. Change it to 26.3 before installing."
            ));
        }
    }
    let tag = option_env!("RADBYTE_RELEASE_TAG")
        .filter(|tag| !tag.is_empty())
        .or(Some(concat!("v", env!("CARGO_PKG_VERSION"))))
        .filter(|tag| {
            tag.starts_with('v')
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        })
        .ok_or("Installation is available in GitHub release builds only.")?;
    let repo = option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte");
    if !repo
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/.-_".contains(&b))
    {
        return Err("Invalid release repository".into());
    }
    let target = match std::env::consts::ARCH {
        "x86_64" => "x86_64-pc-windows-msvc",
        "aarch64" => "aarch64-pc-windows-msvc",
        _ => return Err("Unsupported architecture".into()),
    };
    let preferred_name = format!("rad-version-26.3-{target}.dll");
    let legacy_name = format!("rad_version_26_3-{target}.dll");
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
            source: "local user-supplied data; provenance not verified".into(),
            generated_utc: utc_now(),
            notes: None,
            library: Some(VersionLibraryMeta {
                path: name,
                abi_version: 1,
                target_os: "windows".into(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
