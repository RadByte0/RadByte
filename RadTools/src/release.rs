use std::path::Path;

const TARGETS: [(&str, &str, &str, &str); 6] = [
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
        ("Version libraries", "version"),
    ] {
        text.push_str(&format!("<details>\n<summary>{title}</summary>\n\n"));
        if service == "version" {
            text.push_str("**Minecraft 26.3**\n\n");
        }
        for (target, exe, _, libext) in TARGETS {
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
            let file = if service == "version" {
                format!("rad-version-26.3-{target}{libext}")
            } else {
                format!("rad-{service}-{target}{exe}")
            };
            if !assets.join(&file).is_file() {
                return Err(format!("Missing release asset: {file}"));
            }
            text.push_str(&format!("- **{os} {arch}:** [Download]({base}/{file})\n"));
        }
        text.push_str("\n</details>\n\n");
    }
    std::fs::write(output, text).map_err(|e| e.to_string())
}
