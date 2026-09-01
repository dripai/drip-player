<script setup lang="ts">
import { ref, computed, nextTick, onMounted } from 'vue'
import { usePlayerStore, type PlaylistItem } from '../store/player'
import { Plus, Music, Video, FolderOpen, Folder, Loader2 } from 'lucide-vue-next'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useI18n } from 'vue-i18n'
import TreeNode from './TreeNode.vue'
import LoginDialog from './LoginDialog.vue'
import { MEDIA_EXTENSIONS } from '../utils/mediaCapabilities'

const store = usePlayerStore()
const { t, locale } = useI18n()
const urlInput = ref('')
const folderTree = ref<any>(null)
const expandedFolders = ref<Set<string>>(new Set())
const isResolvingUrl = ref(false)
const playlistContainer = ref<HTMLElement | null>(null)
const focusedPlaylistItemId = ref<string | null>(null)
const urlFeedback = ref('')
const urlFeedbackIsError = ref(false)

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
  const url = urlInput.value.trim()
  if (!url || isResolvingUrl.value) return
  pendingUrl.value = url
  urlFeedback.value = ''
  urlFeedbackIsError.value = false
  focusedPlaylistItemId.value = null
  try {
    const result = await store.addUrl(url)
    urlInput.value = ''
    urlFeedback.value = result.outcome === 'added'
      ? (locale.value === 'zh' ? '已添加到播放列表' : 'Added to playlist')
      : (locale.value === 'zh' ? '该地址已在播放列表中' : 'Already in playlist')

    focusedPlaylistItemId.value = result.item_id
    await nextTick()
    playlistContainer.value
      ?.querySelector<HTMLElement>(`[data-playlist-item-id="${focusedPlaylistItemId.value}"]`)
      ?.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
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
      urlFeedback.value = locale.value === 'zh' ? `添加失败：${errorStr}` : `Failed to add: ${errorStr}`
      urlFeedbackIsError.value = true
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
async function handleTrackDoubleClick(item: PlaylistItem) {

    if (item.download_status === 'downloading') {
        console.log('Track is downloading, please wait...');
        return;
    }

    if (item.origin.kind === 'remote') {
        await store.playRemoteTrack(item.id);
    } else {
        await store.play(item.id);
    }
}

/**
 * 检查曲目是否正在下载
 */
function isDownloading(item: PlaylistItem) {
    return item.download_status === 'downloading';
}

/**
 * 获取曲目显示标题
 */
function getTitle(item: PlaylistItem) {
    return item.title || (item.origin.kind === 'local'
        ? item.origin.path.split(/[/\\]/).pop() || item.origin.path
        : item.origin.url)
}

/**
 * 判断是否为视频文件
 */
function isVideo(item: PlaylistItem) {
    return item.media_type === 'Video'
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
async function showContextMenu(e: MouseEvent, itemId: string) {
    e.preventDefault()
    try {
        await invoke('show_track_context_menu', { itemId, locale: locale.value })
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

    <div ref="playlistContainer" class="flex-1 overflow-y-auto p-2 space-y-0.5">
        <!-- Folder Tree -->
        <div v-if="folderTree" class="mb-2">
            <TreeNode
                :item="folderTree"
                :level="0"
                :expanded-folders="expandedFolders"
                @toggle-folder="toggleFolder"
                @play-track="playTrack"
                :current-track="store.currentTrack"
            />
        </div>

        <!-- Playlist -->
        <div
            v-for="item in store.playlist"
            :key="item.id"
            :data-playlist-item-id="item.id"
            @dblclick="handleTrackDoubleClick(item)"
            @contextmenu="showContextMenu($event, item.id)"
            class="group flex items-center px-2 py-1 rounded transition-colors select-none"
            :class="{
                'bg-zinc-200 dark:bg-zinc-800 text-blue-600 dark:text-blue-400': store.currentItemId === item.id,
                'ring-1 ring-blue-500 bg-blue-50 dark:bg-blue-950/40': focusedPlaylistItemId === item.id,
                'cursor-pointer hover:bg-zinc-200 dark:hover:bg-zinc-800': !isDownloading(item),
                'cursor-not-allowed opacity-60': isDownloading(item)
            }"
        >
            <div class="mr-2 text-zinc-400" :class="{'text-blue-500': store.currentItemId === item.id}">
                <div v-if="isDownloading(item)" class="w-3.5 h-3.5 border-2 border-blue-500 border-t-transparent rounded-full animate-spin"></div>
                <Video v-else-if="isVideo(item)" class="w-3.5 h-3.5" />
                <Music v-else class="w-3.5 h-3.5" />
            </div>
            <div class="flex-1 min-w-0">
                <div class="truncate text-xs font-medium">
                    {{ getTitle(item) }}
                </div>
                <div v-if="isDownloading(item)" class="text-[10px] text-blue-500">
                    {{ locale === 'zh' ? '下载中...' : 'Downloading...' }}
                </div>
                <div v-else-if="item.origin.kind === 'remote' && !item.cached_path" class="text-[10px] text-zinc-400">
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
                @input="urlFeedback = ''; focusedPlaylistItemId = null"
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
        <div
            v-if="urlFeedback"
            class="text-xs"
            :class="urlFeedbackIsError ? 'text-red-500' : 'text-blue-500'"
        >
            {{ urlFeedback }}
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
