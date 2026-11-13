#!/usr/bin/env python3
"""Collect license texts of third-party Rust crates into one file.
Usage: python tools/third_party.py OUT_FILE   (from the noveltl-rs folder)"""
import json, os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKIP = {"src", "tests", "benches", "examples", "target"}
KEYS = ("LICEN", "COPYING", "NOTICE", "COPYRIGHT")
NO_TEXT = {"CC0-1.0", "Unlicense"}  # public domain: nothing to reproduce


def is_license(name, folder):
    up = name.upper()
    if any(k in up for k in KEYS) or up.startswith(("OFL", "UFL")):
        return True
    return folder == "fonts" and up.endswith(".TXT")


def is_text(p):
    # e.g. winapi's libwinapi_oemlicense.a matches by name but is a binary
    with open(p, "rb") as f:
        return b"\0" not in f.read(8192)


def find_files(pkg_dir):
    found = []
    for name in sorted(os.listdir(pkg_dir)):
        p = os.path.join(pkg_dir, name)
        if os.path.isfile(p):
            if is_license(name, "") and is_text(p):
                found.append(p)
        elif name not in SKIP and os.path.isdir(p):
            for sub in sorted(os.listdir(p)):
                sp = os.path.join(p, sub)
                if os.path.isfile(sp) and is_license(sub, name) and is_text(sp):
                    found.append(sp)
    return found


def read(p):
    with open(p, encoding="utf-8", errors="replace") as f:
        return f.read().replace("\r\n", "\n").replace("\r", "\n").strip()


def spdx_parts(lic):
    return [x for x in (lic or "").replace("(", " ").replace(")", " ").replace("/", " OR ").split()
            if x not in ("OR", "AND", "WITH")]


def main():
    out = sys.argv[1]
    r = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"],
                       cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    if r.returncode:
        sys.stderr.write(r.stderr)
        return r.returncode or 1
    pkgs = sorted((p for p in json.loads(r.stdout)["packages"] if p.get("source")),
                  key=lambda p: (p["name"], p["version"]))

    texts = {}      # text -> [pkg labels]
    by_lic = {}     # spdx token -> [(filename, text)] from other packages
    missing = []    # packages without files
    for p in pkgs:
        label = f'{p["name"]} {p["version"]}'
        files = find_files(os.path.dirname(p["manifest_path"]))
        if not files:
            missing.append(p)
        for f in files:
            t = read(f)
            if not t:
                continue
            lst = texts.setdefault(t, [])
            if label not in lst:
                lst.append(label)
            up = os.path.basename(f).upper()
            toks = spdx_parts(p.get("license"))
            # plain LICENSE of a single-license package is that license's text
            plain = len(toks) == 1 and up.split(".")[0] in ("LICENSE", "LICENCE", "COPYING")
            for tok in toks:
                tl = tok.upper().split("-")
                # LICENSE-MIT / LICENSE-APACHE style names
                if plain or (tl[0] in up and ("LICEN" in up or "COPYING" in up)):
                    by_lic.setdefault(tok, {}).setdefault(t, None)

    std = {}        # spdx token -> text (for packages without files)
    for p in missing:
        for tok in spdx_parts(p.get("license")):
            if tok in by_lic:
                std[tok] = next(iter(by_lic[tok]))
    bad = sorted({p["name"] for p in missing
                  if not any(t in std or t in NO_TEXT for t in spdx_parts(p.get("license")))})

    L = []
    L.append("""Third-party licenses / Лицензии сторонних библиотек
===================================================

This file lists the third-party Rust libraries (and fonts) built into NovelTL
and their license texts. NovelTL itself is licensed under GPL-3.0-or-later
(see the LICENSE file). This file was generated automatically from Cargo.lock.

В этом файле перечислены сторонние библиотеки Rust (и шрифты), встроенные в
NovelTL, и тексты их лицензий. Сама NovelTL распространяется по лицензии
GPL-3.0-or-later (см. файл LICENSE). Файл создан автоматически из Cargo.lock.

Packages / Пакеты
-----------------""")
    miss_names = {p["name"] + " " + p["version"] for p in missing}
    for p in pkgs:
        s = f'{p["name"]} {p["version"]} — {p.get("license") or "unknown"} — {p.get("repository") or ""}'
        if p.get("license") in NO_TEXT:
            s += " (public domain dedication, no license text required)"
        elif f'{p["name"]} {p["version"]}' in miss_names:
            s += " (license text: see standard text below)"
        L.append(s)
    sep = "=" * 78
    for t, names in texts.items():
        L += ["", sep, "Applies to: " + ", ".join(names), "", t]
    if std:
        L += ["", sep, "Standard license texts / Стандартные тексты лицензий", sep]
        for tok, t in sorted(std.items()):
            users = ", ".join(f'{p["name"]} {p["version"]}' for p in missing
                              if tok in spdx_parts(p.get("license")))
            L += ["", sep, f"{tok} — applies to: {users}", "", t]
    data = "\n".join(L) + "\n"
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(data)
    if bad:
        sys.stderr.write("WARNING: no license text found for: " + ", ".join(bad) + "\n")
    print(f"packages: {len(pkgs)}, unique texts: {len(texts)}, size: {len(data.encode('utf-8'))} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
