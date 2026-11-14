"""Use Debian's ~ prerelease ordering while retaining the application's SemVer."""
import argparse
from pathlib import Path
import re
import subprocess
import tempfile

from release_policy import parse_tag


def prepare(directory, version):
    release = parse_tag("v" + version)
    files = list(directory.glob("*.deb"))
    if len(files) != 1:
        raise ValueError("Expected exactly one Debian package")
    package = files[0]
    wanted = version.replace("-rc.", "~rc.")
    actual = subprocess.check_output(["dpkg-deb", "-f", str(package), "Version"], text=True).strip()
    if actual not in {version, wanted}:
        raise ValueError("Debian package version differs from the release version")
    if release.prerelease and actual != wanted:
        with tempfile.TemporaryDirectory(prefix="deb-version-", dir=directory) as scratch:
            unpacked = Path(scratch) / "unpacked"
            subprocess.run(["dpkg-deb", "--raw-extract", str(package), str(unpacked)], check=True)
            control = unpacked / "DEBIAN/control"
            control.write_text(re.sub(r"^Version: .*?$", "Version: " + wanted, control.read_text(), flags=re.MULTILINE))
            replacement = Path(scratch) / "package.deb"
            subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(unpacked), str(replacement)], check=True)
            replacement.replace(package)
    if subprocess.check_output(["dpkg-deb", "-f", str(package), "Version"], text=True).strip() != wanted:
        raise ValueError("Debian prerelease version verification failed")
    print(f"Debian package version verified: {wanted}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    arguments = parser.parse_args()
    prepare(arguments.directory, arguments.version)
