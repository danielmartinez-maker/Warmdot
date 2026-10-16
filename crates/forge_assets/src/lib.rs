//! Project-local asset discovery, bounded PNG decoding, and animation data.
//!
//! Imported file paths are always resolved beneath the canonical project root. The catalog
//! ignores symlinks, keeps stable GUIDs in the project metadata directory, and can be refreshed
//! by a runtime loop to observe hot updates.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use forge_core::Guid;
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};

pub const CURRENT_ASSET_INDEX_SCHEMA: u32 = 1;
pub const CURRENT_SPRITE_SHEET_SCHEMA: u32 = 1;
pub const CURRENT_ANIMATION_CLIP_SCHEMA: u32 = 1;
const MAX_PNG_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TEXTURE_DIMENSION: u32 = 16_384;
const MAX_TEXTURE_PIXELS: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Png,
    SpriteSheet,
    AnimationClip,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRecord {
    pub guid: Guid,
    pub relative_path: String,
    pub kind: AssetKind,
    pub source_bytes: u64,
    pub fingerprint: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetChangeKind {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetChange {
    pub kind: AssetChangeKind,
    pub guid: Guid,
    pub relative_path: String,
}

#[derive(Debug)]
pub enum AssetError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Image(image::ImageError),
    Invalid(String),
    UnknownGuid(Guid),
    WrongKind { guid: Guid, expected: AssetKind },
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "asset I/O failed: {error}"),
            Self::Json(error) => write!(f, "asset metadata is invalid: {error}"),
            Self::Image(error) => write!(f, "PNG decoding failed: {error}"),
            Self::Invalid(message) => f.write_str(message),
            Self::UnknownGuid(guid) => write!(f, "asset {guid} is not in this project catalog"),
            Self::WrongKind { guid, expected } => {
                write!(f, "asset {guid} is not a {expected:?} resource")
            }
        }
    }
}

impl Error for AssetError {}

