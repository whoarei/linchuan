"""Linux Xvfb smoke: ensure the packaged UI bootstraps its embedded LAN service."""
import json
import os
import pathlib
import subprocess
import sys
import time
import urllib.request
version = json.loads(pathlib.Path('package.json').read_text())['version']
process = subprocess.Popen([sys.argv[1], *sys.argv[2:]], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
try:
    for attempt in range(90):
        if process.poll() is not None:
            raise RuntimeError('Native app exited: ' + process.stderr.read().decode(errors='replace'))
        sockets = set()
        for fd in pathlib.Path(f'/proc/{process.pid}/fd').iterdir():
            try:
                value = os.readlink(fd)
                if value.startswith('socket:['):
                    sockets.add(value[8:-1])
            except OSError:
                pass
        for row in pathlib.Path(f'/proc/{process.pid}/net/tcp').read_text().splitlines()[1:]:
            values = row.split()
            if values[3] != '0A' or values[9] not in sockets:
                continue
            port = int(values[1].split(':')[1], 16)
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/api/info', timeout=2) as response:
                    info = json.load(response)
                    if info.get('version') == version and info.get('protocol') == 1:
                        print(f'Native UI started the LAN service on dynamic port {port}')
                        sys.exit(0)
            except Exception:
                pass
        time.sleep(1)
    raise RuntimeError('The native UI did not start the embedded service')
finally:
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
