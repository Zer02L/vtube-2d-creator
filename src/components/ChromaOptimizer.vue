<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  Eye,
  FolderOpen,
  Pause,
  Play,
  Upload,
} from '@lucide/vue';

import { useAvatarStore } from '../stores/avatar';
import {
  applyChromaKeyInPlace,
  extractChromaPreviewFrames,
  type ChromaPreviewFrames,
} from '../utils/chroma';
import { exportChromaKeyedWebmInBrowser } from '../utils/loopEngine';
import type {
  ChromaKeyConfig,
  OptimizeProgressEvent,
  PreviewSheet,
  TranscodeResult,
  VideoProbeInfo,
  VideoResolution,
} from '../types/avatar';

const store = useAvatarStore();
const runningInTauri = isTauri();

const resolution = ref<VideoResolution>('1080p');
const chroma = ref<ChromaKeyConfig>({
  enabled: true,
  colorHex: '#00FF00',
  similarity: 0.3,
  blend: 0.1,
});

// Input / preview state
const selectedFileName = ref('');
const inputObjectUrl = ref('');
const localInputPath = ref('');
const probeInfo = ref<VideoProbeInfo | null>(null);
const isDragOver = ref(false);
const showOriginal = ref(false);
const isPlaying = ref(true);
const currentTime = ref(0);
const videoDuration = ref(0);
const isExtracting = ref(false);
const extractProgress = ref(0);
const frameIndex = ref(0);

// Processing state
const isProcessing = ref(false);
const progress = ref(0);
const statusMessage = ref('');
const errorMessage = ref('');
const customOutputPath = ref('');

// Result state
const resultUrl = ref('');
const resultNativePath = ref('');
const resultName = ref('');
const resultMeta = ref<{
  durationSec: number;
  frameCount: number;
  fileSizeBytes: number;
  width: number;
  height: number;
  hasAlpha: boolean;
} | null>(null);
const assignedSlot = ref<'idle' | 'talk' | null>(null);

const previewCanvas = ref<HTMLCanvasElement | null>(null);
let previewFrames: ChromaPreviewFrames | null = null;
// Кэш уже обработанных хромакеем кадров: тяжёлая обработка делается
// один раз на кадр, а не на каждый animation frame.
let keyedFrames: (HTMLCanvasElement | null)[] = [];
let rafId: number | null = null;
let lastTickTime = 0;
let lastDrawnIdx = -1;

const resolutionMap: Record<VideoResolution, string> = {
  '480p': '854x480',
  '720p': '1280x720',
  '1080p': '1920x1080',
  '2k': '2560x1440',
};

const resolutionLabels: VideoResolution[] = ['480p', '720p', '1080p', '2k'];

const alphaNote = computed(() =>
  runningInTauri
    ? 'Десктопный режим: FFmpeg кодирует настоящий альфа-канал (yuva420p).'
    : 'Веб-режим: браузерный MediaRecorder может не сохранить альфу в WebM. Для настоящей прозрачности используйте десктопное приложение.'
);

