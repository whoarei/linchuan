// Chrome 74 supports ES modules/dynamic import, but not optional chaining or
// nullish coalescing. Check emitted assets, not only our authored source.
import { parse } from 'acorn';
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';

function check(source, file) {
 const tree = parse(source, { ecmaVersion: 2020, sourceType: 'module' });
 function visit(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'ChainExpression' || (node.type === 'LogicalExpression' && node.operator === '??')) {
   throw new Error(`${file}: syntax unsupported by WebView 74 at offset ${node.start}`);
  }
  for (const value of Object.values(node)) {
   if (Array.isArray(value)) value.forEach(visit);
   else if (value && typeof value === 'object') visit(value);
  }
 }
 visit(tree);
}

async function inspect(file) {
 const source = await readFile(file, 'utf8');
 if (file.endsWith('.html')) {
  for (const match of source.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)) check(match[1], file);
 } else check(source, file);
}
const files = process.argv.slice(2);
if (!files.length) {
 files.push('dist/index.html');
 for (const file of await readdir('dist/assets')) if (file.endsWith('.js')) files.push(path.join('dist/assets', file));
}
for (const file of files) await inspect(file);
console.log(`WebView 74 syntax check passed (${files.length} files). Tauri-injected scripts require separate device validation.`);
