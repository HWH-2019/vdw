import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const assets = resolve(root, '../vdw/dashboard-assets');
const libraries = ['react', 'react-dom', 'scheduler', '@tanstack/react-query', '@tanstack/query-core'];
if (!process.argv.includes('--verify')) {
  writeFileSync(resolve(assets, 'THIRD_PARTY_NOTICES.txt'), libraries.map(name =>
    `${name}\n${'='.repeat(name.length)}\n${readFileSync(resolve(root, 'node_modules', name, 'LICENSE'), 'utf8')}`
  ).join('\n\n'));
}
function inventory(path, base = path) {
  return readdirSync(path, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap(entry => {
    const file = resolve(path, entry.name);
    return entry.isDirectory() ? inventory(file, base) : [relative(base, file).replaceAll('\\', '/')];
  });
}
function hash(path) { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
const sources = ['package.json', 'package-lock.json', 'tsconfig.json', 'vite.config.ts', 'index.html',
  ...inventory(resolve(root, 'src')).map(p => `src/${p}`),
  ...inventory(resolve(root, 'public')).map(p => `public/${p}`), 'scripts/manifest.mjs'];
const manifest = {
  schema_version: 1,
  source_hash: createHash('sha256').update(sources.sort().map(p => `${p}:${hash(resolve(root, p))}\n`).join('')).digest('hex'),
  sources: Object.fromEntries(sources.sort().map(p => [p, hash(resolve(root, p))])),
  files: Object.fromEntries(inventory(assets).filter(p => p !== 'manifest.json').map(p => [p, hash(resolve(assets, p))])),
};
if (process.argv.includes('--verify')) {
  if (JSON.stringify(manifest) !== JSON.stringify(JSON.parse(readFileSync(resolve(assets, 'manifest.json'), 'utf8')))) {
    throw new Error('Dashboard assets are stale or corrupt. Run: cd vdw-dashboard && npm ci && npm run build');
  }
  console.log('Dashboard sources and embedded asset hashes match.');
} else {
  writeFileSync(resolve(assets, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
}
