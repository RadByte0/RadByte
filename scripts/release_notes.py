#!/usr/bin/env python3
"""Generate expandable GitHub Release links from the staged assets."""

import os
from pathlib import Path
from urllib.parse import quote

from stage_github_release import TARGETS


def main() -> None:
    repository = os.environ["RELEASE_REPOSITORY"]
    tag = quote(os.environ["RELEASE_TAG"], safe="")
    base = f"https://github.com/{repository}/releases/download/{tag}"
    assets = Path("release-assets")

    def link(label: str, name: str) -> str:
        if not (assets / name).is_file():
            raise FileNotFoundError(f"Missing release asset: {name}")
        return f"[{label}]({base}/{quote(name, safe='')})"

    lines = [
        "Download the three launchers for your operating system and architecture.",
        "RadServer requires compatible, locally supplied version data; see the repository README.", "",
    ]
    labels = {"windows": "Windows", "linux": "Linux", "darwin": "macOS"}
    for service in ("server", "proxy", "limbo"):
        lines += ["<details>", f"<summary>Rad{service.title()}</summary>", ""]
        for target, (suffix, _, _) in TARGETS.items():
            os_name = next(name for key, name in labels.items() if key in target)
            arch = "x64" if target.startswith("x86_64") else "ARM64"
            asset = link("Download", f"rad-{service}-{target}{suffix}")
            checksum = link("SHA-256", f"SHA256SUMS-{target}.txt")
            lines.append(f"- **{os_name} {arch}:** {asset} · {checksum}")
        lines += ["", "</details>", ""]
    lines += ["<details>", "<summary>Version libraries</summary>", "", "**Minecraft 26.3**", ""]
    for target, (_, library, libext) in TARGETS.items():
        os_name = next(name for key, name in labels.items() if key in target)
        arch = "x64" if target.startswith("x86_64") else "ARM64"
        asset = link("Download", f"{library}-{target}{libext}")
        checksum = link("SHA-256", f"SHA256SUMS-{target}.txt")
        lines.append(f"- **{os_name} {arch}:** {asset} · {checksum}")
    lines += ["", "</details>", "", link("Version package setup script", "generate_version_package.ps1"), ""]
    Path("release-notes.md").write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    main()
