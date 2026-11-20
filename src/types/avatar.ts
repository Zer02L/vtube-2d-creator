export type VideoResolution = '480p' | '720p' | '1080p' | '2k';

export interface ChromaKeyConfig {
  enabled: boolean;
  colorHex: string; // e.g. '#00FF00'
  similarity: number; // 0.0 - 1.0
  blend: number; // 0.0 - 1.0
}

export interface VideoSlot {
  id: string;
  name: string;
  originalPath?: string;
  optimizedWebmPath: string;
  duration?: number;
  isLoop: boolean;
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
