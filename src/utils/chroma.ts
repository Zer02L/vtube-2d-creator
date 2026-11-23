import type { ChromaKeyConfig } from '../types/avatar';

function ensureEven(n: number): number {
  const rounded = Math.max(2, Math.round(n));
  return rounded % 2 === 0 ? rounded : rounded - 1;
}

/** Разбирает '#RRGGBB' / 'RRGGBB' / '0xRRGGBB' в RGB-компоненты. */
export function hexToRgb(hex: string): [number, number, number] {
  const clean = hex.trim().replace(/^#/, '').replace(/^0x/i, '');
  const full = clean.length === 3 ? clean.split('').map((c) => c + c).join('') : clean;
  const r = parseInt(full.slice(0, 2), 16);
  const g = parseInt(full.slice(2, 4), 16);
  const b = parseInt(full.slice(4, 6), 16);
  return [
    Number.isFinite(r) ? r : 0,
    Number.isFinite(g) ? g : 255,
    Number.isFinite(b) ? b : 0,
  ];
}

/**
 * Удаляет цвет фона «на месте» в RGBA-буфере canvas.
 * similarity — жёсткий радиус (пиксели внутри становятся полностью прозрачными),
 * blend — мягкая зона на границе (плавное сглаживание контура).
 */
export function applyChromaKeyInPlace(data: Uint8ClampedArray, ck: ChromaKeyConfig) {
  const [kr, kg, kb] = hexToRgb(ck.colorHex);
  const simDist = ck.similarity * 442; // максимальное евклидово расстояние в RGB ~441.67
  const blendDist = ck.blend * 442;

  for (let i = 0; i < data.length; i += 4) {
    const dr = data[i] - kr;
    const dg = data[i + 1] - kg;
    const db = data[i + 2] - kb;
    const dist = Math.sqrt(dr * dr + dg * dg + db * db);
    if (dist <= simDist) {
      data[i + 3] = 0;
    } else if (blendDist > 0 && dist < simDist + blendDist) {
      const factor = (dist - simDist) / blendDist;
      data[i + 3] = Math.round(data[i + 3] * factor);
    }
  }
}

/** Превращает пресет разрешения в чётные размеры, вписывающиеся в источник. */
export function resolveResolutionDims(
  resolution: string | null | undefined,
  srcW: number,
  srcH: number
): { width: number; height: number } {
  const baseW = Math.max(2, srcW || 640);
  const baseH = Math.max(2, srcH || 480);
  if (!resolution || resolution === 'source') {
    return { width: ensureEven(baseW), height: ensureEven(baseH) };
  }

  const presets: Record<string, [number, number]> = {
    '480p': [854, 480],
    '720p': [1280, 720],
    '1080p': [1920, 1080],
    '2k': [2560, 1440],
  };
  let maxW = baseW;
  let maxH = baseH;
  const preset = presets[resolution];
  if (preset) {
    [maxW, maxH] = preset;
  } else if (/^\d+x\d+$/i.test(resolution)) {
    const [w, h] = resolution.toLowerCase().split('x').map(Number);
    maxW = w || baseW;
    maxH = h || baseH;
  }

  const scale = Math.min(1, maxW / baseW, maxH / baseH);
  return { width: ensureEven(baseW * scale), height: ensureEven(baseH * scale) };
}

export interface ChromaPreviewFrames {
  frames: HTMLCanvasElement[];
  width: number;
  height: number;
  durationSec: number;
  /** Частота кадров предпросмотра: столько кадров приходится на секунду видео. */
  fps: number;
  dispose: () => void;
}

type RvfcVideo = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: (now: number, meta: { mediaTime: number }) => void) => number;
  cancelVideoFrameCallback?: (handle: number) => void;
};

function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err) => {
        clearTimeout(timer);
        reject(err);
      }
    );
  });
}

function waitForMeta(video: HTMLVideoElement): Promise<void> {
  return new Promise((resolve, reject) => {
    const ok = () => {
      cleanup();
      resolve();
    };
    const fail = () => {
      cleanup();
      reject(new Error('Не удалось загрузить метаданные видео'));
    };
    const cleanup = () => {
      video.removeEventListener('loadedmetadata', ok);
      video.removeEventListener('error', fail);
    };
    video.addEventListener('loadedmetadata', ok, { once: true });
    video.addEventListener('error', fail, { once: true });
  });
}

