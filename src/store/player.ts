import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'

export interface Track {
  source: any
}

export interface PlayerState {
  is_playing: boolean
  progress: number
  duration: number
  current_index: number | null
  current_track: Track | null
}

export type PlayMode = 'sequential' | 'random' | 'repeat_one' | 'repeat_all'

export const usePlayerStore = defineStore('player', {
  state: () => ({
    playlist: [] as Track[],
    isPlaying: false,
    progress: 0,
    duration: 0,
    currentIndex: null as number | null,
    currentTrack: null as Track | null,
    playMode: (localStorage.getItem('playMode') as PlayMode) || 'sequential' as PlayMode,
  }),
  actions: {
    async loadPlaylist() {
      try {
        const playlist = await invoke('get_playlist') as Track[]
        console.log('Loaded playlist:', playlist.map((t: Track) => {
          if (t.source.Remote) {
            return { title: t.source.Remote.title, is_downloading: t.source.Remote.is_downloading, cached_path: t.source.Remote.cached_path }
          }
          return { local: t.source.Local }
        }))
        this.playlist = playlist
      } catch (e) {
        console.error('Failed to load playlist', e)
      }
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
      // extraSubtitleLang: optional third language for subtitles (default: zh, en)
      await invoke('download_and_play', { index, extraSubtitleLang: extraSubtitleLang || null })
    },
    async syncState() {
      try {
        const state = await invoke<PlayerState>('get_state')
        this.isPlaying = state.is_playing
        this.progress = state.progress
        this.duration = state.duration
        this.currentIndex = state.current_index
        this.currentTrack = state.current_track
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
