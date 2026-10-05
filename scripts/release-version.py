#!/usr/bin/env python3
"""Increment the Cargo base patch once per first-parent commit since its change."""
import argparse
import pathlib
import re
import subprocess
import tomllib


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def package_version(text):
    return tomllib.loads(text)["package"]["version"]


def validate(version):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("Release versions must be major.minor.patch without prerelease suffixes")
    return tuple(map(int, version.split(".")))


def release_version(root):
    if git(root, "rev-parse", "--is-shallow-repository") != "false":
        raise ValueError("Version calculation requires full Git history (fetch-depth: 0)")
    base = package_version(git(root, "show", "HEAD:Cargo.toml"))
    major, minor, patch = validate(base)
    commits = git(root, "log", "--first-parent", "--format=%H", "--", "Cargo.toml").splitlines()
    for commit in commits:
        parents = git(root, "rev-list", "--parents", "-n", "1", commit).split()
        previous = package_version(git(root, "show", f"{parents[1]}:Cargo.toml")) if len(parents) > 1 else None
        if previous != base:
            count = int(git(root, "rev-list", "--first-parent", "--count", f"{commit}..HEAD"))
            return f"{major}.{minor}.{patch + count}"
    raise ValueError("Cannot find the commit that introduced the Cargo base version")


def apply_version(root, version):
    validate(version)
    manifest_path, lock_path = root / "Cargo.toml", root / "Cargo.lock"
    manifest, lock = manifest_path.read_text(), lock_path.read_text()
    name = tomllib.loads(manifest)["package"]["name"]
    package = re.compile(r"(?ms)(^\[package\]\s*\n)(.*?)(?=^\[|\Z)")
    manifest, count = package.subn(
        lambda m: m[1] + re.sub(r'^version\s*=\s*"[^"]+"', f'version = "{version}"', m[2], count=1, flags=re.M),
        manifest,
    )
    if count != 1 or package_version(manifest) != version:
        raise ValueError("Expected one Cargo package version")
    pattern = re.compile(r'(?m)(^name = "' + re.escape(name) + r'"\nversion = ")[^"]+("$)')
    lock, count = pattern.subn(lambda m: m[1] + version + m[2], lock)
    if count != 1:
        raise ValueError("Expected one root package in Cargo.lock")
    manifest_path.write_text(manifest)
    lock_path.write_text(lock)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Update Cargo.toml and Cargo.lock for this build")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    version = release_version(root)
    if args.apply:
        apply_version(root, version)
    print(version)
