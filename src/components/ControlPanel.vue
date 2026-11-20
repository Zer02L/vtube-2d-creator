<script setup lang="ts">
import { ref } from 'vue';
import { useAvatarStore } from '../stores/avatar';

const store = useAvatarStore();

const sampleEmotions = [
  { id: 'happy', name: 'Радость (Happy)', hotkey: 'Ctrl+1', icon: '😄' },
  { id: 'angry', name: 'Злость (Angry)', hotkey: 'Ctrl+2', icon: '😡' },
  { id: 'shock', name: 'Шок (Shock)', hotkey: 'Ctrl+3', icon: '😲' },
  { id: 'sad', name: 'Грусть (Sad)', hotkey: 'Ctrl+4', icon: '😢' },
];

const sampleOneShots = [
  { id: 'wave', name: 'Помахать рукой', hotkey: 'F1', icon: '👋' },
  { id: 'laugh', name: 'Смех / Ха-ха', hotkey: 'F2', icon: '🤣' },
  { id: 'cheer', name: 'Ура / Салют', hotkey: 'F3', icon: '🎉' },
];

const obsUrl = ref(`http://localhost:${store.serverPort}/avatar`);
const copied = ref(false);

function copyObsUrl() {
  navigator.clipboard.writeText(obsUrl.value);
  copied.value = true;
  setTimeout(() => (copied.value = false), 2000);
}
</script>

<template>
  <div class="bg-slate-900 border border-slate-800 rounded-xl p-5 flex flex-col gap-6 text-slate-200">
    <!-- Header with OBS URL -->
    <div class="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3 border-b border-slate-800 pb-4">
      <div>
        <h3 class="font-semibold text-lg text-white">Панель управления и триггеры</h3>
        <p class="text-xs text-slate-400">Управление микрофоном, эмоциями и вывод в OBS</p>
      </div>

      <!-- OBS Link Widget -->
      <div class="flex items-center gap-2 bg-slate-950 border border-slate-800 rounded-lg p-1.5 pl-3">
        <span class="text-xs text-slate-400 font-mono">OBS:</span>
        <code class="text-xs text-emerald-400 font-mono select-all">{{ obsUrl }}</code>
        <button
          @click="copyObsUrl"
          class="px-2.5 py-1 text-xs font-medium rounded bg-slate-800 hover:bg-slate-700 text-slate-200 transition-colors cursor-pointer"
        >
          {{ copied ? 'Скопировано!' : 'Копировать' }}
        </button>
      </div>
    </div>

    <!-- Microphone & VAD -->
    <div class="flex flex-col gap-3 bg-slate-950/60 p-4 rounded-xl border border-slate-800/80">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <span class="text-sm">🎙️</span>
          <span class="text-xs font-semibold text-slate-200">Активация по голосу (VAD)</span>
        </div>
        <span
          :class="[
            'text-xs px-2 py-0.5 rounded-full font-medium transition-colors',
            store.isSpeaking ? 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30' : 'bg-slate-800 text-slate-400'
          ]"
        >
          {{ store.isSpeaking ? 'Говорит (Talk)' : 'Покой (Idle)' }}
        </span>
      </div>

      <!-- Volume Bar -->
      <div class="flex flex-col gap-1">
        <div class="w-full bg-slate-800 h-3 rounded-full overflow-hidden relative">
          <!-- Active Volume -->
          <div
            class="h-full bg-gradient-to-r from-emerald-500 to-amber-500 transition-all duration-75"
            :style="{ width: `${store.currentVolume * 100}%` }"
          />
          <!-- Threshold Marker line -->
          <div
            class="absolute top-0 bottom-0 w-1 bg-white shadow-sm shadow-black"
            :style="{ left: `${store.manifest.vad.threshold * 100}%` }"
          />
        </div>

        <div class="flex justify-between text-xs text-slate-500 font-mono mt-1">
          <span>0%</span>
          <span>Порог срабатывания: {{ (store.manifest.vad.threshold * 100).toFixed(0) }}%</span>
          <span>100%</span>
        </div>
      </div>

      <!-- Threshold Slider -->
      <div class="flex items-center gap-3 pt-1">
        <label class="text-xs text-slate-400">Чувствительность:</label>
        <input
          type="range"
          min="0.01"
          max="0.8"
          step="0.01"
          v-model.number="store.manifest.vad.threshold"
          class="flex-1 accent-emerald-500 cursor-pointer"
        />
        <!-- Mic test simulator button -->
        <button
          @mousedown="store.setVolume(0.5)"
          @mouseup="store.setVolume(0)"
          @mouseleave="store.setVolume(0)"
          class="px-2.5 py-1 text-xs rounded bg-slate-800 hover:bg-slate-700 text-slate-300 transition-colors cursor-pointer select-none"
        >
          Тест голоса (Зажать)
        </button>
      </div>
    </div>

    <!-- Emotion Loops -->
    <div class="flex flex-col gap-2">
      <div class="flex items-center justify-between">
        <label class="text-xs font-semibold text-slate-300">Состояния эмоций (Переключение лупов)</label>
        <button
          v-if="store.currentEmotionId"
          @click="store.triggerEmotion(null)"
          class="text-xs text-emerald-400 hover:underline cursor-pointer"
        >
          Сбросить в Покой
        </button>
      </div>

      <div class="grid grid-cols-2 sm:grid-cols-4 gap-2">
        <button
          v-for="emo in sampleEmotions"
          :key="emo.id"
          @click="store.triggerEmotion(emo.id)"
          :class="[
            'p-3 rounded-lg border text-left flex flex-col gap-1 transition-all cursor-pointer',
            store.currentEmotionId === emo.id
              ? 'bg-emerald-600/20 border-emerald-500 text-white shadow-sm shadow-emerald-500/20'
              : 'bg-slate-800/80 border-slate-700 text-slate-300 hover:bg-slate-800'
          ]"
        >
          <div class="flex items-center justify-between">
            <span class="text-lg">{{ emo.icon }}</span>
            <span class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-slate-900 text-slate-400">{{ emo.hotkey }}</span>
          </div>
          <span class="text-xs font-medium">{{ emo.name }}</span>
        </button>
      </div>
    </div>

    <!-- One-shot Reactions -->
    <div class="flex flex-col gap-2">
      <label class="text-xs font-semibold text-slate-300">Одиночные реакции (One-Shot с возвратом)</label>
      <div class="grid grid-cols-3 gap-2">
        <button
          v-for="one in sampleOneShots"
          :key="one.id"
          @click="store.triggerOneShot(one.id)"
          :class="[
            'p-2.5 rounded-lg border text-left flex items-center justify-between transition-all cursor-pointer',
            store.activeOneShotId === one.id
              ? 'bg-amber-600/30 border-amber-500 text-white animate-pulse'
              : 'bg-slate-800/80 border-slate-700 text-slate-300 hover:bg-slate-800'
          ]"
        >
          <div class="flex items-center gap-2">
            <span>{{ one.icon }}</span>
            <span class="text-xs font-medium">{{ one.name }}</span>
          </div>
          <span class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-slate-900 text-slate-400">{{ one.hotkey }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
