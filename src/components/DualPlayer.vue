<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useAvatarStore } from '../stores/avatar';

const store = useAvatarStore();

const playerA = ref<HTMLVideoElement | null>(null);
const playerB = ref<HTMLVideoElement | null>(null);
const frameCanvas = ref<HTMLCanvasElement | null>(null);

const activeSlot = ref<'A' | 'B'>('A');
const srcA = ref('');
const srcB = ref('');

const activeFrames = computed(() => store.activeSlot?.previewFrames ?? null);
const activeFps = computed(() => store.activeSlot?.previewFps ?? 12);
const framesMode = computed(() => (activeFrames.value?.length ?? 0) > 0);

let rafId: number | null = null;
let lastTick = 0;
let frameIdx = 0;
let elapsedSec = 0;

// Watch changes to the active video URL from state machine (fallback path)
watch(
  () => store.activeVideoUrl,
  async (newUrl) => {
    if (!newUrl || framesMode.value) return;

    if (activeSlot.value === 'A') {
      if (srcB.value !== newUrl) {
        srcB.value = newUrl;
        await nextTick();
        playerB.value?.load();
      }
      playerB.value?.play().then(() => {
        activeSlot.value = 'B';
        playerA.value?.pause();
      }).catch(() => {
        // Handle autoplay policy or format delay
      });
    } else {
      if (srcA.value !== newUrl) {
        srcA.value = newUrl;
        await nextTick();
        playerA.value?.load();
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

function renderFrame(now: number) {
  rafId = requestAnimationFrame(renderFrame);
  const frames = activeFrames.value;
  const canvas = frameCanvas.value;
  if (!frames || frames.length === 0 || !canvas) {
    lastTick = now;
    return;
  }

  const delta = Math.min(0.25, (now - lastTick) / 1000);
  lastTick = now;
  elapsedSec += delta;

  if (elapsedSec * activeFps.value >= frames.length) {
    elapsedSec = 0;
    if (store.activeOneShotId) store.clearOneShot();
  }
  frameIdx = Math.floor(elapsedSec * activeFps.value) % frames.length;

  const frame = frames[frameIdx];
  if (canvas.width !== frame.width || canvas.height !== frame.height) {
    canvas.width = frame.width;
    canvas.height = frame.height;
  }
  canvas.getContext('2d')?.drawImage(frame, 0, 0);
}

function syncFrameMode() {
  if (framesMode.value) {
    playerA.value?.pause();
    playerB.value?.pause();
    if (rafId === null) {
      frameIdx = 0;
      elapsedSec = 0;
      lastTick = performance.now();
      rafId = requestAnimationFrame(renderFrame);
    }
  } else if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
    frameIdx = 0;
    elapsedSec = 0;
    activeSlot.value = 'A';
  }
}

watch([framesMode, () => store.activeSlot], () => {
  frameIdx = 0;
  elapsedSec = 0;
  syncFrameMode();
}, { immediate: true, flush: 'post' });

function onEnded() {
  // If playing a one-shot reaction, release it once finished
  if (store.activeOneShotId) {
    store.clearOneShot();
  }
}

onBeforeUnmount(() => {
  if (rafId !== null) cancelAnimationFrame(rafId);
});
</script>

<template>
  <div class="relative w-full h-full flex items-center justify-center overflow-hidden bg-transparent select-none">
    <!-- Canvas playback: FFmpeg-extracted frames, no WebKit/GStreamer WebM decode -->
    <canvas
      v-show="framesMode"
      ref="frameCanvas"
      class="absolute inset-0 w-full h-full object-contain pointer-events-none"
    />

    <!-- Fallback video players (browser mode / sources without preview frames) -->
    <video
      v-show="!framesMode"
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

    <video
      v-show="!framesMode"
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
