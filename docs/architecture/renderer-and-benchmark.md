# Renderer and benchmark contract

The first graphics slice is a native wgpu surface renderer for the Forge runtime. Runtime sprite and active-camera snapshots are copied into Forge-owned render types, ordered by layer/order/stable object ID, then submitted as one instanced quad batch. This slice renders solid tint colors; it does not yet decode or sample PNG textures.

## Benchmark

Run the renderer-benchmark executable on a desktop with a working graphics adapter:

    cargo run --release -p renderer-benchmark -- --sprites 10000 --warmup 120 --samples 600

The default is 10,000 sprites per frame, 120 successful warm-up frames, and 600 successful measured frames at 1280x720. The width, height, sprites, warmup, and samples options can be overridden.

The JSON report includes GPU adapter/backend/driver details, p50/p95/p99, min/mean/max CPU frame time, p50-derived FPS, and every raw sample. The timed interval covers surface acquisition, CPU command encoding, queue submission, and present enqueue. It does not wait for GPU completion and does not include GPU timestamp queries; it must not be presented as GPU frame time or as a hardware-independent engine throughput result.

The run is interactive and should stay visible and unoccluded. Frames skipped because the surface is temporarily unavailable are not included in warm-up or measurement counts. The executable prints no result if the window closes before it collects the requested samples.
