import type { ChromaKeyConfig, LoopCandidate, LoopMode } from '../types/avatar';
import { applyChromaKeyInPlace, resolveResolutionDims } from './chroma';

export interface ExtractedPreviewData {
  frames: HTMLCanvasElement[];
  analysisFrames: Uint8ClampedArray[];
  timestamps: number[];
  width: number;
  height: number;
  sourceWidth: number;
  sourceHeight: number;
  duration: number;
  previewFps: number;
  dispose: () => void;
}

export interface LoopPlaybackStep {
  indexA: number;
  indexB: number;
  weightB: number;
  isSeamZone: boolean;
  isBoundary: boolean;
}

export interface BrowserExportResult {
  blob: Blob;
  url: string;
  durationSec: number;
  frameCount: number;
  fileSizeBytes: number;
  originalSeamSimilarity: number;
  outputSeamSimilarity: number;
}

const ANALYSIS_DIM = 48;

function ensureEven(n: number): number {
  const rounded = Math.max(2, Math.round(n));
  return rounded % 2 === 0 ? rounded : rounded - 1;
}

function waitForEvent(target: EventTarget, eventName: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const onOk = () => {
      cleanup();
      resolve();
    };
    const onErr = () => {
      cleanup();
      reject(new Error(`Ошибка загрузки видео при ожидании события '${eventName}'`));
    };
    const cleanup = () => {
      target.removeEventListener(eventName, onOk);
      target.removeEventListener('error', onErr);
    };
    target.addEventListener(eventName, onOk, { once: true });
    target.addEventListener('error', onErr, { once: true });
  });
}

async function seekVideo(video: HTMLVideoElement, timeSec: number): Promise<void> {
  const target = Math.max(0, Math.min(timeSec, Math.max(0, video.duration - 0.001)));
  if (Math.abs(video.currentTime - target) < 0.0005 && video.readyState >= 2) {
    return;
  }
  const wait = waitForEvent(video, 'seeked');
  video.currentTime = target;
  await wait;
  // WebKit часто сигнализирует 'seeked' до того, как кадр реально
  // presentations-ready. Даём декодеру время обновить изображение.
  await new Promise((resolve) => setTimeout(resolve, 60));
}

// ВАЖНО (WebKit/GStreamer): createImageBitmap() из переиспользуемого canvas
// не гарантирует копию — первый кадр получается нормальным, а последующие
// портятся (серый шум). Поэтому каждый кадр сохраняется в собственный canvas.
function snapshotFrame(source: CanvasImageSource, width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  canvas.getContext('2d')!.drawImage(source, 0, 0, width, height);
  return canvas;
}

type RvfcVideo = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: (now: number, meta: { mediaTime: number }) => void) => number;
  cancelVideoFrameCallback?: (handle: number) => void;
};

// WebKit: после seek() drawImage(video) отдаёт «серый шум» вместо кадра,
// даже если readyState === 4 и событие seeked уже пришло. Надёжный способ —
// requestVideoFrameCallback: он вызывается на реально показанном кадре.
function captureFrames(
  video: HTMLVideoElement,
  targetCount: number,
  onFrame: (timeSec: number) => void,
  opts: { startSec?: number; endSec?: number; onProgress?: (percent: number) => void } = {}
): Promise<void> {
  const v = video as RvfcVideo;
  const startSec = opts.startSec ?? 0;
  const endSec = opts.endSec ?? Infinity;

  if (typeof v.requestVideoFrameCallback !== 'function') {
    return (async () => {
      for (let i = 0; i < targetCount; i++) {
        const t = startSec + (i / Math.max(1, targetCount - 1)) * Math.max(0, endSec - startSec);
        await seekVideo(v, t);
        onFrame(t);
        opts.onProgress?.(Math.round(((i + 1) / targetCount) * 100));
      }
      v.pause();
    })();
  }

  return (async () => {
    if (Math.abs(v.currentTime - startSec) > 0.01) await seekVideo(v, startSec);
    await v.play();
    await new Promise<void>((resolve, reject) => {
      let handle = 0;
      let done = false;
      const finish = () => {
        if (done) return;
        done = true;
        clearTimeout(timeout);
        v.cancelVideoFrameCallback?.(handle);
        v.pause();
        v.removeEventListener('error', onError);
        resolve();
      };
      const onError = () => {
        if (done) return;
        done = true;
        clearTimeout(timeout);
        reject(new Error('Ошибка декодирования видео'));
      };
      const timeout = setTimeout(finish, 60000);
      v.addEventListener('error', onError, { once: true });
      let count = 0;
      const step = (_now: number, meta: { mediaTime: number }) => {
        if (done) return;
        const t = Number.isFinite(meta.mediaTime) ? meta.mediaTime : count / 30;
        if (t < startSec - 0.05) {
          handle = v.requestVideoFrameCallback!(step);
          return;
        }
        if (t > endSec + 0.05) {
          finish();
          return;
        }
        onFrame(t);
        count++;
        opts.onProgress?.(Math.round((count / targetCount) * 100));
        if (count >= targetCount || v.ended) finish();
        else handle = v.requestVideoFrameCallback!(step);
      };
      handle = v.requestVideoFrameCallback!(step);
    });
  })();
}

