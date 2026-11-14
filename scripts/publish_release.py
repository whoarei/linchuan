"""Publish verified installers, keeping RC releases out of the stable latest feed."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from release_policy import REPO, parse_tag
from updates import installers, public_key, verify_release


def command(args, check=True):
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and result.returncode:
        raise RuntimeError(result.stderr.strip() or "GitHub command failed")
    return result


def release_info(repository, tag):
    # GitHub's get-by-tag endpoint only returns published releases. The list
    # endpoint includes drafts when authenticated, including a failed upload
    # that must be verified and resumed before the release becomes public.
    result = command(["api", f"repos/{repository}/releases?per_page=100", "--paginate", "--jq", ".[] | @json"])
    releases = (json.loads(line) for line in result.stdout.splitlines() if line.strip())
    matches = [release for release in releases if release["tag_name"] == tag]
    if len(matches) > 1:
        raise ValueError("Multiple releases use this tag; refusing to choose a draft")
    return matches[0] if matches else None


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_files(directory, release):
    expected = {path.name for path in installers(directory, release.version).values()}
    expected |= {name + ".sig" for name in expected}
    expected |= {f"Linchuan_{release.version}_android_arm64.apk", "latest.json", "SHA256SUMS"}
    if {path.name for path in directory.iterdir()} != expected:
        raise ValueError("Release must contain exactly four installers, three signatures, latest.json and SHA256SUMS")
    verify_release(directory, release.tag, public_key())
    lines = (directory / "SHA256SUMS").read_text().splitlines()
    checksums = dict(line.split("  ", 1)[::-1] for line in lines)
    if len(lines) != 8 or set(checksums) != expected - {"SHA256SUMS"}:
        raise ValueError("Release checksums are incomplete or duplicated")
    for name, expected_digest in checksums.items():
        if digest(directory / name) != expected_digest:
            raise ValueError(f"Checksum mismatch: {name}")


def verify_assets(info, directory, repository, tag):
    files = {path.name: path for path in directory.iterdir()}
    assets = {asset["name"]: asset for asset in info["assets"]}
    if len(info["assets"]) != 9 or set(assets) != set(files):
        raise ValueError("Uploaded release assets do not match the verified files")
    for name, file in files.items():
        asset = assets[name]
        expected = digest(file)
        if asset["size"] != file.stat().st_size:
            raise ValueError(f"Uploaded size mismatch: {name}")
        if asset.get("digest"):
            if asset["digest"] != "sha256:" + expected:
                raise ValueError(f"Uploaded digest mismatch: {name}")
        else:
            with tempfile.TemporaryDirectory(prefix="linchuan-release-check-") as scratch:
                command(["release", "download", tag, "--repo", repository, "--pattern", name, "--dir", scratch])
                if digest(Path(scratch) / name) != expected:
                    raise ValueError(f"Downloaded digest mismatch: {name}")


def publish(directory, tag, repository, notes):
    release = parse_tag(tag)
    verify_files(directory, release)
    existing = release_info(repository, tag)
    if existing is not None and not existing["draft"]:
        raise ValueError("Release is already published; refusing to replace it")
    files = [str(path) for path in sorted(directory.iterdir())]
    if existing is None:
        args = ["release", "create", tag, *files, "--repo", repository, "--verify-tag", "--draft", "--latest=false", "--title", f"邻传 {tag}", "--notes-file", str(notes)]
        if release.prerelease:
            args.append("--prerelease")
        command(args)
    else:
        command(["release", "upload", tag, *files, "--repo", repository, "--clobber"])
    staged = release_info(repository, tag)
    if not staged or not staged["draft"] or staged["tag_name"] != tag:
        raise ValueError("Expected a draft release for the exact version tag")
    verify_assets(staged, directory, repository, tag)
    command(["release", "edit", tag, "--repo", repository, "--draft=false", f"--prerelease={str(release.prerelease).lower()}", f"--latest={str(not release.prerelease).lower()}", "--title", f"邻传 {tag}", "--notes-file", str(notes)])
    published = release_info(repository, tag)
    if not published or published["draft"] or published["prerelease"] != release.prerelease or published["tag_name"] != tag:
        raise ValueError("Published release status does not match the stable/RC policy")
    verify_assets(published, directory, repository, tag)
    print(f"Published {'RC pre-release' if release.prerelease else 'stable latest release'}: {published['html_url']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()
    release = parse_tag(args.tag)
    notes = REPO / "docs/releases" / f"{release.version}.md"
    if not notes.is_file():
        parser.error(f"Missing release notes: {notes.name}")
    publish(args.directory, args.tag, args.repository, notes)


if __name__ == "__main__":
    main()