function formatTime(sec: number): string {
  if (!Number.isFinite(sec)) return '0:00';
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function formatBytes(bytes: number): string {
  if (!bytes) return '—';
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КБ`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} МБ`;
}

function getKeyedFrame(index: number): HTMLCanvasElement {
  const source = previewFrames!.frames[index];
  const cached = keyedFrames[index];
  if (cached) return cached;

  const out = document.createElement('canvas');
  out.width = source.width;
  out.height = source.height;
  const ctx = out.getContext('2d')!;
  ctx.drawImage(source, 0, 0);
  const image = ctx.getImageData(0, 0, out.width, out.height);
  applyChromaKeyInPlace(image.data, chroma.value);
  ctx.putImageData(image, 0, 0);
  keyedFrames[index] = out;
  return out;
}

function renderFrame(force = false) {
  const canvas = previewCanvas.value;
  const data = previewFrames;
  if (!canvas || !data || data.frames.length === 0) return;

  const idx = Math.max(0, Math.min(data.frames.length - 1, frameIndex.value));
  if (!force && idx === lastDrawnIdx) return;
  lastDrawnIdx = idx;

  const { width, height } = data;
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  ctx.clearRect(0, 0, width, height);
  const frame =
    chroma.value.enabled && !showOriginal.value ? getKeyedFrame(idx) : data.frames[idx];
  ctx.drawImage(frame, 0, 0);
}

function tick(now: number) {
  rafId = requestAnimationFrame(tick);
  const data = previewFrames;
  if (!data || data.frames.length === 0) {
    lastTickTime = now;
    return;
  }

  if (isPlaying.value) {
    const deltaSec = Math.min(0.25, (now - lastTickTime) / 1000);
    currentTime.value = Math.min(data.durationSec, currentTime.value + deltaSec);
    const idx = Math.floor((currentTime.value / data.durationSec) * data.frames.length);
    if (idx >= data.frames.length) currentTime.value = 0;
    frameIndex.value = Math.max(0, Math.min(data.frames.length - 1, idx));
  }
  lastTickTime = now;
  renderFrame();
}

function stopLoop() {
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
}

function invalidateKeyedFrames() {
  keyedFrames = new Array(previewFrames?.frames.length ?? 0).fill(null);
  lastDrawnIdx = -1;
  renderFrame(true);
}

watch([chroma, showOriginal], () => invalidateKeyedFrames(), { deep: true });

function resetResult() {
  if (resultUrl.value) URL.revokeObjectURL(resultUrl.value);
  resultUrl.value = '';
  resultNativePath.value = '';
  resultName.value = '';
  resultMeta.value = null;
  assignedSlot.value = null;
}

async function loadNativePreview(path: string): Promise<ChromaPreviewFrames> {
  optsProgress(45, 'FFmpeg извлекает кадры...');
  const sheet = await invoke<PreviewSheet>('extract_preview_sheet', { inputPath: path });
  optsProgress(80, 'Разбор спрайтлиста...');
  const bytes = await invoke<ArrayBuffer>('read_video_bytes', { path: sheet.path });
  const blobUrl = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
  try {
    const image = new Image();
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error('Не удалось прочитать спрайтлист кадров'));
      image.src = blobUrl;
    });

    const frames: HTMLCanvasElement[] = [];
    for (let i = 0; i < sheet.frameCount; i++) {
      const frame = document.createElement('canvas');
      frame.width = sheet.tileWidth;
      frame.height = sheet.tileHeight;
      frame.getContext('2d')!.drawImage(
        image,
        (i % sheet.columns) * sheet.tileWidth,
        Math.floor(i / sheet.columns) * sheet.tileHeight,
        sheet.tileWidth,
        sheet.tileHeight,
        0,
        0,
        sheet.tileWidth,
        sheet.tileHeight
      );
      frames.push(frame);
    }
    optsProgress(100, 'Кадры предпросмотра готовы');
    return {
      frames,
      width: sheet.tileWidth,
      height: sheet.tileHeight,
      durationSec: sheet.durationSec,
      fps: frames.length / sheet.durationSec,
      dispose: () => {
        frames.length = 0;
      },
    };
  } finally {
    URL.revokeObjectURL(blobUrl);
  }
}

function optsProgress(percent: number, message: string) {
  extractProgress.value = percent;
  statusMessage.value = message;
}

async function loadFile(file: File) {
  errorMessage.value = '';
  resetResult();
  selectedFileName.value = file.name;
  stopLoop();

  if (inputObjectUrl.value) URL.revokeObjectURL(inputObjectUrl.value);
  if (previewFrames) {
    previewFrames.dispose();
    previewFrames = null;
  }
  keyedFrames = [];
  lastDrawnIdx = -1;
  inputObjectUrl.value = URL.createObjectURL(file);
  localInputPath.value = '';
  probeInfo.value = null;
  currentTime.value = 0;
  frameIndex.value = 0;

  if (runningInTauri) {
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const info = await invoke<VideoProbeInfo>('stage_uploaded_video', bytes, {
        headers: { 'x-file-name': file.name },
      });
      probeInfo.value = info;
      localInputPath.value = info.path;
      videoDuration.value = info.durationSec;
    } catch (err) {
      errorMessage.value = err instanceof Error ? err.message : String(err);
    }
  }

  isExtracting.value = true;
  extractProgress.value = 0;
  try {
    if (runningInTauri && localInputPath.value) {
      // Нативный путь: пиксели от FFmpeg, WebKit/GStreamer не участвует.
      previewFrames = await loadNativePreview(localInputPath.value);
    } else {
      previewFrames = await extractChromaPreviewFrames(inputObjectUrl.value, {
        maxFrames: Math.min(96, Math.max(60, Math.ceil((probeInfo.value?.durationSec ?? 6) * 8))),
        maxSide: 360,
        onProgress: (pct, msg) => optsProgress(pct, msg),
      });
    }
    videoDuration.value = previewFrames.durationSec;
    keyedFrames = new Array(previewFrames.frames.length).fill(null);
    lastDrawnIdx = -1;
    isPlaying.value = true;
    lastTickTime = performance.now();
    renderFrame(true);
    rafId = requestAnimationFrame(tick);
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  } finally {
    isExtracting.value = false;
    statusMessage.value = '';
  }
}

