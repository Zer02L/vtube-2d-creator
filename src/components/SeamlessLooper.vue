<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import {
  AlertTriangle,
  ArrowLeftRight,
  Blend,
  CheckCircle2,
  Download,
  Eye,
  Film,
  FolderOpen,
  Layers,
  Pause,
  Play,
  Repeat,
  Scissors,
  SkipForward,
  Sliders,
  Sparkles,
  Upload,
  Wand2,
} from '@lucide/vue';

import { useAvatarStore } from '../stores/avatar';
import type {
  ChromaKeyConfig,
  LoopAnalysisResult,
  LoopCandidate,
  LoopMode,
  LoopProcessResult,
  LoopProgressEvent,
  OutputFormat,
  VideoProbeInfo,
} from '../types/avatar';
import {
  analyzeFramesInBrowser,
  buildLoopPlaybackPlan,
  computeFrameSimilarity,
  exportLoopInBrowser,
  extractPreviewFrames,
  renderBlendedFrame,
  renderSeamComparison,
  type ExtractedPreviewData,
  type LoopPlaybackStep,
} from '../utils/loopEngine';

const props = defineProps<{
  active: boolean;
}>();

const store = useAvatarStore();
const runningInTauri = isTauri();

// File & metadata state
const probeInfo = ref<VideoProbeInfo | null>(null);
const localInputPath = ref('');
const customOutputPath = ref('');
const inputObjectUrl = ref('');
const fileName = ref('');
const isDragOver = ref(false);

// Preview extraction state
const previewData = ref<ExtractedPreviewData | null>(null);
const isExtractingPreview = ref(false);
const extractProgress = ref(0);

// Loop parameters
const mode = ref<LoopMode>('crossfade');
const startSec = ref(0);
const endSec = ref(0);
const crossfadeSec = ref(0.45);
const bridgeSec = ref(0.4);
const exactEndFrame = ref(false);

// Export parameters
const outputFormat = ref<OutputFormat>('webm');
const resolution = ref<string>('source');
const targetFps = ref<string>('source');
const chromaKey = ref<ChromaKeyConfig>({
  enabled: false,
  colorHex: '#00FF00',
  similarity: 0.3,
  blend: 0.1,
});

// Analysis state
const isAnalyzing = ref(false);
const originalSeamSimilarity = ref<number | null>(null);
const candidates = ref<LoopCandidate[]>([]);

// Preview & Inspector UI state
const previewTab = ref<'loop' | 'seam' | 'result'>('loop');
const isPlaying = ref(true);
const playbackSpeed = ref<0.5 | 1>(1);
const currentStepIdx = ref(0);
const isInSeamZone = ref(false);
const isAtBoundary = ref(false);

const seamViewMode = ref<'onion' | 'split' | 'diff'>('onion');
const onionOpacity = ref(0.5);
const seamRenderedSim = ref<number | null>(null);

// Processing & result state
const isProcessing = ref(false);
const processProgress = ref(0);
const processMessage = ref('');
const errorMessage = ref('');
const processResult = ref<LoopProcessResult | null>(null);
const resultMediaUrl = ref('');
const assignedBadge = ref<'idle' | 'talk' | null>(null);

// Canvas refs
const previewCanvas = ref<HTMLCanvasElement | null>(null);
const seamCanvas = ref<HTMLCanvasElement | null>(null);

let scratchACanvas: HTMLCanvasElement | null = null;
let scratchBCanvas: HTMLCanvasElement | null = null;
let rafId: number | null = null;
let lastTickTime = 0;
let unlistenDragDrop: (() => void) | null = null;

const durationTotal = computed(() => probeInfo.value?.durationSec ?? previewData.value?.duration ?? 0);
const trimmedDuration = computed(() => Math.max(0, endSec.value - startSec.value));
const maxCrossfadeSec = computed(() => Math.max(0.1, Number((trimmedDuration.value * 0.45).toFixed(2))));

watch(trimmedDuration, (dur) => {
  const maxFade = Math.max(0.1, dur * 0.45);
  if (crossfadeSec.value > maxFade) {
    crossfadeSec.value = Number(maxFade.toFixed(2));
  }
});

const playbackPlan = computed<LoopPlaybackStep[]>(() => {
  const data = previewData.value;
  if (!data || data.frames.length === 0) return [];
  return buildLoopPlaybackPlan(data.frames.length, data.previewFps, {
    mode: mode.value,
    startSec: startSec.value,
    endSec: endSec.value,
    crossfadeSec: crossfadeSec.value,
    bridgeSec: bridgeSec.value,
    exactEndFrame: exactEndFrame.value,
  });
});

const previewFirstAndLastSteps = computed(() => {
  const plan = playbackPlan.value;
  if (plan.length === 0) return null;
  return {
    first: plan[0],
    last: plan[plan.length - 1],
  };
});

const previewSeamSimilarity = computed<number | null>(() => {
  const data = previewData.value;
  const pair = previewFirstAndLastSteps.value;
  if (!data || !pair) return null;
  if (
    pair.first.weightB <= 0.001 &&
    pair.last.weightB <= 0.001 &&
    data.analysisFrames[pair.first.indexA] &&
    data.analysisFrames[pair.last.indexA]
  ) {
    return computeFrameSimilarity(
      data.analysisFrames[pair.first.indexA],
      data.analysisFrames[pair.last.indexA]
    );
  }
  return seamRenderedSim.value;
});

const timelineRangeStyle = computed(() => {
  const total = Math.max(0.01, durationTotal.value);
  const left = (startSec.value / total) * 100;
  const width = ((endSec.value - startSec.value) / total) * 100;
  return {
    left: `${Math.max(0, Math.min(100, left))}%`,
    width: `${Math.max(0, Math.min(100 - left, width))}%`,
  };
});

const timelineSeamZoneStyle = computed(() => {
  const total = Math.max(0.01, durationTotal.value);
  if (mode.value === 'crossfade' || (mode.value === 'smart_cut' && crossfadeSec.value > 0.02)) {
    const fade = Math.min(crossfadeSec.value, trimmedDuration.value * 0.45);
    const left = ((endSec.value - fade) / total) * 100;
    const width = (fade / total) * 100;
    return {
      left: `${Math.max(0, left)}%`,
      width: `${Math.max(0, Math.min(100 - left, width))}%`,
    };
  }
  const left = (endSec.value / total) * 100;
  return { left: `${Math.max(0, Math.min(99, left))}%`, width: '1.5%' };
});

