#!/usr/bin/env python3
"""Build the distribution archive and its source manifest from one file list.

The r2 bundle was assembled with ad-hoc `zip` invocations, so the archive could
carry a member name the manifest did not know about (`SBOM.cdx.json` versus
`sbom.cdx.json`). Here the manifest is written from the staged file list and the
archive is written *from that same list*, so the two cannot diverge. Members are
stored with a fixed timestamp, and with the source file's own executable bit, so
the same inputs always produce the same archive bytes and the compiled binaries
extract executable.

    python scripts/make_bundle.py --source-dir . --artifacts ../artifacts \
        --output ../nirnay-quant-production-upgrade-2026-11-13-r3.zip

Layout produced inside the archive:

    nirnay-quant/**          the source tree, without build caches
    artifacts/**             compiled wheels and binaries, if supplied
    sbom.cdx.json            copied from --source-dir unless --sbom says otherwise
    SOURCE-MANIFEST.sha256   hash of every file above

Two files are written beside the archive and not inside it: `<archive>.sha256`
and `<archive>.manifest.sha256`, the latter a copy of the archive's own manifest
so a release can publish the manifest it verified against.

The manifest excludes itself.
"""
from __future__ import annotations

import argparse
import hashlib
import shutil
import stat
import sys
import tempfile
import zipfile
from pathlib import Path

SOURCE_DIR_NAME = "nirnay-quant"
MANIFEST_NAME = "SOURCE-MANIFEST.sha256"
RECORD_FILES = (MANIFEST_NAME,)
SBOM_NAME = "sbom.cdx.json"

EXCLUDED_DIRS = {
    "__pycache__", ".pytest_cache", ".ruff_cache", ".mypy_cache",
    ".venv", "venv", "target", "build", "dist", "node_modules",
    ".git", ".idea", ".vscode",
}
EXCLUDED_FILES = {".DS_Store"}
EXCLUDED_SUFFIXES = (".pyc", ".pyo", ".swp")
# `pip install -e` leaves these beside the sources; the r0 bundle shipped 18 of
# them because only a literal `build/` directory was excluded.
EXCLUDED_PACKAGING_SUFFIX = ".egg-info"

# Fixed timestamps keep the archive reproducible across rebuilds. Permissions are
# *not* fixed: the archive carries the staged file's own mode, because a bundle of
# 0o644 binaries extracts into files nobody can execute.
ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)


def archive_mode(path: Path) -> int:
    """0o755 for anything executable in the source, 0o644 otherwise."""
    original = stat.S_IMODE(path.stat().st_mode)
    return 0o755 if original & 0o111 else 0o644


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    return 1


def ignored(directory: str, names: list[str]) -> set[str]:
    """Return the bundle-stage entries that must not be copied."""
    # The manifest is a record of the bundle, not payload: it is written at the
    # bundle root by `write_manifest` and stays outside the manifest it describes.
    excluded = EXCLUDED_DIRS | EXCLUDED_FILES | set(RECORD_FILES)
    return {
        name
        for name in names
        if name in excluded or name.endswith(EXCLUDED_PACKAGING_SUFFIX)
    }