// WebKit/GStreamer может отдать «пустой» кадр (вся альфа = 0), если декодер
// ещё не подготовил кадр после seek. Проверяем альфу, чтобы не показывать
// пользователю прозрачный превью-канвас вместо видео.
function isCanvasBlank(ctx: CanvasRenderingContext2D, width: number, height: number): boolean {
  const { data } = ctx.getImageData(0, 0, width, height);
  for (let i = 3; i < data.length; i += 4 * 37) {
    if (data[i] !== 0) return false;
  }
  return true;
}

export async function extractPreviewFrames(
  videoUrl: string,
  onProgress?: (percent: number) => void
): Promise<ExtractedPreviewData> {
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  video.crossOrigin = 'anonymous';
  // WebKit/GStreamer отдаёт кадры для drawImage только если <video> реально
  // присутствует в дереве отрисовки. Офскрин-элемент (left:-10000px) молча
  // даёт прозрачные кадры. Достаточно крошечного полупрозрачного элемента в углу.
  video.style.position = 'fixed';
  video.style.left = '0';
  video.style.top = '0';
  video.style.width = '320px';
  video.style.height = '180px';
  video.style.opacity = '0.01';
  video.style.pointerEvents = 'none';
  video.style.zIndex = '0';
  video.setAttribute('aria-hidden', 'true');
  document.body.appendChild(video);
  video.src = videoUrl;

  await waitForEvent(video, 'loadedmetadata');

  const sourceWidth = video.videoWidth || 640;
  const sourceHeight = video.videoHeight || 480;
  const duration = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 1.0;

  const maxSide = 480;
  const scale = Math.min(1, maxSide / Math.max(sourceWidth, sourceHeight));
  const width = ensureEven(sourceWidth * scale);
  const height = ensureEven(sourceHeight * scale);

  const previewFps = Math.max(10, Math.min(24, Math.floor(180 / Math.max(duration, 0.5))));
  const totalCount = Math.max(2, Math.floor(duration * previewFps));

  const previewCanvas = document.createElement('canvas');
  previewCanvas.width = width;
  previewCanvas.height = height;
  // ВАЖНО (WebKit/GStreamer): ни один canvas, куда рисуется видео или
  // ImageBitmap, не должен использовать willReadFrequently:true — в WebKit
  // drawImage() в такой контекст даёт испорченные кадры (серый шум вместо
  // изображения). Ниже все контексты создаются без этого флага.
  const previewCtx = previewCanvas.getContext('2d');
  if (!previewCtx) {
    throw new Error('Не удалось инициализировать 2D-контекст предпросмотра');
  }

  const analysisCanvas = document.createElement('canvas');
  analysisCanvas.width = ANALYSIS_DIM;
  analysisCanvas.height = ANALYSIS_DIM;
  const analysisCtx = analysisCanvas.getContext('2d');
  if (!analysisCtx) {
    throw new Error('Не удалось инициализировать 2D-контекст анализа');
  }

  const frames: HTMLCanvasElement[] = [];
  const analysisFrames: Uint8ClampedArray[] = [];
  const timestamps: number[] = [];

  try {
    if (video.readyState < 2) await waitForEvent(video, 'loadeddata');

    let blankFrames = 0;
    const captureFrame = (t: number) => {
      previewCtx.clearRect(0, 0, width, height);
      previewCtx.drawImage(video, 0, 0, width, height);
      if (isCanvasBlank(previewCtx, width, height)) blankFrames++;
      frames.push(snapshotFrame(previewCanvas, width, height));
      analysisCtx.clearRect(0, 0, ANALYSIS_DIM, ANALYSIS_DIM);
      analysisCtx.drawImage(previewCanvas, 0, 0, ANALYSIS_DIM, ANALYSIS_DIM);
      analysisFrames.push(analysisCtx.getImageData(0, 0, ANALYSIS_DIM, ANALYSIS_DIM).data);
      timestamps.push(t);
      onProgress?.(Math.round((frames.length / totalCount) * 100));
    };

    await captureFrames(video, totalCount, captureFrame, {
      endSec: Math.max(0, duration - 1 / previewFps),
      onProgress: (pct) => onProgress?.(pct),
    });

    if (blankFrames === totalCount) {
      throw new Error(
        'Не удалось извлечь кадры: WebKit/GStreamer не декодирует видео. ' +
          'Перезапустите приложение через nix-shell (в нём отключены аппаратные декодеры и подключены GStreamer-плагины).'
      );
    }
  } catch (err) {
    frames.length = 0;
    throw err;
  } finally {
    video.removeAttribute('src');
    video.load();
    video.remove();
  }

  return {
    frames,
    analysisFrames,
    timestamps,
    width,
    height,
    sourceWidth,
    sourceHeight,
    duration,
    previewFps,
    dispose: () => {
      frames.length = 0;
    },
  };
}

