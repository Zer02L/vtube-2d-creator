use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSlotInfo {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub is_loop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionInfo {
    pub id: String,
    pub name: String,
    pub hotkey: String,
    pub idle_video_id: String,
    pub talk_video_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadSettings {
    pub threshold: f32,
    pub attack_ms: u32,
    pub hold_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarManifest {
    pub version: String,
    pub name: String,
    pub resolution: String,
    pub vad: VadSettings,
    pub default_idle_video_id: String,
    pub default_talk_video_id: String,
    pub videos: HashMap<String, VideoSlotInfo>,
    pub emotions: Vec<EmotionInfo>,
}
