import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const source = join(root, 'app-icon.png')
const tauri = join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
const temporaryRoot = resolve(tmpdir())
const output = mkdtempSync(join(temporaryRoot, 'shadow-player-icons-'))

try {
  for (const extra of [[], ['--png', '192', '--png', '512']]) {
    execFileSync(process.execPath, [tauri, 'icon', source, '--output', output, ...extra], {
      cwd: root,
      stdio: 'inherit',
    })
  }

  const desktop = [
    '32x32.png',
    '64x64.png',
    '128x128.png',
    '128x128@2x.png',
    'icon.png',
    'icon.ico',
    'icon.icns',
  ].map(name => [name, join('src-tauri', 'icons', name)])

  const files = [
    ...desktop,
    ['192x192.png', 'src-tauri/icons/icon-192.png'],
    ['512x512.png', 'src-tauri/icons/icon-512.png'],
    ['icon.png', 'public/icon.png'],
    ['icon.ico', 'public/icon.ico'],
  ]

  // Verify every generated input before replacing any project asset.
  for (const [name] of files) readFileSync(join(output, name))
  for (const [name, relativePath] of files) {
    const destination = join(root, relativePath)
    mkdirSync(dirname(destination), { recursive: true })
    copyFileSync(join(output, name), destination)
  }
  console.log('Updated desktop icons and frontend icons from app-icon.png.')
} finally {
  const resolvedOutput = resolve(output)
  if (dirname(resolvedOutput) !== temporaryRoot || !resolvedOutput.startsWith(temporaryRoot + sep + 'shadow-player-icons-')) {
    throw new Error('Refusing to remove an unexpected icon staging directory.')
  }
  rmSync(resolvedOutput, { recursive: true, force: true })
}