function meanAbsDiffNorm(a: Uint8ClampedArray, b: Uint8ClampedArray): number {
  if (a.length === 0 || a.length !== b.length) return 1;
  let sum = 0;
  for (let i = 0; i < a.length; i++) {
    sum += Math.abs(a[i] - b[i]);
  }
  return sum / (a.length * 255);
}

function motionDiffNorm(
  a0: Uint8ClampedArray,
  a1: Uint8ClampedArray,
  bPrev: Uint8ClampedArray,
  bCurr: Uint8ClampedArray
): number {
  const len = a0.length;
  if (len === 0 || a1.length !== len || bPrev.length !== len || bCurr.length !== len) return 1;
  let sum = 0;
  for (let i = 0; i < len; i++) {
    const da = a1[i] - a0[i];
    const db = bCurr[i] - bPrev[i];
    sum += Math.abs(da - db);
  }
  return Math.min(1, sum / (len * 255));
}

export function computeFrameSimilarity(a: Uint8ClampedArray, b: Uint8ClampedArray): number {
  const dist = meanAbsDiffNorm(a, b);
  return Math.max(0, Math.min(100, (1 - dist) * 100));
}

export function analyzeFramesInBrowser(
  analysisFrames: Uint8ClampedArray[],
  sampleFps: number,
  durationSec: number,
  minLoopSec?: number,
  maxCandidates = 5
): { originalSeamSimilarity: number; candidates: LoopCandidate[] } {
  if (analysisFrames.length < 2) {
    return { originalSeamSimilarity: 100, candidates: [] };
  }

  const originalSeamSimilarity = computeFrameSimilarity(
    analysisFrames[0],
    analysisFrames[analysisFrames.length - 1]
  );

  const minSec = Math.min(
    durationSec * 0.85,
    minLoopSec ?? Math.max(0.4, Math.min(1.2, durationSec * 0.35))
  );
  const minFrames = Math.min(
    analysisFrames.length - 1,
    Math.max(2, Math.round(minSec * sampleFps))
  );

  interface RawCand {
    startSec: number;
    endSec: number;
    combined: number;
    poseDist: number;
    motionDist: number;
  }

  const raw: RawCand[] = [];
  for (let i = 0; i < analysisFrames.length; i++) {
    const jStart = i + minFrames;
    if (jStart >= analysisFrames.length) break;
    for (let j = jStart; j < analysisFrames.length; j++) {
      const poseDist = meanAbsDiffNorm(analysisFrames[i], analysisFrames[j]);
      const motionDist =
        i + 1 < analysisFrames.length && j >= 1
          ? motionDiffNorm(
              analysisFrames[i],
              analysisFrames[i + 1],
              analysisFrames[j - 1],
              analysisFrames[j]
            )
          : poseDist;
      const combined = 0.78 * poseDist + 0.22 * motionDist;
      raw.push({
        startSec: i / sampleFps,
        endSec: j / sampleFps,
        combined,
        poseDist,
        motionDist,
      });
    }
  }

  raw.sort((a, b) => a.combined - b.combined);

  const kept: LoopCandidate[] = [];
  for (const cand of raw) {
    const overlaps = kept.some(
      (k) => Math.abs(cand.startSec - k.startSec) < 0.35 && Math.abs(cand.endSec - k.endSec) < 0.35
    );
    if (overlaps) continue;
    kept.push({
      startSec: Math.round(cand.startSec * 1000) / 1000,
      endSec: Math.round(cand.endSec * 1000) / 1000,
      durationSec: Math.round(Math.max(0, cand.endSec - cand.startSec) * 1000) / 1000,
      similarity: Math.max(0, Math.min(100, (1 - cand.combined) * 100)),
      poseSimilarity: Math.max(0, Math.min(100, (1 - cand.poseDist) * 100)),
      motionSimilarity: Math.max(0, Math.min(100, (1 - cand.motionDist) * 100)),
    });
    if (kept.length >= maxCandidates) break;
  }

  return { originalSeamSimilarity, candidates: kept };
}

