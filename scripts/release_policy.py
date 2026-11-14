"""Shared stable/RC version rules for signing and GitHub release builds."""
import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import re
import subprocess
import tomllib

REPO = Path(__file__).resolve().parent.parent
TAG_PATTERN = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-rc\.([1-9][0-9]*))?")


@dataclass(frozen=True)
class ReleaseVersion:
    tag: str
    major: int
    minor: int
    patch: int
    rc: int | None

    @property
    def version(self):
        return self.tag[1:]

    @property
    def prerelease(self):
        return self.rc is not None

    @property
    def android_code(self):
        if self.minor >= 100 or self.patch >= 100 or (self.rc is not None and self.rc > 998):
            raise ValueError("Android releases require minor/patch < 100 and RC number between 1 and 998")
        code = self.major * 10_000_000 + self.minor * 100_000 + self.patch * 1000 + (self.rc if self.prerelease else 999)
        if not 0 < code <= 2_100_000_000:
            raise ValueError("Android versionCode exceeds the supported range")
        return code


def parse_tag(tag):
    match = TAG_PATTERN.fullmatch(tag)
    if not match:
        raise ValueError("Release tag must be vX.Y.Z or vX.Y.Z-rc.N (N starts at 1)")
    major, minor, patch, rc = match.groups()
    return ReleaseVersion(tag, int(major), int(minor), int(patch), int(rc) if rc else None)


def validate_source_version(version, root=REPO):
    versions = [json.loads((root / name).read_text())["version"] for name in ("package.json", "src-tauri/tauri.conf.json")]
    versions += [tomllib.loads((root / name).read_text())["package"]["version"] for name in ("crates/core/Cargo.toml", "src-tauri/Cargo.toml")]
    lock = json.loads((root / "package-lock.json").read_text())
    versions += [lock["version"], lock["packages"][""]["version"]]
    packages = tomllib.loads((root / "Cargo.lock").read_text())["package"]
    for name in ("linchuan", "linchuan-core"):
        versions.append(next(package["version"] for package in packages if package["name"] == name))
    if any(value != version for value in versions):
        raise ValueError("Release tag, frontend, Tauri, Rust versions and lockfiles must match")


def metadata(tag, root=REPO):
    release = parse_tag(tag)
    validate_source_version(release.version, root)
    notes = root / "docs/releases" / f"{release.version}.md"
    if not notes.is_file():
        raise ValueError(f"Missing release notes: docs/releases/{release.version}.md")
    return {"tag": tag, "version": release.version, "prerelease": str(release.prerelease).lower(), "android_code": str(release.android_code)}


def verify_apk(aapt, apk, tag):
    release = parse_tag(tag)
    output = subprocess.check_output([aapt, "dump", "badging", str(apk)], text=True)
    line = next((line for line in output.splitlines() if line.startswith("package:")), "")
    fields = dict(re.findall(r"([A-Za-z]+)='([^']*)'", line))
    identifier = json.loads((REPO / "src-tauri/tauri.conf.json").read_text())["identifier"]
    if fields.get("name") != identifier or fields.get("versionName") != release.version or fields.get("versionCode") != str(release.android_code):
        raise ValueError("APK application ID, versionName or versionCode does not match the release")
    print("Android package identity and release version verified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "android-config", "verify-apk"])
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--aapt")
    parser.add_argument("--apk", type=Path)
    args = parser.parse_args()
    release = parse_tag(args.tag)
    if args.command == "prepare":
        values = metadata(args.tag)
        if args.github_output:
            with args.github_output.open("a") as stream:
                for key, value in values.items():
                    stream.write(f"{key}={value}\n")
        print(json.dumps(values))
    elif args.command == "android-config":
        if not args.output:
            parser.error("android-config requires --output")
        args.output.write_text(json.dumps({"bundle": {"android": {"versionCode": release.android_code}}}))
    else:
        if not args.aapt or not args.apk:
            parser.error("verify-apk requires --aapt and --apk")
        verify_apk(args.aapt, args.apk, args.tag)


if __name__ == "__main__":
    main()
