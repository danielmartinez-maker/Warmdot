//! Sparse 32x32 tilemap chunks, deterministic documents, and undoable paint edits.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use forge_core::Guid;
use serde::{Deserialize, Serialize};

pub const CHUNK_SIDE: i32 = 32;
pub const CHUNK_AREA: usize = (CHUNK_SIDE * CHUNK_SIDE) as usize;
pub const CURRENT_TILE_MAP_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TileCell {
    pub tile_index: u32,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    #[serde(default)]
    pub rotation_quarters: u8,
    #[serde(default)]
    pub solid: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TileEntry {
    pub local_index: u16,
    pub cell: TileCell,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TileChunkDocument {
    pub coordinate: ChunkCoord,
    pub entries: Vec<TileEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TileMapDocument {
    pub schema_version: u32,
    pub resource_guid: Guid,
    pub tileset_guid: Guid,
    /// Tile cell dimensions in world units.
    pub tile_size: [f32; 2],
    /// Tile dimensions in pixels within the referenced PNG tileset.
    #[serde(default = "default_texture_tile_size")]
    pub texture_tile_size: [u32; 2],
    pub chunks: Vec<TileChunkDocument>,
}

#[derive(Debug)]
pub enum TileMapError {
    Json(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for TileMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "tilemap JSON is invalid: {error}"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl Error for TileMapError {}

impl From<serde_json::Error> for TileMapError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl TileMapDocument {
    pub fn empty(
        resource_guid: Guid,
        tileset_guid: Guid,
        tile_size: [f32; 2],
    ) -> Result<Self, TileMapError> {
        let document = Self {
            schema_version: CURRENT_TILE_MAP_SCHEMA,
            resource_guid,
            tileset_guid,
            tile_size,
            texture_tile_size: default_texture_tile_size(),
            chunks: Vec::new(),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn from_json(source: &str) -> Result<Self, TileMapError> {
        let document: Self = serde_json::from_str(source)?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_canonical_json(&self) -> Result<String, TileMapError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(TileMapError::Json)
    }

    pub fn validate(&self) -> Result<(), TileMapError> {
        if self.schema_version != CURRENT_TILE_MAP_SCHEMA {
            return Err(TileMapError::Invalid(format!(
                "tilemap schema {} is unsupported; this build reads schema {}",
                self.schema_version, CURRENT_TILE_MAP_SCHEMA
            )));
        }
        if !self
            .tile_size
            .iter()
            .all(|value| value.is_finite() && *value > 0.0)
        {
            return Err(TileMapError::Invalid(
                "tile width and height must be finite positive values".into(),
            ));
        }
        if self.texture_tile_size.contains(&0) {
            return Err(TileMapError::Invalid(
                "tileset pixel width and height must be positive".into(),
            ));
        }
        let mut chunks = BTreeSet::new();
        for chunk in &self.chunks {
            if !chunks.insert(chunk.coordinate) {
                return Err(TileMapError::Invalid(
                    "tilemap contains a duplicate chunk coordinate".into(),
                ));
            }
            if chunk.entries.is_empty() {
                return Err(TileMapError::Invalid(
                    "empty chunks must be omitted from tilemap documents".into(),
                ));
            }
            let mut local_indices = BTreeSet::new();
            for entry in &chunk.entries {
                if usize::from(entry.local_index) >= CHUNK_AREA
                    || !local_indices.insert(entry.local_index)
                    || entry.cell.rotation_quarters > 3
                {
                    return Err(TileMapError::Invalid(
                        "tile entry has a duplicate or out-of-range cell index or rotation".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn instantiate(&self) -> Result<ChunkedTileMap, TileMapError> {
        self.validate()?;
        let mut map = ChunkedTileMap::new();
        for chunk in &self.chunks {
            for entry in &chunk.entries {
                let local = usize::from(entry.local_index);
                let x = local % CHUNK_SIDE as usize;
                let y = local / CHUNK_SIDE as usize;
                let coordinate = TileCoord {
                    x: chunk
                        .coordinate
                        .x
                        .checked_mul(CHUNK_SIDE)
                        .and_then(|base| base.checked_add(x as i32))
                        .ok_or_else(|| {
                            TileMapError::Invalid(
                                "tilemap x coordinate overflows world space".into(),
                            )
                        })?,
                    y: chunk
                        .coordinate
                        .y
                        .checked_mul(CHUNK_SIDE)
                        .and_then(|base| base.checked_add(y as i32))
                        .ok_or_else(|| {
                            TileMapError::Invalid(
                                "tilemap y coordinate overflows world space".into(),
                            )
                        })?,
                };
                map.set_tile(coordinate, Some(entry.cell));
            }
        }
        Ok(map)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileChange {
    pub coordinate: TileCoord,
    pub before: Option<TileCell>,
    pub after: Option<TileCell>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TileEdit {
    changes: Vec<TileChange>,
}

impl TileEdit {
    pub fn changes(&self) -> &[TileChange] {
        &self.changes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    fn apply(&self, map: &mut ChunkedTileMap, after: bool) {
        for change in &self.changes {
            map.set_tile(
                change.coordinate,
                if after { change.after } else { change.before },
            );
        }
    }
}

pub struct ChunkedTileMap {
    chunks: BTreeMap<ChunkCoord, Box<[Option<TileCell>; CHUNK_AREA]>>,
    tile_count: usize,
}

impl Default for ChunkedTileMap {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkedTileMap {
    pub fn new() -> Self {
        Self {
            chunks: BTreeMap::new(),
            tile_count: 0,
        }
    }

    pub fn get(&self, coordinate: TileCoord) -> Option<TileCell> {
        let (chunk, local) = split_coordinate(coordinate);
        self.chunks.get(&chunk).and_then(|cells| cells[local])
    }

    pub fn set_tile(&mut self, coordinate: TileCoord, cell: Option<TileCell>) -> Option<TileCell> {
        let (chunk_coordinate, local_index) = split_coordinate(coordinate);
        let previous = self
            .chunks
            .get(&chunk_coordinate)
            .and_then(|cells| cells[local_index]);
        if previous == cell {
            return previous;
        }
        if previous.is_none() && cell.is_some() {
            self.tile_count += 1;
        } else if previous.is_some() && cell.is_none() {
            self.tile_count -= 1;
        }

        let remove_chunk = {
            let cells = self
                .chunks
                .entry(chunk_coordinate)
                .or_insert_with(|| Box::new([None; CHUNK_AREA]));
            cells[local_index] = cell;
            cell.is_none() && cells.iter().all(Option::is_none)
        };
        if remove_chunk {
            self.chunks.remove(&chunk_coordinate);
        }
        previous
    }

    /// Applies many cells as one edit, keeping each coordinate's original value only once.
    pub fn paint(
        &mut self,
        cells: impl IntoIterator<Item = (TileCoord, Option<TileCell>)>,
    ) -> TileEdit {
        let mut changes = BTreeMap::<TileCoord, TileChange>::new();
        for (coordinate, after) in cells {
            changes
                .entry(coordinate)
                .and_modify(|change| change.after = after)
                .or_insert_with(|| TileChange {
                    coordinate,
                    before: self.get(coordinate),
                    after,
                });
        }
        let changes = changes
            .into_values()
            .filter(|change| change.before != change.after)
            .collect::<Vec<_>>();
        for change in &changes {
            self.set_tile(change.coordinate, change.after);
        }
        TileEdit { changes }
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn tile_count(&self) -> usize {
        self.tile_count
    }

    pub fn iter(&self) -> impl Iterator<Item = (TileCoord, TileCell)> + '_ {
        self.chunks.iter().flat_map(|(chunk, cells)| {
            cells.iter().enumerate().filter_map(move |(index, cell)| {
                cell.map(|cell| {
                    let x = index % CHUNK_SIDE as usize;
                    let y = index / CHUNK_SIDE as usize;
                    (
                        TileCoord {
                            x: chunk.x * CHUNK_SIDE + x as i32,
                            y: chunk.y * CHUNK_SIDE + y as i32,
                        },
                        cell,
                    )
                })
            })
        })
    }

    pub fn to_document(
        &self,
        resource_guid: Guid,
        tileset_guid: Guid,
        tile_size: [f32; 2],
    ) -> Result<TileMapDocument, TileMapError> {
        let chunks = self
            .chunks
            .iter()
            .map(|(coordinate, cells)| TileChunkDocument {
                coordinate: *coordinate,
                entries: cells
                    .iter()
                    .enumerate()
                    .filter_map(|(local_index, cell)| {
                        cell.map(|cell| TileEntry {
                            local_index: local_index as u16,
                            cell,
                        })
                    })
                    .collect(),
            })
            .collect();
        let document = TileMapDocument {
            schema_version: CURRENT_TILE_MAP_SCHEMA,
            resource_guid,
            tileset_guid,
            tile_size,
            texture_tile_size: default_texture_tile_size(),
            chunks,
        };
        document.validate()?;
        Ok(document)
    }
}

#[derive(Clone, Debug)]
pub struct TileMapHistory {
    edits: Vec<TileEdit>,
    state_ids: Vec<u64>,
    cursor: usize,
    saved_state: Option<u64>,
    next_state_id: u64,
    max_entries: usize,
}

impl TileMapHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            edits: Vec::new(),
            state_ids: vec![0],
            cursor: 0,
            saved_state: Some(0),
            next_state_id: 1,
            max_entries: max_entries.max(1),
        }
    }

    pub fn execute_stroke(
        &mut self,
        map: &mut ChunkedTileMap,
        cells: impl IntoIterator<Item = (TileCoord, Option<TileCell>)>,
    ) -> bool {
        let edit = map.paint(cells);
        if edit.is_empty() {
            return false;
        }
        self.edits.truncate(self.cursor);
        self.state_ids.truncate(self.cursor + 1);
        self.edits.push(edit);
        self.cursor += 1;
        let state_id = self.next_state_id;
        self.next_state_id = self.next_state_id.saturating_add(1);
        self.state_ids.push(state_id);
        if self.edits.len() > self.max_entries {
            self.edits.remove(0);
            self.state_ids.remove(0);
            self.cursor -= 1;
        }
        true
    }

    pub fn undo(&mut self, map: &mut ChunkedTileMap) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.edits[self.cursor].apply(map, false);
        true
    }

    pub fn redo(&mut self, map: &mut ChunkedTileMap) -> bool {
        if self.cursor >= self.edits.len() {
            return false;
        }
        self.edits[self.cursor].apply(map, true);
        self.cursor += 1;
        true
    }

    pub fn mark_saved(&mut self) {
        self.saved_state = self.state_ids.get(self.cursor).copied();
    }

    pub fn is_dirty(&self) -> bool {
        self.state_ids.get(self.cursor).copied() != self.saved_state
    }

    pub fn undo_depth(&self) -> usize {
        self.cursor
    }

    pub fn redo_depth(&self) -> usize {
        self.edits.len() - self.cursor
    }
}

fn default_texture_tile_size() -> [u32; 2] {
    [16, 16]
}

fn split_coordinate(coordinate: TileCoord) -> (ChunkCoord, usize) {
    let chunk_x = coordinate.x.div_euclid(CHUNK_SIDE);
    let chunk_y = coordinate.y.div_euclid(CHUNK_SIDE);
    let local_x = coordinate.x.rem_euclid(CHUNK_SIDE) as usize;
    let local_y = coordinate.y.rem_euclid(CHUNK_SIDE) as usize;
    (
        ChunkCoord {
            x: chunk_x,
            y: chunk_y,
        },
        local_y * CHUNK_SIDE as usize + local_x,
    )
}

#[cfg(test)]
mod schema_fixture_tests {
    use super::*;

    #[test]
    fn schema_v1_tilemap_fixture_round_trips_negative_chunk_coordinates() {
        let document = TileMapDocument::from_json(include_str!(
            "../tests/fixtures/schema-v1/world.forgetilemap"
        ))
        .expect("load tilemap fixture");
        assert_eq!(document.chunks[0].coordinate, ChunkCoord { x: -1, y: 2 });
        assert_eq!(document.chunks[0].entries[0].local_index, 1023);

        let canonical = document
            .to_canonical_json()
            .expect("serialize tilemap fixture");
        let restored = TileMapDocument::from_json(&canonical).expect("reload tilemap fixture");
        assert_eq!(
            canonical,
            restored
                .to_canonical_json()
                .expect("serialize restored tilemap")
        );
        assert_eq!(restored.chunks[0].entries[0].cell.tile_index, 7);
    }
}
