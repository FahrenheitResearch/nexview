//! ONNX Runtime inference for tornado prediction models.
//!
//! Supports two architectures, auto-detected from the ONNX model metadata:
//!   - ResNet3D (24ch, dual-head): input (1,24,8,128,128) → output (1,4)
//!   - Swin3D  (12ch, single-head): input (1,12,8,128,128) → output (1,2)

use super::convert::{RadarSequence, TornadoPrediction, GRID_SIZE};
use std::path::{Path, PathBuf};

/// Which model architecture is loaded.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ModelType {
    /// ResNet3D: 24 input channels, 4 outputs (det_neg, det_pos, pred_neg, pred_pos)
    ResNet3D,
    /// Video Swin Transformer: 12 input channels, 2 outputs (neg, pos)
    Swin3D,
}

/// Wraps the ONNX model for tornado prediction inference.
pub struct TornadoPredictor {
    session: ort::session::Session,
    model_type: ModelType,
}

impl TornadoPredictor {
    /// Load the ONNX model from a file path.
    /// Automatically detects whether it's a ResNet3D (24ch) or Swin3D (12ch) model.
    pub fn load(model_path: &Path) -> Result<Self, String> {
        let session = ort::session::Session::builder()
            .map_err(|e| format!("Failed to create session builder: {}", e))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| format!("Failed to set opt level: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| format!("Failed to load model from {:?}: {}", model_path, e))?;

        // Detect model type from input shape
        let model_type = Self::detect_model_type(&session, model_path);
        log::info!(
            "Loaded {:?} tornado model from {:?}",
            model_type,
            model_path
        );

        Ok(TornadoPredictor { session, model_type })
    }

    /// Detect model type by inspecting the ONNX input dimensions.
    fn detect_model_type(session: &ort::session::Session, model_path: &Path) -> ModelType {
        // First, try to infer from filename
        let filename = model_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");
        if filename.contains("swin") {
            return ModelType::Swin3D;
        }
        if filename.contains("resnet") {
            return ModelType::ResNet3D;
        }

        // Fall back to inspecting input shape
        // Input shape is [batch, channels, time, height, width]
        // ResNet3D: channels=24, Swin3D: channels=12
        if let Some(input) = session.inputs().first() {
            if let ort::value::ValueType::Tensor { ty: _, shape, .. } = input.dtype() {
                // shape is Shape (deref to SmallVec<[i64; 4]>); index 1 is channels
                if let Some(&ch) = shape.get(1) {
                    if ch == 12 {
                        return ModelType::Swin3D;
                    }
                }
            }
        }

        // Default to ResNet3D for backwards compatibility
        ModelType::ResNet3D
    }

    /// Find the model file. Searches models/ next to executable for .onnx files.
    pub fn find_model() -> Option<PathBuf> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))?;

        let candidates = [
            // Swin3D (preferred — higher accuracy on temporal data)
            exe_dir.join("models").join("swin3d_tornado.onnx"),
            // ResNet3D variants
            exe_dir.join("models").join("resnet3d_tornado.onnx"),
            exe_dir.join("models").join("resnet3d-18-tornet.onnx"),
            exe_dir.join("models").join("tornado_model.onnx"),
        ];

        for path in &candidates {
            if path.exists() {
                return Some(path.clone());
            }
        }

        // Check for any .onnx file in models/
        let models_dir = exe_dir.join("models");
        if models_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&models_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("onnx") {
                        return Some(path);
                    }
                }
            }
        }

        None
    }

    /// Returns the detected model type name (for UI display).
    pub fn model_name(&self) -> &'static str {
        match self.model_type {
            ModelType::ResNet3D => "ResNet3D-18",
            ModelType::Swin3D => "Swin3D",
        }
    }

    /// Run inference on a converted radar sequence.
    pub fn predict(&mut self, sequence: &RadarSequence) -> Result<TornadoPrediction, String> {
        match self.model_type {
            ModelType::ResNet3D => self.predict_resnet3d(sequence),
            ModelType::Swin3D => self.predict_swin3d(sequence),
        }
    }

    /// ResNet3D: 24ch input, 4-value dual-head output.
    fn predict_resnet3d(&mut self, sequence: &RadarSequence) -> Result<TornadoPrediction, String> {
        let input_data = sequence.to_model_input();
        let shape: Vec<i64> = vec![1, 24, 8, GRID_SIZE as i64, GRID_SIZE as i64];
        let input_tensor = ort::value::Tensor::from_array((shape, input_data.into_boxed_slice()))
            .map_err(|e| format!("Failed to create input tensor: {}", e))?;

        let outputs = self
            .session
            .run(ort::inputs![input_tensor])
            .map_err(|e| format!("Inference failed: {}", e))?;

        let output = outputs.values().next().ok_or("No output tensor")?;
        let extracted = output
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract output: {}", e))?;
        let raw: Vec<f32> = extracted.1.to_vec();

        log::info!("ResNet3D raw output ({} values): {:?}", raw.len(), &raw[..raw.len().min(10)]);

        if raw.len() < 4 {
            return Err(format!("Expected 4 outputs, got {}", raw.len()));
        }

        let det_prob = softmax_binary(raw[0], raw[1]);
        let pred_prob = softmax_binary(raw[2], raw[3]);
        log::info!("ResNet3D softmax: det={:.4} pred={:.4}", det_prob, pred_prob);

        Ok(TornadoPrediction {
            detection_prob: det_prob,
            prediction_prob: pred_prob,
            combined_score: (det_prob + pred_prob) / 2.0,
            storm_lat: sequence.center_lat,
            storm_lon: sequence.center_lon,
            station: sequence.station.clone(),
            num_frames: sequence.num_frames,
            dual_head: true,
        })
    }

    /// Swin3D: 12ch input, 2-value single-head output.
    fn predict_swin3d(&mut self, sequence: &RadarSequence) -> Result<TornadoPrediction, String> {
        let input_data = sequence.to_model_input_12ch();
        let shape: Vec<i64> = vec![1, 12, 8, GRID_SIZE as i64, GRID_SIZE as i64];
        let input_tensor = ort::value::Tensor::from_array((shape, input_data.into_boxed_slice()))
            .map_err(|e| format!("Failed to create input tensor: {}", e))?;

        let outputs = self
            .session
            .run(ort::inputs![input_tensor])
            .map_err(|e| format!("Inference failed: {}", e))?;

        let output = outputs.values().next().ok_or("No output tensor")?;
        let extracted = output
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract output: {}", e))?;
        let raw: Vec<f32> = extracted.1.to_vec();

        log::info!("Swin3D raw output ({} values): {:?}", raw.len(), &raw[..raw.len().min(10)]);

        if raw.len() < 2 {
            return Err(format!("Expected 2 outputs, got {}", raw.len()));
        }

        let tornado_prob = softmax_binary(raw[0], raw[1]);
        log::info!("Swin3D tornado_prob={:.4}", tornado_prob);

        Ok(TornadoPrediction {
            detection_prob: tornado_prob,
            prediction_prob: tornado_prob,
            combined_score: tornado_prob,
            storm_lat: sequence.center_lat,
            storm_lon: sequence.center_lon,
            station: sequence.station.clone(),
            num_frames: sequence.num_frames,
            dual_head: false,
        })
    }
}

/// Softmax for binary classification: returns P(positive class)
fn softmax_binary(neg_logit: f32, pos_logit: f32) -> f32 {
    let max = neg_logit.max(pos_logit);
    let exp_neg = (neg_logit - max).exp();
    let exp_pos = (pos_logit - max).exp();
    exp_pos / (exp_neg + exp_pos)
}
