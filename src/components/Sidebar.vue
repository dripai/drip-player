<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { usePlayerStore, type ResolvedTrack, isSourceRemote, isSourceLocal } from '../store/player'
import { Plus, Music, Video, FolderOpen, Folder, Loader2 } from 'lucide-vue-next'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useI18n } from 'vue-i18n'
import TreeNode from './TreeNode.vue'
import LoginDialog from './LoginDialog.vue'
import { MEDIA_EXTENSIONS, trackHasVideo } from '../utils/mediaCapabilities'

const store = usePlayerStore()
const { t, locale } = useI18n()
const urlInput = ref('')
const folderTree = ref<any>(null)
const expandedFolders = ref<Set<string>>(new Set())
const isResolvingUrl = ref(false)

// Login dialog state
const showLoginDialog = ref(false)
const loginPlatform = ref('')
const loginUrl = ref('')
const pendingUrl = ref('')

interface LoginRequiredInfo {
  platform: string
  login_url: string
  message: string
}

onMounted(async () => {
    // 监听清空文件夹树事件
    await listen('clear-folder-tree', () => {
        clearFolderTree()
    })

    // 监听 URL 解析状态
    await listen('url-resolving', (event: any) => {
        isResolvingUrl.value = event.payload as boolean
    })
})

/**
 * 添加网络 URL
 */
async function addUrl() {
  if (!urlInput.value || isResolvingUrl.value) return
  const url = urlInput.value
  urlInput.value = ''
  pendingUrl.value = url
  try {
    await store.addUrl(url)
  } catch (e: any) {
    console.error('Failed to add URL:', e)
    const errorStr = String(e)

    // 检查是否需要登录
    const loginInfo = await invoke<LoginRequiredInfo | null>('check_login_required', { error: errorStr })
    if (loginInfo) {
      loginPlatform.value = loginInfo.platform
      loginUrl.value = loginInfo.login_url
      showLoginDialog.value = true
    } else {
      // 其他错误恢复 URL
      urlInput.value = url
    }
  }
}

/**
 * 重试添加 URL (通常在登录成功后)
 */
async function retryUrl(url: string) {
  urlInput.value = url
  await addUrl()
}

/**
 * 添加本地媒体文件
 */
async function addLocalFiles() {
    try {
        const selected = await open({
            multiple: true,
            filters: [{
                name: 'Media Files',
                extensions: MEDIA_EXTENSIONS
            }]
        });

        if (selected && selected.length > 0) {
            const paths = Array.isArray(selected) ? selected : [selected];
            await invoke('add_local_files', { paths });
        }
    } catch (err) {
        console.error('Failed to open file dialog:', err);
    }
}

/**
 * 添加本地文件夹
 */
async function addFolder() {
    try {
        const selected = await open({
            directory: true,
        });

        if (selected) {
            const folderPath = Array.isArray(selected) ? selected[0] : selected;
            const tree = await invoke('get_folder_tree', { folderPath }) as any;
            folderTree.value = tree;
            // 自动展开根目录
            if (tree && tree.Folder) {
                expandedFolders.value.add(tree.Folder.path);
            }
        }
    } catch (err) {
        console.error('Failed to add folder:', err);
    }
}

/**
 * 切换文件夹展开/折叠状态
 */
function toggleFolder(path: string) {
    if (expandedFolders.value.has(path)) {
        expandedFolders.value.delete(path);
    } else {
        expandedFolders.value.add(path);
    }
}

/**
 * 播放曲目
 * 直接播放文件夹树中的文件，不添加到播放列表
 */
async function playTrack(track: any) {
    try {
        await invoke('play_track_directly', { item: { Track: track } });
        // 同步状态以更新播放状态（isPlaying等）
        await store.syncState();
    } catch (err) {
        console.error('Failed to play track directly:', err);
    }
}

/**
 * 处理曲目双击事件
 * 如果是远程曲目且未下载，则先下载
 */
async function handleTrackDoubleClick(index: number) {
    const track = store.playlist[index];

    if (isSourceRemote(track.source) && track.source.Remote.download_status === 'Downloading') {
        console.log('Track is downloading, please wait...');
        return;
    }

    if (isSourceRemote(track.source)) {
        // Remote track - download first, then play
        await store.playRemoteTrack(index);
    } else {
        // Local track - play directly
        await store.play(index);
    }
}

/**
 * 检查曲目是否正在下载
 */
function isDownloading(track: ResolvedTrack) {
    return isSourceRemote(track.source) && track.source.Remote.download_status === 'Downloading';
}

/**
 * 获取曲目显示标题
 */
function getTitle(track: ResolvedTrack) {
    if (isSourceLocal(track.source)) {
        const path = track.source.Local.path
        return path.split(/[/\\]/).pop() || path
    }
    if (isSourceRemote(track.source)) {
        if (track.source.Remote.cached_path) {
            const path = track.source.Remote.cached_path
            return path.split(/[/\\]/).pop() || path
        }
        return track.title || track.source.Remote.url
    }
    return 'Unknown Track'
}

/**
 * 判断是否为视频文件
 */
function isVideo(track: ResolvedTrack) {
    return trackHasVideo(track)
}

const totalTracks = computed(() => {
    let count = store.playlist.length;
    if (folderTree.value) {
        count += countTracksInTree(folderTree.value);
    }
    return count;
});

/**
 * 递归统计文件夹树中的曲目数量
 */
