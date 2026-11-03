import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const workspaceCrates = [
  'easilygit-app',
  'easilygit-desktop',
  'easilygit-domain',
  'easilygit-git',
];

function fail(message) {
  console.error(message);
  process.exit(1);
}

function isSemver(value) {
  return /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(value);
}

async function read(relativePath) {
  return readFile(path.join(root, relativePath), 'utf8');
}

async function write(relativePath, contents) {
  await writeFile(path.join(root, relativePath), contents);
}

function replaceExactlyOne(contents, pattern, replacement, description) {
  const countPattern = new RegExp(
    pattern.source,
    pattern.flags.includes('g') ? pattern.flags : `${pattern.flags}g`,
  );
  const matches = [...contents.matchAll(countPattern)];
  if (matches.length !== 1) {
    fail(`Expected exactly one ${description}, found ${matches.length}.`);
  }
  return contents.replace(pattern, replacement);
}

async function setVersion(version) {
  if (!isSemver(version)) fail(`Invalid semantic version: ${version}`);

  const cargoToml = await read('Cargo.toml');
  await write(
    'Cargo.toml',
    replaceExactlyOne(
      cargoToml,
      /(^\[workspace\.package\]\s*\n(?:[^\n]*\n)*?version\s*=\s*)"[^"]+"/m,
      `$1"${version}"`,
      'workspace package version in Cargo.toml',
    ),
  );

  let cargoLock = await read('Cargo.lock');
  for (const crate of workspaceCrates) {
    const blockPattern = new RegExp(
      `(\\[\\[package\\]\\]\\nname = "${crate}"\\nversion = ")[^"]+(")`,
    );
    cargoLock = replaceExactlyOne(
      cargoLock,
      blockPattern,
      `$1${version}$2`,
      `${crate} package entry in Cargo.lock`,
    );
  }
  await write('Cargo.lock', cargoLock);

  const tauriConfig = JSON.parse(await read('apps/desktop/src-tauri/tauri.conf.json'));
  tauriConfig.version = version;
  await write('apps/desktop/src-tauri/tauri.conf.json', `${JSON.stringify(tauriConfig, null, 2)}\n`);

  const packageJsonPath = 'apps/desktop/package.json';
  const packageJson = JSON.parse(await read(packageJsonPath));
  packageJson.version = version;
  await write(packageJsonPath, `${JSON.stringify(packageJson, null, 2)}\n`);

  const packageLockPath = 'apps/desktop/package-lock.json';
  const packageLock = JSON.parse(await read(packageLockPath));
  packageLock.version = version;
  packageLock.packages[''].version = version;
  await write(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`);

  console.log(`Updated MosSaver version to ${version}.`);
}

async function checkVersion(expectedVersion) {
  if (expectedVersion && !isSemver(expectedVersion)) {
    fail(`Invalid semantic version: ${expectedVersion}`);
  }

  const cargoToml = await read('Cargo.toml');
  const workspaceMatch = cargoToml.match(/^\[workspace\.package\]\s*\n(?:[^\n]*\n)*?version\s*=\s*"([^"]+)"/m);
  if (!workspaceMatch) fail('Could not find workspace package version in Cargo.toml.');
  const versions = new Map([['Cargo.toml [workspace.package]', workspaceMatch[1]]]);

  const cargoLock = await read('Cargo.lock');
  for (const crate of workspaceCrates) {
    const blockPattern = new RegExp(`\\[\\[package\\]\\]\\nname = "${crate}"\\nversion = "([^"]+)"`);
    const match = cargoLock.match(blockPattern);
    if (!match) fail(`Could not find ${crate} package entry in Cargo.lock.`);
    versions.set(`Cargo.lock ${crate}`, match[1]);
  }

  const tauriConfig = JSON.parse(await read('apps/desktop/src-tauri/tauri.conf.json'));
  versions.set('tauri.conf.json', tauriConfig.version);

  const packageJson = JSON.parse(await read('apps/desktop/package.json'));
  versions.set('package.json', packageJson.version);

  const packageLock = JSON.parse(await read('apps/desktop/package-lock.json'));
  versions.set('package-lock.json', packageLock.version);
  versions.set('package-lock.json root package', packageLock.packages?.['']?.version);

  const distinctVersions = new Set(versions.values());
  if (distinctVersions.size !== 1 || (expectedVersion && !distinctVersions.has(expectedVersion))) {
    console.error('MosSaver version fields do not match:');
    for (const [source, version] of versions) console.error(`  ${source}: ${version ?? '(missing)'}`);
    if (expectedVersion) console.error(`  expected: ${expectedVersion}`);
    process.exit(1);
  }

  console.log(`MosSaver version is consistent: ${[...distinctVersions][0]}.`);
}

const [command, value, ...extra] = process.argv.slice(2);
if (extra.length) fail('Too many arguments.');

if (command === 'set' && value) {
  await setVersion(value);
} else if (command === 'check' && !value) {
  await checkVersion();
} else if (command === 'check') {
  await checkVersion(value);
} else {
  fail('Usage: node scripts/version.mjs set <x.y.z> | check [x.y.z]');
}
