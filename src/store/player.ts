import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { useSettingsStore, type PlayMode } from './settings'
export type { PlayMode } from './settings'

export interface LibrarySourceLocal {
  Local: {
    path: string
  }
}

export interface LibrarySourceRemote {
  Remote: {
    url: string
    id: string
    cached_path?: string | null
    media_type: 'Audio' | 'Video'
    download_status: 'NotDownloaded' | 'Downloading' | 'Downloaded'
  }
}

export interface LibraryTrack {
  media_id?: string
  id: string
  title: string
  media_type: 'Audio' | 'Video'
  source: LibrarySourceLocal | LibrarySourceRemote
  parent?: string | null
}

export type ResolvedTrack = LibraryTrack

export type LibraryItem =
  | { Track: LibraryTrack }
  | { Folder: { name: string; path: string; children: LibraryItem[] } }

export type PlaylistOrigin =
  | { kind: 'local'; path: string }
  | { kind: 'remote'; url: string; provider: string; external_id: string }

export interface PlaylistItem {
  id: string
  media_id: string
  canonical_key: string
  title: string
  media_type: 'Audio' | 'Video'
  origin: PlaylistOrigin
  cached_path?: string | null
  download_status: 'not_downloaded' | 'downloaded'
  added_at: number
}

export interface PlaylistSnapshot {
  revision: number
  items: PlaylistItem[]
}

export interface MediaAsset {
  id: string
  media_id: string
  kind: 'playback' | 'subtitle'
  path: string
  language: string | null
  source: 'local' | 'download'
}

export interface Media {
  id: string
  canonical_key: string
  title: string
  media_type: 'Audio' | 'Video'
  origin: PlaylistOrigin
  assets: MediaAsset[]
}
export type PlaybackStatus = 'preparing' | 'ready' | 'playing' | 'paused' | 'buffering' | 'ended' | 'stopped' | 'failed' | 'external'
export interface PlaybackSession {
  id: number
  media: Media
  playlist_entry_id: string | null
  plan: { engine: 'browser_video' | 'external_video' | 'audio'; path: string } | null
  status: PlaybackStatus
  position: number
  duration: number
  error: string | null
}
export interface PlaybackSnapshot { revision: number; session: PlaybackSession | null }

export function isSourceLocal(source: LibrarySourceLocal | LibrarySourceRemote): source is LibrarySourceLocal { return 'Local' in source }
export function isSourceRemote(source: LibrarySourceLocal | LibrarySourceRemote): source is LibrarySourceRemote { return 'Remote' in source }

export const usePlayerStore = defineStore('player', {
  state: () => ({
    playlist: [] as PlaylistItem[], playlistRevision: 0,
    refreshingPlaylist: false,
    playbackRevision: 0, session: null as PlaybackSession | null,
    lastAdvancedSession: null as number | null,
    learningActive: false,
    error: '',
  }),
  getters: {
    playMode: (): PlayMode => useSettingsStore().data.play_mode,
    isPlaying: state => state.session?.status === 'playing' || state.session?.status === 'buffering',
    duration: state => state.session?.duration || 0,
    progress: state => state.session && state.session.duration > 0 ? Math.min(1, state.session.position / state.session.duration) : 0,
    currentItemId: state => state.session?.playlist_entry_id || null,
    currentTrack: (state): ResolvedTrack | null => {
      const media = state.session?.media
      if (!media) return null
      const cached = media.assets.find(asset => asset.kind === 'playback')?.path
      const source = media.origin.kind === 'local' ? { Local: { path: media.origin.path } } : {
        Remote: { url: media.origin.url, id: media.origin.external_id, cached_path: cached,
          media_type: media.media_type, download_status: cached ? 'Downloaded' as const : 'NotDownloaded' as const } }
      return { id: media.id, media_id: media.id, title: media.title, media_type: media.media_type, source }
    },
  },
  actions: {
    acceptPlaylist(snapshot: PlaylistSnapshot) {
      if (snapshot.revision < this.playlistRevision) return
      this.playlistRevision = snapshot.revision
      this.playlist = snapshot.items
    },
    async loadPlaylist() {
      try {
        const snapshot = await invoke<PlaylistSnapshot>('get_playlist')
        this.acceptPlaylist(snapshot)
      } catch (error) { this.error = String(error) }
    },
    async refreshPlaylist() {
      if (this.refreshingPlaylist) return
      this.refreshingPlaylist = true
      this.error = ''
      try {
        this.acceptPlaylist(await invoke<PlaylistSnapshot>('refresh_playlist'))
        await this.syncState()
      } catch (error) { this.error = String(error) }
      finally { this.refreshingPlaylist = false }
    },
    acceptSnapshot(snapshot: PlaybackSnapshot) {
      if (snapshot.revision < this.playbackRevision) return
      this.playbackRevision = snapshot.revision
      this.session = snapshot.session
      if (snapshot.session?.error) this.error = snapshot.session.error
      const session = snapshot.session
      if (!this.learningActive && session?.status === 'ended' && session.playlist_entry_id && this.lastAdvancedSession !== session.id) {
        this.lastAdvancedSession = session.id
        void this.advance(false, true).catch(error => { this.error = String(error) })
      }
    },
    async syncState() {
      try { this.acceptSnapshot(await invoke<PlaybackSnapshot>('get_state')) }
      catch (error) { this.error = String(error) }
    },
    async play(itemId: string) {
      this.error = ''
      try { await invoke('play_item', { itemId }) }
      finally { await this.syncState() }
    },
    async playPath(path: string) {
      this.error = ''
      try { await invoke('play_track_directly', { path }) }
      finally { await this.syncState() }
    },
    async pause() { if (this.session) await invoke('pause', { sessionId: this.session.id }) },
    async resume() { if (this.session) await invoke('resume', { sessionId: this.session.id }) },
    async seek(progress: number) {
      if (this.session) await invoke('seek', { sessionId: this.session.id, position: this.duration * progress })
    },
    async advance(backwards: boolean, automatic = false) {
      if (!this.session) {
        const entry = backwards ? this.playlist[this.playlist.length - 1] : this.playlist[0]
        if (entry) await this.play(entry.id)
        return
      }
      await invoke('advance_playback', { sessionId: this.session.id, backwards, automatic })
      await this.syncState()
    },
    async replay() {
      if (this.currentItemId) await this.play(this.currentItemId)
      else if (this.session?.media.origin.kind === 'local') await this.playPath(this.session.media.origin.path)
    },
    async setPlayMode(mode: PlayMode) { return useSettingsStore().update({ play_mode: mode }) },
  },
})