const playheadPercent = computed(() => {
  const data = previewData.value;
  const plan = playbackPlan.value;
  if (!data || plan.length === 0 || durationTotal.value <= 0) return 0;
  const step = plan[currentStepIdx.value % plan.length];
  const t = data.timestamps[step.indexA] ?? startSec.value;
  return Math.max(0, Math.min(100, (t / durationTotal.value) * 100));
});

const modeCards: { id: LoopMode; title: string; subtitle: string; icon: unknown }[] = [
  {
    id: 'crossfade',
    title: 'Кроссфейд',
    subtitle: 'Мягкое смешение конца и начала (xfade)',
    icon: Blend,
  },
  {
    id: 'ping_pong',
    title: 'Пинг-понг',
    subtitle: 'Вперёд + реверс (100% бесшовность)',
    icon: ArrowLeftRight,
  },
  {
    id: 'morph',
    title: 'Морфинг-мост',
    subtitle: 'Оптическая достройка кадров стыка',
    icon: Wand2,
  },
  {
    id: 'smart_cut',
    title: 'Умная обрезка',
    subtitle: 'Поиск совпадающих поз + подрезка',
    icon: Scissors,
  },
];

function ensureScratchCanvases(width: number, height: number) {
  if (!scratchACanvas) scratchACanvas = document.createElement('canvas');
  if (!scratchBCanvas) scratchBCanvas = document.createElement('canvas');
  if (scratchACanvas.width !== width || scratchACanvas.height !== height) {
    scratchACanvas.width = width;
    scratchACanvas.height = height;
  }
  if (scratchBCanvas.width !== width || scratchBCanvas.height !== height) {
    scratchBCanvas.width = width;
    scratchBCanvas.height = height;
  }
  return {
    ctxA: scratchACanvas.getContext('2d')!,
    ctxB: scratchBCanvas.getContext('2d')!,
  };
}

function renderCurrentPreviewFrame() {
  const canvas = previewCanvas.value;
  const data = previewData.value;
  const plan = playbackPlan.value;
  if (!canvas || !data || plan.length === 0) return;

  if (canvas.width !== data.width || canvas.height !== data.height) {
    canvas.width = data.width;
    canvas.height = data.height;
  }
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const idx = currentStepIdx.value % plan.length;
  const step = plan[idx];
  const { ctxA, ctxB } = ensureScratchCanvases(data.width, data.height);

  renderBlendedFrame(
    ctx,
    data.frames[step.indexA],
    data.frames[step.indexB],
    step.weightB,
    data.width,
    data.height,
    ctxA,
    ctxB
  );

  isInSeamZone.value = step.isSeamZone;
  isAtBoundary.value = step.isBoundary || idx === 0;
}

function updateSeamInspector() {
  const canvas = seamCanvas.value;
  const data = previewData.value;
  const pair = previewFirstAndLastSteps.value;
  if (!canvas || !data || !pair) return;

  if (canvas.width !== data.width || canvas.height !== data.height) {
    canvas.width = data.width;
    canvas.height = data.height;
  }

  const { ctxA, ctxB } = ensureScratchCanvases(data.width, data.height);

  const firstCanvas = document.createElement('canvas');
  firstCanvas.width = data.width;
  firstCanvas.height = data.height;
  renderBlendedFrame(
    firstCanvas.getContext('2d')!,
    data.frames[pair.first.indexA],
    data.frames[pair.first.indexB],
    pair.first.weightB,
    data.width,
    data.height,
    ctxA,
    ctxB
  );

  const lastCanvas = document.createElement('canvas');
  lastCanvas.width = data.width;
  lastCanvas.height = data.height;
  renderBlendedFrame(
    lastCanvas.getContext('2d')!,
    data.frames[pair.last.indexA],
    data.frames[pair.last.indexB],
    pair.last.weightB,
    data.width,
    data.height,
    ctxA,
    ctxB
  );

  seamRenderedSim.value = renderSeamComparison(
    canvas,
    firstCanvas,
    lastCanvas,
    seamViewMode.value,
    onionOpacity.value
  );
}

function stopRaf() {
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
}

function startRaf() {
  stopRaf();
  lastTickTime = performance.now();
  const loop = (now: number) => {
    rafId = requestAnimationFrame(loop);
    if (!props.active || previewTab.value !== 'loop' || !isPlaying.value) {
      lastTickTime = now;
      return;
    }
    const data = previewData.value;
    const plan = playbackPlan.value;
    if (!data || plan.length === 0) return;

    const stepInterval = 1000 / (data.previewFps * playbackSpeed.value);
    if (now - lastTickTime >= stepInterval) {
      lastTickTime = now;
      currentStepIdx.value = (currentStepIdx.value + 1) % plan.length;
      renderCurrentPreviewFrame();
    }
  };
  rafId = requestAnimationFrame(loop);
}

watch(
  [playbackPlan, previewTab, seamViewMode, onionOpacity],
  () => {
    currentStepIdx.value = 0;
    renderCurrentPreviewFrame();
    updateSeamInspector();
  },
  { flush: 'post' }
);

watch(
  () => props.active,
  (isActive) => {
    if (isActive) {
      lastTickTime = performance.now();
      renderCurrentPreviewFrame();
      updateSeamInspector();
    }
  }
);

function cleanupLoadedMedia() {
  if (previewData.value) {
    previewData.value.dispose();
    previewData.value = null;
  }
  if (inputObjectUrl.value) {
    URL.revokeObjectURL(inputObjectUrl.value);
    inputObjectUrl.value = '';
  }
  if (resultMediaUrl.value) {
    URL.revokeObjectURL(resultMediaUrl.value);
    resultMediaUrl.value = '';
  }
}

function guessMimeFromPath(path: string): string {
  const lower = path.toLowerCase();
  if (lower.endsWith('.mp4') || lower.endsWith('.m4v')) return 'video/mp4';
  if (lower.endsWith('.mov')) return 'video/quicktime';
  if (lower.endsWith('.gif')) return 'image/gif';
  return 'video/webm';
}

