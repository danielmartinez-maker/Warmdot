# Colony simulation sample

Run the sample with:

    cargo run -p colony-simulation

The window starts with 64 workers, a central colony, and four food caches. Workers travel between their assigned cache and the colony on Forge fixed ticks. A completed return trip increments the delivered count shown in the window title.

Press Space or B to add a worker. The population is capped at 256. This sample demonstrates a second runtime loop and ECS-backed crowd simulation; it does not include persisted resources, construction, pathfinding around obstacles, or a standalone game UI.
