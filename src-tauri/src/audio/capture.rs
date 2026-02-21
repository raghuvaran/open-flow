use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use tokio::sync::mpsc;

pub struct AudioCapture {
    stream: Option<Stream>,
}

fn resample(samples: &[f32], from_rate: u32) -> Vec<f32> {
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

fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
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
