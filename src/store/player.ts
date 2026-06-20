import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'

export interface PlaylistEntry {
  id: string
  item_id: string
  added_at: number
}

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

export interface PlayerState {
  is_playing: boolean
  progress: number
  duration: number
  current_index: number | null
  current_item: LibraryItem | null
}

export type PlayMode = 'sequential' | 'random' | 'repeat_one' | 'repeat_all'

// Type guard functions
export function isSourceLocal(source: LibrarySourceLocal | LibrarySourceRemote): source is LibrarySourceLocal {
  return 'Local' in source
}

export function isSourceRemote(source: LibrarySourceLocal | LibrarySourceRemote): source is LibrarySourceRemote {
  return 'Remote' in source
}

export const usePlayerStore = defineStore('player', {
  state: () => ({
    playlistEntries: [] as PlaylistEntry[],
    libraryTree: [] as LibraryItem[],
    playlist: [] as ResolvedTrack[],
    isPlaying: false,
    progress: 0,
    duration: 0,
    currentIndex: null as number | null,
    currentTrack: null as ResolvedTrack | null,
    playMode: (localStorage.getItem('playMode') as PlayMode) || 'sequential' as PlayMode,
  }),
  actions: {
    async loadPlaylist() {
      try {
        const [playlistEntries, libraryTree] = await Promise.all([
          invoke('get_playlist') as Promise<PlaylistEntry[]>,
          invoke('get_library_tree') as Promise<LibraryItem[]>,
        ])

        this.playlistEntries = playlistEntries
        this.libraryTree = libraryTree

        const libraryMap = this.buildLibraryMap(libraryTree)
        this.playlist = playlistEntries
          .map(entry => libraryMap[entry.item_id])
          .filter((track): track is ResolvedTrack => !!track)

        console.log('Loaded playlist entries:', this.playlistEntries)
        console.log('Resolved playlist items:', this.playlist)
      } catch (e) {
        console.error('Failed to load playlist', e)
      }
    },

    buildLibraryMap(items: LibraryItem[]) {
      const map: Record<string, ResolvedTrack> = {}

      function flatten(item: LibraryItem) {
        if ('Track' in item) {
          map[item.Track.id] = item.Track
        } else if ('Folder' in item) {
          item.Folder.children.forEach(child => flatten(child))
        }
      }

      items.forEach(item => flatten(item))
      return map
    },

    async play(index: number) {
      await invoke('play_track', { index })
    },
    async pause() {
      await invoke('pause')
    },
    async resume() {
      await invoke('resume')
    },
    async seek(progress: number) {
      // optimistic update
      this.progress = progress
      await invoke('seek', { progress })
    },
    async addUrl(url: string) {
      await invoke('add_url_for_download', { url })
    },
    async playRemoteTrack(index: number, extraSubtitleLang?: string) {
      // This will download if needed, then play
      await invoke('download_and_play', { index, extraSubtitleLang: extraSubtitleLang || null })
    },
    async syncState() {
      try {
        const state = await invoke<PlayerState>('get_state')
        this.isPlaying = state.is_playing
        this.progress = state.progress
        this.duration = state.duration
        this.currentIndex = state.current_index

        if (state.current_item && 'Track' in state.current_item) {
          this.currentTrack = state.current_item.Track
        } else {
          this.currentTrack = null
        }
      } catch (e) {
        console.error('Failed to sync state', e)
      }
    },
    async reportPlaybackError() {
      console.log('Reporting playback error to backend...')
      await invoke('on_playback_error')
    },
    setPlayMode(mode: PlayMode) {
      this.playMode = mode
      localStorage.setItem('playMode', mode)
    },
    getNextIndex(): number | null {
      if (this.playlist.length === 0) return null
      if (this.currentIndex === null) return 0

      switch (this.playMode) {
        case 'sequential':
          // Stop at end
          if (this.currentIndex >= this.playlist.length - 1) return null
          return this.currentIndex + 1
        case 'random':
          // Random track (avoid same track if possible)
          if (this.playlist.length === 1) return 0
          let nextIdx: number
          do {
            nextIdx = Math.floor(Math.random() * this.playlist.length)
          } while (nextIdx === this.currentIndex && this.playlist.length > 1)
          return nextIdx
        case 'repeat_one':
          return this.currentIndex
        case 'repeat_all':
          return (this.currentIndex + 1) % this.playlist.length
        default:
          return null
      }
    },
    getPrevIndex(): number | null {
      if (this.playlist.length === 0) return null
      if (this.currentIndex === null) return this.playlist.length - 1

      switch (this.playMode) {
        case 'sequential':
          if (this.currentIndex <= 0) return null
          return this.currentIndex - 1
        case 'random':
          if (this.playlist.length === 1) return 0
          let prevIdx: number
          do {
            prevIdx = Math.floor(Math.random() * this.playlist.length)
          } while (prevIdx === this.currentIndex && this.playlist.length > 1)
          return prevIdx
        case 'repeat_one':
          return this.currentIndex
        case 'repeat_all':
          return this.currentIndex === 0 ? this.playlist.length - 1 : this.currentIndex - 1
        default:
          return null
      }
    }
  }
})