function onFileInputChange(ev: Event) {
  const target = ev.target as HTMLInputElement;
  if (target.files?.[0]) void loadFile(target.files[0]);
  target.value = '';
}

function onDrop(ev: DragEvent) {
  isDragOver.value = false;
  const file = ev.dataTransfer?.files?.[0];
  if (file) void loadFile(file);
}

function pickColor(ev: MouseEvent) {
  const canvas = previewCanvas.value;
  if (!canvas) return;
  const rect = canvas.getBoundingClientRect();
  const x = Math.floor(((ev.clientX - rect.left) / rect.width) * canvas.width);
  const y = Math.floor(((ev.clientY - rect.top) / rect.height) * canvas.height);
  if (x < 0 || y < 0 || x >= canvas.width || y >= canvas.height) return;
  const source = previewFrames?.frames[frameIndex.value];
  const pixel = source?.getContext('2d')?.getImageData(x, y, 1, 1).data;
  if (!pixel) return;
  chroma.value.colorHex =
    '#' +
    [pixel[0], pixel[1], pixel[2]]
      .map((v) => v.toString(16).padStart(2, '0'))
      .join('')
      .toUpperCase();
}

function togglePlayback() {
  if (!previewFrames) return;
  isPlaying.value = !isPlaying.value;
  lastTickTime = performance.now();
}

function onSeek(ev: Event) {
  const data = previewFrames;
  if (!data || data.durationSec <= 0) return;
  currentTime.value = Math.max(0, Math.min(data.durationSec, Number((ev.target as HTMLInputElement).value)));
  frameIndex.value = Math.max(
    0,
    Math.min(data.frames.length - 1, Math.floor((currentTime.value / data.durationSec) * data.frames.length))
  );
  renderFrame();
}

async function startOptimization() {
  if (!inputObjectUrl.value || isProcessing.value) return;
  isProcessing.value = true;
  progress.value = 3;
  statusMessage.value = 'Подготовка...';
  errorMessage.value = '';
  resetResult();

  try {
    if (runningInTauri && localInputPath.value) {
      const channel = new Channel<OptimizeProgressEvent>();
      channel.onmessage = (ev) => {
        progress.value = ev.percent;
        statusMessage.value = ev.message;
      };

      const result = await invoke<TranscodeResult>('optimize_video', {
        options: {
          inputPath: localInputPath.value,
          outputPath: customOutputPath.value.trim() || '',
          resolution: resolutionMap[resolution.value],
          chromaKey: chroma.value.enabled
            ? {
                color_hex: chroma.value.colorHex,
                similarity: chroma.value.similarity,
                blend: chroma.value.blend,
              }
            : null,
        },
        onProgress: channel,
      });

      const bytes = await invoke<ArrayBuffer>('read_video_bytes', {
        path: result.outputPath,
      });
      resultUrl.value = URL.createObjectURL(new Blob([bytes], { type: 'video/webm' }));
      resultNativePath.value = result.outputPath;
      resultName.value = result.outputPath.split(/[\\/]/).pop() || 'avatar_transparent.webm';
      resultMeta.value = {
        durationSec: result.durationSec,
        frameCount: result.frameCount,
        fileSizeBytes: result.fileSizeBytes,
        width: result.width,
        height: result.height,
        hasAlpha: result.hasAlpha,
      };
    } else {
      const result = await exportChromaKeyedWebmInBrowser(
        inputObjectUrl.value,
        {
          resolution: resolution.value,
          chromaKey: chroma.value,
          targetFps: 24,
        },
        (pct, msg) => {
          progress.value = pct;
          statusMessage.value = msg;
        }
      );
      resultUrl.value = result.url;
      resultName.value = `${selectedFileName.value.replace(/\.[^.]+$/, '') || 'avatar'}_transparent.webm`;
      resultMeta.value = {
        durationSec: result.durationSec,
        frameCount: result.frameCount,
        fileSizeBytes: result.fileSizeBytes,
        width: result.width,
        height: result.height,
        hasAlpha: false,
      };
    }
    statusMessage.value = 'Оптимизация завершена!';
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
    statusMessage.value = 'Ошибка конвертации';
  } finally {
    isProcessing.value = false;
  }
}

