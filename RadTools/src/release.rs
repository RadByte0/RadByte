use std::path::Path;

struct TargetInfo {
    triple: &'static str,
    exe: &'static str,
    library: &'static str,
    ext: &'static str,
    label: &'static str,
    is_other: bool,
}

const TARGETS: [TargetInfo; 11] = [
    TargetInfo {
        triple: "x86_64-pc-windows-msvc",
        exe: ".exe",
        library: "rad_version_26_3",
        ext: ".dll",
        label: "Windows x64",
        is_other: false,
    },
    TargetInfo {
        triple: "aarch64-pc-windows-msvc",
        exe: ".exe",
        library: "rad_version_26_3",
        ext: ".dll",
        label: "Windows ARM64",
        is_other: false,
    },
    TargetInfo {
        triple: "x86_64-unknown-linux-gnu",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux x64",
        is_other: false,
    },
    TargetInfo {
        triple: "aarch64-unknown-linux-gnu",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux ARM64",
        is_other: false,
    },
    TargetInfo {
        triple: "x86_64-unknown-linux-musl",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux x64 (musl)",
        is_other: false,
    },
    TargetInfo {
        triple: "aarch64-unknown-linux-musl",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux ARM64 (musl)",
        is_other: false,
    },
    TargetInfo {
        triple: "x86_64-apple-darwin",
        exe: "",
        library: "librad_version_26_3",
        ext: ".dylib",
        label: "macOS x64",
        is_other: false,
    },
    TargetInfo {
        triple: "aarch64-apple-darwin",
        exe: "",
        library: "librad_version_26_3",
        ext: ".dylib",
        label: "macOS ARM64",
        is_other: false,
    },
    TargetInfo {
        triple: "riscv64gc-unknown-linux-musl",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux RISC-V 64 (musl)",
        is_other: true,
    },
    TargetInfo {
        triple: "powerpc64le-unknown-linux-musl",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux POWER64LE (musl)",
        is_other: true,
    },
    TargetInfo {
        triple: "i686-unknown-linux-musl",
        exe: "",
        library: "librad_version_26_3",
        ext: ".so",
        label: "Linux x86 32-bit (musl)",
        is_other: true,
    },
];

pub fn stage(target: &str, build: &Path, out: &Path) -> Result<(), String> {
    let info = TARGETS
        .iter()
        .find(|t| t.triple == target)
        .ok_or("Unsupported release target")?;
    let exe = info.exe;
    let library = info.library;
    let ext = info.ext;
    let mut files: Vec<(String, String)> = ["server", "proxy", "limbo"]
        .iter()
        .map(|service| {
            (
                format!("rad-{service}{exe}"),
                format!("rad-{service}-{target}{exe}"),
            )
        })
        .collect();
    files.push((
        format!("{library}{ext}"),
        format!("rad-version-26.3-{target}{ext}"),
    ));
    let api_lib = if library.starts_with("lib") {
        "librad_api"
    } else {
        "rad_api"
    };
    files.push((
        format!("{api_lib}{ext}"),
        format!("rad-api-v1-{target}{ext}"),
    ));
    for (src, _) in &files {
        if !build.join(src).is_file() {
            return Err(format!(
                "Missing build output: {}",
                build.join(src).display()
            ));
        }
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for (src, dst) in files {
        std::fs::copy(build.join(src), out.join(dst)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn notes(repository: &str, tag: &str, assets: &Path, output: &Path) -> Result<(), String> {
    if !repository
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Invalid repository or tag".into());
    }
    let base = format!("https://github.com/{repository}/releases/download/{tag}");
    let mut text = String::from(
        "Download the three launchers for your operating system and architecture.\nRadServer requires compatible, locally supplied version data; see the repository README.\n\n",
    );
    for (title, service) in [
        ("RadServer", "server"),
        ("RadProxy", "proxy"),
        ("RadLimbo", "limbo"),
    ] {
        text.push_str(&format!("<details>\n<summary>{title}</summary>\n\n"));
        for info in TARGETS.iter().filter(|t| !t.is_other) {
            let file = format!("rad-{service}-{}{}", info.triple, info.exe);
            if !assets.join(&file).is_file() {
                return Err(format!("Missing release asset: {file}"));
            }
            text.push_str(&format!(
                "- **{}:** [Download]({base}/{file})\n",
                info.label
            ));
        }
        text.push_str("\n<details>\n<summary>Other</summary>\n\n");
        for info in TARGETS.iter().filter(|t| t.is_other) {
            let file = format!("rad-{service}-{}{}", info.triple, info.exe);
            if !assets.join(&file).is_file() {
                return Err(format!("Missing release asset: {file}"));
            }
            text.push_str(&format!(
                "- **{}:** [Download]({base}/{file})\n",
                info.label
            ));
        }
        text.push_str("\n</details>\n\n</details>\n\n");
    }
    std::fs::write(output, text).map_err(|e| e.to_string())
}
