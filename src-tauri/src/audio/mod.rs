pub mod vad;

use vad::VoiceActivityDetector;

pub struct AudioEngine {
    pub vad: VoiceActivityDetector,
}

impl AudioEngine {
    pub fn new(threshold: f32, attack_ms: u32, hold_ms: u32) -> Self {
        Self {
            vad: VoiceActivityDetector::new(threshold, attack_ms, hold_ms),
        }
    }
}
