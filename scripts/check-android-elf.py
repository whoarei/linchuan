import subprocess
import sys
output = subprocess.check_output([sys.argv[1], '-lW', sys.argv[2]], text=True)
loads = [line.split() for line in output.splitlines() if line.strip().startswith('LOAD')]
if not loads or any(int(row[-1], 16) < 16384 for row in loads):
    raise SystemExit('Android native library must support 16 KiB pages')
print('Android native library supports 16 KiB pages')
