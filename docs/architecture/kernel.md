# Runtime kernel contracts

## Ownership

Scene and project documents are versioned authoring data. Runtime objects live in Forge's world API and are compiled from scenes. A persisted object is addressed by its stable GUID; a runtime entity handle is valid only for the current world instance.

The ECS implementation is an internal adapter. Public systems use Forge component and world types rather than Bevy ECS types. The Rapier implementation is similarly hidden behind Forge's body, collider, event, and raycast types.

## Coordinates and physics

World coordinates use +X right and +Y down. The default physics gravity is therefore [0, 9.81]. Distances are world units. Body angular velocity and transform rotation are radians; collider dimensions are scaled by the absolute object scale. A circle under non-uniform scale uses the larger axis so the resulting circle encloses the requested scaled dimensions.

Fixed update systems run in registration order. Input events due at the current simulation time are applied before systems. Systems update world state before the physics step. Collision events produced by that step are delivered to systems on the following fixed tick, which gives gameplay systems a stable event barrier.

## Time

The clock uses an accumulator and a fixed simulation interval. It caps accepted frame time and limits fixed steps per display frame. Additional whole ticks are dropped and reported; the remainder stays below one step and becomes the interpolation fraction. This prevents a long stall from causing unbounded catch-up work.

## Boundaries still to implement

P0 still needs the native render host, texture importer, sprite animation, Luau, an editor command bus, tilemap editing, audio, retained game UI, and a reproducible standalone build pipeline. Simulation-scale chunked worlds and navigation are later milestones.
