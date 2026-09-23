//! Audio sample-counter mix.

pub struct MixInputState {
    pub samples_sent: i64,
    pub clip_samples: i64,
}

impl MixInputState {
    pub fn new(duration_secs: f64, speed: f64, sample_rate: u32) -> Self {
        Self {
            samples_sent: 0,
            clip_samples: (duration_secs * speed * sample_rate as f64).round() as i64,
        }
    }
    pub fn is_exhausted(&self) -> bool {
        self.samples_sent >= self.clip_samples
    }
    pub fn record_sent(&mut self, samples: i64) {
        self.samples_sent += samples;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_samples_accounts_for_speed() {
        let state = MixInputState::new(2.0, 2.0, 48000);
        assert_eq!(state.clip_samples, 192000);
    }

    #[test]
    fn exhausted_after_sending_enough() {
        let mut state = MixInputState::new(1.0, 1.0, 48000);
        assert!(!state.is_exhausted());
        state.record_sent(48000);
        assert!(state.is_exhausted());
    }
}
