use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use std::sync::Arc;
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

/// Watch for audio device additions/removals via CoreAudio.
/// Calls `on_change` whenever the device list changes.
/// Returns a guard — listener stays active until the guard is dropped.
#[cfg(target_os = "macos")]
pub fn watch_device_changes<F: Fn() + Send + 'static>(on_change: F) -> Result<DeviceWatcher> {
    use std::os::raw::c_void;

    const SYSTEM_OBJECT: u32 = 1;
    const DEVICES_SELECTOR: u32 = u32::from_be_bytes(*b"dev#");
    const GLOBAL_SCOPE: u32 = u32::from_be_bytes(*b"glob");

    #[repr(C)]
    struct PropAddr { selector: u32, scope: u32, element: u32 }

    extern "C" {
        fn AudioObjectAddPropertyListener(
            id: u32, addr: *const PropAddr,
            cb: unsafe extern "C" fn(u32, u32, *const PropAddr, *mut c_void) -> i32,
            data: *mut c_void,
        ) -> i32;
    }

    unsafe extern "C" fn on_devices_changed(
        _id: u32, _n: u32, _addr: *const PropAddr, data: *mut c_void,
    ) -> i32 {
        let cb = unsafe { &*(data as *const Box<dyn Fn() + Send>) };
        cb();
        0
    }

    let addr = PropAddr { selector: DEVICES_SELECTOR, scope: GLOBAL_SCOPE, element: 0 };
    let closure: Arc<Box<dyn Fn() + Send>> = Arc::new(Box::new(on_change));
    let raw = Arc::into_raw(closure);

    let status = unsafe {
        AudioObjectAddPropertyListener(SYSTEM_OBJECT, &addr, on_devices_changed, raw as *mut c_void)
    };
    if status != 0 {
        unsafe { Arc::from_raw(raw); }
        anyhow::bail!("AudioObjectAddPropertyListener failed: {}", status);
    }

    Ok(DeviceWatcher { data: raw })
}

#[cfg(target_os = "macos")]
pub struct DeviceWatcher {
    data: *const Box<dyn Fn() + Send>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for DeviceWatcher {}
#[cfg(target_os = "macos")]
unsafe impl Sync for DeviceWatcher {}

#[cfg(target_os = "macos")]
impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        use std::os::raw::c_void;

        #[repr(C)]
        struct PropAddr { selector: u32, scope: u32, element: u32 }

        extern "C" {
            fn AudioObjectRemovePropertyListener(
                id: u32, addr: *const PropAddr,
                cb: unsafe extern "C" fn(u32, u32, *const PropAddr, *mut c_void) -> i32,
                data: *mut c_void,
            ) -> i32;
        }

        unsafe extern "C" fn on_devices_changed(
            _id: u32, _n: u32, _addr: *const PropAddr, data: *mut c_void,
        ) -> i32 {
            let cb = unsafe { &*(data as *const Box<dyn Fn() + Send>) };
            cb();
            0
        }

        let addr = PropAddr {
            selector: u32::from_be_bytes(*b"dev#"),
            scope: u32::from_be_bytes(*b"glob"),
            element: 0,
        };
        unsafe {
            AudioObjectRemovePropertyListener(1, &addr, on_devices_changed, self.data as *mut c_void);
            Arc::from_raw(self.data);
        }
    }
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

    #[test]
    fn list_input_devices_returns_vec() {
        // Should not panic; may be empty in CI
        let devices = list_input_devices().unwrap();
        assert!(devices.len() < 1000); // sanity check
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn device_watcher_registers_and_drops() {
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c = called.clone();
        let watcher = watch_device_changes(move || { c.store(true, std::sync::atomic::Ordering::Relaxed); }).unwrap();
        // Just verify it doesn't panic on creation or drop
        drop(watcher);
    }
}
