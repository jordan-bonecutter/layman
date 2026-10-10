#!/usr/bin/env python3
"""Install Layman's launcher and logo for the current user (no root needed)."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess


def exec_value(path):
    # Desktop Entry string escapes are decoded before Exec argument quoting.
    value = str(path)
    if any(ord(c) < 32 for c in value) or "=" in value or "%" in value:
        raise ValueError("Binary path cannot contain control characters, '=' or '%'.")
    quoted = '"' + "".join("\\" + c if c in '\\"`$' else c for c in value) + '"'
    return quoted.replace("\\", "\\\\")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", help="Layman binary path (default: PATH or ~/.cargo/bin/layman)")
    args = parser.parse_args()
    binary = Path(args.binary or shutil.which("layman") or Path.home() / ".cargo/bin/layman").expanduser().resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"No executable at {binary}. Run 'cargo install --path . --locked' first, or supply its path.")
    try:
        command = exec_value(binary)
    except ValueError as error:
        parser.error(str(error))

    data_home = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share").expanduser()
    if not data_home.is_absolute():
        parser.error("XDG_DATA_HOME must be an absolute path.")
    packaging = Path(__file__).resolve().parent
    applications = data_home / "applications"
    icons = data_home / "icons/hicolor"
    icon_dir = icons / "scalable/apps"
    applications.mkdir(parents=True, exist_ok=True)
    icon_dir.mkdir(parents=True, exist_ok=True)

    desktop = applications / "dev.layman.Layman.desktop"
    template = (packaging / desktop.name).read_text(encoding="utf-8")
    desktop.write_text(template.replace("Exec=layman\n", f"Exec={command}\n"), encoding="utf-8")
    desktop.chmod(0o644)
    icon = icon_dir / "dev.layman.Layman.svg"
    shutil.copyfile(packaging / icon.name, icon)
    icon.chmod(0o644)

    for program, options in [
        ("update-desktop-database", [str(applications)]),
        ("gtk-update-icon-cache", ["--force", "--ignore-theme-index", str(icons)]),
    ]:
        if executable := shutil.which(program):
            result = subprocess.run([executable, *options], capture_output=True, text=True)
            if result.returncode:
                print(f"Optional cache refresh failed: {program}: {result.stderr.strip()}")
    print(f"Installed launcher: {desktop}\nInstalled logo: {icon}\nLaunches: {binary}")


if __name__ == "__main__":
    main()