function downloadResult() {
  if (!resultUrl.value) return;
  const a = document.createElement('a');
  a.href = resultUrl.value;
  a.download = resultName.value || 'avatar_transparent.webm';
  a.click();
}

async function revealOutputFolder() {
  if (!runningInTauri || !resultNativePath.value) return;
  try {
    await invoke('reveal_in_folder', { path: resultNativePath.value });
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  }
}

function assignToAvatar(slot: 'idle' | 'talk') {
  if (!resultUrl.value) return;
  store.assignLoopToSlot(slot, resultUrl.value, resultName.value, resultMeta.value?.durationSec);
  assignedSlot.value = slot;
}

onBeforeUnmount(() => {
  stopLoop();
  previewFrames?.dispose();
  if (inputObjectUrl.value) URL.revokeObjectURL(inputObjectUrl.value);
  if (resultUrl.value) URL.revokeObjectURL(resultUrl.value);
});
</script>

<template>
  <div class="bg-slate-900 border border-slate-800 rounded-xl p-5 flex flex-col gap-5 text-slate-200">
    <div class="flex items-center justify-between border-b border-slate-800 pb-3">
      <div>
        <h3 class="font-semibold text-lg text-white">Встроенный видео-конвертер и хромакей</h3>
        <p class="text-xs text-slate-400">Преобразование видео в WebM с прозрачностью и живым предпросмотром</p>
      </div>
      <span
        :class="[
          'px-2.5 py-1 text-xs rounded-full border font-mono',
          runningInTauri
            ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20'
            : 'bg-sky-500/10 text-sky-400 border-sky-500/20'
        ]"
      >
        {{ runningInTauri ? 'FFmpeg Native' : 'Browser Canvas' }}
      </span>
    </div>

    <!-- Drag & Drop Area -->
    <label
      @dragover.prevent="isDragOver = true"
      @dragleave="isDragOver = false"
      @drop.prevent="onDrop"
      :class="[
        'border-2 border-dashed transition-colors rounded-xl p-5 flex flex-col items-center justify-center gap-2 cursor-pointer bg-slate-950/40',
        isDragOver ? 'border-emerald-400 bg-emerald-950/20' : 'border-slate-700 hover:border-emerald-500/50'
      ]"
    >
      <input type="file" accept="video/*,image/gif" class="hidden" @change="onFileInputChange" />
      <div class="w-10 h-10 rounded-full bg-slate-800 flex items-center justify-center text-xl">
        <Upload class="w-5 h-5 text-emerald-400" />
      </div>
      <div class="text-center">
        <p class="text-sm font-medium text-slate-200">
          {{ selectedFileName || 'Перетащите видеофайл сюда или нажмите для выбора' }}
        </p>
        <p class="text-xs text-slate-500 mt-0.5">Поддерживаются MOV (ProRes), MP4, WebM, GIF, AVI</p>
        <p v-if="probeInfo" class="text-[11px] font-mono text-slate-400 mt-1">
          {{ probeInfo.width }}×{{ probeInfo.height }} · {{ probeInfo.fps.toFixed(1) }} FPS ·
          {{ probeInfo.durationSec.toFixed(2) }}с
        </p>
      </div>
    </label>

    <!-- Live chroma-key preview -->
    <div v-if="inputObjectUrl" class="bg-slate-950/60 border border-slate-800 rounded-xl p-4 flex flex-col gap-3">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2 text-xs font-semibold text-slate-200">
          <Eye class="w-4 h-4 text-emerald-400" />
          Предпросмотр хромакея
        </div>
        <label class="flex items-center gap-2 text-xs text-slate-400 cursor-pointer">
          <input type="checkbox" v-model="showOriginal" class="rounded accent-emerald-500 cursor-pointer" />
          Показать оригинал
        </label>
      </div>

      <div
        class="relative min-h-[220px] rounded-xl border border-slate-800 overflow-hidden flex items-center justify-center"
        style="background-image: linear-gradient(45deg, #111827 25%, transparent 25%), linear-gradient(-45deg, #111827 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #111827 75%), linear-gradient(-45deg, transparent 75%, #111827 75%); background-size: 20px 20px; background-position: 0 0, 0 10px, 10px -10px, -10px 0px; background-color: #0b0f19;"
      >
        <!-- Покадровый предпросмотр: видео декодируется один раз при загрузке,
             дальше canvas рисует готовые кадры без обращения к <video>. -->
        <div
          v-if="isExtracting"
          class="absolute inset-0 flex flex-col items-center justify-center gap-2 text-xs text-slate-300"
        >
          <div class="w-48 h-1.5 bg-slate-800 rounded-full overflow-hidden">
            <div class="h-full bg-emerald-500 transition-all" :style="{ width: `${extractProgress}%` }" />
          </div>
          <span class="font-mono text-slate-400">{{ statusMessage || 'Индексация кадров...' }}</span>
        </div>
        <canvas
          v-show="!isExtracting"
          ref="previewCanvas"
          @click="pickColor"
          class="relative max-h-[300px] w-auto h-auto object-contain cursor-crosshair"
          title="Кликните по кадру, чтобы взять цвет фона пипеткой"
        />
        <div class="absolute top-2 left-2 px-2 py-1 rounded bg-slate-950/80 text-[10px] font-mono text-slate-300">
          {{ showOriginal || !chroma.enabled ? 'ОРИГИНАЛ' : 'ХРОМАКЕЙ' }}
        </div>
        <div class="absolute bottom-2 right-2 px-2 py-1 rounded bg-slate-950/80 text-[10px] text-slate-400">
          Пипетка: клик по кадру
        </div>
      </div>

      <div class="flex items-center gap-3">
        <button
          @click="togglePlayback"
          :disabled="!previewFrames"
          class="p-2 rounded-lg bg-slate-800 hover:bg-slate-700 disabled:opacity-40 text-white cursor-pointer"
        >
          <Pause v-if="isPlaying" class="w-4 h-4" />
          <Play v-else class="w-4 h-4" />
        </button>
        <span class="text-[11px] font-mono text-slate-400 w-24 text-right">
          {{ formatTime(currentTime) }} / {{ formatTime(videoDuration || probeInfo?.durationSec || 0) }}
        </span>
        <input
          type="range"
          min="0"
          :max="Math.max(0.1, videoDuration || probeInfo?.durationSec || 1)"
          step="0.01"
          :value="currentTime"
          @input="onSeek"
          :disabled="!previewFrames"
          class="flex-1 accent-emerald-500 cursor-pointer disabled:opacity-40"
        />
      </div>

      <p class="text-[11px] text-slate-500">{{ alphaNote }}</p>
    </div>

    <!-- Settings Grid -->
    <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
      <div class="flex flex-col gap-2">
        <label class="text-xs font-semibold text-slate-300">Целевое разрешение</label>
        <div class="grid grid-cols-4 gap-2">
          <button
            v-for="res in resolutionLabels"
            :key="res"
            @click="resolution = res"
            :class="[
              'py-2 px-3 rounded-lg text-xs font-medium border transition-colors cursor-pointer',
              resolution === res
                ? 'bg-emerald-600 border-emerald-500 text-white'
                : 'bg-slate-800 border-slate-700 text-slate-300 hover:bg-slate-700'
            ]"
          >
            {{ res.toUpperCase() }}
          </button>
        </div>
      </div>

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
          min="0"
          max="0.5"
          step="0.01"
          v-model.number="chroma.blend"
          class="accent-emerald-500 cursor-pointer"
        />
      </div>
    </div>

    <div v-if="runningInTauri" class="flex flex-col gap-1">
      <label class="text-xs text-slate-400">Путь сохранения (необязательно)</label>
      <input
        v-model="customOutputPath"
        type="text"
        placeholder="По умолчанию рядом с исходным файлом (*_transparent.webm)"
        class="bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs font-mono text-slate-300 focus:outline-none focus:border-emerald-500"
      />
    </div>

    <!-- Convert Action & Progress -->
    <div class="flex flex-col gap-3 pt-2">
      <button
        @click="startOptimization"
        :disabled="!inputObjectUrl || isProcessing"
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
        <span class="text-xs text-slate-400 text-center font-mono">
          {{ statusMessage }} — {{ progress.toFixed(0) }}%
        </span>
      </div>

      <div
        v-if="errorMessage"
        class="p-3 rounded-xl bg-rose-500/10 border border-rose-500/30 text-xs text-rose-300 flex items-start gap-2"
      >
        <AlertTriangle class="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
        <span class="break-words">{{ errorMessage }}</span>
      </div>
    </div>

    <!-- Result -->
    <div
      v-if="resultUrl"
      class="bg-slate-950/60 border border-emerald-500/40 rounded-xl p-4 flex flex-col gap-3"
    >
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2 text-emerald-400">
          <CheckCircle2 class="w-4 h-4" />
          <span class="text-xs font-semibold uppercase tracking-wider">Готово</span>
        </div>
        <span class="text-[11px] font-mono text-slate-400">{{ resultName }}</span>
      </div>

      <div
        class="relative rounded-xl border border-slate-800 overflow-hidden flex items-center justify-center"
        style="background-image: linear-gradient(45deg, #111827 25%, transparent 25%), linear-gradient(-45deg, #111827 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #111827 75%), linear-gradient(-45deg, transparent 75%, #111827 75%); background-size: 20px 20px; background-position: 0 0, 0 10px, 10px -10px, -10px 0px; background-color: #0b0f19;"
      >
        <video
          :src="resultUrl"
          autoplay
          loop
          muted
          playsinline
          controls
          class="max-h-[260px] w-auto h-auto object-contain"
        />
      </div>

      <div v-if="resultMeta" class="grid grid-cols-4 gap-2 text-center bg-slate-900/70 p-2.5 rounded-lg border border-slate-800">
        <div>
          <div class="text-[10px] text-slate-500">Длительность</div>
          <div class="text-xs font-mono text-slate-200">{{ resultMeta.durationSec.toFixed(2) }}с</div>
        </div>
        <div>
          <div class="text-[10px] text-slate-500">Размер</div>
          <div class="text-xs font-mono text-slate-200">{{ resultMeta.width }}×{{ resultMeta.height }}</div>
        </div>
        <div>
          <div class="text-[10px] text-slate-500">Кадров</div>
          <div class="text-xs font-mono text-slate-200">{{ resultMeta.frameCount }}</div>
        </div>
        <div>
          <div class="text-[10px] text-slate-500">Файл</div>
          <div class="text-xs font-mono text-slate-200">{{ formatBytes(resultMeta.fileSizeBytes) }}</div>
        </div>
      </div>

      <p v-if="resultMeta && !resultMeta.hasAlpha" class="text-[11px] text-amber-300/90">
        ⚠ Альфа-канал в этом файле может отсутствовать (ограничение браузерного кодировщика).
      </p>

      <div class="flex flex-wrap items-center gap-2">
        <button
          @click="downloadResult"
          class="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-xs font-medium text-white cursor-pointer flex items-center gap-1.5"
        >
          <Download class="w-3.5 h-3.5" />
          Скачать WebM
        </button>
        <button
          v-if="runningInTauri && resultNativePath"
          @click="revealOutputFolder"
          class="px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-xs font-medium text-slate-200 cursor-pointer flex items-center gap-1.5"
        >
          <FolderOpen class="w-3.5 h-3.5" />
          Показать в папке
        </button>
        <div class="flex-1" />
        <span class="text-[11px] text-slate-400">Назначить аватару:</span>
        <button
          @click="assignToAvatar('idle')"
          :class="[
            'px-3 py-1.5 rounded-lg text-xs font-medium border cursor-pointer',
            assignedSlot === 'idle'
              ? 'bg-emerald-600/20 border-emerald-500 text-emerald-300'
              : 'bg-slate-800 border-slate-700 text-slate-300 hover:bg-slate-700'
          ]"
        >
          Покой
        </button>
        <button
          @click="assignToAvatar('talk')"
          :class="[
            'px-3 py-1.5 rounded-lg text-xs font-medium border cursor-pointer',
            assignedSlot === 'talk'
              ? 'bg-emerald-600/20 border-emerald-500 text-emerald-300'
              : 'bg-slate-800 border-slate-700 text-slate-300 hover:bg-slate-700'
          ]"
        >
          Разговор
        </button>
      </div>
    </div>
  </div>
</template>