async function loadPreviewFromUrl(url: string, fallbackName: string) {
  isExtractingPreview.value = true;
  extractProgress.value = 0;
  try {
    const extracted = await extractPreviewFrames(url, (pct) => {
      extractProgress.value = pct;
    });
    if (previewData.value) {
      previewData.value.dispose();
    }
    previewData.value = extracted;

    if (!probeInfo.value) {
      probeInfo.value = {
        path: fallbackName,
        width: extracted.sourceWidth,
        height: extracted.sourceHeight,
        fps: 30,
        durationSec: extracted.duration,
        frameCount: Math.round(extracted.duration * 30),
        codecName: 'browser',
        pixFmt: 'rgba',
        hasAlpha: true,
        fileSizeBytes: 0,
      };
    }

    startSec.value = 0;
    endSec.value = Number(durationTotal.value.toFixed(2));
    crossfadeSec.value = Number(Math.min(0.45, durationTotal.value * 0.25).toFixed(2));
    if (extracted.analysisFrames.length >= 2) {
      originalSeamSimilarity.value = computeFrameSimilarity(
        extracted.analysisFrames[0],
        extracted.analysisFrames[extracted.analysisFrames.length - 1]
      );
    }
    renderCurrentPreviewFrame();
    updateSeamInspector();
    startRaf();
  } finally {
    isExtractingPreview.value = false;
  }
}

async function loadFromNativePath(path: string) {
  errorMessage.value = '';
  processResult.value = null;
  candidates.value = [];
  assignedBadge.value = null;
  cleanupLoadedMedia();

  localInputPath.value = path;
  fileName.value = path.split(/[\\/]/).pop() || path;

  try {
    const info = await invoke<VideoProbeInfo>('probe_video', { path });
    probeInfo.value = info;
    const rawBuf = await invoke<ArrayBuffer>('read_video_bytes', { path });
    const blob = new Blob([rawBuf], { type: guessMimeFromPath(path) });
    inputObjectUrl.value = URL.createObjectURL(blob);
    await loadPreviewFromUrl(inputObjectUrl.value, fileName.value);
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  }
}

async function loadFromFileObject(file: File) {
  errorMessage.value = '';
  processResult.value = null;
  candidates.value = [];
  assignedBadge.value = null;
  cleanupLoadedMedia();

  fileName.value = file.name;

  try {
    if (runningInTauri) {
      const buf = new Uint8Array(await file.arrayBuffer());
      const info = await invoke<VideoProbeInfo>('stage_uploaded_video', buf, {
        headers: { 'x-file-name': file.name },
      });
      probeInfo.value = info;
      localInputPath.value = info.path;
    } else {
      probeInfo.value = null;
      localInputPath.value = file.name;
    }
    inputObjectUrl.value = URL.createObjectURL(file);
    await loadPreviewFromUrl(inputObjectUrl.value, file.name);
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  }
}

function onFileInputChange(ev: Event) {
  const target = ev.target as HTMLInputElement;
  if (target.files && target.files[0]) {
    void loadFromFileObject(target.files[0]);
    target.value = '';
  }
}

function onBrowserDrop(ev: DragEvent) {
  isDragOver.value = false;
  if (runningInTauri) return; // Tauri native drag-drop listener handles it
  const file = ev.dataTransfer?.files?.[0];
  if (file) {
    void loadFromFileObject(file);
  }
}

function jumpNearSeam() {
  const data = previewData.value;
  const plan = playbackPlan.value;
  if (!data || plan.length === 0) return;
  const backSteps = Math.round(1.5 * data.previewFps);
  currentStepIdx.value = Math.max(0, plan.length - Math.min(plan.length - 1, backSteps));
  isPlaying.value = true;
  previewTab.value = 'loop';
  renderCurrentPreviewFrame();
}

function applyCandidate(cand: LoopCandidate) {
  startSec.value = Number(cand.startSec.toFixed(2));
  endSec.value = Number(cand.endSec.toFixed(2));
  if (mode.value !== 'smart_cut' && cand.similarity >= 94) {
    mode.value = 'smart_cut';
    crossfadeSec.value = 0.08;
  }
  previewTab.value = 'loop';
}

async function runAutoAnalyze() {
  if (!previewData.value && !localInputPath.value) return;
  isAnalyzing.value = true;
  errorMessage.value = '';

  try {
    if (runningInTauri && localInputPath.value) {
      const res = await invoke<LoopAnalysisResult>('analyze_video_loop', {
        path: localInputPath.value,
        minLoopSec: null,
      });
      originalSeamSimilarity.value = res.originalSeamSimilarity;
      candidates.value = res.candidates;
      if (res.candidates[0]) {
        applyCandidate(res.candidates[0]);
      }
    } else if (previewData.value) {
      const res = analyzeFramesInBrowser(
        previewData.value.analysisFrames,
        previewData.value.previewFps,
        previewData.value.duration
      );
      originalSeamSimilarity.value = res.originalSeamSimilarity;
      candidates.value = res.candidates;
      if (res.candidates[0]) {
        applyCandidate(res.candidates[0]);
      }
    }
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  } finally {
    isAnalyzing.value = false;
  }
}

function resolveResolutionOption(): string | null {
  switch (resolution.value) {
    case '480p':
      return '854x480';
    case '720p':
      return '1280x720';
    case '1080p':
      return '1920x1080';
    case '2k':
      return '2560x1440';
    default:
      return null;
  }
}

