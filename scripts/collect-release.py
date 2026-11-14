"""Collect the three desktop installers without depending on artifact directory layout."""
import pathlib
import shutil
import argparse
from release_policy import parse_tag

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source', type=pathlib.Path)
parser.add_argument('destination', type=pathlib.Path)
parser.add_argument('--version', required=True)
args = parser.parse_args()
version = parse_tag('v' + args.version).version
source, destination = args.source, args.destination
destination.mkdir(parents=True, exist_ok=True)
for suffix, target in (('.deb', 'amd64.deb'), ('.AppImage', 'amd64.AppImage'), ('.exe', 'x64-setup.exe')):
    files = list(source.rglob('*' + suffix))
    if len(files) != 1:
        raise SystemExit(f'Expected exactly one {suffix} installer, found {len(files)}')
    shutil.copy2(files[0], destination / f'Linchuan_{version}_{target}')
