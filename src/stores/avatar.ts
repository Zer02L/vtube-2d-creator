import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import type { AvatarManifest } from '../types/avatar';

export const useAvatarStore = defineStore('avatar', () => {
  // Current loaded avatar manifest
  const manifest = ref<AvatarManifest>({
    version: '1.0.0',
    name: 'Default Avatar',
    resolution: '1080p',
    vad: {
      inputDeviceId: 'default',
      threshold: 0.15,
      attackMs: 50,
      holdMs: 300,
    },
    defaultIdleVideoId: 'idle_default',
    defaultTalkVideoId: 'talk_default',
    videos: {},
    emotions: [],
    oneShots: [],
  });

  // Runtime State Machine
  const isSpeaking = ref(false);
  const currentEmotionId = ref<string | null>(null);
  const activeOneShotId = ref<string | null>(null);

  // Audio measurement
  const currentVolume = ref(0); // 0.0 to 1.0

  // Server & OBS status
  const serverPort = ref(12345);
  const isServerRunning = ref(false);
  const isOverlayOpen = ref(false);

  // Active video URL resolver
  const activeVideoUrl = computed(() => {
    // 1. One-shot reaction has highest priority
    if (activeOneShotId.value) {
      const oneShot = manifest.value.oneShots.find(o => o.id === activeOneShotId.value);
      if (oneShot && manifest.value.videos[oneShot.videoId]) {
        return manifest.value.videos[oneShot.videoId].optimizedWebmPath;
      }
    }

    // 2. Emotion state
    if (currentEmotionId.value) {
      const emotion = manifest.value.emotions.find(e => e.id === currentEmotionId.value);
      if (emotion) {
        const vidId = isSpeaking.value ? emotion.talkVideoId : emotion.idleVideoId;
        if (manifest.value.videos[vidId]) {
          return manifest.value.videos[vidId].optimizedWebmPath;
        }
      }
    }

    // 3. Default Idle / Talk state
    const defaultId = isSpeaking.value ? manifest.value.defaultTalkVideoId : manifest.value.defaultIdleVideoId;
    return manifest.value.videos[defaultId]?.optimizedWebmPath || '';
  });

  function setSpeaking(speaking: boolean) {
    isSpeaking.value = speaking;
  }

  function setVolume(vol: number) {
    currentVolume.value = vol;
    // Simple VAD logic with threshold
    if (vol > manifest.value.vad.threshold && !isSpeaking.value) {
      setSpeaking(true);
    } else if (vol <= manifest.value.vad.threshold && isSpeaking.value) {
      setSpeaking(false);
    }
  }

  function triggerEmotion(emotionId: string | null) {
    if (currentEmotionId.value === emotionId) {
      currentEmotionId.value = null; // Toggle off back to default
    } else {
      currentEmotionId.value = emotionId;
    }
  }

  function triggerOneShot(oneShotId: string) {
    activeOneShotId.value = oneShotId;
  }

  function clearOneShot() {
    activeOneShotId.value = null;
  }

  return {
    manifest,
    isSpeaking,
    currentEmotionId,
    activeOneShotId,
    currentVolume,
    serverPort,
    isServerRunning,
    isOverlayOpen,
    activeVideoUrl,
    setSpeaking,
    setVolume,
    triggerEmotion,
    triggerOneShot,
    clearOneShot,
  };
});
