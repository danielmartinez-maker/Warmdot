//! Native wgpu renderer for textured and tinted 2D sprite quads.
//!
//! The renderer keeps the window and surface on the host side, extracts Forge-owned draw
//! snapshots, sorts by authored layer/order/stable object ID, then submits contiguous texture
//! batches with instanced quad draws. Textures are registered by stable asset GUID.

use std::{collections::HashMap, error::Error, fmt, mem::size_of, sync::Arc};

use bytemuck::{Pod, Zeroable};
use forge_core::Guid;
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
    pub anchor: [f32; 2],
    pub tint: [f32; 4],
    pub uv_rect: [f32; 4],
}

impl From<ExtractedSprite> for SpriteInstance {
    fn from(sprite: ExtractedSprite) -> Self {
        Self {
            position: sprite.translation,
            size: sprite.size,
            scale: sprite.scale,
            rotation_radians: sprite.rotation_radians,
            anchor: sprite.anchor,
            tint: sprite.tint,
            uv_rect: sprite.uv_rect,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexturedSprite {
    pub instance: SpriteInstance,
    pub texture_asset: Option<Guid>,
}

impl From<ExtractedSprite> for TexturedSprite {
    fn from(sprite: ExtractedSprite) -> Self {
        Self {
            instance: SpriteInstance::from(sprite),
            texture_asset: sprite.asset,
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
    InvalidTexture(&'static str),
    UnknownTexture(Guid),
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
            Self::InvalidTexture(message) => write!(f, "invalid sprite texture: {message}"),
            Self::UnknownTexture(guid) => {
                write!(f, "sprite texture asset {guid} is not registered")
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

struct UploadedTexture {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
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
    texture_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
    white_texture: wgpu::Texture,
    white_bind_group: wgpu::BindGroup,
    textures: HashMap<Guid, UploadedTexture>,
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
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forge2d-texture-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("forge2d-nearest-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let white_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("forge2d-white-fallback-texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_view = white_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let white_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forge2d-white-fallback-bind-group"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&white_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&texture_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forge2d-sprite-pipeline-layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let instance_attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32x2,
            3 => Float32,
            4 => Float32x2,
            5 => Float32x4,
            6 => Float32x4
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
            texture_layout,
            texture_sampler,
            white_texture,
            white_bind_group,
            textures: HashMap::new(),
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

    /// Uploads or replaces an RGBA8 image under its stable project asset GUID.
    pub fn register_texture(
        &mut self,
        guid: Guid,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> Result<(), RendererError> {
        let max_dimension = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max_dimension || height > max_dimension {
            return Err(RendererError::InvalidTexture(
                "dimensions must be non-zero and supported by the graphics device",
            ));
        }
        let expected_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(RendererError::InvalidTexture("image byte count overflowed"))?;
        if u64::try_from(rgba8.len()).ok() != Some(expected_bytes) {
            return Err(RendererError::InvalidTexture(
                "RGBA8 byte length does not match image dimensions",
            ));
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("forge2d-asset-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba8,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("forge2d-asset-texture-bind-group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.texture_sampler),
                },
            ],
        });
        self.textures.insert(
            guid,
            UploadedTexture {
                _texture: texture,
                bind_group,
            },
        );
        Ok(())
    }

    /// Removes a project image from GPU memory. Sprites still referencing it return an error.
    pub fn remove_texture(&mut self, guid: Guid) -> bool {
        self.textures.remove(&guid).is_some()
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
            .map(TexturedSprite::from)
            .collect::<Vec<_>>();
        self.render_textured(&sprites, camera, clear_color)
    }

    pub fn render_textured(
        &mut self,
        sprites: &[TexturedSprite],
        camera: Camera,
        clear_color: [f64; 4],
    ) -> Result<RenderOutcome, RendererError> {
        let instances = sprites
            .iter()
            .map(|sprite| sprite.instance)
            .collect::<Vec<_>>();
        let texture_assets = sprites
            .iter()
            .map(|sprite| sprite.texture_asset)
            .collect::<Vec<_>>();
        self.render_internal(&instances, Some(&texture_assets), camera, clear_color)
    }

    pub fn render(
        &mut self,
        sprites: &[SpriteInstance],
        camera: Camera,
        clear_color: [f64; 4],
    ) -> Result<RenderOutcome, RendererError> {
        self.render_internal(sprites, None, camera, clear_color)
    }

    fn render_internal(
        &mut self,
        sprites: &[SpriteInstance],
        texture_assets: Option<&[Option<Guid>]>,
        camera: Camera,
        clear_color: [f64; 4],
    ) -> Result<RenderOutcome, RendererError> {
        let instance_count =
            u32::try_from(sprites.len()).map_err(|_| RendererError::TooManySprites)?;
        if let Some(texture_assets) = texture_assets {
            if texture_assets.len() != sprites.len() {
                return Err(RendererError::InvalidTexture(
                    "texture assignments do not match the sprite count",
                ));
            }
            for asset in texture_assets.iter().flatten() {
                if !self.textures.contains_key(asset) {
                    return Err(RendererError::UnknownTexture(*asset));
                }
            }
        }
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
            let mut start = 0;
            while start < instance_count {
                let asset =
                    texture_assets.and_then(|assets| assets.get(start as usize).copied().flatten());
                let bind_group = match asset {
                    Some(guid) => {
                        &self
                            .textures
                            .get(&guid)
                            .expect("texture assets were validated before rendering")
                            .bind_group
                    }
                    None => &self.white_bind_group,
                };
                pass.set_bind_group(1, bind_group, &[]);
                let mut end = start + 1;
                while end < instance_count {
                    let next_asset = texture_assets
                        .and_then(|assets| assets.get(end as usize).copied().flatten());
                    if next_asset != asset {
                        break;
                    }
                    end += 1;
                }
                pass.draw(0..6, start..end);
                start = end;
            }
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