function countTracksInTree(item: any): number {
    if (item.Track) return 1;
    if (item.Folder) {
        return item.Folder.children.reduce((sum: number, child: any) => sum + countTracksInTree(child), 0);
    }
    return 0;
}

/**
 * 显示曲目右键菜单
 */
async function showContextMenu(e: MouseEvent, index: number) {
    e.preventDefault()
    try {
        await invoke('show_track_context_menu', { index, locale: locale.value })
    } catch (err) {
        console.error('Failed to show context menu:', err)
    }
}

/**
 * 显示播放列表右键菜单（如清空列表）
 */
async function showClearMenu(e: MouseEvent) {
    e.preventDefault()
    try {
        await invoke('show_playlist_context_menu', { locale: locale.value })
    } catch (err) {
        console.error('Failed to show context menu:', err)
    }
}

/**
 * 清空文件夹树
 */
function clearFolderTree() {
    folderTree.value = null
    expandedFolders.value.clear()
}
</script>

<template>
  <div class="flex flex-col h-full bg-zinc-50 dark:bg-zinc-900/50">
    <div
        class="p-4 border-b dark:border-zinc-800 flex justify-between items-center"
        @contextmenu="showClearMenu"
    >
        <h2 class="font-semibold text-sm uppercase text-zinc-500 dark:text-zinc-400">{{ t('sidebar.playlist') }}</h2>
        <span class="text-xs text-zinc-400">{{ totalTracks }} {{ t('sidebar.tracks') }}</span>
    </div>

    <div class="flex-1 overflow-y-auto p-2 space-y-0.5">
        <!-- Folder Tree -->
        <div v-if="folderTree" class="mb-2">
            <TreeNode
                :item="folderTree"
                :level="0"
                :expanded-folders="expandedFolders"
                @toggle-folder="toggleFolder"
                @play-track="playTrack"
                :current-index="store.currentIndex"
                :current-track="store.currentTrack"
                :playlist="store.playlist"
            />
        </div>

        <!-- Playlist -->
        <div
            v-for="(track, index) in store.playlist"
            :key="index"
            @dblclick="handleTrackDoubleClick(index)"
            @contextmenu="showContextMenu($event, index)"
            class="group flex items-center px-2 py-1 rounded transition-colors select-none"
            :class="{
                'bg-zinc-200 dark:bg-zinc-800 text-blue-600 dark:text-blue-400': store.currentIndex === index,
                'cursor-pointer hover:bg-zinc-200 dark:hover:bg-zinc-800': !isDownloading(track),
                'cursor-not-allowed opacity-60': isDownloading(track)
            }"
        >
            <div class="mr-2 text-zinc-400" :class="{'text-blue-500': store.currentIndex === index}">
                <div v-if="isDownloading(track)" class="w-3.5 h-3.5 border-2 border-blue-500 border-t-transparent rounded-full animate-spin"></div>
                <Video v-else-if="isVideo(track)" class="w-3.5 h-3.5" />
                <Music v-else class="w-3.5 h-3.5" />
            </div>
            <div class="flex-1 min-w-0">
                <div class="truncate text-xs font-medium">
                    {{ getTitle(track) }}
                </div>
                <div v-if="isDownloading(track)" class="text-[10px] text-blue-500">
                    {{ locale === 'zh' ? '下载中...' : 'Downloading...' }}
                </div>
                <div v-else-if="isSourceRemote(track.source) && !track.source.Remote.cached_path" class="text-[10px] text-zinc-400">
                    {{ locale === 'zh' ? '未下载 - 双击下载并播放' : 'Not downloaded - Double click to download' }}
                </div>
            </div>
        </div>
    </div>

    <div class="p-4 border-t dark:border-zinc-800 bg-white dark:bg-zinc-900 space-y-2">
        <div class="flex gap-2">
            <input
                v-model="urlInput"
                type="text"
                :placeholder="isResolvingUrl ? (locale === 'zh' ? '解析中...' : 'Resolving...') : t('sidebar.addUrl')"
                :disabled="isResolvingUrl"
                class="flex-1 px-3 py-2 text-sm rounded-md border dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:text-white disabled:opacity-50"
                @keyup.enter="addUrl"
            />
            <button
                @click="addUrl"
                :disabled="isResolvingUrl || !urlInput"
                class="p-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 flex-shrink-0 disabled:opacity-50 disabled:cursor-not-allowed"
                title="Add URL"
            >
                <Loader2 v-if="isResolvingUrl" class="w-4 h-4 animate-spin" />
                <Plus v-else class="w-4 h-4" />
            </button>
        </div>
        <div class="flex gap-2">
            <button
                @click="addLocalFiles"
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-sm rounded-md border dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-600 dark:text-zinc-400"
            >
                <FolderOpen class="w-4 h-4" />
                <span>{{ t('sidebar.addFiles') }}</span>
            </button>
            <button
                @click="addFolder"
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-sm rounded-md border dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-600 dark:text-zinc-400"
            >
                <Folder class="w-4 h-4" />
                <span>{{ t('sidebar.addFolder') }}</span>
            </button>
        </div>
    </div>

    <!-- Login Dialog -->
    <LoginDialog
      :show="showLoginDialog"
      :platform="loginPlatform"
      :login-url="loginUrl"
      :original-url="pendingUrl"
      @close="showLoginDialog = false"
      @retry="retryUrl"
    />
  </div>
</template>
