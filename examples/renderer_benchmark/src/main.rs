use std::{env, error::Error, sync::Arc, time::Instant};

use forge_renderer::{Camera, GpuRenderer, RenderOutcome, SpriteInstance};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[derive(Clone, Copy)]
struct Options {
    sprites: usize,
    warmup_frames: usize,
    sample_frames: usize,
    width: u32,
    height: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sprites: 10_000,
            warmup_frames: 120,
            sample_frames: 600,
            width: 1280,
            height: 720,
        }
    }
}

impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut options = Self::default();
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value after {flag}"))?;
            match flag.as_str() {
                "--sprites" => options.sprites = value.parse()?,
                "--warmup" => options.warmup_frames = value.parse()?,
                "--samples" => options.sample_frames = value.parse()?,
                "--width" => options.width = value.parse()?,
                "--height" => options.height = value.parse()?,
                _ => return Err(format!("unknown option {flag}").into()),
            }
        }
        if options.sprites == 0 || options.sprites > 1_000_000 {
            return Err("--sprites must be between 1 and 1000000".into());
        }
        if options.sample_frames == 0 || options.sample_frames > 60_000 {
            return Err("--samples must be between 1 and 60000".into());
        }
        if options.width == 0 || options.height == 0 {
            return Err("--width and --height must be positive".into());
        }
        Ok(options)
    }
}

struct BenchmarkState {
    window: Arc<Window>,
    renderer: GpuRenderer,
    sprites: Vec<SpriteInstance>,
    camera: Camera,
    options: Options,
    warmup_complete: usize,
    samples_ms: Vec<f64>,
}

impl BenchmarkState {
    fn new(window: Arc<Window>, renderer: GpuRenderer, options: Options) -> Self {
        let size = window.inner_size();
        let pixels_per_world_unit = 12.0;
        let camera = Camera {
            pixels_per_world_unit,
            ..Camera::default()
        };
        let sprites = make_sprites(
            options.sprites,
            size.width.max(1),
            size.height.max(1),
            pixels_per_world_unit,
        );
        Self {
            window,
            renderer,
            sprites,
            camera,
            options,
            warmup_complete: 0,
            samples_ms: Vec::with_capacity(options.sample_frames),
        }
    }

    fn draw_frame(&mut self) -> Result<Option<String>, Box<dyn Error>> {
        let started = Instant::now();
        let outcome =
            self.renderer
                .render(&self.sprites, self.camera, [0.025, 0.035, 0.055, 1.0])?;
        let frame_time_ms = started.elapsed().as_secs_f64() * 1000.0;
        if outcome == RenderOutcome::Skipped {
            return Ok(None);
        }
        if self.warmup_complete < self.options.warmup_frames {
            self.warmup_complete += 1;
            return Ok(None);
        }
        self.samples_ms.push(frame_time_ms);
        if self.samples_ms.len() < self.options.sample_frames {
            return Ok(None);
        }
        Ok(Some(self.report()?))
    }

    fn report(&self) -> Result<String, serde_json::Error> {
        let mut sorted = self.samples_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
        let median = percentile(&sorted, 0.50);
        let report = serde_json::json!({
            "schema_version": 1,
            "benchmark": "forge2d_instanced_sprite_batch",
            "adapter": {
                "name": self.renderer.adapter().name,
                "device_type": self.renderer.adapter().device_type,
                "backend": self.renderer.adapter().backend,
                "driver": self.renderer.adapter().driver,
                "driver_info": self.renderer.adapter().driver_info
            },
            "sprites_per_frame": self.sprites.len(),
            "warmup_frames": self.warmup_complete,
            "sample_frames": self.samples_ms.len(),
            "timing_scope": "CPU wall time around surface acquisition, command encoding, queue submission, and present enqueue; it excludes GPU completion time.",
            "gpu_timestamp_queries": "not measured",
            "cpu_frame_time_ms": {
                "min": sorted[0],
                "mean": mean,
                "p50": median,
                "p95": percentile(&sorted, 0.95),
                "p99": percentile(&sorted, 0.99),
                "max": sorted[sorted.len() - 1]
            },
            "fps_at_p50_cpu_time": if median > 0.0 { 1000.0 / median } else { 0.0 },
            "raw_cpu_frame_time_ms": self.samples_ms
        });
        serde_json::to_string_pretty(&report)
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((fraction * sorted.len() as f64).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

fn make_sprites(
    count: usize,
    width: u32,
    height: u32,
    pixels_per_world_unit: f32,
) -> Vec<SpriteInstance> {
    let aspect = width as f64 / height as f64;
    let columns = ((count as f64 * aspect).sqrt().ceil() as usize).max(1);
    let rows = count.div_ceil(columns);
    let world_width = width as f32 / pixels_per_world_unit;
    let world_height = height as f32 / pixels_per_world_unit;
    let cell_width = world_width / columns as f32;
    let cell_height = world_height / rows as f32;

    (0..count)
        .map(|index| {
            let column = index % columns;
            let row = index / columns;
            let hash = (index as u64).wrapping_mul(2_654_435_761);
            let tint = [
                0.32 + (hash & 0xff) as f32 / 510.0,
                0.32 + ((hash >> 8) & 0xff) as f32 / 510.0,
                0.32 + ((hash >> 16) & 0xff) as f32 / 510.0,
                1.0,
            ];
            SpriteInstance {
                position: [
                    (column as f32 + 0.5 - columns as f32 * 0.5) * cell_width,
                    (row as f32 + 0.5 - rows as f32 * 0.5) * cell_height,
                ],
                size: [cell_width * 0.76, cell_height * 0.76],
                scale: [1.0, 1.0],
                rotation_radians: (index % 16) as f32 * 0.0125,
                tint,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
            }
        })
        .collect()
}

struct App {
    options: Options,
    state: Option<BenchmarkState>,
}

impl App {
    fn new(options: Options) -> Self {
        Self {
            options,
            state: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Forge 2D renderer benchmark")
            .with_inner_size(PhysicalSize::new(self.options.width, self.options.height));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("could not create benchmark window"),
        );
        match GpuRenderer::new(event_loop.owned_display_handle(), window.clone()) {
            Ok(renderer) => {
                self.state = Some(BenchmarkState::new(window.clone(), renderer, self.options));
                window.request_redraw();
            }
            Err(error) => {
                eprintln!("renderer initialization failed: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.renderer.resize(size.width, size.height),
            WindowEvent::RedrawRequested => match state.draw_frame() {
                Ok(Some(report)) => {
                    println!("{report}");
                    event_loop.exit();
                }
                Ok(None) => state.window.request_redraw(),
                Err(error) => {
                    eprintln!("benchmark rendering failed: {error}");
                    event_loop.exit();
                }
            },
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse()?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut App::new(options))?;
    Ok(())
}