export function buildLoopPlaybackPlan(
  totalFrames: number,
  fps: number,
  params: {
    mode: LoopMode;
    startSec: number;
    endSec: number;
    crossfadeSec: number;
    bridgeSec: number;
    exactEndFrame: boolean;
  }
): LoopPlaybackStep[] {
  if (totalFrames <= 0) return [];
  if (totalFrames === 1) {
    return [{ indexA: 0, indexB: 0, weightB: 0, isSeamZone: false, isBoundary: true }];
  }

  const startIdx = Math.max(0, Math.min(totalFrames - 2, Math.round(params.startSec * fps)));
  const endIdx = Math.max(startIdx + 1, Math.min(totalFrames - 1, Math.round(params.endSec * fps)));
  const segFrames = endIdx - startIdx + 1;

  const steps: LoopPlaybackStep[] = [];

  const appendCrossfade = () => {
    let fadeFrames = Math.max(1, Math.min(Math.floor(segFrames * 0.45), Math.round(params.crossfadeSec * fps)));
    if (segFrames <= 2 * fadeFrames + 1) {
      fadeFrames = Math.max(1, Math.floor((segFrames - 1) / 2));
    }
    const bodyStart = startIdx + fadeFrames;
    const bodyEnd = endIdx - fadeFrames;

    for (let idx = bodyStart; idx <= bodyEnd; idx++) {
      steps.push({
        indexA: idx,
        indexB: idx,
        weightB: 0,
        isSeamZone: false,
        isBoundary: false,
      });
    }

    for (let k = 0; k < fadeFrames; k++) {
      const idxA = Math.min(endIdx, bodyEnd + 1 + k);
      const idxB = Math.min(endIdx, startIdx + k);
      steps.push({
        indexA: idxA,
        indexB: idxB,
        weightB: (k + 1) / (fadeFrames + 1),
        isSeamZone: true,
        isBoundary: !params.exactEndFrame && k === fadeFrames - 1,
      });
    }

    if (params.exactEndFrame) {
      steps.push({
        indexA: bodyStart,
        indexB: bodyStart,
        weightB: 0,
        isSeamZone: true,
        isBoundary: true,
      });
    }
  };

  if (params.mode === 'crossfade') {
    appendCrossfade();
  } else if (params.mode === 'smart_cut') {
    if (params.crossfadeSec > 0.02 && segFrames >= 4) {
      appendCrossfade();
    } else {
      for (let idx = startIdx; idx <= endIdx; idx++) {
        steps.push({
          indexA: idx,
          indexB: idx,
          weightB: 0,
          isSeamZone: idx === startIdx || idx === endIdx,
          isBoundary: !params.exactEndFrame && idx === endIdx,
        });
      }
      if (params.exactEndFrame) {
        steps.push({
          indexA: startIdx,
          indexB: startIdx,
          weightB: 0,
          isSeamZone: true,
          isBoundary: true,
        });
      }
    }
  } else if (params.mode === 'ping_pong') {
    for (let idx = startIdx; idx <= endIdx; idx++) {
      steps.push({
        indexA: idx,
        indexB: idx,
        weightB: 0,
        isSeamZone: idx === endIdx,
        isBoundary: false,
      });
    }
    const revStop = params.exactEndFrame ? startIdx : startIdx + 1;
    for (let idx = endIdx - 1; idx >= revStop; idx--) {
      steps.push({
        indexA: idx,
        indexB: idx,
        weightB: 0,
        isSeamZone: idx <= startIdx + 2,
        isBoundary: idx === revStop,
      });
    }
    if (steps.length > 0) {
      steps[steps.length - 1].isBoundary = true;
    }
  } else {
    // morph
    for (let idx = startIdx; idx <= endIdx; idx++) {
      steps.push({
        indexA: idx,
        indexB: idx,
        weightB: 0,
        isSeamZone: false,
        isBoundary: false,
      });
    }
    const bridgeFrames = Math.max(2, Math.round(params.bridgeSec * fps));
    const maxK = params.exactEndFrame ? bridgeFrames : bridgeFrames - 1;
    for (let k = 1; k <= maxK; k++) {
      const t = k / bridgeFrames;
      const eased = t * t * (3 - 2 * t);
      steps.push({
        indexA: endIdx,
        indexB: startIdx,
        weightB: eased,
        isSeamZone: true,
        isBoundary: k === maxK,
      });
    }
  }

  return steps;
}

