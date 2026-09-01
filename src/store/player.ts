import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'

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
  canonical_key: string
  title: string
  media_type: 'Audio' | 'Video'
  origin: PlaylistOrigin
  cached_path?: string | null
  download_status: 'not_downloaded' | 'downloading' | 'downloaded'
  added_at: number
}

export interface PlaylistSnapshot {
  revision: number
  items: PlaylistItem[]
}

export interface PlayerState {
  is_playing: boolean
  progress: number
  duration: number
  current_item_id: string | null
  current_item: LibraryItem | null
}

export interface AddUrlResult {
  outcome: 'added' | 'already_present'
  item_id: string
}

export type PlayMode = 'sequential' | 'random' | 'repeat_one' | 'repeat_all'

export function isSourceLocal(source: LibrarySourceLocal | LibrarySourceRemote): source is LibrarySourceLocal {
  return 'Local' in source
}

export function isSourceRemote(source: LibrarySourceLocal | LibrarySourceRemote): source is LibrarySourceRemote {
  return 'Remote' in source
}

export const usePlayerStore = defineStore('player', {
  state: () => ({
    playlist: [] as PlaylistItem[],
    playlistRevision: 0,
    isPlaying: false,
    progress: 0,
    duration: 0,
    currentItemId: null as string | null,
    currentTrack: null as ResolvedTrack | null,
    playMode: ((localStorage.getItem('playMode') as PlayMode) || 'sequential') as PlayMode,
  }),
  actions: {
    async loadPlaylist() {
      try {
        const snapshot = await invoke<PlaylistSnapshot>('get_playlist')
        if (snapshot.revision < this.playlistRevision) return
        this.playlistRevision = snapshot.revision
        this.playlist = snapshot.items
      } catch (error) {
        console.error('Failed to load playlist', error)
      }
    },

    async play(itemId: string) {
      const item = this.playlist.find(candidate => candidate.id === itemId)
      if (item?.origin.kind === 'remote') {
        await this.playRemoteTrack(itemId)
        return
      }
      await invoke('play_item', { itemId })
    },
    async pause() {
      await invoke('pause')
    },
    async resume() {
      await invoke('resume')
    },
    async seek(progress: number) {
      this.progress = progress
      await invoke('seek', { progress })
    },
    async addUrl(url: string): Promise<AddUrlResult> {
      const result = await invoke<AddUrlResult>('add_url_for_download', { url })
      await this.loadPlaylist()
      return result
    },
    async playRemoteTrack(itemId: string, extraSubtitleLang?: string) {
      await invoke('download_and_play', {
        itemId,
        extraSubtitleLang: extraSubtitleLang || null,
      })
      await Promise.all([this.loadPlaylist(), this.syncState()])
    },
    async syncState() {
      try {
        const state = await invoke<PlayerState>('get_state')
        this.isPlaying = state.is_playing
        this.progress = state.progress
        this.duration = state.duration
        this.currentItemId = state.current_item_id
        this.currentTrack = state.current_item && 'Track' in state.current_item
          ? state.current_item.Track
          : null
      } catch (error) {
        console.error('Failed to sync state', error)
      }
    },
    async reportPlaybackError() {
      await invoke('on_playback_error')
    },
    setPlayMode(mode: PlayMode) {
      this.playMode = mode
      localStorage.setItem('playMode', mode)
    },
    getNextItemId(): string | null {
      if (this.playlist.length === 0) return null
      const currentIndex = this.currentItemId
        ? this.playlist.findIndex(item => item.id === this.currentItemId)
        : -1
      if (currentIndex < 0) return this.playlist[0].id

      switch (this.playMode) {
        case 'sequential':
          return currentIndex >= this.playlist.length - 1
            ? null
            : this.playlist[currentIndex + 1].id
        case 'random': {
          if (this.playlist.length === 1) return this.playlist[0].id
          let nextIndex: number
          do {
            nextIndex = Math.floor(Math.random() * this.playlist.length)
          } while (nextIndex === currentIndex)
          return this.playlist[nextIndex].id
        }
        case 'repeat_one':
          return this.playlist[currentIndex].id
        case 'repeat_all':
          return this.playlist[(currentIndex + 1) % this.playlist.length].id
      }
    },
    getPrevItemId(): string | null {
      if (this.playlist.length === 0) return null
      const currentIndex = this.currentItemId
        ? this.playlist.findIndex(item => item.id === this.currentItemId)
        : -1
      if (currentIndex < 0) return this.playlist[this.playlist.length - 1].id

      switch (this.playMode) {
        case 'sequential':
          return currentIndex <= 0 ? null : this.playlist[currentIndex - 1].id
        case 'random': {
          if (this.playlist.length === 1) return this.playlist[0].id
          let previousIndex: number
          do {
            previousIndex = Math.floor(Math.random() * this.playlist.length)
          } while (previousIndex === currentIndex)
          return this.playlist[previousIndex].id
        }
        case 'repeat_one':
          return this.playlist[currentIndex].id
        case 'repeat_all':
          return this.playlist[
            currentIndex === 0 ? this.playlist.length - 1 : currentIndex - 1
          ].id
      }
    },
  },
})