async function startExport() {
  if (!probeInfo.value && !inputObjectUrl.value) return;
  isProcessing.value = true;
  processProgress.value = 5;
  processMessage.value = 'Запуск обработки...';
  errorMessage.value = '';
  assignedBadge.value = null;

  if (resultMediaUrl.value) {
    URL.revokeObjectURL(resultMediaUrl.value);
    resultMediaUrl.value = '';
  }

  try {
    if (runningInTauri && localInputPath.value) {
      const channel = new Channel<LoopProgressEvent>();
      channel.onmessage = (ev) => {
        processProgress.value = ev.percent;
        processMessage.value = ev.message;
      };

      const res = await invoke<LoopProcessResult>('process_video_loop', {
        options: {
          inputPath: localInputPath.value,
          outputPath: customOutputPath.value.trim() || null,
          mode: mode.value,
          startSec: startSec.value,
          endSec: endSec.value,
          crossfadeSec: crossfadeSec.value,
          bridgeSec: bridgeSec.value,
          exactEndFrame: exactEndFrame.value,
          format: outputFormat.value,
          resolution: resolveResolutionOption(),
          targetFps: targetFps.value === 'source' ? null : Number(targetFps.value),
          chromaKey: chromaKey.value.enabled
            ? {
                color_hex: chromaKey.value.colorHex,
                similarity: chromaKey.value.similarity,
                blend: chromaKey.value.blend,
              }
            : null,
        },
        onProgress: channel,
      });

      processResult.value = res;
      const bytes = await invoke<ArrayBuffer>('read_video_bytes', { path: res.outputPath });
      const blob = new Blob([bytes], { type: guessMimeFromPath(res.outputPath) });
      resultMediaUrl.value = URL.createObjectURL(blob);
      previewTab.value = 'result';
    } else if (inputObjectUrl.value) {
      const res = await exportLoopInBrowser(
        inputObjectUrl.value,
        {
          mode: mode.value,
          startSec: startSec.value,
          endSec: endSec.value,
          crossfadeSec: crossfadeSec.value,
          bridgeSec: bridgeSec.value,
          exactEndFrame: exactEndFrame.value,
          resolution: resolveResolutionOption(),
          targetFps: targetFps.value === 'source' ? null : Number(targetFps.value),
          chromaKey: chromaKey.value,
        },
        (pct, msg) => {
          processProgress.value = pct;
          processMessage.value = msg;
        }
      );

      const outName = `${fileName.value.replace(/\.[^.]+$/, '') || 'avatar'}_loop.webm`;
      resultMediaUrl.value = res.url;
      processResult.value = {
        outputPath: outName,
        durationSec: res.durationSec,
        frameCount: res.frameCount,
        fileSizeBytes: res.fileSizeBytes,
        originalSeamSimilarity: res.originalSeamSimilarity,
        outputSeamSimilarity: res.outputSeamSimilarity,
        ffmpegArgs: [],
      };
      previewTab.value = 'result';
    }
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  } finally {
    isProcessing.value = false;
  }
}

function downloadResult() {
  if (!resultMediaUrl.value || !processResult.value) return;
  const a = document.createElement('a');
  a.href = resultMediaUrl.value;
  a.download = processResult.value.outputPath.split(/[\\/]/).pop() || 'avatar_loop.webm';
  a.click();
}

async function revealOutputFolder() {
  if (!runningInTauri || !processResult.value) return;
  try {
    await invoke('reveal_in_folder', { path: processResult.value.outputPath });
  } catch (err) {
    errorMessage.value = err instanceof Error ? err.message : String(err);
  }
}

function assignToAvatar(slot: 'idle' | 'talk') {
  if (!resultMediaUrl.value || !processResult.value) return;
  const name = processResult.value.outputPath.split(/[\\/]/).pop() || 'Loop Video';
  store.assignLoopToSlot(slot, resultMediaUrl.value, name, processResult.value.durationSec);
  assignedBadge.value = slot;
}

function formatBytes(bytes: number): string {
  if (!bytes) return '—';
  if (bytes < 1024) return `${bytes} Б`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КБ`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} МБ`;
}

onMounted(async () => {
  if (runningInTauri) {
    try {
      unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
        if (!props.active) return;
        if (event.payload.type === 'enter' || event.payload.type === 'over') {
          isDragOver.value = true;
        } else if (event.payload.type === 'leave') {
          isDragOver.value = false;
        } else if (event.payload.type === 'drop') {
          isDragOver.value = false;
          const droppedPath = event.payload.paths?.[0];
          if (droppedPath) {
            void loadFromNativePath(droppedPath);
          }
        }
      });
    } catch {
      // Ignore if webview drag-drop is unavailable
    }
  }
});

onBeforeUnmount(() => {
  stopRaf();
  unlistenDragDrop?.();
  cleanupLoadedMedia();
});
</script>

