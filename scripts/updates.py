"""Sign and verify Tauri v2 updater assets. Private keys never enter release folders."""
import argparse
import base64
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

REPO = Path(__file__).resolve().parent.parent
DOWNLOADS = "https://github.com/whoarei/linchuan/releases/download"
TARGETS = {
    "windows-x86_64-nsis": "x64-setup.exe",
    "linux-x86_64-appimage": "amd64.AppImage",
    "linux-x86_64-deb": "amd64.deb",
}


def version_for(tag):
    if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
        raise ValueError("Only stable vX.Y.Z tags can publish updates")
    return tag[1:]


def installers(root, version):
    return {target: root / f"Linchuan_{version}_{suffix}" for target, suffix in TARGETS.items()}


def public_key():
    return json.loads((REPO / "src-tauri/tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"]


def validate_source_version(version):
    versions = [json.loads((REPO / name).read_text())["version"] for name in ("package.json", "src-tauri/tauri.conf.json")]
    versions += [tomllib.loads((REPO / name).read_text())["package"]["version"] for name in ("crates/core/Cargo.toml", "src-tauri/Cargo.toml")]
    if any(value != version for value in versions):
        raise ValueError("The release tag, frontend, Tauri configuration and Rust package versions must match")


def verify_signature(file, signature, pubkey, version):
    public_lines = base64.b64decode(pubkey, validate=True).decode().splitlines()
    lines = base64.b64decode(signature.strip(), validate=True).decode().splitlines()
    if len(public_lines) != 2 or len(lines) != 4 or not lines[2].startswith("trusted comment: "):
        raise ValueError("Invalid updater signature format")
    public = base64.b64decode(public_lines[1], validate=True)
    signed = base64.b64decode(lines[1], validate=True)
    if len(public) != 42 or len(signed) != 74 or public[:2] != b"Ed" or signed[:2] not in (b"Ed", b"ED") or public[2:10] != signed[2:10]:
        raise ValueError("Updater signing key mismatch")
    key = Ed25519PublicKey.from_public_bytes(public[10:])
    if signed[:2] == b"ED":
        with file.open("rb") as stream:
            data = hashlib.file_digest(stream, "blake2b").digest()
    else:
        data = file.read_bytes()
    key.verify(signed[10:], data)
    trusted = lines[2][len("trusted comment: "):]
    key.verify(base64.b64decode(lines[3], validate=True), signed[10:] + trusted.encode())
    fields = [field.removeprefix("version:") for field in trusted.split("\t") if field.startswith("version:")]
    if fields != [version]:
        raise ValueError("Signature does not bind the announced app version")


def manifest(root, tag, notes):
    version = version_for(tag)
    entries = {}
    for target, file in installers(root, version).items():
        if not file.is_file():
            raise ValueError(f"Missing update installer: {file.name}")
        entries[target] = {"url": f"{DOWNLOADS}/{tag}/{file.name}", "signature": Path(str(file) + ".sig").read_text().strip()}
    return {"version": version, "notes": notes, "pub_date": datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z"), "platforms": entries}


def verify_release(root, tag, pubkey):
    version = version_for(tag)
    paths = installers(root, version)
    allowed = {file.name for file in paths.values()} | {file.name + ".sig" for file in paths.values()} | {"latest.json", "SHA256SUMS", f"Linchuan_{version}_android_arm64.apk"}
    if any(file.name not in allowed or not file.is_file() for file in root.iterdir()):
        raise ValueError("Release directory contains unexpected files; keep signing keys and passwords outside it")
    feed = json.loads((root / "latest.json").read_text())
    if feed.get("version") != version or set(feed.get("platforms", {})) != set(TARGETS):
        raise ValueError("Update manifest version or platforms do not match the release")
    date = datetime.datetime.fromisoformat(feed["pub_date"].replace("Z", "+00:00"))
    if date.tzinfo is None:
        raise ValueError("Update publication time must include a timezone")
    for target, file in installers(root, version).items():
        entry = feed["platforms"][target]
        signature = Path(str(file) + ".sig").read_text().strip()
        if entry.get("url") != f"{DOWNLOADS}/{tag}/{file.name}" or entry.get("signature") != signature:
            raise ValueError("Manifest URL or signature does not match the installer")
        verify_signature(file, signature, pubkey, version)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["sign", "verify"])
    parser.add_argument("directory", type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--key-path", type=Path)
    parser.add_argument("--password-file", type=Path)
    parser.add_argument("--notes", type=Path)
    parser.add_argument("--metadata", type=Path, help="Copy detached signatures and latest.json from this trusted staging directory before verification")
    args = parser.parse_args()
    version = version_for(args.tag)
    validate_source_version(version)
    paths = installers(args.directory, version)
    if args.command == "sign":
        if not args.key_path and not os.environ.get("TAURI_SIGNING_PRIVATE_KEY"):
            raise ValueError("Provide --key-path or TAURI_SIGNING_PRIVATE_KEY")
        env = dict(os.environ)
        if args.password_file:
            env["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] = args.password_file.read_text().strip()
        for file in paths.values():
            command = [str(REPO / "node_modules/.bin/tauri"), "signer", "sign", "--app-version", version]
            if args.key_path:
                command += ["--private-key-path", str(args.key_path)]
            command.append(str(file))
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError(f"Updater signing failed for {file.name}; check the signing key and password")
        notes_file = args.notes or REPO / f"docs/releases/{version}.md"
        feed = manifest(args.directory, args.tag, notes_file.read_text() if notes_file.exists() else "")
        (args.directory / "latest.json").write_text(json.dumps(feed, ensure_ascii=False, indent=2) + "\n")
    elif args.metadata:
        for name in [file.name + ".sig" for file in paths.values()] + ["latest.json"]:
            shutil.copy2(args.metadata / name, args.directory / name)
    verify_release(args.directory, args.tag, public_key())
    print("Verified three signed updater packages and the release manifest.")


if __name__ == "__main__":
    main()