function seekTo(video: HTMLVideoElement, timeSec: number): Promise<void> {
  if (Math.abs(video.currentTime - timeSec) < 0.002 && video.readyState >= 2) {
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const ok = () => {
      cleanup();
      resolve();
    };
    const fail = () => {
      cleanup();
      reject(new Error('Ошибка перемотки видео'));
    };
    const cleanup = () => {
      clearTimeout(timer);
      video.removeEventListener('seeked', ok);
      video.removeEventListener('error', fail);
    };
    const timer = setTimeout(() => {
      cleanup();
      resolve(); // последний доступный кадр всё равно пробуем нарисовать
    }, 2500);
    video.addEventListener('seeked', ok, { once: true });
    video.addEventListener('error', fail, { once: true });
    video.currentTime = timeSec;
  });
}

function waitForPresentedFrame(video: RvfcVideo): Promise<void> {
  if (typeof video.requestVideoFrameCallback !== 'function') {
    return new Promise((resolve) => setTimeout(resolve, 60));
  }
  return new Promise((resolve) => {
    let handle = 0;
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      video.cancelVideoFrameCallback?.(handle);
      resolve();
    };
    // В WebKit колбэк после seek иногда не приходит — не блокируем извлечение.
    const timer = setTimeout(finish, 180);
    handle = video.requestVideoFrameCallback!(finish);
  });
}

/**
 * Извлекает небольшой набор кадров один раз, чтобы предпросмотр хромакея
 * не зависел от живого видеопотока. Прогресс идёт по числу обработанных
 * меток времени, а не по real-time воспроизведению: WebKit/GStreamer
 * иногда перестаёт отдавать кадры при playbackRate > 1 и прогресс застывал.
 */
export async function extractChromaPreviewFrames(
  videoUrl: string,
  opts: {
    maxFrames?: number;
    maxSide?: number;
    onProgress?: (percent: number, message: string) => void;
  } = {}
): Promise<ChromaPreviewFrames> {
  const maxFrames = Math.max(8, opts.maxFrames ?? 64);
  const maxSide = opts.maxSide ?? 360;

  const video = document.createElement('video') as RvfcVideo;
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  // WebKit отдаёт кадры только для элемента, реально присутствующего в DOM.
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

  const frames: HTMLCanvasElement[] = [];
  try {
    opts.onProgress?.(2, 'Загрузка видео...');
    await withTimeout(waitForMeta(video), 15000, 'Таймаут загрузки видео');

    const vw = video.videoWidth || 640;
    const vh = video.videoHeight || 480;
    const duration = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 1;
    const scale = Math.min(1, maxSide / Math.max(vw, vh));
    const width = ensureEven(vw * scale);
    const height = ensureEven(vh * scale);

    const scratch = document.createElement('canvas');
    scratch.width = width;
    scratch.height = height;
    const ctx = scratch.getContext('2d');
    if (!ctx) throw new Error('Не удалось создать 2D-контекст предпросмотра');

    // Небольшой прогрев декодера без ожидания реального времени.
    try {
      await video.play();
      video.pause();
    } catch {
      // muted-видео обычно запускается без ограничений.
    }

    let failures = 0;
    for (let i = 0; i < maxFrames; i++) {
      const targetTime = (i / (maxFrames - 1)) * Math.max(0, duration - 0.05);
      try {
        await seekTo(video, targetTime);
        await waitForPresentedFrame(video);
        ctx.clearRect(0, 0, width, height);
        ctx.drawImage(video, 0, 0, width, height);

        const frame = document.createElement('canvas');
        frame.width = width;
        frame.height = height;
        frame.getContext('2d')!.drawImage(scratch, 0, 0);
        frames.push(frame);
        failures = 0;
      } catch {
        failures++;
        if (failures >= 5) {
          throw new Error(
            'Не удалось извлечь кадры: WebKit/GStreamer не декодирует видео. ' +
              'Перезапустите приложение через nix-shell.'
          );
        }
      }
      opts.onProgress?.(
        Math.round(((i + 1) / maxFrames) * 100),
        `Индексация кадров (${frames.length}/${maxFrames})...`
      );
    }

    if (frames.length === 0) {
      throw new Error('Не удалось извлечь ни одного кадра предпросмотра');
    }
    opts.onProgress?.(100, 'Кадры предпросмотра готовы');

    return {
      frames,
      width,
      height,
      durationSec: duration,
      fps: frames.length / duration,
      dispose: () => {
        frames.length = 0;
      },
    };
  } catch (err) {
    frames.length = 0;
    throw err;
  } finally {
    video.pause();
    video.removeAttribute('src');
    video.load();
    video.remove();
  }
}
