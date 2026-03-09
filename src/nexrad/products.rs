use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RadarProduct {
    Reflectivity,       // REF / DREF
    Velocity,           // VEL / DVEL
    SpectrumWidth,      // SW / DSW
    DifferentialReflectivity, // ZDR
    CorrelationCoefficient,  // RHO / CC
    DifferentialPhase,       // PHI / KDP
    SpecificDiffPhase,       // KDP
    HydrometeorClass,        // HHC
    VIL,                         // Vertically Integrated Liquid (derived)
    EchoTops,                    // Echo Tops (derived)
    StormRelativeVelocity,       // SRV (computed from VEL + storm motion)
    Unknown,
}

impl RadarProduct {
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "REF" | "DREF" => RadarProduct::Reflectivity,
            "VEL" | "DVEL" => RadarProduct::Velocity,
            "SW" | "DSW" => RadarProduct::SpectrumWidth,
            "ZDR" => RadarProduct::DifferentialReflectivity,
            "RHO" | "CC" => RadarProduct::CorrelationCoefficient,
            "PHI" => RadarProduct::DifferentialPhase,
            "KDP" => RadarProduct::SpecificDiffPhase,
            "HHC" => RadarProduct::HydrometeorClass,
            "VIL" => RadarProduct::VIL,
            "ET" => RadarProduct::EchoTops,
            "SRV" => RadarProduct::StormRelativeVelocity,
            _ => RadarProduct::Unknown,
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            RadarProduct::Reflectivity => "Reflectivity (REF)",
            RadarProduct::Velocity => "Velocity (VEL)",
            RadarProduct::SpectrumWidth => "Spectrum Width (SW)",
            RadarProduct::DifferentialReflectivity => "Diff. Reflectivity (ZDR)",
            RadarProduct::CorrelationCoefficient => "Corr. Coefficient (CC)",
            RadarProduct::DifferentialPhase => "Diff. Phase (PHI)",
            RadarProduct::SpecificDiffPhase => "Specific Diff. Phase (KDP)",
            RadarProduct::HydrometeorClass => "Hydrometeor Class (HHC)",
            RadarProduct::VIL => "Vert. Integrated Liquid (VIL)",
            RadarProduct::EchoTops => "Echo Tops (ET)",
            RadarProduct::StormRelativeVelocity => "Storm Rel. Velocity (SRV)",
            RadarProduct::Unknown => "Unknown",
        }
    }

    pub fn short_name(&self) -> &str {
        match self {
            RadarProduct::Reflectivity => "REF",
            RadarProduct::Velocity => "VEL",
            RadarProduct::SpectrumWidth => "SW",
            RadarProduct::DifferentialReflectivity => "ZDR",
            RadarProduct::CorrelationCoefficient => "CC",
            RadarProduct::DifferentialPhase => "PHI",
            RadarProduct::SpecificDiffPhase => "KDP",
            RadarProduct::HydrometeorClass => "HHC",
            RadarProduct::VIL => "VIL",
            RadarProduct::EchoTops => "ET",
            RadarProduct::StormRelativeVelocity => "SRV",
            RadarProduct::Unknown => "???",
        }
    }

    pub fn unit(&self) -> &str {
        match self {
            RadarProduct::Reflectivity => "dBZ",
            RadarProduct::Velocity => "kts",
            RadarProduct::SpectrumWidth => "kts",
            RadarProduct::DifferentialReflectivity => "dB",
            RadarProduct::CorrelationCoefficient => "",
            RadarProduct::DifferentialPhase => "deg",
            RadarProduct::SpecificDiffPhase => "deg/km",
            RadarProduct::HydrometeorClass => "",
            RadarProduct::VIL => "kg/m²",
            RadarProduct::EchoTops => "km",
            RadarProduct::StormRelativeVelocity => "kts",
            RadarProduct::Unknown => "",
        }
    }

    pub fn all_products() -> &'static [RadarProduct] {
        &[
            RadarProduct::Reflectivity,
            RadarProduct::Velocity,
            RadarProduct::SpectrumWidth,
            RadarProduct::DifferentialReflectivity,
            RadarProduct::CorrelationCoefficient,
            RadarProduct::SpecificDiffPhase,
            RadarProduct::StormRelativeVelocity,
        ]
    }
}
