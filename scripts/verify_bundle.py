#!/usr/bin/env python3
"""Verify a release bundle case-sensitively, on any filesystem.

The r2 bundle shipped `SBOM.cdx.json` at the archive root while
`SOURCE-MANIFEST.sha256` listed `sbom.cdx.json`. macOS agreed with itself
(APFS is case-insensitive); the Ubuntu runner would not have. `shasum -c`
cannot catch this because the shell resolves the name through the filesystem.

This verifier compares manifest paths and archive member names as exact
strings, and asserts that the archive's member list equals the manifest's path
list plus the one record file. It needs no case-sensitive filesystem.

    python scripts/verify_bundle.py --bundle /path/to/extracted
    python scripts/verify_bundle.py --bundle /path/to/extracted --zip /path/to/bundle.zip
    python scripts/verify_bundle.py --bundle /path/to/extracted --zip bundle.zip --self-test

Executable payloads are checked too: a Mach-O or ELF file that the archive stores
as 0o644 extracts into a binary nobody can run, which is what the r3 bundle did to
all five of its compiled binaries. `--self-test` proves that check can fail by
clearing one executable bit in a copy of the archive.

Record files excluded from the manifest:
    SOURCE-MANIFEST.sha256  describes the bundle, so it cannot hash itself
"""
from __future__ import annotations

import argparse
import hashlib
import stat
import sys
import tempfile
import zipfile
from pathlib import Path, PurePosixPath

MANIFEST_NAME = "SOURCE-MANIFEST.sha256"
RECORD_FILES = {MANIFEST_NAME}
IGNORED = {".DS_Store"}

