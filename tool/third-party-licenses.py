#!/usr/bin/env python3
"""Write THIRD-PARTY-LICENSES.txt for the MatRust binaries (stdlib only).

usage: third-party-licenses.py OUT_FILE TARGET_TRIPLE [REPO_ROOT]
       third-party-licenses.py --self-test

Lists every package in the resolved dependency graph of the three binaries
(desktop feature) with name, version and license expression. A crate with only
a license_file is listed as "see full text below" and that file is appended in
full; the font licences are appended in full too. Exits non-zero if a crate has
neither field (or its license_file is missing).
"""
import contextlib
import io
import json
import pathlib
import subprocess
import sys
import tempfile


def shipped(meta):
    """Ids of packages reachable from matrust through normal and build dependencies."""
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [i for i in pkgs if pkgs[i]["name"] == "matrust" and pkgs[i]["source"] is None]
    # Dev-dependencies never ship.
    seen, stack = set(), list(roots)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] in (None, "build") for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    return [pkgs[i] for i in sorted(seen, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"]))]


def render(meta, triple, root):
    """Return (text, crate count, license_file count); exit non-zero on a crate without licence info."""
    rows, texts, missing = [], [], []
    for p in shipped(meta):
        name = f'{p["name"]} {p["version"]}'
        if p.get("license"):
            rows.append(f'{name}: {p["license"]}')
            continue
        path = pathlib.Path(p["manifest_path"]).parent / p["license_file"] if p.get("license_file") else None
        if path is None or not path.is_file():
            why = "no license or license_file field" if path is None else f"license_file {path} not found"
            missing.append(name)
            print(f"NO LICENCE: {name} ({why})", file=sys.stderr)
            continue
        rows.append(f"{name}: see full text below")
        texts.append((f"Licence file of {name} ({p['license_file']})",
                      path.read_text(encoding="utf-8", errors="replace").rstrip()))
    if missing:
        sys.exit("crates without licence information: " + ", ".join(missing))
    lines = ["MatRust third-party licences", "",
             f"Crates in the dependency graph for {triple} ({len(rows)}):", ""] + rows
    for f in sorted((pathlib.Path(root) / "assets" / "fonts").glob("*-LICENSE.txt")):
        texts.append((f"Font licence: {f.name}", f.read_text(encoding="utf-8", errors="replace").rstrip()))
    for heading, body in texts:
        lines += ["", "=" * 72, heading, "=" * 72, "", body]
    return "\n".join(lines) + "\n", len(rows), sum(r.endswith(": see full text below") for r in rows)


def self_test():
    """Fake metadata: one crate with license, one with only license_file, then one with neither."""
    def pkg(name, tmp, **fields):
        return dict({"id": name, "name": name, "version": "1.0.0", "source": "registry",
                     "manifest_path": str(tmp / name / "Cargo.toml")}, **fields)

    def meta_of(packages):
        root = {"id": "matrust", "name": "matrust", "version": "0.1.0", "source": None, "license": "MIT"}
        dep = {"kind": None}
        nodes = [{"id": "matrust", "deps": [{"pkg": p["id"], "dep_kinds": [dep]} for p in packages]}]
        nodes += [{"id": p["id"], "deps": []} for p in packages]
        return {"packages": [root] + packages, "resolve": {"nodes": nodes}}

    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "b" / "ext").mkdir(parents=True)
        (tmp / "b" / "ext" / "LICENSE").write_text("Full B licence\nsecond line \u201cquoted\u201d\n", encoding="utf-8")
        a = pkg("a", tmp, license="MIT OR Apache-2.0", license_file=None)
        b = pkg("b", tmp, license=None, license_file="ext/LICENSE")
        c = pkg("c", tmp, license=None, license_file=None)

        text, n, files = render(meta_of([a]), "t", tmp)
        assert "a 1.0.0: MIT OR Apache-2.0" in text and (n, files) == (2, 0), text

        text, n, files = render(meta_of([a, b]), "t", tmp)
        assert "b 1.0.0: see full text below" in text and (n, files) == (3, 1), text
        assert "Licence file of b 1.0.0 (ext/LICENSE)\n" + "=" * 72 + "\n\nFull B licence\nsecond line \u201cquoted\u201d\n" in text, text

        err = io.StringIO()
        try:
            with contextlib.redirect_stderr(err):
                render(meta_of([a, b, c]), "t", tmp)
        except SystemExit as e:
            assert e.code not in (None, 0) and "c 1.0.0" in str(e.code), e.code
        else:
            raise AssertionError("crate without licence information passed")
        assert "NO LICENCE: c 1.0.0" in err.getvalue() and "b 1.0.0" not in err.getvalue(), err.getvalue()
    print("self-test OK: license, license_file, missing")


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    out, triple = sys.argv[1], sys.argv[2]
    root = pathlib.Path(sys.argv[3] if len(sys.argv) > 3 else pathlib.Path(__file__).resolve().parent.parent)
    raw = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", triple,
         "--features", "desktop"],
        cwd=root, check=True, stdout=subprocess.PIPE).stdout
    text, n, files = render(json.loads(raw), triple, root)
    # UTF-8 so appended licence texts stay verbatim (hdf5's LICENSE has curly quotes).
    pathlib.Path(out).write_text(text, encoding="utf-8")
    print(f"{n} crates, {files} via license_file (full text appended), 0 without licence information")


if __name__ == "__main__":
    main()
