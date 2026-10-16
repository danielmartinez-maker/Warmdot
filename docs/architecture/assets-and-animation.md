# Assets and 2D animation

The project asset catalog scans the project Assets directory for PNG images, sprite-sheet metadata, and animation-clip metadata. It skips symlinks, rejects paths outside the project root, caps source file sizes, and assigns stable GUIDs stored in .forge/asset-index.json. Calling refresh compares a deterministic FNV-1a fingerprint and returns added, modified, and removed records for development hot updates.

PNG decoding is limited to 16,384 pixels per side and 32 million total pixels before RGBA allocation. Sprite-sheet frame rectangles must fit inside the referenced texture and frame names must be unique. Animation clips must reference existing sprite frames and specify finite positive durations.

AnimationPlayer advances from simulation time, handles large delta values without stepping once per frame interval, loops clips, and stops non-looping clips on their last frame. Rendering these decoded texture pixels and animation clips through the GPU sprite batch is a later integration step.