# Mach-O (thin little/big endian, fat) and ELF headers: the payloads a consumer
# must be able to execute after extraction.
EXECUTABLE_MAGIC = (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xca\xfe\xba\xbe")


# Path components that must never reach a release bundle: build caches, virtual
# environments and packaging metadata. The r0 bundle shipped 18 `*.egg-info`
# files because the bundler excluded only a literal `build/` directory, so the
# verifier now refuses them whether or not the bundler does.
CACHE_COMPONENTS = {
    "__pycache__",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".cache",
    ".venv",
    "venv",
    "target",
    "build",
    "dist",
    "node_modules",
    ".git",
}
CACHE_SUFFIXES = (".egg-info", ".pyc", ".pyo")


def cache_path(name: str) -> str | None:
    for part in name.split("/"):
        if part in CACHE_COMPONENTS or part.endswith(CACHE_SUFFIXES):
            return part
    return None


def is_executable_payload(head: bytes, name: str) -> bool:
    if name.endswith((".whl", ".zip", ".gz", ".tar")):
        return False
    return head.startswith(EXECUTABLE_MAGIC)


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    print(f"{len(messages)} bundle gate(s) failed", file=sys.stderr)
    return 1


def read_manifest(path: Path) -> tuple[dict[str, str], list[str]]:
    errors: list[str] = []
    entries: dict[str, str] = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        digest, _, name = line.partition("  ")
        if not digest or not name:
            errors.append(f"{MANIFEST_NAME}:{number}: not `hash  path` format")
            continue
        if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
            errors.append(f"{MANIFEST_NAME}:{number}: `{digest}` is not a lowercase SHA-256")
        if name.startswith("/") or "\\" in name or ".." in PurePosixPath(name).parts:
            errors.append(f"{MANIFEST_NAME}:{number}: `{name}` is not a clean relative POSIX path")
        if name in entries:
            errors.append(f"{MANIFEST_NAME}:{number}: `{name}` is listed twice")
        entries[name] = digest
    return entries, errors


def list_files(root: Path) -> dict[str, Path]:
    found: dict[str, Path] = {}
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        found[relative] = path
    return found


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_directory(bundle: Path, entries: dict[str, str]) -> list[str]:
    errors: list[str] = []
    present = list_files(bundle)

    # Exact string membership, never a filesystem lookup: this is the check that
    # a case-insensitive volume cannot perform.
    for name, expected in entries.items():
        if name not in present:
            near = sorted(candidate for candidate in present if candidate.lower() == name.lower())
            hint = f" (found `{near[0]}` instead)" if near else ""
            errors.append(f"listed file is missing: `{name}`{hint}")
            continue
        actual = sha256(present[name])
        if actual != expected:
            errors.append(f"hash mismatch: `{name}` is {actual}, manifest says {expected}")

    for name in present:
        component = cache_path(name)
        if component is not None:
            errors.append(f"build cache in the extracted tree: `{name}` (component `{component}`)")

    for name, path in present.items():
        if name not in entries:
            continue
        with path.open("rb") as handle:
            head = handle.read(4)
        mode = stat.S_IMODE(path.stat().st_mode)
        if is_executable_payload(head, name) and not mode & 0o111:
            errors.append(f"extracted file is not executable: `{name}` (mode {mode:04o})")

    unlisted = sorted(set(present) - set(entries) - RECORD_FILES - IGNORED)
    for name in unlisted:
        errors.append(f"file is present but not listed: `{name}`")
    return errors


def check_zip(archive_path: Path, entries: dict[str, str]) -> list[str]:
    errors: list[str] = []
    with zipfile.ZipFile(archive_path) as archive:
        members = [name for name in archive.namelist() if not name.endswith("/")]
        duplicates = {name for name in members if members.count(name) > 1}
        for name in sorted(duplicates):
            errors.append(f"archive contains `{name}` more than once")

        for name in members:
            component = cache_path(name)
            if component is not None:
                errors.append(f"build cache in the archive: `{name}` (component `{component}`)")

        expected = set(entries) | RECORD_FILES
        for name in sorted(expected - set(members)):
            near = sorted(member for member in members if member.lower() == name.lower())
            hint = f" (archive stores `{near[0]}`)" if near else ""
            errors.append(f"manifest path missing from archive: `{name}`{hint}")
        for name in sorted(set(members) - expected):
            near = sorted(candidate for candidate in expected if candidate.lower() == name.lower())
            hint = f" (manifest lists `{near[0]}`)" if near else ""
            errors.append(f"archive member not accounted for: `{name}`{hint}")

        for name, digest in entries.items():
            if name not in members:
                continue
            payload = archive.read(name)
            if hashlib.sha256(payload).hexdigest() != digest:
                errors.append(f"archive member hash mismatch: `{name}`")
            mode = (archive.getinfo(name).external_attr >> 16) & 0o777
            if is_executable_payload(payload[:4], name) and not mode & 0o111:
                errors.append(
                    f"archive stores executable payload `{name}` with mode {mode:04o}"
                )
    return errors


def self_test(archive_path: Path, entries: dict[str, str]) -> list[str]:
    """Negative control: clear one executable bit and require verification to fail."""
    with zipfile.ZipFile(archive_path) as archive:
        members = [info for info in archive.infolist() if not info.filename.endswith("/")]
        payloads = [(info, archive.read(info.filename)) for info in members]

    victim = next(
        (
            info.filename
            for info, payload in payloads
            if is_executable_payload(payload[:4], info.filename)
            and (info.external_attr >> 16) & 0o111
        ),
        None,
    )
    if victim is None:
        return ["self-test: archive contains no executable payload to disable"]

    with tempfile.TemporaryDirectory(prefix="nirnay-self-test-") as holder:
        broken = Path(holder) / "mode-cleared.zip"
        with zipfile.ZipFile(broken, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for info, payload in payloads:
                if info.filename == victim:
                    info.external_attr = (stat.S_IFREG | 0o644) << 16
                archive.writestr(info, payload)
        found = check_zip(broken, entries)

    if not any(victim in message and "executable" in message for message in found):
        return [f"self-test: clearing the executable bit on `{victim}` did not fail verification"]

    print(f"PASS: self-test - clearing the executable bit on `{victim}` fails verification")
    return []


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Case-sensitive release bundle verification")
    parser.add_argument("--bundle", type=Path, required=True, help="extracted bundle root")
    parser.add_argument("--zip", type=Path, help="distribution archive to cross-check")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="prove the executable-mode check can fail, using a copy of --zip",
    )
    args = parser.parse_args(argv)

    manifest = args.bundle / MANIFEST_NAME
    if not manifest.is_file():
        return fail([f"{manifest} is missing"])

    entries, errors = read_manifest(manifest)
    if not entries:
        return fail(errors + ["manifest is empty"])

    errors.extend(check_directory(args.bundle, entries))
    if args.zip:
        if not args.zip.is_file():
            errors.append(f"{args.zip} is missing")
        else:
            errors.extend(check_zip(args.zip, entries))
            if args.self_test and not errors:
                errors.extend(self_test(args.zip, entries))
    elif args.self_test:
        errors.append("--self-test needs --zip")

    if errors:
        return fail(errors)

    scope = f"{len(entries)} manifest entries"
    if args.zip:
        scope += ", archive member list and per-member hashes"
    print(f"PASS: {scope} verified case-sensitively against {args.bundle}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
