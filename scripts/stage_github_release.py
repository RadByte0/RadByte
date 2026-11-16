#!/usr/bin/env python3
"""Stage platform-specific binaries as individually downloadable release assets."""

import argparse
import shutil
from pathlib import Path


TARGETS = {
    "x86_64-pc-windows-msvc": (".exe", "rad_version_26_3", ".dll"),
    "aarch64-pc-windows-msvc": (".exe", "rad_version_26_3", ".dll"),
    "x86_64-unknown-linux-gnu": ("", "librad_version_26_3", ".so"),
    "aarch64-unknown-linux-gnu": ("", "librad_version_26_3", ".so"),
    "x86_64-apple-darwin": ("", "librad_version_26_3", ".dylib"),
    "aarch64-apple-darwin": ("", "librad_version_26_3", ".dylib"),
}


def stage(target: str, build_dir: Path, output_dir: Path) -> None:
    exe_suffix, library_name, library_suffix = TARGETS[target]
    files = [
        (f"rad-{service}{exe_suffix}", f"rad-{service}-{target}{exe_suffix}")
        for service in ("server", "proxy", "limbo")
    ]
    files.append((f"{library_name}{library_suffix}", f"{library_name}-{target}{library_suffix}"))
    for source, _ in files:
        if not (build_dir / source).is_file():
            raise FileNotFoundError(f"Missing build output: {build_dir / source}")
    output_dir.mkdir(parents=True, exist_ok=True)
    for source, destination in files:
        staged = output_dir / destination
        shutil.copy2(build_dir / source, staged)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--build-dir", type=Path)
    parser.add_argument("--output-dir", type=Path, default=Path("release-assets"))
    args = parser.parse_args()
    stage(args.target, args.build_dir or Path("target") / args.target / "release", args.output_dir)


if __name__ == "__main__":
    main()
