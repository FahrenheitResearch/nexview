use wgpu;
use wgpu::util::DeviceExt;
use crate::nexrad::{Level2Sweep, RadarProduct};
use crate::render::ColorTable;
use crate::render::radar::RenderedSweep;
use crate::nexrad::sites::RadarSite;

/// GPU compute shader radar renderer using wgpu.
/// Performs inverse-mapping (same algorithm as CPU RadarRenderer)
/// but runs on the GPU for significantly higher throughput.
pub struct GpuRadarRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

// Matches the WGSL Params struct layout (std430)
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuParams {
    image_size: u32,
    num_radials: u32,
    max_range: f32,
    scale: f32,
    center: f32,
    num_color_entries: u32,
    _pad: [u32; 2],
}

// Matches the WGSL RadialInfo struct layout
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuRadialInfo {
    azimuth: f32,
    half_spacing: f32,
    first_gate_range: f32,
    gate_size: f32,
    gate_count: u32,
    data_offset: u32,
    _pad: [u32; 2],
}

// Color table entry for GPU
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuColorEntry {
    value: f32,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    _pad: [f32; 3],
}

const SHADER_SOURCE: &str = r#"
struct Params {
    image_size: u32,
    num_radials: u32,
    max_range: f32,
    scale: f32,
    center: f32,
    num_color_entries: u32,
}

struct RadialInfo {
    azimuth: f32,
    half_spacing: f32,
    first_gate_range: f32,
    gate_size: f32,
    gate_count: u32,
    data_offset: u32,
}

