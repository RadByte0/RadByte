use std::path::Path;

const TARGETS: [(&str, &str, &str, &str); 8] = [
    ("x86_64-pc-windows-msvc", ".exe", "rad_version_26_3", ".dll"),
    (
        "aarch64-pc-windows-msvc",
        ".exe",
        "rad_version_26_3",
        ".dll",
    ),
    ("x86_64-unknown-linux-gnu", "", "librad_version_26_3", ".so"),
    (
        "aarch64-unknown-linux-gnu",
        "",
        "librad_version_26_3",
        ".so",
    ),
    (
        "x86_64-unknown-linux-musl",
        "",
        "librad_version_26_3",
        ".so",
    ),
    (
        "aarch64-unknown-linux-musl",
        "",
        "librad_version_26_3",
        ".so",
    ),
    ("x86_64-apple-darwin", "", "librad_version_26_3", ".dylib"),
    ("aarch64-apple-darwin", "", "librad_version_26_3", ".dylib"),
];

pub fn stage(target: &str, build: &Path, out: &Path) -> Result<(), String> {
    let (_, exe, library, ext) = TARGETS
        .iter()
        .find(|t| t.0 == target)
        .ok_or("Unsupported release target")?;
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
        for (target, exe, _, _) in TARGETS {
            let os = if target.contains("windows") {
                "Windows"
            } else if target.contains("linux") {
                "Linux"
            } else {
                "macOS"
            };
            let arch = if target.starts_with("x86_64") {
                "x64"
            } else {
                "ARM64"
            };
            let abi = if target.contains("musl") {
                " (musl)"
            } else {
                ""
            };
            let file = format!("rad-{service}-{target}{exe}");
            if !assets.join(&file).is_file() {
                return Err(format!("Missing release asset: {file}"));
            }
            text.push_str(&format!(
                "- **{os} {arch}{abi}:** [Download]({base}/{file})\n"
            ));
        }
        text.push_str("\n</details>\n\n");
    }
    std::fs::write(output, text).map_err(|e| e.to_string())
}