export function renderBlendedFrame(
  ctx: CanvasRenderingContext2D,
  frameA: CanvasImageSource,
  frameB: CanvasImageSource,
  weightB: number,
  width: number,
  height: number,
  scratchA: CanvasRenderingContext2D,
  scratchB: CanvasRenderingContext2D
): void {
  if (weightB <= 0.001) {
    ctx.clearRect(0, 0, width, height);
    ctx.drawImage(frameA, 0, 0, width, height);
    return;
  }
  if (weightB >= 0.999) {
    ctx.clearRect(0, 0, width, height);
    ctx.drawImage(frameB, 0, 0, width, height);
    return;
  }

  scratchA.clearRect(0, 0, width, height);
  scratchA.drawImage(frameA, 0, 0, width, height);
  const dataA = scratchA.getImageData(0, 0, width, height);

  scratchB.clearRect(0, 0, width, height);
  scratchB.drawImage(frameB, 0, 0, width, height);
  const dataB = scratchB.getImageData(0, 0, width, height);

  const out = scratchA.createImageData(width, height);
  const pA = dataA.data;
  const pB = dataB.data;
  const pOut = out.data;
  const wB = weightB;
  const wA = 1 - wB;

  for (let i = 0; i < pA.length; i++) {
    pOut[i] = Math.round(pA[i] * wA + pB[i] * wB);
  }

  ctx.putImageData(out, 0, 0);
}

