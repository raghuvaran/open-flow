use anyhow::Result;
use ndarray::Array2;
use ort::session::Session;
use ort::value::Value;
use std::path::Path;

pub struct SileroVad {
    session: Session,
    h: Vec<f32>,
    c: Vec<f32>,
    threshold: f32,
}

impl SileroVad {
    pub fn new(model_path: &Path, threshold: f32) -> Result<Self> {
        let session = Session::builder()?.commit_from_file(model_path)?;
        Ok(Self {
            session,
            h: vec![0.0f32; 128],
            c: vec![0.0f32; 128],
            threshold,
        })
    }

    pub fn process_frame(&mut self, audio: &[f32]) -> Result<f32> {
        let input = Array2::from_shape_vec((1, audio.len()), audio.to_vec())?;
        let h = ndarray::Array3::from_shape_vec((2, 1, 64), self.h.clone())?;
        let c = ndarray::Array3::from_shape_vec((2, 1, 64), self.c.clone())?;

        let input_val = Value::from_array(input)?;
        let h_val = Value::from_array(h)?;
        let c_val = Value::from_array(c)?;
        let sr_val = Value::from_array(([1i64], vec![16000i64]))?;

        let outputs = self.session.run(ort::inputs![
            input_val, h_val, c_val, sr_val,
        ])?;

        let (_shape, prob_data) = outputs[0].try_extract_tensor::<f32>()?;
        let prob = prob_data[0];

        let (_shape, h_out) = outputs[1].try_extract_tensor::<f32>()?;
        let (_shape, c_out) = outputs[2].try_extract_tensor::<f32>()?;
        self.h = h_out.to_vec();
        self.c = c_out.to_vec();

        Ok(prob)
    }

    pub fn is_speech(&mut self, audio: &[f32]) -> Result<bool> {
        Ok(self.process_frame(audio)? > self.threshold)
    }

    pub fn reset(&mut self) {
        self.h.fill(0.0);
        self.c.fill(0.0);
    }
}
