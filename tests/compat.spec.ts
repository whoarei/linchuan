import { test, expect } from '@playwright/test';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
const run = promisify(execFile);

test('compatibility guard accepts legacy modules and rejects unsupported inline syntax', async () => {
 const directory = await mkdtemp(path.join(tmpdir(), 'linchuan-compat-'));
 try {
  const file = path.join(directory, 'sample.html');
  await writeFile(file, '<script type="module">const a = {}; const b = a && a.name; import("./other.js");</script>');
  await expect(run(process.execPath, ['scripts/check-webview-compat.mjs', file])).resolves.toBeDefined();
  for (const source of ['const b = a?.name;', 'const b = a ?? "fallback";']) {
   await writeFile(file, `<script>${source}</script>`);
   await expect(run(process.execPath, ['scripts/check-webview-compat.mjs', file])).rejects.toThrow();
  }
 } finally {
  await rm(directory, { recursive: true, force: true });
 }
});
