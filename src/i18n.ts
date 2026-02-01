import { createI18n } from 'vue-i18n'

const messages = {
  en: {
    app: {
      title: 'Drip Player'
    },
    sidebar: {
      playlist: 'Playlist',
      tracks: 'tracks',
      addUrl: 'Paste URL (YouTube, etc)...',
      addFiles: 'Add Files',
      addFolder: 'Add Folder'
    },
    menu: {
      removeFromPlaylist: 'Remove from playlist',
      clearPlaylist: 'Clear entire playlist',
      clearFolderTree: 'Clear folder tree'
    },
    player: {
      noTrack: 'No Track Playing',
      unknown: 'Unknown',
      playMode: {
        sequential: 'Sequential',
        random: 'Shuffle',
        repeat_one: 'Repeat One',
        repeat_all: 'Repeat All'
      },
      subtitle: {
        title: 'Subtitles',
        off: 'Off',
        noSubtitles: 'No subtitles available'
      }
    },
    login: {
      required: 'Login Required',
      message: '{platform} requires you to log in to access this video. Please log in using your browser, then try again.',
      oauthRecommended: 'Quick Authorization (Recommended)',
      oauthDesc: 'Click the button below to authorize. If you are already logged in to your browser, you only need to click to confirm.',
      oauthButton: 'Authorize with Google',
      authorizing: 'Authorizing...',
      manualSteps: 'Or login manually:',
      step1: 'Click "Open Browser" to go to the login page',
      step2: 'Log in with your account',
      step3: 'Close the browser and click "Retry"',
      openBrowser: 'Open Browser',
      retry: 'Retry'
    }
  },
  zh: {
    app: {
      title: 'Drip Player'
    },
    sidebar: {
      playlist: '播放列表',
      tracks: '首歌曲',
      addUrl: '粘贴 URL（YouTube、B站等）...',
      addFiles: '添加文件',
      addFolder: '添加文件夹'
    },
    menu: {
      removeFromPlaylist: '从播放列表移除',
      clearPlaylist: '清空播放列表',
      clearFolderTree: '清空文件夹树'
    },
    player: {
      noTrack: '未播放',
      unknown: '未知',
      playMode: {
        sequential: '顺序播放',
        random: '随机播放',
        repeat_one: '单曲循环',
        repeat_all: '列表循环'
      },
      subtitle: {
        title: '字幕',
        off: '关闭',
        noSubtitles: '无可用字幕'
      }
    },
    login: {
      required: '需要登录',
      message: '{platform} 需要登录才能访问此视频。请在浏览器中登录后重试。',
      oauthRecommended: '快速授权（推荐）',
      oauthDesc: '点击下方按钮进行授权。如果浏览器已登录，只需点击确认即可。',
      oauthButton: '使用 Google 授权',
      authorizing: '授权中...',
      manualSteps: '或手动登录：',
      step1: '点击"打开浏览器"前往登录页面',
      step2: '使用您的账号登录',
      step3: '关闭浏览器后点击"重试"',
      openBrowser: '打开浏览器',
      retry: '重试'
    }
  }
}

const i18n = createI18n({
  legacy: false,
  locale: localStorage.getItem('locale') || 'zh',
  fallbackLocale: 'en',
  messages
})

export default i18n
