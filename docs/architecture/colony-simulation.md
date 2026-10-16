# Colony simulation sample

Run the sample with:

    cargo run -p colony-simulation

The sample starts with 64 workers, a central colony, four food caches, and a small grid with blocked cells. Workers take queued resource jobs, navigate around blocked cells with a deterministic breadth-first search, carry food to the colony, then queue the next job. The board uses a 30-by-16 grid with two world units per tile.

Left-click a tile to switch it between walkable and blocked. The home and food-cache tiles stay protected. Workers recalculate routes after each terrain edit. Press Space or B to add a worker up to the 256-worker cap. Press P to pause and 1, 2, or 3 to set simulation speed to 1x, 2x, or 3x.

The window title reports population, queued jobs, deliveries, frame rate, average runtime update cost, and object count. This remains a small sample: jobs and terrain are in memory, workers use a sample resource loop, and there is no save system, construction menu, or generalized pathfinding service.
