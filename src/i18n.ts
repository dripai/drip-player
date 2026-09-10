import { createI18n } from 'vue-i18n'

const messages = {
  en: {
    downloads: {
      title: 'Downloads', start: 'Download', placeholder: 'Paste a media URL',
      parse: 'Analyze', reparse: 'Analyze again', redownload: 'Download again', quality: 'Quality / format',
      account: 'Sign-in source', public: 'Without sign-in', originalVideo: 'Video · original resolution',
      originalAudio: 'Audio · original format', mp3Only: 'Audio only · MP3', partial: 'Partial content: {duration}',
      selectFormat: 'Select a format', noFullFormat: 'No complete format available', reparseSource: 'Analyze again after changing the sign-in source.',
      activeCount: 'Downloads ({count} active)', empty: 'No downloads',
      clearHistory: 'Clear history',
      cancel: 'Cancel', continue: 'Continue', retry: 'Retry', progress: 'Download progress',
      saveTo: 'Save to', eta: '{seconds}s remaining', login: 'Log in',
      loginNeeded: 'Log in to {platform} in your browser, then retry.',
      directoryChanged: 'Interrupted · directory switch',
      phase: {
        resolving: 'Resolving', awaiting_selection: 'Choose a format', downloading: 'Downloading', verifying: 'Verifying', publishing: 'Saving',
        completed: 'Completed', failed: 'Failed', canceled: 'Canceled', interrupted: 'Interrupted',
        recovery_required: 'Recovery required'
      }
    },
    settings: {
      title: 'Settings',
      general: 'General',
      savedAutomatically: 'Changes are saved automatically.',
      loading: 'Loading settings…',
      loadFailed: 'Unable to load settings.',
      theme: 'Appearance',
      themeSystem: 'System',
      themeLight: 'Light',
      themeDark: 'Dark',
      language: 'Language',
      downloadDirectory: 'Download directory',
      chooseDownloadDirectory: 'Choose download directory',
      saveDownloadDirectory: 'Save download directory',
      choose: 'Choose',
      save: 'Save',
      saving: 'Saving…',
      closeToTray: 'Close to system tray',
      closeToTrayHint: 'Keep the player running when the main window is closed.',
      reload: 'Reload settings',
      dismiss: 'Dismiss'
    },
    app: {
      title: 'Shadow Player'
    },
    sidebar: {
      playlist: 'Playlist',
      tracks: 'tracks',
      addUrl: 'Paste URL (YouTube, etc)...',
      addFiles: 'Add Files',
      addFolder: 'Add Folder',
      refresh: 'Refresh playlist',
      directory: 'Change directory'
    },
    menu: {
      refresh: 'Refresh',
      rename: 'Rename',
      delete: 'Delete',
      removeFromPlaylist: 'Remove from playlist',
      clearPlaylist: 'Clear entire playlist',
      clearFolderTree: 'Clear folder tree'
    },
    fileAction: {
      name: 'Name', cancel: 'Cancel', working: 'Working…',
      deleteTitle: 'Permanently delete file?', deleteConfirm: 'Delete permanently',
      deleteHint: 'This media file will be permanently deleted. Subtitles and learning records will be kept.',
      recordingBusy: 'Stop and save the current recording first.'
    },
    player: {
      hevcUnsupported: 'This device could not decode HEVC video. Check the system HEVC decoder or download an H.264 format.',
      videoFailed: 'Video playback failed',
      noTrack: 'No Track Playing',
      externalControls: 'Control playback in the MPV window',
      externalStopped: 'MPV window closed',
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
    downloads: {
      title: '下载', start: '下载', placeholder: '粘贴媒体链接',
      parse: '解析', reparse: '重新解析', redownload: '重新下载', quality: '下载格式',
      account: '登录来源', public: '不使用登录', originalVideo: '视频 · 原始分辨率',
      originalAudio: '音频 · 原始格式', mp3Only: '仅音频 · MP3', partial: '仅部分内容 {duration}',
      selectFormat: '选择下载格式', noFullFormat: '暂无完整内容格式', reparseSource: '更换登录来源后请重新解析。',
      activeCount: '下载（{count} 个进行中）', empty: '暂无下载任务',
      clearHistory: '清理历史记录',
      cancel: '取消', continue: '继续', retry: '重试', progress: '下载进度',
      saveTo: '保存到', eta: '剩余 {seconds} 秒', login: '登录',
      loginNeeded: '请在浏览器中登录 {platform} 后重试。',
      directoryChanged: '已中断 · 切换保存目录',
      phase: {
        resolving: '解析中', awaiting_selection: '待选择格式', downloading: '下载中', verifying: '校验中', publishing: '保存中',
        completed: '已完成', failed: '失败', canceled: '已取消', interrupted: '已中断',
        recovery_required: '需要恢复'
      }
    },
    settings: {
      title: '设置',
      general: '通用设置',
      savedAutomatically: '修改后自动保存。',
      loading: '正在读取设置…',
      loadFailed: '无法读取设置。',
      theme: '外观',
      themeSystem: '跟随系统',
      themeLight: '浅色',
      themeDark: '深色',
      language: '界面语言',
      downloadDirectory: '下载保存目录',
      chooseDownloadDirectory: '选择下载保存目录',
      saveDownloadDirectory: '保存下载目录',
      choose: '选择',
      save: '保存',
      saving: '保存中…',
      closeToTray: '关闭主窗口时最小化到托盘',
      closeToTrayHint: '关闭后继续在后台运行，可从托盘恢复窗口。',
      reload: '重新加载设置',
      dismiss: '关闭'
    },
    app: {
      title: '影子播放器'
    },
    sidebar: {
      playlist: '播放列表',
      tracks: '首歌曲',
      addUrl: '粘贴 URL（YouTube、B站等）...',
      addFiles: '添加文件',
      addFolder: '添加文件夹',
      refresh: '刷新播放列表',
      directory: '切换目录'
    },
    menu: {
      refresh: '刷新',
      rename: '重命名',
      delete: '删除',
      removeFromPlaylist: '从播放列表移除',
      clearPlaylist: '清空播放列表',
      clearFolderTree: '清空文件夹树'
    },
    fileAction: {
      name: '名称', cancel: '取消', working: '处理中…',
      deleteTitle: '永久删除文件？', deleteConfirm: '永久删除',
      deleteHint: '此媒体文件将被永久删除，无法撤销。字幕文件和学习记录会保留。',
      recordingBusy: '请先停止并保存当前录音。'
    },
    player: {
      hevcUnsupported: '当前设备无法解码 HEVC 视频，请检查系统 HEVC 解码器，或重新下载 H.264 格式。',
      videoFailed: '视频播放失败',
      noTrack: '未播放',
      externalControls: '请在 MPV 窗口中控制播放',
      externalStopped: 'MPV 窗口已关闭',
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
  locale: 'zh',
  fallbackLocale: 'en',
  messages
})

export default i18n
