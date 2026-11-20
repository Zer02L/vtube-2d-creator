<script setup lang="ts">
import { ref, watch } from 'vue';
import { useAvatarStore } from '../stores/avatar';

const store = useAvatarStore();

const playerA = ref<HTMLVideoElement | null>(null);
const playerB = ref<HTMLVideoElement | null>(null);

const activeSlot = ref<'A' | 'B'>('A');
const srcA = ref('');
const srcB = ref('');

// Watch changes to the active video URL from state machine
watch(
  () => store.activeVideoUrl,
  (newUrl) => {
    if (!newUrl) return;

    if (activeSlot.value === 'A') {
      // Load into B
      if (srcB.value !== newUrl) {
        srcB.value = newUrl;
      }
      playerB.value?.play().then(() => {
        activeSlot.value = 'B';
        playerA.value?.pause();
      }).catch(() => {
        // Handle autoplay policy or format delay
      });
    } else {
      // Load into A
      if (srcA.value !== newUrl) {
        srcA.value = newUrl;
      }
      playerA.value?.play().then(() => {
        activeSlot.value = 'A';
        playerB.value?.pause();
      }).catch(() => {
        // Handle autoplay policy
      });
    }
  },
  { immediate: true }
);

function onEnded() {
  // If playing a one-shot reaction, release it once finished
  if (store.activeOneShotId) {
    store.clearOneShot();
  }
}
</script>

<template>
  <div class="relative w-full h-full flex items-center justify-center overflow-hidden bg-transparent select-none">
    <!-- Video Player A -->
    <video
      ref="playerA"
      :src="srcA"
      autoplay
      loop
      muted
      playsinline
      @ended="onEnded"
      :class="[
        'absolute inset-0 w-full h-full object-contain pointer-events-none transition-opacity duration-75',
        activeSlot === 'A' ? 'opacity-100 z-10' : 'opacity-0 z-0'
      ]"
    />

    <!-- Video Player B (Ping-Pong Standby) -->
    <video
      ref="playerB"
      :src="srcB"
      autoplay
      loop
      muted
      playsinline
      @ended="onEnded"
      :class="[
        'absolute inset-0 w-full h-full object-contain pointer-events-none transition-opacity duration-75',
        activeSlot === 'B' ? 'opacity-100 z-10' : 'opacity-0 z-0'
      ]"
    />

    <!-- Placeholder when no video loaded -->
    <div
      v-if="!store.activeVideoUrl"
      class="text-slate-400 text-sm flex flex-col items-center gap-2 border-2 border-dashed border-slate-700 p-8 rounded-xl"
    >
      <div class="w-12 h-12 rounded-full bg-slate-800 flex items-center justify-center text-slate-500">
        🎭
      </div>
      <p class="font-medium text-slate-300">Аватар не загружен</p>
      <p class="text-xs text-slate-500">Добавьте видео в формате WebM через оптимизатор</p>
    </div>
  </div>
</template>
