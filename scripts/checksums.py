import hashlib
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
with (root / 'SHA256SUMS').open('w') as out:
    for file in sorted(root.iterdir()):
        if file.is_file() and file.name != 'SHA256SUMS':
            digest = hashlib.file_digest(file.open('rb'), 'sha256').hexdigest()
            out.write(f'{digest}  {file.name}\n')
