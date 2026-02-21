/// Collects audio frames and detects speech segments using VAD results.
/// Outputs complete audio segments ready for ASR.
pub struct Chunker {
    buffer: Vec<f32>,
    is_speaking: bool,
    silence_frames: u32,
    silence_threshold_frames: u32, // e.g., 700ms / 30ms = ~23 frames
}

impl Chunker {
    pub fn new(silence_threshold_ms: u64) -> Self {
        Self {
            buffer: Vec::new(),
            is_speaking: false,
            silence_frames: 0,
            silence_threshold_frames: (silence_threshold_ms / 30) as u32,
        }
    }

    /// Feed a frame + VAD result. Returns Some(segment) when speech ends.
    pub fn feed(&mut self, frame: &[f32], is_speech: bool) -> Option<Vec<f32>> {
        if is_speech {
            self.is_speaking = true;
            self.silence_frames = 0;
            self.buffer.extend_from_slice(frame);
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
