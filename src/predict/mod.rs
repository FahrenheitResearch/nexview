//! Tornado prediction using DeepGuess ResNet3D models.
//!
//! Converts NEXRAD Level2 radar data into the model's expected input format
//! and runs inference via ONNX Runtime.

pub mod convert;
pub mod autoscan;
#[cfg(feature = "tornado-predict")]
pub mod inference;

pub use convert::RadarSequence;
pub use autoscan::{AutoScanManager, ScanResult};
#[cfg(feature = "tornado-predict")]
pub use inference::TornadoPredictor;