def stage(source: Path, stage: Path, artifacts: Path | None, sbom: Path | None) -> list[str]:
    """Copy payload files into the staging tree and return their names."""
    shutil.copytree(
        source,
        stage / SOURCE_DIR_NAME,
        ignore=ignored,
        symlinks=True,
    )
    if artifacts is not None:
        shutil.copytree(artifacts, stage / "artifacts", ignore=ignored, symlinks=True)

    chosen = sbom if sbom is not None else source / SBOM_NAME
    if chosen.is_file():
        shutil.copyfile(chosen, stage / chosen.name)

    paths: list[str] = []
    for path in sorted(stage.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(stage).as_posix()
        if relative in RECORD_FILES or path.name in RECORD_FILES:
            continue
        if path.suffix in EXCLUDED_SUFFIXES or path.name in EXCLUDED_FILES:
            continue
        paths.append(relative)
    return paths


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_manifest(stage: Path, paths: list[str]) -> Path:
    manifest = stage / MANIFEST_NAME
    lines = [f"{sha256(stage / path)}  {path}" for path in paths]
    manifest.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return manifest


def write_archive(stage: Path, output: Path, paths: list[str]) -> None:
    members = list(paths) + [name for name in RECORD_FILES if (stage / name).is_file()]
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name in members:
            info = zipfile.ZipInfo(name, date_time=ZIP_EPOCH)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (stat.S_IFREG | archive_mode(stage / name)) << 16
            archive.writestr(info, (stage / name).read_bytes())


def verify_archive(stage: Path, output: Path) -> list[str]:
    """Re-read the archive and compare it with the manifest, as exact strings."""
    errors: list[str] = []
    manifest = {}
    for line in (stage / MANIFEST_NAME).read_text(encoding="utf-8").splitlines():
        digest, _, name = line.partition("  ")
        manifest[name] = digest

    with zipfile.ZipFile(output) as archive:
        members = [name for name in archive.namelist() if not name.endswith("/")]
        expected = set(manifest) | set(RECORD_FILES)
        for name in sorted(expected - set(members)):
            errors.append(f"archive is missing `{name}`")
        for name in sorted(set(members) - expected):
            errors.append(f"archive member `{name}` is not in the manifest")
        for name, digest in manifest.items():
            if name in members and hashlib.sha256(archive.read(name)).hexdigest() != digest:
                errors.append(f"archive member `{name}` does not match its manifest hash")
        if len(members) != len(set(members)):
            errors.append("archive contains duplicate member names")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Assemble a reproducible release bundle")
    parser.add_argument("--source-dir", type=Path, default=Path("."))
    parser.add_argument("--artifacts", type=Path, help="directory of compiled wheels and binaries")
    parser.add_argument("--sbom", type=Path, help=f"defaults to <source-dir>/{SBOM_NAME}")
    parser.add_argument("--output", type=Path, required=True, help="archive path to write")
    parser.add_argument("--stage", type=Path, help="keep the staged tree at this path")
    args = parser.parse_args(argv)

    source = args.source_dir.resolve()
    if not source.is_dir():
        return fail([f"{source} is not a directory"])

    # A directory of compiled outputs that lives inside the source tree is
    # packed twice: once as artifacts/** and once as <source>/artifacts/**, so
    # the released bundle ships wheels and binaries inside the source tree it
    # claims to describe. That is how v0.1.2 shipped; refuse it instead.
    if args.artifacts is not None:
        artifacts = args.artifacts.resolve()
        if artifacts == source:
            return fail(
                [
                    f"--artifacts points at the source tree {source} itself; pass the"
                    " directory that holds the compiled output, outside the checkout."
                ]
            )
        if source in artifacts.parents:
            return fail(
                [
                    f"{artifacts} is inside the source tree {source}: it would be packed as"
                    " build output inside the sources as well as at artifacts/. Build it"
                    " outside the checkout."
                ]
            )
        if not artifacts.is_dir():
            return fail([f"{artifacts} is not a directory"])

    holder = None
    if args.stage:
        staging = args.stage.resolve()
        if staging.exists():
            shutil.rmtree(staging)
        staging.mkdir(parents=True)
    else:
        holder = tempfile.TemporaryDirectory(prefix="nirnay-bundle-")
        staging = Path(holder.name)

    try:
        paths = stage(source, staging, args.artifacts, args.sbom)
        if not paths:
            return fail(["nothing to bundle"])
        write_manifest(staging, paths)
        write_archive(staging, args.output.resolve(), paths)
        errors = verify_archive(staging, args.output.resolve())
        if errors:
            return fail(errors)

        digest = sha256(args.output.resolve())
        sidecar = args.output.resolve().with_name(args.output.name + ".sha256")
        sidecar.write_text(f"{digest}  {args.output.name}\n", encoding="utf-8")

        # The manifest lives at the bundle root and nowhere else unless it is
        # copied out, which is how the first tagged release reached its
        # provenance step with no manifest to record. Name it after the archive
        # so two releases can sit in one directory without colliding.
        manifest_copy = args.output.resolve().with_name(args.output.name + ".manifest.sha256")
        shutil.copyfile(staging / MANIFEST_NAME, manifest_copy)

        print(f"staged {len(paths)} files")
        print(f"manifest sha256  {sha256(staging / MANIFEST_NAME)}")
        print(f"archive sha256   {digest}")
        print(f"archive entries  {len(paths) + len(RECORD_FILES)}")
        executables = [
            name
            for name in paths
            if stat.S_IMODE((staging / name).stat().st_mode) & 0o111
        ]
        print(f"executable       {len(executables)} member(s)")
        print(f"wrote            {args.output.resolve()}")
        print(f"wrote            {sidecar}")
        print(f"wrote            {manifest_copy}")
        missing = [name for name in RECORD_FILES if not (staging / name).is_file()]
        if missing:
            print(f"note: archive has no {' or '.join(missing)}", file=sys.stderr)
        return 0
    finally:
        if holder is not None:
            holder.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
