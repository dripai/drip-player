import { createWriteStream } from 'node:fs'
import { chmod, copyFile, mkdir, readdir, rename, rm, stat } from 'node:fs/promises'
import { basename, dirname, join, resolve } from 'node:path'
import { pipeline } from 'node:stream/promises'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'

const rootDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const libDir = resolve(rootDir, 'lib')
const cacheDir = resolve(rootDir, '.tool-cache')

const platform = process.platform
const arch = process.arch
const isWindows = platform === 'win32'

const tools = {
  win32: {
    ytDlp: 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe',
    ffmpegArchive: 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip',
  },
  darwin: {
    ytDlp: 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos',
    ffmpeg: arch === 'arm64'
      ? 'https://www.osxexperts.net/ffmpeg81arm.zip'
      : 'https://www.osxexperts.net/ffmpeg80intel.zip',
    ffprobe: arch === 'arm64'
      ? 'https://www.osxexperts.net/ffprobe81arm.zip'
      : 'https://www.osxexperts.net/ffprobe80intel.zip',
    ffplay: arch === 'arm64'
      ? 'https://www.osxexperts.net/ffplay81arm.zip'
      : 'https://www.osxexperts.net/ffplay80intel.zip',
  },
  linux: {
    ytDlp: arch === 'arm64'
      ? 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64'
      : 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux',
    ffmpegArchive: arch === 'arm64'
      ? 'https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz'
      : 'https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz',
  },
}

const selected = tools[platform]
const downloadTimeoutMs = Number(process.env.DRIP_TOOL_DOWNLOAD_TIMEOUT_MS ?? 600000)

if (!selected) {
  console.log(`No bundled media tools configured for ${platform}/${arch}; skipping.`)
  process.exit(0)
}

await mkdir(libDir, { recursive: true })
await mkdir(cacheDir, { recursive: true })

await installYtDlp(selected.ytDlp)

if (platform === 'darwin') {
  await installDarwinFfmpegTools(selected)
} else {
  await installFfmpegArchive(selected.ffmpegArchive)
}

console.log(`Media tools prepared in ${libDir}`)

async function installYtDlp(url) {
  const outputName = isWindows ? 'yt-dlp.exe' : 'yt-dlp'
  const outputPath = join(libDir, outputName)
  if (await isNonEmptyFile(outputPath)) {
    return
  }

  await download(url, outputPath)
  await makeExecutable(outputPath)
}

async function installDarwinFfmpegTools(config) {
  for (const name of ['ffmpeg', 'ffprobe', 'ffplay']) {
    const outputPath = join(libDir, name)
    if (await isNonEmptyFile(outputPath)) {
      continue
    }

    const archive = join(cacheDir, `${name}-macos.zip`)
    const extractDir = join(cacheDir, `${name}-macos`)
    await download(config[name], archive)
    await rm(extractDir, { recursive: true, force: true })
    await mkdir(extractDir, { recursive: true })
    await run('unzip', ['-q', archive, '-d', extractDir])

    const executable = await findFile(extractDir, name)
    if (!executable) {
      throw new Error(`Could not find ${name} in ${archive}`)
    }

    await copyFile(executable, outputPath)
    await makeExecutable(outputPath)
  }
}

async function installFfmpegArchive(url) {
  const required = isWindows
    ? ['ffmpeg.exe', 'ffprobe.exe', 'ffplay.exe']
    : ['ffmpeg', 'ffprobe']

  const ready = await Promise.all(required.map((name) => isNonEmptyFile(join(libDir, name))))
  if (ready.every(Boolean)) {
    return
  }

  const archive = join(cacheDir, basename(new URL(url).pathname))
  const extractDir = join(cacheDir, `${platform}-ffmpeg`)
  await download(url, archive)
  await rm(extractDir, { recursive: true, force: true })
  await mkdir(extractDir, { recursive: true })

  if (archive.endsWith('.zip')) {
    if (isWindows) {
      await run('powershell', ['-NoProfile', '-Command', `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${extractDir}' -Force`])
    } else {
      await run('unzip', ['-q', archive, '-d', extractDir])
    }
  } else if (archive.endsWith('.tar.xz')) {
    await run('tar', ['-xJf', archive, '-C', extractDir])
  } else {
    throw new Error(`Unsupported FFmpeg archive: ${archive}`)
  }

  for (const name of required) {
    const executable = await findFile(extractDir, name)
    if (!executable) {
      throw new Error(`Could not find ${name} in ${archive}`)
    }

    const outputPath = join(libDir, name)
    await copyFile(executable, outputPath)
    await makeExecutable(outputPath)
  }

  if (platform === 'linux') {
    console.log('ffplay is not included in the Linux static FFmpeg package; ffmpeg/ffprobe were bundled.')
  }
}

async function download(url, outputPath) {
  if (await isNonEmptyFile(outputPath)) {
    return
  }

  console.log(`Downloading ${url}`)
  const response = await fetch(url, {
    redirect: 'follow',
    signal: AbortSignal.timeout(downloadTimeoutMs),
  })
  if (!response.ok || !response.body) {
    throw new Error(`Failed to download ${url}: ${response.status} ${response.statusText}`)
  }

  await mkdir(dirname(outputPath), { recursive: true })
  const tmpPath = `${outputPath}.download`
  await rm(tmpPath, { force: true })
  await pipeline(response.body, createWriteStream(tmpPath))
  await rename(tmpPath, outputPath)
}

async function findFile(dir, fileName) {
  const entries = await readdir(dir, { withFileTypes: true })
  for (const entry of entries) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) {
      const found = await findFile(path, fileName)
      if (found) {
        return found
      }
    } else if (entry.name === fileName) {
      return path
    }
  }
  return null
}

async function makeExecutable(path) {
  if (!isWindows) {
    await chmod(path, 0o755)
    if (platform === 'darwin') {
      await runOptional('xattr', ['-cr', path])
      await runOptional('codesign', ['-s', '-', path])
    }
  } else {
    await stat(path)
  }
}

async function isNonEmptyFile(path) {
  try {
    const info = await stat(path)
    return info.isFile() && info.size > 0
  } catch {
    return false
  }
}

async function run(command, args) {
  await new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { stdio: 'inherit' })
    child.on('error', reject)
    child.on('exit', (code) => {
      if (code === 0) {
        resolvePromise()
      } else {
        reject(new Error(`${command} exited with code ${code}`))
      }
    })
  })
}

async function runOptional(command, args) {
  try {
    await run(command, args)
  } catch (error) {
    console.warn(`${command} failed for ${args.at(-1)}: ${error.message}`)
  }
}
