use crate::nexrad::{Level2Sweep, RadarProduct};
use crate::nexrad::level2::{MomentData, RadialData};

pub struct SRVComputer;

impl SRVComputer {
    /// Compute Storm Relative Velocity
    /// storm_dir_deg: direction storm is moving FROM (meteorological convention)
    /// storm_speed_kts: storm motion speed in knots
    pub fn compute(
        velocity_sweep: &Level2Sweep,
        storm_dir_deg: f32,
        storm_speed_kts: f32,
    ) -> Level2Sweep {
        let storm_speed_ms = storm_speed_kts * 0.51444;
        let storm_dir_rad = storm_dir_deg.to_radians();

        // Storm motion components (meteorological "from" convention):
        // u = east-west component, v = north-south component
        let storm_u = -storm_speed_ms * storm_dir_rad.sin();
        let storm_v = -storm_speed_ms * storm_dir_rad.cos();

        let radials = velocity_sweep
            .radials
            .iter()
            .map(|radial| {
                let az_rad = radial.azimuth.to_radians();

                // Project storm motion onto this radial direction
                // Positive radial = away from radar
                let storm_component = storm_u * az_rad.sin() + storm_v * az_rad.cos();

                // Convert storm component from m/s to kts for subtraction
                let storm_component_kts = storm_component / 0.51444;

                let moments = radial
                    .moments
                    .iter()
                    .map(|moment| {
                        if moment.product == RadarProduct::Velocity {
                            let srv_data: Vec<f32> = moment
                                .data
                                .iter()
                                .map(|&vel| {
                                    if vel.is_nan() {
                                        f32::NAN
                                    } else {
                                        vel - storm_component_kts
                                    }
                                })
                                .collect();

                            MomentData {
                                product: RadarProduct::StormRelativeVelocity,
                                gate_count: moment.gate_count,
                                first_gate_range: moment.first_gate_range,
                                gate_size: moment.gate_size,
                                data: srv_data,
                            }
                        } else {
                            moment.clone()
                        }
                    })
                    .collect();

                RadialData {
                    azimuth: radial.azimuth,
                    elevation: radial.elevation,
                    azimuth_spacing: radial.azimuth_spacing,
                    moments,
                }
            })
            .collect();

        Level2Sweep {
            elevation_number: velocity_sweep.elevation_number,
            elevation_angle: velocity_sweep.elevation_angle,
            radials,
        }
    }

    /// Auto-estimate storm motion using a simplified approach.
    /// Returns (direction_from_deg, speed_kts).
    ///
    /// This is a rough approximation: it computes the mean u/v components
    /// from all valid velocity gates to estimate the bulk flow, then uses
    /// that as a proxy for storm motion. A more sophisticated implementation
    /// would use the Bunkers right-mover method with wind profile data.
    pub fn estimate_storm_motion(velocity_sweep: &Level2Sweep) -> (f32, f32) {
        let mut sum_u: f64 = 0.0;
        let mut sum_v: f64 = 0.0;
        let mut count: u64 = 0;

        for radial in &velocity_sweep.radials {
            let az_rad = (radial.azimuth as f64).to_radians();
            for moment in &radial.moments {
                if moment.product == RadarProduct::Velocity {
                    for &vel in &moment.data {
                        if !vel.is_nan() {
                            // Velocity is radial: positive = away from radar
                            // Decompose into u (east) and v (north) components
                            let vel_f64 = vel as f64;
                            sum_u += vel_f64 * az_rad.sin();
                            sum_v += vel_f64 * az_rad.cos();
                            count += 1;
                        }
                    }
                }
            }
        }

        if count == 0 {
            // Default: from SW at 30 kts (common storm motion in central US)
            return (240.0, 30.0);
        }

        let mean_u = sum_u / count as f64;
        let mean_v = sum_v / count as f64;

        // Convert u,v back to direction/speed
        // Direction the wind is FROM (meteorological convention)
        let speed_kts = (mean_u * mean_u + mean_v * mean_v).sqrt() / 0.51444;
        let dir_rad = mean_u.atan2(mean_v);
        let dir_from = (dir_rad.to_degrees() + 180.0).rem_euclid(360.0);

        (dir_from as f32, speed_kts as f32)
    }
}
