<script setup lang="ts">
import { ref } from 'vue';
import DualPlayer from './components/DualPlayer.vue';
import ControlPanel from './components/ControlPanel.vue';
import ChromaOptimizer from './components/ChromaOptimizer.vue';
import SeamlessLooper from './components/SeamlessLooper.vue';

const activeTab = ref<'stream' | 'looper' | 'optimizer' | 'profile'>('stream');
</script>

<template>
  <div class="min-h-screen flex flex-col bg-slate-950 text-slate-100 font-sans">
    <!-- Top Navigation Bar -->
    <header class="h-14 border-b border-slate-800 bg-slate-900/90 px-6 flex items-center justify-between select-none">
      <div class="flex items-center gap-3">
        <div class="w-8 h-8 rounded-lg bg-emerald-600 flex items-center justify-center text-lg shadow-md shadow-emerald-900/50">
          🎭
        </div>
        <div>
          <h1 class="text-sm font-bold tracking-wide text-white">2D VTuber Creator & Studio</h1>
          <p class="text-[10px] text-slate-400 font-mono">Tauri 2 + Rust + Vue 3</p>
        </div>
      </div>

      <!-- Navigation Tabs -->
      <nav class="flex items-center gap-1 bg-slate-950 p-1 rounded-lg border border-slate-800">
        <button
          @click="activeTab = 'stream'"
          :class="[
            'px-3.5 py-1.5 rounded-md text-xs font-medium transition-colors cursor-pointer',
            activeTab === 'stream' ? 'bg-slate-800 text-white shadow-sm' : 'text-slate-400 hover:text-slate-200'
          ]"
        >
          🎙️ Стрим и триггеры
        </button>
        <button
          @click="activeTab = 'looper'"
          :class="[
            'px-3.5 py-1.5 rounded-md text-xs font-medium transition-colors cursor-pointer',
            activeTab === 'looper' ? 'bg-slate-800 text-white shadow-sm' : 'text-slate-400 hover:text-slate-200'
          ]"
        >
          🔁 Бесшовный цикл
        </button>
        <button
          @click="activeTab = 'optimizer'"
          :class="[
            'px-3.5 py-1.5 rounded-md text-xs font-medium transition-colors cursor-pointer',
            activeTab === 'optimizer' ? 'bg-slate-800 text-white shadow-sm' : 'text-slate-400 hover:text-slate-200'
          ]"
        >
          ✂️ Конвертер и Хромакей
        </button>
      </nav>

      <!-- Actions / Indicators -->
      <div class="flex items-center gap-3">
        <span class="flex items-center gap-1.5 text-xs text-emerald-400 bg-emerald-500/10 px-2.5 py-1 rounded-full border border-emerald-500/20">
          <span class="w-2 h-2 rounded-full bg-emerald-400 animate-pulse" />
          OBS Сервер активен
        </span>
      </div>
    </header>

    <!-- Main Content Area -->
    <main class="flex-1 p-6 max-w-7xl w-full mx-auto">
      <!-- Tab 1: Stream & Live Avatar Controls -->
      <div v-if="activeTab === 'stream'" class="grid grid-cols-1 lg:grid-cols-12 gap-6 h-full">
        <!-- Controls & Hotkeys (Left) -->
        <div class="lg:col-span-7 flex flex-col gap-6">
          <ControlPanel />
        </div>

        <!-- Live Avatar Preview (Right) -->
        <div class="lg:col-span-5 flex flex-col gap-3">
          <div class="flex items-center justify-between">
            <span class="text-xs font-semibold text-slate-300">Предпросмотр аватара (Alpha Preview)</span>
            <span class="text-[10px] text-slate-500 font-mono">Dual-Buffer Ping-Pong</span>
          </div>

          <!-- Video viewport with transparency grid background -->
          <div
            class="relative flex-1 min-h-[420px] rounded-xl border border-slate-800 overflow-hidden shadow-2xl flex items-center justify-center"
            style="background-image: radial-gradient(#1e293b 1px, transparent 1px); background-size: 16px 16px; background-color: #0b0f19;"
          >
            <DualPlayer />
          </div>
        </div>
      </div>

      <!-- Tab 2: Seamless Video Loop Studio (kept mounted so loaded video persists across tabs) -->
      <div v-show="activeTab === 'looper'">
        <SeamlessLooper :active="activeTab === 'looper'" />
      </div>

      <!-- Tab 3: Video Optimizer / Chroma Key -->
      <div v-if="activeTab === 'optimizer'" class="max-w-3xl mx-auto">
        <ChromaOptimizer />
      </div>
    </main>
  </div>
</template>
