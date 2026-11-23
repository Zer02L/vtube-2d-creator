export type VideoResolution = '480p' | '720p' | '1080p' | '2k';

export type LoopMode = 'crossfade' | 'ping_pong' | 'morph' | 'smart_cut';

export type OutputFormat = 'webm' | 'mp4' | 'gif' | 'mov';

export interface ChromaKeyConfig {
  enabled: boolean;
  colorHex: string; // e.g. '#00FF00'
  similarity: number; // 0.0 - 1.0
  blend: number; // 0.0 - 1.0
}

export interface VideoProbeInfo {
  path: string;
  width: number;
  height: number;
  fps: number;
  durationSec: number;
  frameCount: number;
  codecName: string;
  pixFmt: string;
  hasAlpha: boolean;
  fileSizeBytes: number;
}

export interface LoopCandidate {
  startSec: number;
  endSec: number;
  durationSec: number;
  similarity: number;
  poseSimilarity: number;
  motionSimilarity: number;
}

export interface LoopAnalysisResult {
  probe: VideoProbeInfo;
  originalSeamSimilarity: number;
  sampledFps: number;
  sampledFrames: number;
  candidates: LoopCandidate[];
}

export interface LoopProcessOptions {
  inputPath: string;
  outputPath?: string | null;
  mode: LoopMode;
  startSec?: number | null;
  endSec?: number | null;
  crossfadeSec: number;
  bridgeSec: number;
  exactEndFrame: boolean;
  format: OutputFormat;
  resolution?: string | null;
  targetFps?: number | null;
  chromaKey?: {
    color_hex: string;
    similarity: number;
    blend: number;
  } | null;
}

export interface LoopProgressEvent {
  stage: string;
  percent: number;
  message: string;
}

export interface LoopProcessResult {
  outputPath: string;
  durationSec: number;
  frameCount: number;
  fileSizeBytes: number;
  originalSeamSimilarity: number;
  outputSeamSimilarity: number;
  ffmpegArgs: string[];
}

export interface PreviewSheet {
  path: string;
  columns: number;
  rows: number;
  frameCount: number;
  tileWidth: number;
  tileHeight: number;
  durationSec: number;
}

export interface TranscodeOptions {
  inputPath: string;
  outputPath: string;
  resolution?: string | null;
  chromaKey?: {
    color_hex: string;
    similarity: number;
    blend: number;
  } | null;
}

export interface OptimizeProgressEvent {
  stage: string;
  percent: number;
  message: string;
}

export interface TranscodeResult {
  outputPath: string;
  durationSec: number;
  frameCount: number;
  fileSizeBytes: number;
  width: number;
  height: number;
  hasAlpha: boolean;
  ffmpegArgs: string[];
}

export interface VideoSlot {
  id: string;
  name: string;
  originalPath?: string;
  optimizedWebmPath: string;
  duration?: number;
  isLoop: boolean;
  /** Кадры для UI-превью (FFmpeg), вместо <video> с alpha-WebM. */
  previewFrames?: HTMLCanvasElement[];
  previewFps?: number;
}

export interface EmotionProfile {
  id: string;
  name: string;
  hotkey: string;
  idleVideoId: string;
  talkVideoId: string;
}

export interface OneShotReaction {
  id: string;
  name: string;
  hotkey: string;
  videoId: string;
  priority: number;
}

export interface AudioVADConfig {
  inputDeviceId: string;
  threshold: number; // 0.0 - 1.0 (RMS power)
  attackMs: number; // Delay to trigger talk state
  holdMs: number; // Decay/hold delay after silence
}

export interface AvatarManifest {
  version: string;
  name: string;
  author?: string;
  resolution: VideoResolution;
  vad: AudioVADConfig;
  defaultIdleVideoId: string;
  defaultTalkVideoId: string;
  videos: Record<string, VideoSlot>;
  emotions: EmotionProfile[];
  oneShots: OneShotReaction[];
}
