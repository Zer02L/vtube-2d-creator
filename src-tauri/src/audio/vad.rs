use std::time::{Duration, Instant};

/// Voice Activity Detector using RMS audio power with attack and hold decay filtering.
pub struct VoiceActivityDetector {
    pub threshold: f32,
    pub attack_duration: Duration,
    pub hold_duration: Duration,

    is_speaking: bool,
    above_threshold_since: Option<Instant>,
    last_above_threshold: Option<Instant>,
}

impl VoiceActivityDetector {
    pub fn new(threshold: f32, attack_ms: u32, hold_ms: u32) -> Self {
        Self {
            threshold,
            attack_duration: Duration::from_millis(attack_ms as u64),
            hold_duration: Duration::from_millis(hold_ms as u64),
            is_speaking: false,
            above_threshold_since: None,
            last_above_threshold: None,
        }
    }

    /// Calculate Root-Mean-Square (RMS) amplitude of audio sample slice (normalized -1.0 to 1.0).
    pub fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
        (sum_sq / samples.len() as f32).sqrt()
    }

    /// Process a new batch of audio samples and return current speaking state.
    pub fn process_samples(&mut self, samples: &[f32], now: Instant) -> (f32, bool) {
        let rms = Self::calculate_rms(samples);

        if rms >= self.threshold {
            self.last_above_threshold = Some(now);
            if self.above_threshold_since.is_none() {
                self.above_threshold_since = Some(now);
            }

            // Attack smoothing: must stay above threshold for attack_duration
            if let Some(since) = self.above_threshold_since {
                if now.duration_since(since) >= self.attack_duration {
                    self.is_speaking = true;
                }
            }
        } else {
            self.above_threshold_since = None;

            // Hold / decay smoothing: keep speaking state during brief speech pauses
            if let Some(last) = self.last_above_threshold {
                if now.duration_since(last) >= self.hold_duration {
                    self.is_speaking = false;
                }
            } else {
                self.is_speaking = false;
            }
        }

        (rms, self.is_speaking)
    }
}