export function renderSeamComparison(
  canvas: HTMLCanvasElement,
  firstSource: CanvasImageSource,
  lastSource: CanvasImageSource,
  viewMode: 'onion' | 'split' | 'diff',
  overlayOpacity: number
): number {
  const w = canvas.width;
  const h = canvas.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) return 0;

  const c1 = document.createElement('canvas');
  c1.width = w;
  c1.height = h;
  const ctx1 = c1.getContext('2d')!;
  ctx1.drawImage(firstSource, 0, 0, w, h);
  const d1 = ctx1.getImageData(0, 0, w, h).data;

  const c2 = document.createElement('canvas');
  c2.width = w;
  c2.height = h;
  const ctx2 = c2.getContext('2d')!;
  ctx2.drawImage(lastSource, 0, 0, w, h);
  const d2 = ctx2.getImageData(0, 0, w, h).data;

  const similarity = computeFrameSimilarity(d1, d2);

  ctx.clearRect(0, 0, w, h);

  if (viewMode === 'onion') {
    ctx.globalAlpha = 1;
    ctx.drawImage(c1, 0, 0);
    ctx.globalAlpha = Math.max(0, Math.min(1, overlayOpacity));
    ctx.drawImage(c2, 0, 0);
    ctx.globalAlpha = 1;
  } else if (viewMode === 'split') {
    const mid = Math.floor(w / 2);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, mid, h);
    ctx.clip();
    ctx.drawImage(c1, 0, 0);
    ctx.restore();

    ctx.save();
    ctx.beginPath();
    ctx.rect(mid, 0, w - mid, h);
    ctx.clip();
    ctx.drawImage(c2, 0, 0);
    ctx.restore();

    ctx.strokeStyle = '#10b981';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(mid, 0);
    ctx.lineTo(mid, h);
    ctx.stroke();

    ctx.font = 'bold 11px monospace';
    ctx.fillStyle = 'rgba(15, 23, 42, 0.8)';
    ctx.fillRect(8, 8, 72, 20);
    ctx.fillRect(w - 94, 8, 86, 20);
    ctx.fillStyle = '#34d399';
    ctx.fillText('1-Й КАДР', 14, 22);
    ctx.fillStyle = '#fbbf24';
    ctx.fillText('ПОСЛЕДНИЙ', w - 88, 22);
  } else {
    const out = ctx.createImageData(w, h);
    const p = out.data;
    for (let i = 0; i < d1.length; i += 4) {
      const dr = Math.abs(d1[i] - d2[i]);
      const dg = Math.abs(d1[i + 1] - d2[i + 1]);
      const db = Math.abs(d1[i + 2] - d2[i + 2]);
      const da = Math.abs(d1[i + 3] - d2[i + 3]);
      const maxDiff = Math.max(dr, dg, db, da);
      if (maxDiff < 4) {
        const luma = Math.round((d1[i] + d1[i + 1] + d1[i + 2]) * 0.08);
        const alpha = Math.round(Math.max(d1[i + 3], d2[i + 3]) * 0.35);
        p[i] = luma;
        p[i + 1] = luma + 6;
        p[i + 2] = luma + 10;
        p[i + 3] = alpha;
      } else {
        const heat = Math.min(255, maxDiff * 3);
        p[i] = 255;
        p[i + 1] = Math.max(40, 220 - heat);
        p[i + 2] = Math.min(220, heat);
        p[i + 3] = 235;
      }
    }
    ctx.putImageData(out, 0, 0);
  }

  return similarity;
}

function parseResolutionDims(
  resolution: string | null | undefined,
  srcW: number,
  srcH: number
): { width: number; height: number } {
  if (!resolution || resolution === 'source') {
    return { width: ensureEven(srcW), height: ensureEven(srcH) };
  }
  const [rwStr, rhStr] = resolution.split('x');
  const maxW = Number(rwStr) || srcW;
  const maxH = Number(rhStr) || srcH;
  const scale = Math.min(1, maxW / srcW, maxH / srcH);
  return {
    width: ensureEven(srcW * scale),
    height: ensureEven(srcH * scale),
  };
}

