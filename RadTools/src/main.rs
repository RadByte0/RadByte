//! Cross-platform developer and release tooling for RadByte.
mod provenance;
mod release;
mod version_package;

use std::path::PathBuf;

fn usage() -> &'static str {
    "Usage: rad-tools stage-release <target> [build-directory] [output-directory]\n\
     rad-tools release-notes <repository> <tag> [assets-directory] [output-file]\n\
     rad-tools package-version <data-directory> <library-file> <versions-directory>\n\
     rad-tools audit-provenance [project-directory]"
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rad-tools: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let command = args
        .first()
        .and_then(|s| s.to_str())
        .ok_or_else(|| usage().to_string())?;
    match command {
        "stage-release" if (2..=4).contains(&args.len()) => {
            let target = args[1].to_str().ok_or("Target must be UTF-8")?;
            let build = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("target").join(target).join("release"));
            let out = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("release-assets"));
            release::stage(target, &build, &out)
        }
        "release-notes" if (3..=5).contains(&args.len()) => {
            let repo = args[1].to_str().ok_or("Repository must be UTF-8")?;
            let tag = args[2].to_str().ok_or("Tag must be UTF-8")?;
            let assets = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("release-assets"));
            let output = args
                .get(4)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("release-notes.md"));
            release::notes(repo, tag, &assets, &output)
        }
        "package-version" if args.len() == 4 => version_package::create(
            &PathBuf::from(&args[1]),
            &PathBuf::from(&args[2]),
            &PathBuf::from(&args[3]),
        ),
        "audit-provenance" if args.len() <= 2 => {
            let root = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            provenance::audit(&root)
        }
        _ => Err(usage().into()),
    }
}
