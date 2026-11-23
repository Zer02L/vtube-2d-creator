import { invoke } from '@tauri-apps/api/core';
import type { PreviewSheet } from '../types/avatar';

export interface PreviewFrameSet {
  frames: HTMLCanvasElement[];
  width: number;
  height: number;
  fps: number;
  durationSec: number;
}

/**
 * Кадры для UI-превью снимает FFmpeg, а не WebKit/GStreamer:
 * GStreamer-демuxer WebM с альфой падает с assertion в matroska-demux.c.
 */
export async function loadNativePreviewFrames(
  path: string,
  maxFrames = 64,
  onProgress?: (percent: number, message: string) => void
): Promise<PreviewFrameSet> {
  onProgress?.(40, 'FFmpeg извлекает кадры...');
  const sheet = await invoke<PreviewSheet>('extract_preview_sheet', {
    inputPath: path,
    maxFrames,
  });
  onProgress?.(75, 'Разбор спрайтлиста...');

  const bytes = await invoke<ArrayBuffer>('read_video_bytes', { path: sheet.path });
  const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
  try {
    const image = new Image();
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error('Не удалось прочитать спрайтлист кадров'));
      image.src = url;
    });

    const frames: HTMLCanvasElement[] = [];
    for (let i = 0; i < sheet.frameCount; i++) {
      const frame = document.createElement('canvas');
      frame.width = sheet.tileWidth;
      frame.height = sheet.tileHeight;
      frame
        .getContext('2d')!
        .drawImage(
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
    onProgress?.(100, 'Кадры готовы');
    return {
      frames,
      width: sheet.tileWidth,
      height: sheet.tileHeight,
      fps: frames.length / sheet.durationSec,
      durationSec: sheet.durationSec,
    };
  } finally {
    URL.revokeObjectURL(url);
  }
}
