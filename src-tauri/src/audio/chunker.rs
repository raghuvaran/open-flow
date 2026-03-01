/// Collects audio frames and detects speech segments using VAD results.
/// Outputs complete audio segments ready for ASR.
pub struct Chunker {
    buffer: Vec<f32>,
    is_speaking: bool,
    silence_frames: u32,
    silence_threshold_frames: u32, // e.g., 700ms / 30ms = ~23 frames
    max_samples: usize,            // cap at 60s to prevent unbounded growth
}

impl Chunker {
    pub fn new(silence_threshold_ms: u64) -> Self {
        Self {
            buffer: Vec::new(),
            is_speaking: false,
            silence_frames: 0,
            silence_threshold_frames: (silence_threshold_ms / 30) as u32,
            max_samples: 16000 * 60, // 60s at 16kHz
        }
    }

    /// Feed a frame + VAD result. Returns Some(segment) when speech ends.
    pub fn feed(&mut self, frame: &[f32], is_speech: bool) -> Option<Vec<f32>> {
        if is_speech {
            self.is_speaking = true;
            self.silence_frames = 0;
            self.buffer.extend_from_slice(frame);
            if self.buffer.len() >= self.max_samples {
                self.is_speaking = false;
                self.silence_frames = 0;
                return Some(std::mem::take(&mut self.buffer));
            }
            None
        } else if self.is_speaking {
            self.silence_frames += 1;
            self.buffer.extend_from_slice(frame); // include trailing silence
            if self.silence_frames >= self.silence_threshold_frames {
                self.is_speaking = false;
                self.silence_frames = 0;
                Some(std::mem::take(&mut self.buffer))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Force-flush the buffer (e.g., on hotkey release).
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.buffer.is_empty() {
            None
        } else {
            self.is_speaking = false;
            self.silence_frames = 0;
            Some(std::mem::take(&mut self.buffer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech_frame() -> Vec<f32> { vec![0.5; 480] }
    fn silence_frame() -> Vec<f32> { vec![0.0; 480] }

    #[test]
    fn no_output_on_silence_only() {
        let mut c = Chunker::new(700);
        for _ in 0..100 {
            assert!(c.feed(&silence_frame(), false).is_none());
        }
    }

    #[test]
    fn no_output_during_speech() {
        let mut c = Chunker::new(700);
        for _ in 0..10 {
            assert!(c.feed(&speech_frame(), true).is_none());
        }
    }

    #[test]
    fn emits_segment_after_silence_threshold() {
        let mut c = Chunker::new(700);
        // 10 speech frames
        for _ in 0..10 {
            assert!(c.feed(&speech_frame(), true).is_none());
        }
        // 700ms / 30ms = ~23 silence frames needed
        let mut result = None;
        for _ in 0..30 {
            if let Some(seg) = c.feed(&silence_frame(), false) {
                result = Some(seg);
                break;
            }
        }
        let seg = result.expect("should emit segment after silence");
        // 10 speech + some silence frames, each 480 samples
        assert!(seg.len() >= 10 * 480);
    }

    #[test]
    fn flush_returns_buffered_audio() {
        let mut c = Chunker::new(700);
        for _ in 0..5 {
            c.feed(&speech_frame(), true);
        }
        let seg = c.flush().expect("flush should return buffered audio");
        assert_eq!(seg.len(), 5 * 480);
    }

    #[test]
    fn flush_empty_returns_none() {
        let mut c = Chunker::new(700);
        assert!(c.flush().is_none());
    }

    #[test]
    fn max_samples_cap_forces_emit() {
        let mut c = Chunker::new(700);
        let big_frame = vec![0.5; 480];
        let max = 16000 * 60; // 960000
        let frames_needed = max / 480 + 1;
        let mut emitted = false;
        for _ in 0..frames_needed {
            if c.feed(&big_frame, true).is_some() {
                emitted = true;
                break;
            }
        }
        assert!(emitted, "should force-emit at max_samples");
    }

    #[test]
    fn multiple_segments_from_speech_silence_speech() {
        let mut c = Chunker::new(90); // short threshold: 90ms/30ms = 3 frames
        // First speech burst
        for _ in 0..5 { c.feed(&speech_frame(), true); }
        // Silence to trigger segment
        let mut seg1 = None;
        for _ in 0..5 {
            if let Some(s) = c.feed(&silence_frame(), false) { seg1 = Some(s); break; }
        }
        assert!(seg1.is_some());
        // Second speech burst
        for _ in 0..3 { c.feed(&speech_frame(), true); }
        let mut seg2 = None;
        for _ in 0..5 {
            if let Some(s) = c.feed(&silence_frame(), false) { seg2 = Some(s); break; }
        }
        assert!(seg2.is_some());
    }

    #[test]
    fn state_resets_after_emit() {
        let mut c = Chunker::new(90);
        for _ in 0..5 { c.feed(&speech_frame(), true); }
        for _ in 0..5 { c.feed(&silence_frame(), false); }
        // After emit, silence should not produce anything
        for _ in 0..10 {
            assert!(c.feed(&silence_frame(), false).is_none());
        }
    }
}