<template>
  <div class="grid grid-cols-1 lg:grid-cols-12 gap-6 text-slate-200">
    <!-- Left Column: Dropzone, Live Preview, Seam Inspector & Timeline -->
    <div class="lg:col-span-7 flex flex-col gap-4">
      <!-- Upload / File Bar -->
      <div
        @dragover.prevent="isDragOver = true"
        @dragleave="isDragOver = false"
        @drop.prevent="onBrowserDrop"
        :class="[
          'bg-slate-900 border rounded-xl p-4 transition-colors',
          isDragOver ? 'border-emerald-400 bg-emerald-950/20' : 'border-slate-800'
        ]"
      >
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div class="flex items-center gap-3 min-w-0">
            <div class="w-10 h-10 rounded-lg bg-emerald-500/15 border border-emerald-500/30 flex items-center justify-center text-emerald-400 shrink-0">
              <Repeat class="w-5 h-5" />
            </div>
            <div class="min-w-0">
              <h2 class="text-sm font-semibold text-white truncate">
                {{ fileName || 'Загрузите видео или анимацию для зацикливания' }}
              </h2>
              <div v-if="probeInfo" class="flex flex-wrap items-center gap-2 mt-1 text-[11px] font-mono text-slate-400">
                <span class="px-1.5 py-0.5 rounded bg-slate-800 text-slate-200">{{ probeInfo.width }}×{{ probeInfo.height }}</span>
                <span class="px-1.5 py-0.5 rounded bg-slate-800">{{ probeInfo.fps.toFixed(1) }} FPS</span>
                <span class="px-1.5 py-0.5 rounded bg-slate-800">{{ probeInfo.durationSec.toFixed(2) }}с</span>
                <span
                  :class="[
                    'px-1.5 py-0.5 rounded',
                    probeInfo.hasAlpha ? 'bg-emerald-500/15 text-emerald-400' : 'bg-slate-800 text-slate-400'
                  ]"
                >
                  {{ probeInfo.hasAlpha ? 'Alpha канал' : 'Без Alpha' }}
                </span>
              </div>
              <p v-else class="text-xs text-slate-400 mt-0.5">
                Перетащите MP4, MOV, WebM или GIF либо выберите файл
              </p>
            </div>
          </div>

          <div class="flex items-center gap-2">
            <span
              :class="[
                'text-[10px] font-mono px-2 py-1 rounded-full border',
                runningInTauri
                  ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20'
                  : 'bg-sky-500/10 text-sky-400 border-sky-500/20'
              ]"
            >
              {{ runningInTauri ? 'FFmpeg Native' : 'Browser Canvas' }}
            </span>

            <label class="px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 border border-slate-700 text-xs font-medium text-white cursor-pointer flex items-center gap-1.5 transition-colors">
              <Upload class="w-3.5 h-3.5" />
              <span>Выбрать файл</span>
              <input type="file" accept="video/*,image/gif" class="hidden" @change="onFileInputChange" />
            </label>
          </div>
        </div>

        <!-- Optional local path loader in Tauri mode -->
        <div v-if="runningInTauri" class="mt-3 flex items-center gap-2">
          <input
            v-model="localInputPath"
            type="text"
            placeholder="Или вставьте путь к локальному файлу (например /path/to/avatar.mov)"
            class="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-1.5 text-xs font-mono text-slate-300 focus:outline-none focus:border-emerald-500"
          />
          <button
            @click="localInputPath && loadFromNativePath(localInputPath)"
            :disabled="!localInputPath"
            class="px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 disabled:opacity-40 text-xs font-medium text-slate-200 cursor-pointer"
          >
            Открыть
          </button>
        </div>

        <!-- Frame extraction progress -->
        <div v-if="isExtractingPreview" class="mt-3 flex flex-col gap-1">
          <div class="w-full h-1.5 bg-slate-800 rounded-full overflow-hidden">
            <div class="h-full bg-emerald-500 transition-all duration-150" :style="{ width: `${extractProgress}%` }" />
          </div>
          <span class="text-[11px] font-mono text-slate-400">Индексация кадров для превью и анализа стыка: {{ extractProgress }}%</span>
        </div>
      </div>

      <!-- Preview & Seam Inspector Card -->
      <div class="bg-slate-900 border border-slate-800 rounded-xl p-4 flex flex-col gap-3">
        <!-- Preview sub-tabs + metrics -->
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 pb-3">
          <div class="flex items-center gap-1 bg-slate-950 p-1 rounded-lg border border-slate-800">
            <button
              @click="previewTab = 'loop'"
              :class="[
                'px-2.5 py-1 rounded-md text-xs font-medium flex items-center gap-1.5 cursor-pointer transition-colors',
                previewTab === 'loop' ? 'bg-slate-800 text-white' : 'text-slate-400 hover:text-slate-200'
              ]"
            >
              <Film class="w-3.5 h-3.5" />
              Цикл (Превью)
            </button>
            <button
              @click="previewTab = 'seam'"
              :class="[
                'px-2.5 py-1 rounded-md text-xs font-medium flex items-center gap-1.5 cursor-pointer transition-colors',
                previewTab === 'seam' ? 'bg-slate-800 text-white' : 'text-slate-400 hover:text-slate-200'
              ]"
            >
              <Layers class="w-3.5 h-3.5" />
              Стык: 1-й ↔ Последний
            </button>
            <button
              @click="previewTab = 'result'"
              :disabled="!resultMediaUrl"
              :class="[
                'px-2.5 py-1 rounded-md text-xs font-medium flex items-center gap-1.5 cursor-pointer transition-colors disabled:opacity-40',
                previewTab === 'result' ? 'bg-slate-800 text-white' : 'text-slate-400 hover:text-slate-200'
              ]"
            >
              <CheckCircle2 class="w-3.5 h-3.5" />
              Результат
            </button>
          </div>

          <div class="flex items-center gap-2 text-xs font-mono">
            <span v-if="originalSeamSimilarity !== null" class="px-2 py-0.5 rounded bg-slate-800 text-slate-400">
              До: {{ originalSeamSimilarity.toFixed(1) }}%
            </span>
            <span
              v-if="previewSeamSimilarity !== null"
              :class="[
                'px-2 py-0.5 rounded border',
                previewSeamSimilarity >= 95
                  ? 'bg-emerald-500/15 text-emerald-400 border-emerald-500/30'
                  : 'bg-amber-500/15 text-amber-300 border-amber-500/30'
              ]"
            >
              Стык цикла: {{ previewSeamSimilarity.toFixed(1) }}%
            </span>
          </div>
        </div>

        <!-- Viewport with alpha checkerboard -->
        <div
          class="relative min-h-[320px] rounded-xl border border-slate-800 overflow-hidden flex items-center justify-center"
          style="background-image: linear-gradient(45deg, #111827 25%, transparent 25%), linear-gradient(-45deg, #111827 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #111827 75%), linear-gradient(-45deg, transparent 75%, #111827 75%); background-size: 20px 20px; background-position: 0 0, 0 10px, 10px -10px, -10px 0px; background-color: #0b0f19;"
        >
          <!-- Empty state -->
          <div v-if="!previewData && previewTab !== 'result'" class="text-center p-8 flex flex-col items-center gap-2 text-slate-400">
            <div class="w-12 h-12 rounded-full bg-slate-800/80 flex items-center justify-center text-slate-500">
              <Repeat class="w-6 h-6" />
            </div>
            <p class="text-sm font-medium text-slate-300">Нет загруженной анимации</p>
            <p class="text-xs text-slate-500 max-w-xs">
              Загрузите видео выше, чтобы увидеть предпросмотр стыка, автопоиск лучшего цикла и экспорт без шва
            </p>
          </div>

          <!-- Tab 1: Live Loop Canvas -->
          <template v-else-if="previewTab === 'loop'">
            <canvas ref="previewCanvas" class="max-h-[360px] w-auto h-auto object-contain" />
            <div
              v-if="isAtBoundary"
              class="absolute top-3 left-3 px-2.5 py-1 rounded-full bg-emerald-500/90 text-slate-950 text-[11px] font-bold font-mono shadow-lg"
            >
              ШОВ ЦИКЛА
            </div>
            <div
              v-else-if="isInSeamZone"
              class="absolute top-3 left-3 px-2.5 py-1 rounded-full bg-amber-500/80 text-slate-950 text-[11px] font-bold font-mono shadow-lg"
            >
              ПЕРЕХОД
            </div>
          </template>

          <!-- Tab 2: First vs Last Frame Inspector -->
          <template v-else-if="previewTab === 'seam'">
            <canvas ref="seamCanvas" class="max-h-[360px] w-auto h-auto object-contain" />
          </template>

          <!-- Tab 3: Exported Video Result -->
          <template v-else-if="previewTab === 'result' && resultMediaUrl">
            <img
              v-if="outputFormat === 'gif'"
              :src="resultMediaUrl"
              alt="Готовый GIF цикл"
              class="max-h-[360px] w-auto h-auto object-contain"
            />
            <video
              v-else
              :src="resultMediaUrl"
              autoplay
              loop
              muted
              playsinline
              controls
              class="max-h-[360px] w-auto h-auto object-contain"
            />
          </template>
        </div>

        <!-- Contextual preview controls -->
        <div v-if="previewData && previewTab === 'loop'" class="flex flex-wrap items-center justify-between gap-2">
          <div class="flex items-center gap-2">
            <button
              @click="isPlaying = !isPlaying"
              class="px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-xs font-medium text-white flex items-center gap-1.5 cursor-pointer"
            >
              <Pause v-if="isPlaying" class="w-3.5 h-3.5" />
              <Play v-else class="w-3.5 h-3.5" />
              <span>{{ isPlaying ? 'Пауза' : 'Играть' }}</span>
            </button>

            <button
              @click="jumpNearSeam"
              class="px-3 py-1.5 rounded-lg bg-emerald-600/20 hover:bg-emerald-600/30 border border-emerald-500/30 text-xs font-medium text-emerald-300 flex items-center gap-1.5 cursor-pointer"
            >
              <SkipForward class="w-3.5 h-3.5" />
              <span>К стыку (−1.5с)</span>
            </button>
          </div>

          <div class="flex items-center gap-1 bg-slate-950 p-1 rounded-lg border border-slate-800 text-xs font-mono">
            <button
              @click="playbackSpeed = 0.5"
              :class="[
                'px-2 py-0.5 rounded cursor-pointer',
                playbackSpeed === 0.5 ? 'bg-slate-800 text-emerald-400' : 'text-slate-400'
              ]"
            >
              0.5×
            </button>
            <button
              @click="playbackSpeed = 1"
              :class="[
                'px-2 py-0.5 rounded cursor-pointer',
                playbackSpeed === 1 ? 'bg-slate-800 text-emerald-400' : 'text-slate-400'
              ]"
            >
              1×
            </button>
          </div>
        </div>

        <div v-else-if="previewData && previewTab === 'seam'" class="flex flex-wrap items-center justify-between gap-3">
          <div class="flex items-center gap-1 bg-slate-950 p-1 rounded-lg border border-slate-800 text-xs">
            <button
              @click="seamViewMode = 'onion'"
              :class="['px-2.5 py-1 rounded cursor-pointer', seamViewMode === 'onion' ? 'bg-slate-800 text-white' : 'text-slate-400']"
            >
              Наложение
            </button>
            <button
              @click="seamViewMode = 'split'"
              :class="['px-2.5 py-1 rounded cursor-pointer', seamViewMode === 'split' ? 'bg-slate-800 text-white' : 'text-slate-400']"
            >
              Рядом
            </button>
            <button
              @click="seamViewMode = 'diff'"
              :class="['px-2.5 py-1 rounded cursor-pointer', seamViewMode === 'diff' ? 'bg-slate-800 text-white' : 'text-slate-400']"
            >
              Карта отличий
            </button>
          </div>

          <div v-if="seamViewMode === 'onion'" class="flex items-center gap-2 text-xs">
            <Eye class="w-3.5 h-3.5 text-slate-400" />
            <span class="text-slate-400">Последний кадр:</span>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              v-model.number="onionOpacity"
              class="w-28 accent-emerald-500 cursor-pointer"
            />
            <span class="font-mono text-emerald-400 w-9">{{ Math.round(onionOpacity * 100) }}%</span>
          </div>
        </div>
      </div>

      <!-- Timeline, Trim & Auto-Find Best Loop -->
      <div class="bg-slate-900 border border-slate-800 rounded-xl p-4 flex flex-col gap-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h3 class="text-xs font-semibold text-slate-200 uppercase tracking-wider">Таймлайн и поиск лучшего цикла</h3>
            <p class="text-[11px] text-slate-400">
              Выбран отрезок: {{ startSec.toFixed(2) }}с → {{ endSec.toFixed(2) }}с ({{ trimmedDuration.toFixed(2) }}с)
            </p>
          </div>
          <button
            @click="runAutoAnalyze"
            :disabled="!previewData || isAnalyzing"
            class="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 disabled:opacity-40 text-xs font-medium text-white flex items-center gap-1.5 cursor-pointer transition-colors"
          >
            <Sparkles class="w-3.5 h-3.5" />
            <span>{{ isAnalyzing ? 'Анализ кадров...' : 'Автопоиск лучшего цикла' }}</span>
          </button>
        </div>

        <!-- Visual timeline track -->
        <div class="relative h-6 bg-slate-950 border border-slate-800 rounded-lg overflow-hidden">
          <div class="absolute top-0 bottom-0 bg-emerald-500/20 border-x border-emerald-400/60" :style="timelineRangeStyle" />
          <div class="absolute top-0 bottom-0 bg-amber-500/35" :style="timelineSeamZoneStyle" />
          <div class="absolute top-0 bottom-0 w-0.5 bg-white shadow" :style="{ left: `${playheadPercent}%` }" />
        </div>

        <!-- Start / End Sliders -->
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-3 text-xs">
          <div class="flex flex-col gap-1">
            <div class="flex justify-between text-slate-400">
              <span>Начало (Start)</span>
              <span class="font-mono text-slate-200">{{ startSec.toFixed(2) }}с</span>
            </div>
            <input
              type="range"
              min="0"
              :max="Math.max(0, endSec - 0.2)"
              step="0.02"
              v-model.number="startSec"
              :disabled="!previewData"
              class="accent-emerald-500 cursor-pointer disabled:opacity-40"
            />
          </div>

          <div class="flex flex-col gap-1">
            <div class="flex justify-between text-slate-400">
              <span>Конец (End)</span>
              <span class="font-mono text-slate-200">{{ endSec.toFixed(2) }}с</span>
            </div>
            <input
              type="range"
              :min="Math.min(durationTotal, startSec + 0.2)"
              :max="Math.max(0.2, durationTotal)"
              step="0.02"
              v-model.number="endSec"
              :disabled="!previewData"
              class="accent-emerald-500 cursor-pointer disabled:opacity-40"
            />
          </div>
        </div>

        <!-- Auto-detected Candidates -->
        <div v-if="candidates.length > 0" class="flex flex-col gap-1.5 pt-1 border-t border-slate-800/80">
          <span class="text-[11px] text-slate-400">Найденные совпадения поз и движения (нажмите для применения):</span>
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="(cand, i) in candidates"
              :key="i"
              @click="applyCandidate(cand)"
              class="px-2.5 py-1 rounded-lg bg-slate-950 hover:bg-slate-800 border border-slate-800 hover:border-emerald-500/50 text-[11px] font-mono flex items-center gap-1.5 cursor-pointer transition-colors"
            >
              <span class="text-slate-300">{{ cand.startSec.toFixed(2) }}с → {{ cand.endSec.toFixed(2) }}с</span>
              <span class="text-emerald-400 font-semibold">{{ cand.similarity.toFixed(1) }}%</span>
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Right Column: Algorithm Mode, Settings, Export & Results -->
    <div class="lg:col-span-5 flex flex-col gap-4">
      <!-- Mode Selector -->
      <div class="bg-slate-900 border border-slate-800 rounded-xl p-4 flex flex-col gap-3">
        <div class="flex items-center justify-between">
          <h3 class="text-xs font-semibold text-slate-200 uppercase tracking-wider">Режим зацикливания</h3>
          <Sliders class="w-4 h-4 text-slate-500" />
        </div>

        <div class="grid grid-cols-2 gap-2">
          <button
            v-for="item in modeCards"
            :key="item.id"
            @click="mode = item.id"
            :class="[
              'p-3 rounded-xl border text-left flex flex-col gap-1 transition-all cursor-pointer',
              mode === item.id
                ? 'bg-emerald-600/15 border-emerald-500 text-white shadow-sm shadow-emerald-950/50'
                : 'bg-slate-950/60 border-slate-800 text-slate-300 hover:bg-slate-800/60'
            ]"
          >
            <div class="flex items-center gap-2">
              <component :is="item.icon" class="w-4 h-4 text-emerald-400 shrink-0" />
              <span class="text-xs font-semibold">{{ item.title }}</span>
            </div>
            <span class="text-[11px] text-slate-400 leading-snug">{{ item.subtitle }}</span>
          </button>
        </div>

        <!-- Algorithm Parameters -->
        <div class="pt-2 border-t border-slate-800/80 flex flex-col gap-3">
          <div v-if="mode === 'crossfade' || mode === 'smart_cut'" class="flex flex-col gap-1">
            <div class="flex justify-between text-xs">
              <span class="text-slate-400">
                {{ mode === 'smart_cut' ? 'Микро-кроссфейд на стыке' : 'Длительность кроссфейда' }}
              </span>
              <span class="font-mono text-emerald-400">{{ crossfadeSec.toFixed(2) }}с</span>
            </div>
            <input
              type="range"
              :min="mode === 'smart_cut' ? 0 : 0.08"
              :max="maxCrossfadeSec"
              step="0.02"
              v-model.number="crossfadeSec"
              class="accent-emerald-500 cursor-pointer"
            />
          </div>

          <div v-else-if="mode === 'morph'" class="flex flex-col gap-1">
            <div class="flex justify-between text-xs">
              <span class="text-slate-400">Длительность морфинг-моста</span>
              <span class="font-mono text-emerald-400">{{ bridgeSec.toFixed(2) }}с</span>
            </div>
            <input
              type="range"
              min="0.15"
              max="1.5"
              step="0.05"
              v-model.number="bridgeSec"
              class="accent-emerald-500 cursor-pointer"
            />
            <p class="text-[11px] text-slate-500">
              В нативном режиме используется motion-компенсация FFmpeg (minterpolate) с сохранением Alpha-канала
            </p>
          </div>

          <p v-else class="text-[11px] text-slate-400">
            Проигрывает отрезок вперёд и плавно обратно без дублирования крайнего разворотного кадра.
          </p>

          <!-- Exact End Frame Option -->
          <label class="flex items-start gap-2.5 p-2.5 rounded-lg bg-slate-950/60 border border-slate-800/80 cursor-pointer">
            <input
              type="checkbox"
              v-model="exactEndFrame"
              class="mt-0.5 rounded accent-emerald-500 cursor-pointer"
            />
            <div class="flex flex-col">
              <span class="text-xs font-medium text-slate-200">Точный финальный кадр (кадр N = кадр 0)</span>
              <span class="text-[11px] text-slate-400">
                Гарантирует попиксельное совпадение 1-го и последнего кадров; выключите для максимальной плавности без паузы на повторе
              </span>
            </div>
          </label>
        </div>
      </div>

      <!-- Export Settings -->
      <div class="bg-slate-900 border border-slate-800 rounded-xl p-4 flex flex-col gap-3">
        <h3 class="text-xs font-semibold text-slate-200 uppercase tracking-wider">Настройки экспорта</h3>

        <!-- Format -->
        <div class="flex flex-col gap-1.5">
          <div class="flex items-center justify-between">
            <label class="text-xs text-slate-400">Формат видео</label>
            <span v-if="!runningInTauri" class="text-[10px] text-sky-400">В браузере доступен только WebM</span>
          </div>
          <div class="grid grid-cols-4 gap-1.5">
            <button
              v-for="fmt in (['webm', 'mp4', 'gif', 'mov'] as OutputFormat[])"
              :key="fmt"
              @click="outputFormat = fmt"
              :disabled="!runningInTauri && fmt !== 'webm'"
              :class="[
                'py-1.5 rounded-lg text-xs font-mono font-medium border transition-colors cursor-pointer disabled:opacity-35 disabled:cursor-not-allowed',
                outputFormat === fmt
                  ? 'bg-emerald-600 border-emerald-500 text-white'
                  : 'bg-slate-950 border-slate-800 text-slate-300 hover:bg-slate-800'
              ]"
            >
              {{ fmt.toUpperCase() }}
            </button>
          </div>
        </div>

        <!-- Resolution & FPS -->
        <div class="grid grid-cols-2 gap-3">
          <div class="flex flex-col gap-1">
            <label class="text-xs text-slate-400">Разрешение</label>
            <select
              v-model="resolution"
              class="bg-slate-950 border border-slate-800 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-emerald-500"
            >
              <option value="source">Исходное</option>
              <option value="480p">480p</option>
              <option value="720p">720p</option>
              <option value="1080p">1080p</option>
              <option value="2k">2K (1440p)</option>
            </select>
          </div>

          <div class="flex flex-col gap-1">
            <label class="text-xs text-slate-400">Частота кадров</label>
            <select
              v-model="targetFps"
              class="bg-slate-950 border border-slate-800 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-emerald-500"
            >
              <option value="source">Исходный FPS</option>
              <option value="24">24 FPS</option>
              <option value="30">30 FPS</option>
              <option value="60">60 FPS</option>
            </select>
          </div>
        </div>

        <!-- Optional Chroma Key -->
        <div class="pt-2 border-t border-slate-800/80 flex flex-col gap-2">
          <label class="flex items-center justify-between cursor-pointer">
            <span class="text-xs font-medium text-slate-300">Удалить фон при экспорте (Хромакей)</span>
            <input type="checkbox" v-model="chromaKey.enabled" class="rounded accent-emerald-500 cursor-pointer" />
          </label>
          <div v-if="chromaKey.enabled" class="grid grid-cols-3 gap-2 items-center bg-slate-950/60 p-2.5 rounded-lg border border-slate-800">
            <div class="flex items-center gap-1.5">
              <input type="color" v-model="chromaKey.colorHex" class="w-7 h-7 rounded border border-slate-700 bg-transparent cursor-pointer" />
              <span class="text-[11px] font-mono text-slate-300">{{ chromaKey.colorHex }}</span>
            </div>
            <div class="flex flex-col">
              <span class="text-[10px] text-slate-400">Порог {{ Math.round(chromaKey.similarity * 100) }}%</span>
              <input type="range" min="0.05" max="0.8" step="0.02" v-model.number="chromaKey.similarity" class="accent-emerald-500" />
            </div>
            <div class="flex flex-col">
              <span class="text-[10px] text-slate-400">Сглаж. {{ Math.round(chromaKey.blend * 100) }}%</span>
              <input type="range" min="0" max="0.5" step="0.02" v-model.number="chromaKey.blend" class="accent-emerald-500" />
            </div>
          </div>
        </div>

        <!-- Custom output path (Tauri only) -->
        <div v-if="runningInTauri" class="flex flex-col gap-1">
          <label class="text-xs text-slate-400">Путь сохранения (необязательно)</label>
          <input
            v-model="customOutputPath"
            type="text"
            placeholder="По умолчанию рядом с исходным файлом (*_loop)"
            class="bg-slate-950 border border-slate-800 rounded-lg px-2.5 py-1.5 text-xs font-mono text-slate-300 focus:outline-none focus:border-emerald-500"
          />
        </div>

        <!-- Primary Action -->
        <button
          @click="startExport"
          :disabled="(!previewData && !localInputPath) || isProcessing"
          class="w-full py-2.5 rounded-xl bg-emerald-600 hover:bg-emerald-500 disabled:opacity-45 text-white font-semibold text-sm transition-colors flex items-center justify-center gap-2 cursor-pointer shadow-lg shadow-emerald-950/40"
        >
          <Repeat class="w-4 h-4" :class="{ 'animate-spin': isProcessing }" />
          <span>{{ isProcessing ? 'Обработка цикла...' : 'Создать бесшовный цикл' }}</span>
        </button>

        <!-- Progress bar -->
        <div v-if="isProcessing" class="flex flex-col gap-1">
          <div class="w-full h-2 bg-slate-800 rounded-full overflow-hidden">
            <div class="h-full bg-emerald-500 transition-all duration-200" :style="{ width: `${processProgress}%` }" />
          </div>
          <span class="text-[11px] font-mono text-slate-400 text-center">{{ processMessage }}</span>
        </div>

        <!-- Error box -->
        <div
          v-if="errorMessage"
          class="p-3 rounded-xl bg-rose-500/10 border border-rose-500/30 text-xs text-rose-300 flex items-start gap-2"
        >
          <AlertTriangle class="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
          <span class="break-words">{{ errorMessage }}</span>
        </div>
      </div>

      <!-- Result Card -->
      <div
        v-if="processResult"
        class="bg-slate-900 border border-emerald-500/40 rounded-xl p-4 flex flex-col gap-3 shadow-lg shadow-emerald-950/20"
      >
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 text-emerald-400">
            <CheckCircle2 class="w-4 h-4" />
            <span class="text-xs font-semibold uppercase tracking-wider">Цикл успешно создан</span>
          </div>
          <span class="text-[11px] font-mono text-slate-400">{{ formatBytes(processResult.fileSizeBytes) }}</span>
        </div>

        <div class="grid grid-cols-3 gap-2 text-center bg-slate-950/70 p-2.5 rounded-lg border border-slate-800">
          <div>
            <div class="text-[10px] text-slate-500">Длительность</div>
            <div class="text-xs font-mono text-slate-200">{{ processResult.durationSec.toFixed(2) }}с</div>
          </div>
          <div>
            <div class="text-[10px] text-slate-500">Кадров</div>
            <div class="text-xs font-mono text-slate-200">{{ processResult.frameCount }}</div>
          </div>
          <div>
            <div class="text-[10px] text-slate-500">Стык До → После</div>
            <div class="text-xs font-mono text-emerald-400 font-semibold">
              {{ processResult.originalSeamSimilarity.toFixed(1) }}% → {{ processResult.outputSeamSimilarity.toFixed(1) }}%
            </div>
          </div>
        </div>

        <div class="flex flex-wrap gap-2">
          <button
            @click="downloadResult"
            class="flex-1 py-2 px-3 rounded-lg bg-slate-800 hover:bg-slate-700 text-xs font-medium text-white flex items-center justify-center gap-1.5 cursor-pointer"
          >
            <Download class="w-3.5 h-3.5" />
            <span>Скачать</span>
          </button>

          <button
            v-if="runningInTauri"
            @click="revealOutputFolder"
            class="flex-1 py-2 px-3 rounded-lg bg-slate-800 hover:bg-slate-700 text-xs font-medium text-white flex items-center justify-center gap-1.5 cursor-pointer"
          >
            <FolderOpen class="w-3.5 h-3.5" />
            <span>Открыть папку</span>
          </button>
        </div>

        <div class="pt-2 border-t border-slate-800/80 flex flex-col gap-1.5">
          <span class="text-[11px] text-slate-400">Применить прямо сейчас в плеере аватара:</span>
          <div class="grid grid-cols-2 gap-2">
            <button
              @click="assignToAvatar('idle')"
              :class="[
                'py-1.5 px-2.5 rounded-lg border text-xs font-medium cursor-pointer transition-colors',
                assignedBadge === 'idle'
                  ? 'bg-emerald-600/25 border-emerald-500 text-emerald-300'
                  : 'bg-slate-950 hover:bg-slate-800 border-slate-800 text-slate-200'
              ]"
            >
              {{ assignedBadge === 'idle' ? '✓ Назначено на Idle' : 'На Покой (Idle)' }}
            </button>
            <button
              @click="assignToAvatar('talk')"
              :class="[
                'py-1.5 px-2.5 rounded-lg border text-xs font-medium cursor-pointer transition-colors',
                assignedBadge === 'talk'
                  ? 'bg-emerald-600/25 border-emerald-500 text-emerald-300'
                  : 'bg-slate-950 hover:bg-slate-800 border-slate-800 text-slate-200'
              ]"
            >
              {{ assignedBadge === 'talk' ? '✓ Назначено на Talk' : 'На Разговор (Talk)' }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
