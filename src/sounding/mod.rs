//! SHARPpy-inspired sounding analysis engine in Rust.
//!
//! Provides thermodynamic computations, severe weather parameters,
//! profile interpolation, and an enhanced sounding profile structure.
//! All formulas are scientifically accurate and follow the same
//! equations used in SHARPpy (https://github.com/sharppy/SHARPpy).

pub mod thermo;
pub mod params;
pub mod profile;
pub mod interp;

pub use profile::{SoundingProfile, SoundingParams, SoundingLevel};
