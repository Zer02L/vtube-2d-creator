<script setup lang="ts">
import { ref } from 'vue';
import type { VideoResolution, ChromaKeyConfig } from '../types/avatar';

const resolution = ref<VideoResolution>('1080p');
const chroma = ref<ChromaKeyConfig>({
  enabled: true,
  colorHex: '#00FF00',
  similarity: 0.3,
  blend: 0.1,
});

const isProcessing = ref(false);
const progress = ref(0);
const statusMessage = ref('');
const selectedFileName = ref('');

function onFileSelect(event: Event) {
  const target = event.target as HTMLInputElement;
  if (target.files && target.files[0]) {
    selectedFileName.value = target.files[0].name;
  }
}

function startOptimization() {
  if (!selectedFileName.value) return;
  isProcessing.value = true;
  progress.value = 10;
  statusMessage.value = 'Анализ видеопотока...';

  // Simulation of FFmpeg progress bar (will be bound to Rust Tauri command)
  const interval = setInterval(() => {
    progress.value += 15;
    if (progress.value === 40) statusMessage.value = 'Применение фильтра colorkey...';
    if (progress.value === 70) statusMessage.value = 'Кодирование WebM VP9 (yuva420p)...';
    if (progress.value >= 100) {
      clearInterval(interval);
      isProcessing.value = false;
      statusMessage.value = 'Оптимизация завершена!';
    }
  }, 400);
}
</script>

<template>
  <div class="bg-slate-900 border border-slate-800 rounded-xl p-5 flex flex-col gap-5 text-slate-200">
    <div class="flex items-center justify-between border-b border-slate-800 pb-3">
      <div>
        <h3 class="font-semibold text-lg text-white">Встроенный видео-конвертер и хромакей</h3>
        <p class="text-xs text-slate-400">Преобразование любых видео (MOV, MP4, GIF) в оптимизированный WebM с прозрачностью</p>
      </div>
      <span class="px-2.5 py-1 text-xs rounded-full bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-mono">
        FFmpeg Engine
      </span>
    </div>

    <!-- Drag & Drop Area -->
    <label
      class="border-2 border-dashed border-slate-700 hover:border-emerald-500/50 transition-colors rounded-xl p-6 flex flex-col items-center justify-center gap-2 cursor-pointer bg-slate-950/40"
    >
      <input type="file" accept="video/*,image/gif" class="hidden" @change="onFileSelect" />
      <div class="w-10 h-10 rounded-full bg-slate-800 flex items-center justify-center text-xl">
        🎬
      </div>
      <div class="text-center">
        <p class="text-sm font-medium text-slate-200">
          {{ selectedFileName || 'Перетащите видеофайл сюда или нажмите для выбора' }}
        </p>
        <p class="text-xs text-slate-500 mt-0.5">Поддерживаются MOV (ProRes), MP4, WebM, GIF, AVI</p>
      </div>
    </label>

    <!-- Settings Grid -->
    <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
      <!-- Resolution preset -->
      <div class="flex flex-col gap-2">
        <label class="text-xs font-semibold text-slate-300">Целевое разрешение</label>
        <div class="grid grid-cols-4 gap-2">
          <button
            v-for="res in (['480p', '720p', '1080p', '2k'] as VideoResolution[])"
            :key="res"
            @click="resolution = res"
            :class="[
              'py-2 px-3 rounded-lg text-xs font-medium border transition-colors',
              resolution === res
                ? 'bg-emerald-600 border-emerald-500 text-white'
                : 'bg-slate-800 border-slate-700 text-slate-300 hover:bg-slate-750'
            ]"
          >
            {{ res.toUpperCase() }}
          </button>
        </div>
      </div>

      <!-- Chroma Key Color -->
      <div class="flex flex-col gap-2">
        <div class="flex items-center justify-between">
          <label class="text-xs font-semibold text-slate-300">Удаление фона (Хромакей)</label>
          <input type="checkbox" v-model="chroma.enabled" class="rounded accent-emerald-500 cursor-pointer" />
        </div>
        <div class="flex items-center gap-3">
          <input
            type="color"
            v-model="chroma.colorHex"
            :disabled="!chroma.enabled"
            class="w-10 h-9 rounded cursor-pointer border border-slate-700 bg-transparent disabled:opacity-40"
          />
          <input
            type="text"
            v-model="chroma.colorHex"
            :disabled="!chroma.enabled"
            class="bg-slate-800 border border-slate-700 rounded-lg px-3 py-1.5 text-xs font-mono text-slate-200 w-28 disabled:opacity-40"
          />
          <span class="text-xs text-slate-400">Пипетка / Цвет фона</span>
        </div>
      </div>
    </div>

    <!-- Sliders -->
    <div v-if="chroma.enabled" class="grid grid-cols-1 md:grid-cols-2 gap-4 pt-2 border-t border-slate-800/60">
      <div class="flex flex-col gap-1.5">
        <div class="flex justify-between text-xs">
          <span class="text-slate-400">Чувствительность (Similarity)</span>
          <span class="font-mono text-emerald-400">{{ (chroma.similarity * 100).toFixed(0) }}%</span>
        </div>
        <input
          type="range"
          min="0.05"
          max="0.8"
          step="0.01"
          v-model.number="chroma.similarity"
          class="accent-emerald-500 cursor-pointer"
        />
      </div>

      <div class="flex flex-col gap-1.5">
        <div class="flex justify-between text-xs">
          <span class="text-slate-400">Сглаживание контура (Blend)</span>
          <span class="font-mono text-emerald-400">{{ (chroma.blend * 100).toFixed(0) }}%</span>
        </div>
        <input
          type="range"
          min="0.0"
          max="0.5"
          step="0.01"
          v-model.number="chroma.blend"
          class="accent-emerald-500 cursor-pointer"
        />
      </div>
    </div>

    <!-- Convert Action & Progress -->
    <div class="flex flex-col gap-3 pt-2">
      <button
        @click="startOptimization"
        :disabled="!selectedFileName || isProcessing"
        class="w-full py-2.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white font-medium text-sm transition-colors flex items-center justify-center gap-2 cursor-pointer shadow-lg shadow-emerald-950/40"
      >
        <span v-if="isProcessing">⏳ Конвертация...</span>
        <span v-else>⚡ Запечь в WebM с прозрачным фоном</span>
      </button>

      <div v-if="isProcessing || statusMessage" class="flex flex-col gap-1.5">
        <div class="w-full bg-slate-800 rounded-full h-2 overflow-hidden">
          <div
            class="bg-emerald-500 h-full transition-all duration-300"
            :style="{ width: `${progress}%` }"
          />
        </div>
        <span class="text-xs text-slate-400 text-center font-mono">{{ statusMessage }}</span>
      </div>
    </div>
  </div>
</template>