impl From<std::io::Error> for AssetError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for AssetError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<image::ImageError> for AssetError {
    fn from(error: image::ImageError) -> Self {
        Self::Image(error)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssetIndexDocument {
    schema_version: u32,
    entries: Vec<AssetRecord>,
}

pub struct AssetCatalog {
    project_root: PathBuf,
    assets_root: PathBuf,
    index_path: PathBuf,
    records: BTreeMap<String, AssetRecord>,
    by_guid: BTreeMap<Guid, String>,
}

impl AssetCatalog {
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, AssetError> {
        let project_root = project_root.as_ref().canonicalize()?;
        let assets_root = project_root.join("Assets");
        fs::create_dir_all(&assets_root)?;
        let assets_root = assets_root.canonicalize()?;
        if !assets_root.starts_with(&project_root) {
            return Err(AssetError::Invalid(
                "Assets directory resolves outside the project folder".into(),
            ));
        }

        let metadata_root = project_root.join(".forge");
        fs::create_dir_all(&metadata_root)?;
        let metadata_root = metadata_root.canonicalize()?;
        if !metadata_root.starts_with(&project_root) {
            return Err(AssetError::Invalid(
                "asset metadata directory resolves outside the project folder".into(),
            ));
        }
        let index_path = metadata_root.join("asset-index.json");
        let index_exists = index_path.exists();
        let records = if index_exists {
            if fs::symlink_metadata(&index_path)?.file_type().is_symlink() {
                return Err(AssetError::Invalid(
                    "asset index must not be a symbolic link".into(),
                ));
            }
            let bytes = fs::read(&index_path)?;
            let document: AssetIndexDocument = serde_json::from_slice(&bytes)?;
            if document.schema_version != CURRENT_ASSET_INDEX_SCHEMA {
                return Err(AssetError::Invalid(format!(
                    "asset index schema {} is unsupported; this build reads schema {}",
                    document.schema_version, CURRENT_ASSET_INDEX_SCHEMA
                )));
            }
            let mut records = BTreeMap::new();
            let mut ids = BTreeSet::new();
            for record in document.entries {
                validate_relative_asset_path(&record.relative_path)?;
                if !ids.insert(record.guid)
                    || records
                        .insert(record.relative_path.clone(), record)
                        .is_some()
                {
                    return Err(AssetError::Invalid(
                        "asset index contains duplicate GUIDs or paths".into(),
                    ));
                }
            }
            records
        } else {
            BTreeMap::new()
        };
        let by_guid = records
            .iter()
            .map(|(path, record)| (record.guid, path.clone()))
            .collect();
        let mut catalog = Self {
            project_root,
            assets_root,
            index_path,
            records,
            by_guid,
        };
        catalog.refresh()?;
        if !index_exists {
            catalog.write_index()?;
        }
        Ok(catalog)
    }

    pub fn root(&self) -> &Path {
        &self.project_root
    }

    pub fn entries(&self) -> impl Iterator<Item = &AssetRecord> {
        self.records.values()
    }

    pub fn get(&self, guid: Guid) -> Option<&AssetRecord> {
        self.by_guid
            .get(&guid)
            .and_then(|path| self.records.get(path))
    }

    /// Rescans the Assets directory. Call from a development loop to observe hot file changes.
    pub fn refresh(&mut self) -> Result<Vec<AssetChange>, AssetError> {
        let mut discovered = BTreeMap::<String, (AssetKind, u64, u64)>::new();
        scan_directory(
            &self.project_root,
            &self.assets_root,
            &self.assets_root,
            &mut discovered,
        )?;

        let mut next = BTreeMap::new();
        let mut changes = Vec::new();
        for (path, (kind, source_bytes, fingerprint)) in discovered {
            let guid = self
                .records
                .get(&path)
                .map(|record| record.guid)
                .unwrap_or_else(Guid::new);
            let record = AssetRecord {
                guid,
                relative_path: path.clone(),
                kind,
                source_bytes,
                fingerprint,
            };
            match self.records.get(&path) {
                None => changes.push(AssetChange {
                    kind: AssetChangeKind::Added,
                    guid,
                    relative_path: path.clone(),
                }),
                Some(previous) if previous.fingerprint != fingerprint || previous.kind != kind => {
                    changes.push(AssetChange {
                        kind: AssetChangeKind::Modified,
                        guid,
                        relative_path: path.clone(),
                    });
                }
                Some(_) => {}
            }
            next.insert(path, record);
        }
        for (path, previous) in &self.records {
            if !next.contains_key(path) {
                changes.push(AssetChange {
                    kind: AssetChangeKind::Removed,
                    guid: previous.guid,
                    relative_path: path.clone(),
                });
            }
        }
        changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if !changes.is_empty() {
            self.records = next;
            self.by_guid = self
                .records
                .iter()
                .map(|(path, record)| (record.guid, path.clone()))
                .collect();
            self.write_index()?;
        }
        Ok(changes)
    }

    pub fn load_png(&self, guid: Guid) -> Result<DecodedTexture, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::Png)?;
        let path = self.resolve_record_path(record)?;
        let bytes = read_limited(&path, MAX_PNG_SOURCE_BYTES)?;
        if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
            return Err(AssetError::Invalid(format!(
                "{} is not a valid PNG source",
                record.relative_path
            )));
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four-byte width"));
        let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four-byte height"));
        if width == 0
            || height == 0
            || width > MAX_TEXTURE_DIMENSION
            || height > MAX_TEXTURE_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_TEXTURE_PIXELS
        {
            return Err(AssetError::Invalid(format!(
                "PNG dimensions {width}x{height} exceed the importer limits"
            )));
        }
        let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)?.into_rgba8();
        Ok(DecodedTexture {
            guid,
            width,
            height,
            rgba8: image.into_raw(),
        })
    }

    pub fn load_sprite_sheet(&self, guid: Guid) -> Result<SpriteSheetDocument, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::SpriteSheet)?;
        let bytes = read_limited(&self.resolve_record_path(record)?, MAX_METADATA_BYTES)?;
        let sheet: SpriteSheetDocument = serde_json::from_slice(&bytes)?;
        let texture = self.load_png(sheet.texture)?;
        sheet.validate(texture.width, texture.height)?;
        Ok(sheet)
    }

    pub fn load_animation_clip(&self, guid: Guid) -> Result<AnimationClipDocument, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::AnimationClip)?;
        let bytes = read_limited(&self.resolve_record_path(record)?, MAX_METADATA_BYTES)?;
        let clip: AnimationClipDocument = serde_json::from_slice(&bytes)?;
        let sheet_guid = clip.sprite_sheet;
        let sheet = self.load_sprite_sheet(sheet_guid)?;
        clip.validate(sheet_guid, &sheet)?;
        Ok(clip)
    }

    fn record_of_kind(&self, guid: Guid, expected: AssetKind) -> Result<&AssetRecord, AssetError> {
        let record = self.get(guid).ok_or(AssetError::UnknownGuid(guid))?;
        if record.kind != expected {
            return Err(AssetError::WrongKind { guid, expected });
        }
        Ok(record)
    }

    fn resolve_record_path(&self, record: &AssetRecord) -> Result<PathBuf, AssetError> {
        validate_relative_asset_path(&record.relative_path)?;
        let path = self.project_root.join(&record.relative_path);
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(&self.project_root) {
            return Err(AssetError::Invalid(format!(
                "asset {} resolves outside the project folder",
                record.relative_path
            )));
        }
        Ok(canonical)
    }

    fn write_index(&self) -> Result<(), AssetError> {
        let document = AssetIndexDocument {
            schema_version: CURRENT_ASSET_INDEX_SCHEMA,
            entries: self.records.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&document)?;
        let parent = self
            .index_path
            .parent()
            .ok_or_else(|| AssetError::Invalid("asset index has no parent directory".into()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary
            .persist(&self.index_path)
            .map_err(|error| AssetError::Io(error.error))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedTexture {
    pub guid: Guid,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

impl DecodedTexture {
    pub fn is_well_formed(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.rgba8.len() == self.width as usize * self.height as usize * 4
    }

    pub fn to_image_buffer(&self) -> Option<RgbaImage> {
        RgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteFrame {
    pub name: String,
    pub rect: AtlasRect,
    #[serde(default = "default_pivot")]
    pub pivot: [f32; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteSheetDocument {
    pub schema_version: u32,
    pub texture: Guid,
    pub frames: Vec<SpriteFrame>,
}

impl SpriteSheetDocument {
    pub fn validate(&self, texture_width: u32, texture_height: u32) -> Result<(), AssetError> {
        if self.schema_version != CURRENT_SPRITE_SHEET_SCHEMA {
            return Err(AssetError::Invalid(format!(
                "sprite-sheet schema {} is unsupported",
                self.schema_version
            )));
        }
        if self.frames.is_empty() {
            return Err(AssetError::Invalid(
                "sprite sheet must contain at least one frame".into(),
            ));
        }
        let mut names = BTreeSet::new();
        for frame in &self.frames {
            if frame.name.trim().is_empty() || !names.insert(frame.name.as_str()) {
                return Err(AssetError::Invalid(
                    "sprite-frame names must be non-empty and unique".into(),
                ));
            }
            if frame.rect.width == 0
                || frame.rect.height == 0
                || frame
                    .rect
                    .x
                    .checked_add(frame.rect.width)
                    .is_none_or(|right| right > texture_width)
                || frame
                    .rect
                    .y
                    .checked_add(frame.rect.height)
                    .is_none_or(|bottom| bottom > texture_height)
            {
                return Err(AssetError::Invalid(format!(
                    "sprite frame {} has a rectangle outside the texture",
                    frame.name
                )));
            }
            if !frame.pivot.iter().all(|value| value.is_finite()) {
                return Err(AssetError::Invalid(format!(
                    "sprite frame {} has a non-finite pivot",
                    frame.name
                )));
            }
        }
        Ok(())
    }

    pub fn uv_rect_for_frame(
        &self,
        frame_name: &str,
        texture_width: u32,
        texture_height: u32,
    ) -> Result<[f32; 4], AssetError> {
        self.validate(texture_width, texture_height)?;
        let frame = self
            .frames
            .iter()
            .find(|frame| frame.name == frame_name)
            .ok_or_else(|| {
                AssetError::Invalid(format!("sprite frame {frame_name} does not exist"))
            })?;
        Ok([
            frame.rect.x as f32 / texture_width as f32,
            frame.rect.y as f32 / texture_height as f32,
            frame.rect.width as f32 / texture_width as f32,
            frame.rect.height as f32 / texture_height as f32,
        ])
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationFrame {
    pub sprite_frame: String,
    pub duration_seconds: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationClipDocument {
    pub schema_version: u32,
    pub sprite_sheet: Guid,
    #[serde(default = "default_looping")]
    pub looping: bool,
    pub frames: Vec<AnimationFrame>,
}

impl AnimationClipDocument {
    pub fn validate(
        &self,
        sheet_guid: Guid,
        sheet: &SpriteSheetDocument,
    ) -> Result<(), AssetError> {
        if self.schema_version != CURRENT_ANIMATION_CLIP_SCHEMA {
            return Err(AssetError::Invalid(format!(
                "animation-clip schema {} is unsupported",
                self.schema_version
            )));
        }
        if self.sprite_sheet != sheet_guid {
            return Err(AssetError::Invalid(
                "animation clip does not reference the loaded sprite sheet".into(),
            ));
        }
        if self.frames.is_empty() {
            return Err(AssetError::Invalid(
                "animation clip must contain at least one frame".into(),
            ));
        }
        let frame_names = sheet
            .frames
            .iter()
            .map(|frame| frame.name.as_str())
            .collect::<BTreeSet<_>>();
        if self.frames.iter().any(|frame| {
            frame.sprite_frame.trim().is_empty()
                || !frame_names.contains(frame.sprite_frame.as_str())
                || !frame.duration_seconds.is_finite()
                || frame.duration_seconds <= 0.0
        }) {
            return Err(AssetError::Invalid(
                "animation frames must reference sprite-sheet frames and have finite positive durations".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationPlayer {
    frame_index: usize,
    elapsed_seconds: f64,
    playing: bool,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            frame_index: 0,
            elapsed_seconds: 0.0,
            playing: true,
        }
    }
}

impl AnimationPlayer {
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn current_frame_name<'a>(&self, clip: &'a AnimationClipDocument) -> Option<&'a str> {
        clip.frames
            .get(self.frame_index)
            .map(|frame| frame.sprite_frame.as_str())
    }

    pub fn current_frame_uv_rect(
        &self,
        clip: &AnimationClipDocument,
        sheet: &SpriteSheetDocument,
        texture_width: u32,
        texture_height: u32,
    ) -> Result<[f32; 4], AssetError> {
        let frame_name = self
            .current_frame_name(clip)
            .ok_or_else(|| AssetError::Invalid("animation clip contains no frames".into()))?;
        sheet.uv_rect_for_frame(frame_name, texture_width, texture_height)
    }

    pub fn advance(
        &mut self,
        clip: &AnimationClipDocument,
        delta: Duration,
    ) -> Result<usize, AssetError> {
        if clip.frames.is_empty()
            || clip
                .frames
                .iter()
                .any(|frame| !frame.duration_seconds.is_finite() || frame.duration_seconds <= 0.0)
        {
            return Err(AssetError::Invalid(
                "animation player requires finite positive frame durations".into(),
            ));
        }
        if !self.playing {
            self.frame_index = self.frame_index.min(clip.frames.len() - 1);
            return Ok(self.frame_index);
        }
        let total_duration = clip
            .frames
            .iter()
            .map(|frame| f64::from(frame.duration_seconds))
            .sum::<f64>();
        if !total_duration.is_finite() || total_duration <= 0.0 {
            return Err(AssetError::Invalid(
                "animation clip duration is invalid".into(),
            ));
        }
        let next_time = self.elapsed_seconds + delta.as_secs_f64();
        let clip_time = if clip.looping {
            next_time.rem_euclid(total_duration)
        } else {
            next_time.min(total_duration)
        };
        self.elapsed_seconds = clip_time;
        self.frame_index = frame_at_time(&clip.frames, clip_time);
        if !clip.looping && next_time >= total_duration {
            self.playing = false;
            self.frame_index = clip.frames.len() - 1;
        }
        Ok(self.frame_index)
    }

    pub fn reset(&mut self) {
        self.frame_index = 0;
        self.elapsed_seconds = 0.0;
        self.playing = true;
    }
}

fn frame_at_time(frames: &[AnimationFrame], time: f64) -> usize {
    let mut end = 0.0;
    for (index, frame) in frames.iter().enumerate() {
        end += f64::from(frame.duration_seconds);
        if time < end {
            return index;
        }
    }
    frames.len() - 1
}

fn scan_directory(
    project_root: &Path,
    assets_root: &Path,
    directory: &Path,
    discovered: &mut BTreeMap<String, (AssetKind, u64, u64)>,
) -> Result<(), AssetError> {
    let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let file_type = child.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            scan_directory(project_root, assets_root, &path, discovered)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(kind) = asset_kind(&path) else {
            continue;
        };
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(project_root) || !canonical.starts_with(assets_root) {
            continue;
        }
        let relative = path
            .strip_prefix(project_root)
            .map_err(|_| AssetError::Invalid("asset path escaped project root".into()))?;
        let relative_path = relative
            .to_str()
            .ok_or_else(|| AssetError::Invalid("asset path is not valid UTF-8".into()))?
            .replace('\\', "/");
        validate_relative_asset_path(&relative_path)?;
        let byte_limit = match kind {
            AssetKind::Png => MAX_PNG_SOURCE_BYTES,
            AssetKind::SpriteSheet | AssetKind::AnimationClip => MAX_METADATA_BYTES,
        };
        let bytes = read_limited(&canonical, byte_limit)?;
        let record = (kind, bytes.len() as u64, fingerprint(&bytes));
        discovered.insert(relative_path, record);
    }
    Ok(())
}

fn asset_kind(path: &Path) -> Option<AssetKind> {
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if file_name.ends_with(".forgeatlas.json") {
        Some(AssetKind::SpriteSheet)
    } else if file_name.ends_with(".forgeanim.json") {
        Some(AssetKind::AnimationClip)
    } else if path.extension()?.to_str()?.eq_ignore_ascii_case("png") {
        Some(AssetKind::Png)
    } else {
        None
    }
}

fn validate_relative_asset_path(path: &str) -> Result<(), AssetError> {
    if path.is_empty() || path.contains('\\') || Path::new(path).is_absolute() {
        return Err(AssetError::Invalid(
            "asset path must be a normalized project-relative path".into(),
        ));
    }
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AssetError::Invalid(
            "asset path must not contain traversal or root components".into(),
        ));
    }
    if !path.starts_with("Assets/") {
        return Err(AssetError::Invalid(
            "asset path must stay inside the Assets folder".into(),
        ));
    }
    Ok(())
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, AssetError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit {
        return Err(AssetError::Invalid(format!(
            "asset source is {} bytes; maximum supported size is {limit}",
            metadata.len()
        )));
    }
    Ok(fs::read(path)?)
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn default_pivot() -> [f32; 2] {
    [0.5, 0.5]
}

fn default_looping() -> bool {
    true
}
