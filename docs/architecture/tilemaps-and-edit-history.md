# Tilemaps and edit history

Tilemaps store occupied cells in sparse 32 by 32 chunks. Chunk coordinates use Euclidean division, so negative world cells map to a stable non-negative local index. Empty chunks are removed when the last tile is erased.

Tilemap documents use a versioned JSON schema. Chunks and cell entries serialize deterministically in sorted coordinate/index order. Validation rejects duplicate chunks/cells, empty serialized chunks, invalid tile sizes, out-of-range indices, and rotations outside zero to three quarter-turns.

A paint stroke records the original and final value once per touched coordinate. TileMapHistory applies each stroke as one undo entry, supports redo, caps retained edits, and tracks the saved state even when edits branch after an undo. The editor UI and renderer integration are separate remaining engine work.
