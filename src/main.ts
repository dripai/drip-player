import { createApp } from 'vue'
import { createPinia } from 'pinia'
import i18n from './i18n'
import './style.css'
import App from './App.vue'
import { VideoPlayer } from '@videojs-player/vue'
import 'video.js/dist/video-js.css'

const app = createApp(App)
app.use(createPinia())
app.use(i18n)
app.component('VideoPlayer', VideoPlayer)
app.mount('#app')