struct ColorEntry {
    value: f32,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

@group(0) @binding(0) var output_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(1) var<storage, read> params: Params;
@group(0) @binding(2) var<storage, read> radials: array<RadialInfo>;
@group(0) @binding(3) var<storage, read> gate_data: array<f32>;
@group(0) @binding(4) var<storage, read> color_entries: array<ColorEntry>;
@group(0) @binding(5) var<storage, read> sorted_indices: array<u32>;

fn color_for_value(value: f32) -> vec4<f32> {
    let num_entries = params.num_color_entries;
    if num_entries == 0u {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Below minimum
    if value < color_entries[0].value {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Find bracketing entries
    var lower_idx = 0u;
    var upper_idx = 0u;
    for (var i = 0u; i < num_entries; i = i + 1u) {
        if color_entries[i].value <= value {
            lower_idx = i;
        }
        if color_entries[i].value >= value {
            upper_idx = i;
            break;
        }
        // If we reach the end without finding upper, clamp to last
        if i == num_entries - 1u {
            upper_idx = i;
        }
    }

    let lower = color_entries[lower_idx];
    let upper = color_entries[upper_idx];

    let range = upper.value - lower.value;
    if abs(range) < 0.001 {
        return vec4<f32>(lower.r, lower.g, lower.b, lower.a);
    }

    let t = clamp((value - lower.value) / range, 0.0, 1.0);
    return vec4<f32>(
        lower.r + t * (upper.r - lower.r),
        lower.g + t * (upper.g - lower.g),
        lower.b + t * (upper.b - lower.b),
        lower.a + t * (upper.a - lower.a),
    );
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = gid.x;
    let py = gid.y;
    let size = params.image_size;

    if px >= size || py >= size {
        return;
    }

    let center = params.center;
    let scale = params.scale;
    let max_range = params.max_range;
    let num_radials = params.num_radials;

    let dx = f32(px) - center;
    let dy = center - f32(py);

    let range_m = sqrt(dx * dx + dy * dy) / scale;
    if range_m <= 0.0 || range_m > max_range {
        textureStore(output_tex, vec2<i32>(i32(px), i32(py)), vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    // Azimuth: 0 = north, clockwise
    var az_deg = degrees(atan2(dx, dy));
    if az_deg < 0.0 {
        az_deg = az_deg + 360.0;
    }

    // Binary search sorted radials by azimuth
    // sorted_indices maps sorted position -> radial index
    var lo = 0u;
    var hi = num_radials;
    while lo < hi {
        let mid = (lo + hi) / 2u;
        let mid_az = radials[sorted_indices[mid]].azimuth;
        if mid_az < az_deg {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }

    // lo is the insertion point — find nearest radial
    var best_idx: u32;
    if lo == 0u {
        let dist_first = abs(az_deg - radials[sorted_indices[0]].azimuth);
        let dist_last = 360.0 - radials[sorted_indices[num_radials - 1u]].azimuth + az_deg;
        if abs(dist_last) < dist_first {
            best_idx = sorted_indices[num_radials - 1u];
        } else {
            best_idx = sorted_indices[0];
        }
    } else if lo >= num_radials {
        let dist_last = az_deg - radials[sorted_indices[num_radials - 1u]].azimuth;
        let dist_first = 360.0 - az_deg + radials[sorted_indices[0]].azimuth;
        if abs(dist_first) < abs(dist_last) {
            best_idx = sorted_indices[0];
        } else {
            best_idx = sorted_indices[num_radials - 1u];
        }
    } else {
        let d_prev = abs(az_deg - radials[sorted_indices[lo - 1u]].azimuth);
        let d_next = abs(radials[sorted_indices[lo]].azimuth - az_deg);
        if d_prev <= d_next {
            best_idx = sorted_indices[lo - 1u];
        } else {
            best_idx = sorted_indices[lo];
        }
    }

    let radial = radials[best_idx];

    // Check beam width
    var az_diff = abs(az_deg - radial.azimuth);
    if az_diff > 180.0 {
        az_diff = 360.0 - az_diff;
    }
    if az_diff > radial.half_spacing + 0.1 {
        textureStore(output_tex, vec2<i32>(i32(px), i32(py)), vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    // Gate lookup
    let gate_offset = range_m - radial.first_gate_range;
    if gate_offset < 0.0 {
        textureStore(output_tex, vec2<i32>(i32(px), i32(py)), vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    let gate_idx = u32(gate_offset / radial.gate_size);
    if gate_idx >= radial.gate_count {
        textureStore(output_tex, vec2<i32>(i32(px), i32(py)), vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    let value = gate_data[radial.data_offset + gate_idx];

    // NaN check: in WGSL, NaN != NaN
    if value != value {
        textureStore(output_tex, vec2<i32>(i32(px), i32(py)), vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    let color = color_for_value(value);
    if color.a == 0.0 {
        textureStore(output_tex, vec2<i32>(i32(px), i32(py)), vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    textureStore(output_tex, vec2<i32>(i32(px), i32(py)), color);
}
"#;

impl GpuRadarRenderer {
    /// Create a new GPU radar renderer from eframe's wgpu render state.
    pub fn new(render_state: &egui_wgpu::RenderState) -> Self {
        let device = render_state.device.clone();
        let queue = render_state.queue.clone();

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("radar_compute_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("radar_compute_bgl"),
            entries: &[
                // 0: output texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                // 1: params
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 2: radials
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 3: gate_data
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 4: color_entries
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 5: sorted_indices
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("radar_compute_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("radar_compute_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
        }
    }

    /// Render a sweep on the GPU, returning the same RenderedSweep as CPU path.
    pub fn render_sweep_gpu(
        &self,
        sweep: &Level2Sweep,
        product: RadarProduct,
        site: &RadarSite,
        image_size: u32,
        color_table: &ColorTable,
    ) -> Option<RenderedSweep> {
        // Compute max range
        let max_range_m = sweep.radials.iter()
            .filter_map(|r| {
                r.moments.iter()
                    .filter(|m| m.product == product)
                    .map(|m| m.first_gate_range as f64 + m.gate_count as f64 * m.gate_size as f64)
                    .next()
            })
            .fold(0.0f64, f64::max);

        if max_range_m <= 0.0 {
            return None;
        }

        let range_km = max_range_m / 1000.0;
        let center = image_size as f64 / 2.0;
        let scale = center / max_range_m;

        // Build sorted radial data
        let mut radial_order: Vec<(f32, usize)> = sweep.radials.iter()
            .enumerate()
            .filter_map(|(i, r)| {
                // Only include radials that have this product
                if r.moments.iter().any(|m| m.product == product) {
                    Some((r.azimuth, i))
                } else {
                    None
                }
            })
            .collect();
        radial_order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        if radial_order.is_empty() {
            return None;
        }

        // Build flat gate data and radial info
        let mut all_gate_data: Vec<f32> = Vec::new();
        let mut radial_infos: Vec<GpuRadialInfo> = Vec::with_capacity(sweep.radials.len());

        // We need info for ALL radials (indexed by original index), not just sorted
        for radial in &sweep.radials {
            let moment = radial.moments.iter().find(|m| m.product == product);
            match moment {
                Some(m) => {
                    let data_offset = all_gate_data.len() as u32;
                    // Replace NaN with a sentinel that WGSL can detect via value != value
                    for &v in &m.data {
                        all_gate_data.push(v); // NaN passes through as NaN in f32 buffers
                    }
                    radial_infos.push(GpuRadialInfo {
                        azimuth: radial.azimuth,
                        half_spacing: radial.azimuth_spacing / 2.0,
                        first_gate_range: m.first_gate_range as f32,
                        gate_size: m.gate_size as f32,
                        gate_count: m.gate_count as u32,
                        data_offset,
                        _pad: [0; 2],
                    });
                }
                None => {
                    radial_infos.push(GpuRadialInfo {
                        azimuth: radial.azimuth,
                        half_spacing: radial.azimuth_spacing / 2.0,
                        first_gate_range: 0.0,
                        gate_size: 1.0,
                        gate_count: 0,
                        data_offset: 0,
                        _pad: [0; 2],
                    });
                }
            }
        }

        // Ensure gate data is non-empty (wgpu doesn't like zero-sized buffers)
        if all_gate_data.is_empty() {
            all_gate_data.push(f32::NAN);
        }

        // Sorted indices (maps sorted position -> original radial index)
        let sorted_indices: Vec<u32> = radial_order.iter().map(|(_, i)| *i as u32).collect();

        // Build color entries for GPU
        let gpu_colors: Vec<GpuColorEntry> = color_table.entries.iter().map(|e| {
            GpuColorEntry {
                value: e.value,
                r: e.r as f32 / 255.0,
                g: e.g as f32 / 255.0,
                b: e.b as f32 / 255.0,
                a: e.a as f32 / 255.0,
                _pad: [0.0; 3],
            }
        }).collect();

        let params = GpuParams {
            image_size,
            num_radials: sorted_indices.len() as u32,
            max_range: max_range_m as f32,
            scale: scale as f32,
            center: center as f32,
            num_color_entries: gpu_colors.len() as u32,
            _pad: [0; 2],
        };

        // Create GPU buffers
        let params_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params_buffer"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let radials_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radials_buffer"),
            contents: bytemuck::cast_slice(&radial_infos),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let gate_data_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gate_data_buffer"),
            contents: bytemuck::cast_slice(&all_gate_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let color_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("color_buffer"),
            contents: bytemuck::cast_slice(&gpu_colors),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let sorted_indices_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sorted_indices_buffer"),
            contents: bytemuck::cast_slice(&sorted_indices),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // Create output texture
        let output_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radar_output_texture"),
            size: wgpu::Extent3d {
                width: image_size,
                height: image_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create bind group
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar_compute_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: radials_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: gate_data_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: color_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: sorted_indices_buffer.as_entire_binding(),
                },
            ],
        });

        // Dispatch compute
        let workgroups_x = (image_size + 15) / 16;
        let workgroups_y = (image_size + 15) / 16;

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("radar_compute_encoder"),
        });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("radar_compute_pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }

        // Copy texture to readback buffer
        let bytes_per_pixel = 4u32;
        // wgpu requires rows to be aligned to 256 bytes
        let unpadded_bytes_per_row = image_size * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) / align * align;

        let readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radar_readback_buffer"),
            size: (padded_bytes_per_row * image_size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(image_size),
                },
            },
            wgpu::Extent3d {
                width: image_size,
                height: image_size,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Read back the pixels
        let buffer_slice = readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);

        match rx.recv() {
            Ok(Ok(())) => {}
            _ => {
                log::error!("GPU radar: failed to map readback buffer");
                return None;
            }
        }

        let data = buffer_slice.get_mapped_range();

        // Remove row padding if needed
        let mut pixels = Vec::with_capacity((image_size * image_size * bytes_per_pixel) as usize);
        for row in 0..image_size {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + (image_size * bytes_per_pixel) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }

        drop(data);
        readback_buffer.unmap();

        Some(RenderedSweep {
            pixels,
            width: image_size,
            height: image_size,
            center_lat: site.lat,
            center_lon: site.lon,
            range_km,
        })
    }
}
