#!/usr/bin/env python3
"""Writes THIRD_PARTY_NOTICES.txt: the licenses of everything Bars ships
that others wrote. That's every Rust crate the app is built from on the
platforms it's released for (normal dependencies, not build or dev ones),
with the license files each crate comes with, including those of C/C++
code vendored in -sys crates (llama.cpp), plus what isn't a crate: ONNX
Runtime (linked in from a prebuilt library) and the Inter font. Crates
that come without a license file get the standard text of the licenses
they declare (packaging/notices/licenses).

Run it after changing dependencies: python3 scripts/third-party-notices.py
"""

import json
import os
import re
import subprocess
import sys
import textwrap

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD_PARTY_NOTICES.txt")
# The platforms Bars is released for (see .github/workflows/release.yml).
TARGETS = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)([-._].*)?$", re.I)
# Named like a license, but code (llama.cpp's cmake/license.cmake).
CODE = (".cmake", ".rs", ".py", ".c", ".h", ".cpp", ".hpp", ".js", ".ts", ".json", ".toml", ".yml", ".yaml", ".html", ".sh")
# In -sys crates, vendored code's licenses can sit a level or two down;
# these directories hold tests and examples, not shipped code.
SKIP_DIRS = {"tests", "test", "examples", "docs", "doc", "benches", "fuzz", "ci", ".github"}

OTHERS = [
    (
        "ONNX Runtime 1.28.0 (runs the speech models; linked into Bars)",
        "https://github.com/microsoft/onnxruntime",
        ["packaging/notices/onnxruntime-LICENSE.txt", "packaging/notices/onnxruntime-ThirdPartyNotices.txt"],
    ),
    (
        "Inter 4.0 (the font Bars' windows use)",
        "https://github.com/rsms/inter",
        ["crates/bars/assets/fonts/LICENSE.txt"],
    ),
]


def metadata(target):
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def shipped(meta):
    """Ids of the packages the bars app is built from, but Bars' own."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    app = next(p["id"] for p in meta["packages"] if p["name"] == "bars" and p["id"] in members)
    seen, stack = set(), [app]
    while stack:
        node = nodes[stack.pop()]
        for dep in node["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return seen - members


def license_files(package):
    root = os.path.dirname(package["manifest_path"])
    found = []
    if package.get("license_file"):
        found.append(os.path.normpath(os.path.join(root, package["license_file"])))
    depth = 3 if package["name"].endswith("-sys") or package["name"].startswith("llama-cpp") else 1
    for dirpath, dirnames, filenames in os.walk(root):
        level = os.path.relpath(dirpath, root).count(os.sep) + (dirpath != root)
        dirnames[:] = [d for d in dirnames if d.lower() not in SKIP_DIRS and not d.startswith(".") and level + 1 < depth]
        found += [os.path.join(dirpath, f) for f in filenames if LICENSE_FILE.match(f) and not f.lower().endswith(CODE)]
    return sorted(set(f for f in found if os.path.isfile(f)))


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().replace("\r\n", "\n").strip()


def main():
    packages = {}
    for target in TARGETS:
        meta = metadata(target)
        by_id = {p["id"]: p for p in meta["packages"]}
        for pid in shipped(meta):
            packages[pid] = by_id[pid]

    # Each license text once (the same but for spacing counts as the same),
    # with the crates that come with it.
    crates = sorted(packages.values(), key=lambda p: (p["name"], p["version"]))
    texts, missing = {}, []
    for p in crates:
        root = os.path.dirname(p["manifest_path"])
        found = False
        for path in license_files(p):
            text = read(path)
            if not text:
                continue
            found = True
            rel = os.path.relpath(path, root)
            label = f"{p['name']} {p['version']}" + (f" ({rel})" if os.sep in rel else "")
            texts.setdefault(" ".join(text.split()), (text, []))[1].append(label)
        if not found:
            missing.append(p)

    out = []
    out.append("Third-party software in Bars")
    out.append("============================")
    out.append("")
    out.append("Bars is open source under the MIT license (see LICENSE). It is built")
    out.append("from, and includes, the software below, which its authors license as")
    out.append("given. The speech and polish models aren't part of Bars: they download")
    out.append("separately, with their own licenses.")
    out.append("")
    for name, url, files in OTHERS:
        out.append("=" * 78)
        out.append(name)
        out.append(url)
        out.append("=" * 78)
        for f in files:
            out.append("")
            out.append(read(os.path.join(ROOT, f)))
        out.append("")
    out.append("=" * 78)
    out.append("Rust crates, and the licenses they declare")
    out.append("=" * 78)
    out.append("")
    for p in crates:
        line = f"{p['name']} {p['version']} ({p.get('license') or 'see its license file'})"
        if p.get("repository"):
            line += f" {p['repository']}"
        out.append(line)
    out.append("")
    out.append("=" * 78)
    out.append("The crates' license texts")
    out.append("=" * 78)
    for text, labels in sorted(texts.values(), key=lambda t: t[1][0]):
        out.append("")
        out.append("-" * 78)
        out += textwrap.wrap("For " + ", ".join(labels) + ":", 78, break_on_hyphens=False)
        out.append("-" * 78)
        out.append("")
        out.append(text)
    if missing:
        out.append("")
        out.append("-" * 78)
        out.append("These crates come without a license file. Each is under the license it")
        out.append("declares, copyright its authors; the standard texts of those licenses follow.")
        out.append("-" * 78)
        ids = set()
        for p in missing:
            authors = ", ".join(re.sub(r"\s*<[^>]*>", "", a) for a in p.get("authors") or []) or f"the {p['name']} authors"
            out.append(f"{p['name']} {p['version']} ({p.get('license') or 'unspecified'}), copyright {authors}")
            ids |= set(re.findall(r"[A-Za-z0-9.+-]+", p.get("license") or "")) - {"OR", "AND", "WITH"}
        for spdx in sorted(ids):
            path = os.path.join(ROOT, "packaging", "notices", "licenses", spdx + ".txt")
            if not os.path.exists(path):
                # Every crate offering it also offers one given here.
                print(f"  no standard text for {spdx}", file=sys.stderr)
                continue
            out.append("")
            out.append(f"[{spdx}]")
            out.append(read(path))
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print(f"{OUT}: {len(packages)} crates, {len(texts)} license texts, {len(missing)} without a file", file=sys.stderr)


if __name__ == "__main__":
    main()
