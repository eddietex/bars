#!/usr/bin/env python3
"""Writes latest.json, the manifest the in-app updater reads from the latest
release (crates/bars-core/src/update.rs), from a release's signed update
files: the macOS .app.tar.gz and the Windows -setup.exe, each with its .sig.
Without any signature (no updater key) nothing is written.

    scripts/update-manifest.py v0.3.1 artifacts
"""
import datetime
import json
import pathlib
import re
import sys

REPO = "hink/bars"
# File name -> (updater target, format). Arches as the updater names them.
PATTERNS = [
    (re.compile(r"_(aarch64|x86_64)\.app\.tar\.gz$"), "macos", "app"),
    (re.compile(r"_(x64|arm64)-setup\.exe$"), "windows", "nsis"),
]
ARCH = {"x64": "x86_64", "arm64": "aarch64"}

tag, folder = sys.argv[1], pathlib.Path(sys.argv[2])
platforms = {}
for path in sorted(folder.iterdir()):
    for pattern, os_name, fmt in PATTERNS:
        m = pattern.search(path.name)
        sig = path.with_name(path.name + ".sig")
        if m and sig.exists():
            arch = ARCH.get(m.group(1), m.group(1))
            platforms[f"{os_name}-{arch}"] = {
                "url": f"https://github.com/{REPO}/releases/download/{tag}/{path.name}",
                "signature": sig.read_text().strip(),
                "format": fmt,
            }

if not platforms:
    print("no signed update files; latest.json not written", file=sys.stderr)
    sys.exit(0)
manifest = {
    "version": tag.removeprefix("v"),
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": platforms,
}
(folder / "latest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"latest.json: {', '.join(platforms)}", file=sys.stderr)
