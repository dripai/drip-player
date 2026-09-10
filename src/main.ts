import { createApp } from 'vue'
import { createPinia } from 'pinia'
import i18n from './i18n'
import './style.css'
import Root from './Root.vue'
import 'video.js/dist/video-js.css'

const app = createApp(Root)
app.use(createPinia())
app.use(i18n)
app.mount('#app')