export async function exportLoopInBrowser(
  videoUrl: string,
  params: {
    mode: LoopMode;
    startSec: number;
    endSec: number;
    crossfadeSec: number;
    bridgeSec: number;
    exactEndFrame: boolean;
    resolution?: string | null;
    targetFps?: number | null;
    chromaKey?: ChromaKeyConfig | null;
  },
  onProgress?: (percent: number, message: string) => void
): Promise<BrowserExportResult> {
  onProgress?.(5, 'Декодирование исходных кадров...');

  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  video.crossOrigin = 'anonymous';
  video.src = videoUrl;
  await waitForEvent(video, 'loadedmetadata');

  const srcW = video.videoWidth || 640;
  const srcH = video.videoHeight || 480;
  const duration = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 1.0;
  const { width, height } = parseResolutionDims(params.resolution, srcW, srcH);
  const fps = Math.max(10, Math.min(60, params.targetFps || 30));

  const totalSourceFrames = Math.max(2, Math.floor(duration * fps));
  const rawCanvas = document.createElement('canvas');
  rawCanvas.width = width;
  rawCanvas.height = height;
  const rawCtx = rawCanvas.getContext('2d')!;

  const bitmaps: HTMLCanvasElement[] = [];
  const startIdx = Math.max(0, Math.min(totalSourceFrames - 2, Math.round(params.startSec * fps)));
  const endIdx = Math.max(startIdx + 1, Math.min(totalSourceFrames - 1, Math.round(params.endSec * fps)));

  try {
    await captureFrames(
      video,
      endIdx - startIdx + 1,
      () => {
        rawCtx.clearRect(0, 0, width, height);
        rawCtx.drawImage(video, 0, 0, width, height);
        if (params.chromaKey?.enabled) {
          const img = rawCtx.getImageData(0, 0, width, height);
          applyChromaKeyInPlace(img.data, params.chromaKey);
          rawCtx.putImageData(img, 0, 0);
        }
        bitmaps.push(snapshotFrame(rawCanvas, width, height));
      },
      {
        startSec: startIdx / fps,
        endSec: endIdx / fps,
        onProgress: (pct) => onProgress?.(8 + pct * 0.35, `Подготовка кадров (${pct}%)...`),
      }
    );

    const plan = buildLoopPlaybackPlan(bitmaps.length, fps, {
      mode: params.mode,
      startSec: 0,
      endSec: (bitmaps.length - 1) / fps,
      crossfadeSec: params.crossfadeSec,
      bridgeSec: params.bridgeSec,
      exactEndFrame: params.exactEndFrame,
    });

    const outCanvas = document.createElement('canvas');
    outCanvas.width = width;
    outCanvas.height = height;
    const outCtx = outCanvas.getContext('2d')!;

    const scratchA = document.createElement('canvas');
    scratchA.width = width;
    scratchA.height = height;
    const scratchACtx = scratchA.getContext('2d')!;

    const scratchB = document.createElement('canvas');
    scratchB.width = width;
    scratchB.height = height;
    const scratchBCtx = scratchB.getContext('2d')!;

    // Compute real first-vs-last seam similarity for both original trim and processed plan.
    renderBlendedFrame(
      outCtx,
      bitmaps[0],
      bitmaps[0],
      0,
      width,
      height,
      scratchACtx,
      scratchBCtx
    );
    const origFirst = outCtx.getImageData(0, 0, width, height).data;
    renderBlendedFrame(
      outCtx,
      bitmaps[bitmaps.length - 1],
      bitmaps[bitmaps.length - 1],
      0,
      width,
      height,
      scratchACtx,
      scratchBCtx
    );
    const origLast = outCtx.getImageData(0, 0, width, height).data;
    const originalSeamSimilarity = computeFrameSimilarity(origFirst, origLast);

    const firstStep = plan[0];
    const lastStep = plan[plan.length - 1];
    renderBlendedFrame(
      outCtx,
      bitmaps[firstStep.indexA],
      bitmaps[firstStep.indexB],
      firstStep.weightB,
      width,
      height,
      scratchACtx,
      scratchBCtx
    );
    const procFirst = outCtx.getImageData(0, 0, width, height).data;
    renderBlendedFrame(
      outCtx,
      bitmaps[lastStep.indexA],
      bitmaps[lastStep.indexB],
      lastStep.weightB,
      width,
      height,
      scratchACtx,
      scratchBCtx
    );
    const procLast = outCtx.getImageData(0, 0, width, height).data;
    const outputSeamSimilarity = computeFrameSimilarity(procFirst, procLast);

    const mimeType = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'].find((m) =>
      MediaRecorder.isTypeSupported(m)
    );
    if (!mimeType) {
      throw new Error('Браузер не поддерживает запись WebM через MediaRecorder');
    }

    const stream = outCanvas.captureStream(fps);
    const recorder = new MediaRecorder(stream, {
      mimeType,
      videoBitsPerSecond: 4_000_000,
    });
    const chunks: Blob[] = [];
    recorder.ondataavailable = (ev) => {
      if (ev.data.size > 0) chunks.push(ev.data);
    };

    const stopped = new Promise<void>((resolve) => {
      recorder.onstop = () => resolve();
    });

    recorder.start();
    const frameIntervalMs = 1000 / fps;
    for (let i = 0; i < plan.length; i++) {
      const step = plan[i];
      renderBlendedFrame(
        outCtx,
        bitmaps[step.indexA],
        bitmaps[step.indexB],
        step.weightB,
        width,
        height,
        scratchACtx,
        scratchBCtx
      );
      onProgress?.(45 + ((i + 1) / plan.length) * 52, `Кодирование WebM (${i + 1}/${plan.length})...`);
      await new Promise((r) => setTimeout(r, frameIntervalMs));
    }

    recorder.stop();
    await stopped;

    const blob = new Blob(chunks, { type: mimeType });
    const url = URL.createObjectURL(blob);
    onProgress?.(100, 'Готово!');

    return {
      blob,
      url,
      durationSec: plan.length / fps,
      frameCount: plan.length,
      fileSizeBytes: blob.size,
      originalSeamSimilarity,
      outputSeamSimilarity,
    };
  } finally {
    bitmaps.length = 0;
    video.removeAttribute('src');
    video.load();
  }
}

