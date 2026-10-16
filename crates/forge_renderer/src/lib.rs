//! Native wgpu renderer for solid, tinted 2D sprite quads.
//!
//! The renderer keeps the window and surface on the host side, extracts Forge-owned draw
//! snapshots, sorts by authored layer/order/stable object ID, then submits the sprite batch in
//! one instanced draw call. Asset-backed textures are a later slice; this first renderer colors
//! quads.

use std::{error::Error, fmt, mem::size_of, sync::Arc};

use bytemuck::{Pod, Zeroable};
use forge_world::{ExtractedCamera, ExtractedSprite, RuntimeWorld};
use wgpu::util::DeviceExt;
use winit::{event_loop::OwnedDisplayHandle, window::Window};

#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct SpriteInstance {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub scale: [f32; 2],
    pub rotation_radians: f32,
    pub tint: [f32; 4],
}

impl From<ExtractedSprite> for SpriteInstance {
    fn from(sprite: ExtractedSprite) -> Self {
        Self {
            position: sprite.translation,
            size: sprite.size,
            scale: sprite.scale,
            rotation_radians: sprite.rotation_radians,
            tint: sprite.tint,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: [f32; 2],
    pub rotation_radians: f32,
    pub zoom: [f32; 2],
    pub pixels_per_world_unit: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            rotation_radians: 0.0,
            zoom: [1.0, 1.0],
            pixels_per_world_unit: 1.0,
        }
    }
}

impl From<ExtractedCamera> for Camera {
    fn from(camera: ExtractedCamera) -> Self {
        Self {
            position: camera.translation,
            rotation_radians: camera.rotation_radians,
            zoom: camera.zoom,
            pixels_per_world_unit: camera.pixels_per_world_unit,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
}

#[derive(Debug)]
pub enum RendererError {
    Surface(String),
    Adapter(String),
    Device(String),
    TooManySprites,
}

impl fmt::Display for RendererError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Surface(message) => write!(f, "renderer surface error: {message}"),
            Self::Adapter(message) => write!(f, "no compatible graphics adapter: {message}"),
            Self::Device(message) => write!(f, "graphics device request failed: {message}"),
            Self::TooManySprites => {
                f.write_str("sprite batch exceeds the supported instance count")
            }
        }
    }
}

impl Error for RendererError {}

#[derive(Clone, Debug)]
pub struct AdapterSummary {
    pub name: String,
    pub device_type: String,
    pub backend: String,
    pub driver: String,
    pub driver_info: String,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    viewport: [f32; 4],
    camera: [f32; 4],
}

pub struct GpuRenderer {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    configuration: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    camera_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,
    adapter: AdapterSummary,
}

impl GpuRenderer {
    pub fn new(display: OwnedDisplayHandle, window: Arc<Window>) -> Result<Self, RendererError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| RendererError::Surface(error.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|error| RendererError::Adapter(error.to_string()))?;
        let adapter_info = adapter.get_info();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|error| RendererError::Device(error.to_string()))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format =
            capabilities.formats.first().copied().ok_or_else(|| {
                RendererError::Surface("surface reports no supported formats".into())
            })?;
        let present_mode = capabilities
            .present_modes
            .first()
            .copied()
            .unwrap_or(wgpu::PresentMode::AutoVsync);
        let size = window.inner_size();
        let configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: Vec::new(),
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode,
        };

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forge2d-sprite-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forge2d-camera-uniform"),
            contents: bytemuck::bytes_of(&CameraUniform {
                viewport: [
                    configuration.width as f32,
                    configuration.height as f32,
                    1.0,
                    1.0,
                ],
                camera: [0.0, 0.0, 0.0, 1.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forge2d-camera-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forge2d-camera-bind-group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forge2d-sprite-pipeline-layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let instance_attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32x2,
            3 => Float32,
            4 => Float32x4
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forge2d-sprite-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<SpriteInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let instance_capacity = size_of::<SpriteInstance>() as u64;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("forge2d-sprite-instances"),
            size: instance_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut renderer = Self {
            instance,
            surface,
            window,
            device,
            queue,
            configuration,
            pipeline,
            camera_bind_group,
            camera_buffer,
            instance_buffer,
            instance_capacity,
            adapter: AdapterSummary {
                name: adapter_info.name,
                device_type: format!("{:?}", adapter_info.device_type),
                backend: format!("{:?}", adapter_info.backend),
                driver: adapter_info.driver,
                driver_info: adapter_info.driver_info,
            },
        };
        renderer.configure_surface();
        Ok(renderer)
    }

    pub fn adapter(&self) -> &AdapterSummary {
        &self.adapter
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.configuration.width = width;
        self.configuration.height = height;
        self.configure_surface();
    }

    pub fn render_world(
        &mut self,
        world: &mut RuntimeWorld,
        clear_color: [f64; 4],
    ) -> Result<RenderOutcome, RendererError> {
        let camera = world.extract_camera().map(Camera::from).unwrap_or_default();
        let sprites = world
            .extract_sprites()
            .into_iter()
            .map(SpriteInstance::from)
            .collect::<Vec<_>>();
        self.render(&sprites, camera, clear_color)
    }

    pub fn render(
        &mut self,
        sprites: &[SpriteInstance],
        camera: Camera,
        clear_color: [f64; 4],
    ) -> Result<RenderOutcome, RendererError> {
        let instance_count =
            u32::try_from(sprites.len()).map_err(|_| RendererError::TooManySprites)?;
        let required_bytes = sprites
            .len()
            .checked_mul(size_of::<SpriteInstance>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(RendererError::TooManySprites)?;
        if required_bytes > self.instance_capacity {
            let capacity = required_bytes
                .checked_next_power_of_two()
                .unwrap_or(required_bytes)
                .max(size_of::<SpriteInstance>() as u64);
            self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("forge2d-sprite-instances"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_capacity = capacity;
        }
        if required_bytes != 0 {
            self.queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(sprites));
        }
        let uniform = CameraUniform {
            viewport: [
                self.configuration.width as f32,
                self.configuration.height as f32,
                camera.zoom[0],
                camera.zoom[1],
            ],
            camera: [
                camera.position[0],
                camera.position[1],
                camera.rotation_radians,
                camera.pixels_per_world_unit,
            ],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => {
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                drop(texture);
                self.configure_surface();
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure_surface();
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RendererError::Surface(
                    "surface acquisition reported a validation error".into(),
                ));
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .map_err(|error| RendererError::Surface(error.to_string()))?;
                self.configure_surface();
                return Ok(RenderOutcome::Skipped);
            }
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("forge2d-sprite-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forge2d-sprite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear_color[0],
                            g: clear_color[1],
                            b: clear_color[2],
                            a: clear_color[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
            pass.draw(0..6, 0..instance_count);
        }
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(surface_texture);
        Ok(RenderOutcome::Presented)
    }

    fn configure_surface(&mut self) {
        self.surface.configure(&self.device, &self.configuration);
    }
}
