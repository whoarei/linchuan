"""Collect the three desktop installers without depending on artifact directory layout."""
import pathlib
import shutil
import sys
source, destination = map(pathlib.Path, sys.argv[1:])
destination.mkdir(parents=True, exist_ok=True)
for suffix in ('.deb', '.AppImage', '.exe'):
    files = list(source.rglob('*' + suffix))
    if len(files) != 1:
        raise SystemExit(f'Expected exactly one {suffix} installer, found {len(files)}')
    shutil.copy2(files[0], destination / files[0].name)