export interface BrowserOptimizeResult {
  blob: Blob;
  url: string;
  durationSec: number;
  frameCount: number;
  fileSizeBytes: number;
  width: number;
  height: number;
}

/**
 * Резервный путь конвертации без Tauri: снимает кадры с видео через canvas,
 * применяет хромакей и записывает WebM через MediaRecorder.
 * Внимание: MediaRecorder большинства браузеров не кодирует альфу в WebM —
 * для настоящего yuva420p используйте десктопное приложение (FFmpeg).
 */
export async function exportChromaKeyedWebmInBrowser(
  videoUrl: string,
  params: {
    resolution?: string | null;
    chromaKey: ChromaKeyConfig;
    targetFps?: number | null;
  },
  onProgress?: (percent: number, message: string) => void
): Promise<BrowserOptimizeResult> {
  onProgress?.(3, 'Загрузка видео...');
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  video.src = videoUrl;
  await waitForEvent(video, 'loadedmetadata');

  const srcW = video.videoWidth || 640;
  const srcH = video.videoHeight || 480;
  const duration = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 1;
  const { width, height } = resolveResolutionDims(params.resolution, srcW, srcH);
  const fps = Math.max(10, Math.min(30, params.targetFps || 24));
  const totalFrames = Math.min(1200, Math.max(2, Math.ceil(duration * fps)));

  const rawCanvas = document.createElement('canvas');
  rawCanvas.width = width;
  rawCanvas.height = height;
  const rawCtx = rawCanvas.getContext('2d')!;

  const frames: HTMLCanvasElement[] = [];
  try {
    await captureFrames(
      video,
      totalFrames,
      () => {
        rawCtx.clearRect(0, 0, width, height);
        rawCtx.drawImage(video, 0, 0, width, height);
        if (params.chromaKey.enabled) {
          const img = rawCtx.getImageData(0, 0, width, height);
          applyChromaKeyInPlace(img.data, params.chromaKey);
          rawCtx.putImageData(img, 0, 0);
        }
        frames.push(snapshotFrame(rawCanvas, width, height));
      },
      {
        onProgress: (pct) => onProgress?.(5 + pct * 0.45, `Извлечение кадров (${pct}%)...`),
      }
    );

    onProgress?.(52, 'Подготовка кодировщика WebM...');
    const mimeType = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm'].find((m) =>
      MediaRecorder.isTypeSupported(m)
    );
    if (!mimeType) {
      throw new Error('Браузер не поддерживает запись WebM через MediaRecorder');
    }

    const outCanvas = document.createElement('canvas');
    outCanvas.width = width;
    outCanvas.height = height;
    const outCtx = outCanvas.getContext('2d')!;
    const stream = outCanvas.captureStream(fps);
    const recorder = new MediaRecorder(stream, { mimeType, videoBitsPerSecond: 4_000_000 });
    const chunks: Blob[] = [];
    recorder.ondataavailable = (ev) => {
      if (ev.data.size > 0) chunks.push(ev.data);
    };
    const stopped = new Promise<void>((resolve) => {
      recorder.onstop = () => resolve();
    });

    recorder.start();
    const frameMs = 1000 / fps;
    for (let i = 0; i < frames.length; i++) {
      outCtx.clearRect(0, 0, width, height);
      outCtx.drawImage(frames[i], 0, 0);
      onProgress?.(53 + ((i + 1) / frames.length) * 45, `Кодирование WebM (${i + 1}/${frames.length})...`);
      await new Promise((r) => setTimeout(r, frameMs));
    }
    recorder.stop();
    await stopped;
    stream.getTracks().forEach((track) => track.stop());

    const blob = new Blob(chunks, { type: mimeType });
    onProgress?.(100, 'Готово!');
    return {
      blob,
      url: URL.createObjectURL(blob),
      durationSec: frames.length / fps,
      frameCount: frames.length,
      fileSizeBytes: blob.size,
      width,
      height,
    };
  } finally {
    frames.length = 0;
    video.removeAttribute('src');
    video.load();
  }
}
