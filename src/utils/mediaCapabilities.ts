import { isSourceLocal, isSourceRemote, type ResolvedTrack } from '../store/player'

export const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'm4a', 'aac', 'opus']
export const BROWSER_VIDEO_EXTENSIONS = ['mp4', 'm4v', 'webm']
export const EXTERNAL_VIDEO_EXTENSIONS = ['mkv', 'avi', 'mov', 'flv', 'wmv', 'ts', 'm2ts', 'mpg', 'mpeg', '3gp']
export const VIDEO_EXTENSIONS = [...BROWSER_VIDEO_EXTENSIONS, ...EXTERNAL_VIDEO_EXTENSIONS]
export const MEDIA_EXTENSIONS = [...AUDIO_EXTENSIONS, ...VIDEO_EXTENSIONS]

export function getPathExtension(path: string | null | undefined) {
  return path?.split('.').pop()?.toLowerCase() || ''
}

export function getTrackFilePath(track: ResolvedTrack | null | undefined) {
  if (!track) return null
  if (isSourceLocal(track.source)) return track.source.Local.path
  if (isSourceRemote(track.source) && track.source.Remote.cached_path) return track.source.Remote.cached_path
  return null
}

export function trackHasVideo(track: ResolvedTrack | null | undefined) {
  if (!track) return false
  if (track.media_type === 'Video') return true
  if (isSourceRemote(track.source)) return track.source.Remote.media_type === 'Video'
  return VIDEO_EXTENSIONS.includes(getPathExtension(getTrackFilePath(track)))
}
