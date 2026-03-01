use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use tokio::sync::mpsc;

pub struct AudioCapture {
    stream: Option<Stream>,
}

pub(crate) fn resample(samples: &[f32], from_rate: u32) -> Vec<f32> {
    if from_rate == 16000 { return samples.to_vec(); }
    let ratio = from_rate as f64 / 16000.0;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len).map(|i| {
        let src = i as f64 * ratio;
        let idx = src as usize;
        let frac = src - idx as f64;
        let a = samples[idx.min(samples.len() - 1)];
        let b = samples[(idx + 1).min(samples.len() - 1)];
        a + (b - a) * frac as f32
    }).collect()
}

pub(crate) fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 { return samples.to_vec(); }
    samples.chunks(channels as usize)
        .map(|ch| ch.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub fn list_input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    Ok(host.input_devices()?
        .filter_map(|d| d.name().ok())
        .collect())
}

pub fn start_capture(tx: mpsc::UnboundedSender<Vec<f32>>, device_name: Option<&str>) -> Result<AudioCapture> {
    let host = cpal::default_host();

    let device = if let Some(name) = device_name {
        host.input_devices()?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            .or_else(|| {
                tracing::warn!("Mic '{}' not found, using default", name);
                host.default_input_device()
            })
    } else {
        host.default_input_device()
    }.ok_or_else(|| anyhow::anyhow!("No input device found"))?;

    tracing::info!("Using input device: {}", device.name()?);

    let default_config = device.default_input_config()?;
    tracing::info!("Device config: {}Hz, {} ch, {:?}",
        default_config.sample_rate().0, default_config.channels(), default_config.sample_format());

    let sample_rate = default_config.sample_rate().0;
    let channels = default_config.channels();
    let config: cpal::StreamConfig = default_config.into();

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let mono = to_mono(data, channels);
            let resampled = resample(&mono, sample_rate);
            if !resampled.is_empty() { let _ = tx.send(resampled); }
        },
        |err| tracing::error!("Audio stream error: {}", err),
        None,
    )?;
    stream.play()?;
    Ok(AudioCapture { stream: Some(stream) })
}

impl AudioCapture {
    pub fn stop(&mut self) {
        if let Some(stream) = self.stream.take() {
            drop(stream);
            tracing::info!("Mic stream released");
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) { self.stop(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_identity_at_16k() {
        let input: Vec<f32> = (0..1600).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample(&input, 16000);
        assert_eq!(out.len(), input.len());
        assert_eq!(out, input);
    }

    #[test]
    fn resample_48k_to_16k() {
        let input: Vec<f32> = vec![1.0; 4800]; // 100ms at 48kHz
        let out = resample(&input, 48000);
        assert_eq!(out.len(), 1600); // 100ms at 16kHz
    }

    #[test]
    fn resample_44100_to_16k() {
        let input: Vec<f32> = vec![0.5; 4410]; // 100ms at 44.1kHz
        let out = resample(&input, 44100);
        // 4410 / (44100/16000) = ~1600
        assert!((out.len() as i32 - 1600).abs() <= 1);
    }

    #[test]
    fn resample_preserves_value_range() {
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.1).sin()).collect();
        let out = resample(&input, 48000);
        for s in &out {
            assert!(*s >= -1.0 && *s <= 1.0);
        }
    }

    #[test]
    fn to_mono_passthrough_single_channel() {
        let input = vec![0.1, 0.2, 0.3];
        let out = to_mono(&input, 1);
        assert_eq!(out, input);
    }

    #[test]
    fn to_mono_stereo_averages() {
        let input = vec![0.4, 0.6, 0.2, 0.8]; // 2 stereo frames
        let out = to_mono(&input, 2);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn to_mono_multichannel() {
        let input = vec![0.3, 0.3, 0.3, 0.6, 0.6, 0.6]; // 2 frames, 3 channels
        let out = to_mono(&input, 3);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.3).abs() < 1e-6);
        assert!((out[1] - 0.6).abs() < 1e-6);
    }
}
