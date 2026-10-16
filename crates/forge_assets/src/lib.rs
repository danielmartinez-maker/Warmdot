//! Project-local asset discovery, bounded PNG decoding, and animation data.
//!
//! Imported file paths are always resolved beneath the canonical project root. The catalog
//! ignores symlinks, keeps stable GUIDs in the project metadata directory, and can be refreshed
//! by a runtime loop to observe hot updates.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use forge_core::Guid;
use forge_scene::PrefabDocument;
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};

pub const CURRENT_ASSET_INDEX_SCHEMA: u32 = 1;
pub const CURRENT_SPRITE_SHEET_SCHEMA: u32 = 1;
pub const CURRENT_ANIMATION_CLIP_SCHEMA: u32 = 1;
const MAX_PNG_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SCRIPT_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_AUDIO_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_UI_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_TEXTURE_DIMENSION: u32 = 16_384;
const MAX_TEXTURE_PIXELS: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Png,
    SpriteSheet,
    AnimationClip,
    Script,
    Prefab,
    Audio,
    Ui,
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
pub enum AudioFormat {
    Wav,
    Ogg,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedAudioAsset {
    pub guid: Guid,
    pub relative_path: String,
    pub format: AudioFormat,
    pub path: PathBuf,
    pub source_bytes: u64,
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

    /// Opens the asset catalog without creating folders, assigning GUIDs, or writing metadata.
    /// The checked-in index must exactly match the bounded scan of the project's Assets folder.
    pub fn open_read_only(project_root: impl AsRef<Path>) -> Result<Self, AssetError> {
        let project_root = project_root.as_ref().canonicalize()?;
        let assets_path = project_root.join("Assets");
        let assets_root = match fs::symlink_metadata(&assets_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(AssetError::Invalid(
                    "Assets must be a regular project folder".into(),
                ));
            }
            Ok(_) => {
                let canonical = assets_path.canonicalize()?;
                if !canonical.starts_with(&project_root) {
                    return Err(AssetError::Invalid(
                        "Assets directory resolves outside the project folder".into(),
                    ));
                }
                canonical
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => assets_path,
            Err(error) => return Err(AssetError::Io(error)),
        };

        let metadata_root = project_root.join(".forge");
        let index_path = metadata_root.join("asset-index.json");
        let mut records = BTreeMap::new();
        match fs::symlink_metadata(&metadata_root) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(AssetError::Invalid(
                    "asset metadata directory must be a regular project folder".into(),
                ));
            }
            Ok(_) => {
                let canonical_metadata = metadata_root.canonicalize()?;
                if !canonical_metadata.starts_with(&project_root) {
                    return Err(AssetError::Invalid(
                        "asset metadata directory resolves outside the project folder".into(),
                    ));
                }
                match fs::symlink_metadata(&index_path) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                        return Err(AssetError::Invalid(
                            "asset index must be a regular file".into(),
                        ));
                    }
                    Ok(metadata) => {
                        if metadata.len() > MAX_METADATA_BYTES {
                            return Err(AssetError::Invalid(format!(
                                "asset index exceeds the {} MiB loading limit",
                                MAX_METADATA_BYTES / (1024 * 1024)
                            )));
                        }
                        let bytes = fs::read(&index_path)?;
                        let document: AssetIndexDocument = serde_json::from_slice(&bytes)?;
                        if document.schema_version != CURRENT_ASSET_INDEX_SCHEMA {
                            return Err(AssetError::Invalid(format!(
                                "asset index schema {} is unsupported; this build reads schema {}",
                                document.schema_version, CURRENT_ASSET_INDEX_SCHEMA
                            )));
                        }
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
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(AssetError::Io(error)),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AssetError::Io(error)),
        }

        let mut discovered = BTreeMap::<String, (AssetKind, u64, u64)>::new();
        if assets_root.is_dir() {
            scan_directory(&project_root, &assets_root, &assets_root, &mut discovered)?;
        }

        let mut problems = BTreeSet::new();
        for (path, (kind, source_bytes, fingerprint)) in &discovered {
            match records.get(path) {
                None => {
                    problems.insert(format!("asset index is missing entry for {path}"));
                }
                Some(record) => {
                    if record.kind != *kind {
                        problems.insert(format!("asset index kind is stale for {path}"));
                    }
                    if record.source_bytes != *source_bytes {
                        problems.insert(format!("asset index byte count is stale for {path}"));
                    }
                    if record.fingerprint != *fingerprint {
                        problems.insert(format!("asset index fingerprint is stale for {path}"));
                    }
                }
            }
        }
        for path in records.keys() {
            if !discovered.contains_key(path) {
                problems.insert(format!("indexed asset is missing from Assets: {path}"));
            }
        }
        if !problems.is_empty() {
            return Err(AssetError::Invalid(
                problems.into_iter().collect::<Vec<_>>().join("; "),
            ));
        }

        let by_guid = records
            .iter()
            .map(|(path, record)| (record.guid, path.clone()))
            .collect();
        Ok(Self {
            project_root,
            assets_root,
            index_path,
            records,
            by_guid,
        })
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

    pub fn load_audio(&self, guid: Guid) -> Result<LoadedAudioAsset, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::Audio)?;
        let path = self.resolve_record_path(record)?;
        let source_bytes = fs::metadata(&path)?.len();
        if source_bytes > MAX_AUDIO_SOURCE_BYTES {
            return Err(AssetError::Invalid(format!(
                "{} is {source_bytes} bytes; maximum supported audio size is {MAX_AUDIO_SOURCE_BYTES}",
                record.relative_path
            )));
        }

        let mut file = fs::File::open(&path)?;
        let mut header = [0_u8; 12];
        file.read_exact(&mut header[..4])?;
        let extension = Path::new(&record.relative_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let format = match extension.as_str() {
            "wav" => {
                file.read_exact(&mut header[4..])?;
                if &header[..4] == b"RIFF" && &header[8..12] == b"WAVE" {
                    AudioFormat::Wav
                } else {
                    return Err(AssetError::Invalid(format!(
                        "{} has an invalid WAV container signature",
                        record.relative_path
                    )));
                }
            }
            "ogg" if &header[..4] == b"OggS" => AudioFormat::Ogg,
            "ogg" => {
                return Err(AssetError::Invalid(format!(
                    "{} has an invalid OGG container signature",
                    record.relative_path
                )));
            }
            _ => {
                return Err(AssetError::Invalid(format!(
                    "{} uses an unsupported audio format",
                    record.relative_path
                )));
            }
        };

        Ok(LoadedAudioAsset {
            guid,
            relative_path: record.relative_path.clone(),
            format,
            path,
            source_bytes,
        })
    }

    pub fn load_ui_source(&self, guid: Guid) -> Result<LoadedUiAsset, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::Ui)?;
        let bytes = read_limited(&self.resolve_record_path(record)?, MAX_UI_SOURCE_BYTES)?;
        let source = String::from_utf8(bytes).map_err(|error| {
            AssetError::Invalid(format!(
                "{} is not valid UTF-8: {error}",
                record.relative_path
            ))
        })?;
        Ok(LoadedUiAsset {
            guid,
            relative_path: record.relative_path.clone(),
            source,
        })
    }

    pub fn load_script(&self, guid: Guid) -> Result<LoadedScriptAsset, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::Script)?;
        let bytes = read_limited(&self.resolve_record_path(record)?, MAX_SCRIPT_SOURCE_BYTES)?;
        let source = String::from_utf8(bytes).map_err(|error| {
            AssetError::Invalid(format!(
                "{} is not valid UTF-8: {error}",
                record.relative_path
            ))
        })?;
        Ok(LoadedScriptAsset {
            guid,
            relative_path: record.relative_path.clone(),
            source,
        })
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

    pub fn load_prefab(&self, guid: Guid) -> Result<PrefabDocument, AssetError> {
        let record = self.record_of_kind(guid, AssetKind::Prefab)?;
        let bytes = read_limited(&self.resolve_record_path(record)?, MAX_METADATA_BYTES)?;
        let source = String::from_utf8(bytes).map_err(|error| {
            AssetError::Invalid(format!(
                "{} is not valid UTF-8: {error}",
                record.relative_path
            ))
        })?;
        let prefab = PrefabDocument::from_json(&source).map_err(|diagnostics| {
            AssetError::Invalid(
                diagnostics
                    .into_iter()
                    .map(|diagnostic| diagnostic.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        for dependency in &prefab.dependencies {
            if self.get(*dependency).is_none() {
                return Err(AssetError::Invalid(format!(
                    "{} references missing asset {dependency}",
                    record.relative_path
                )));
            }
        }
        Ok(prefab)
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
pub struct LoadedUiAsset {
    pub guid: Guid,
    pub relative_path: String,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedScriptAsset {
    pub guid: Guid,
    pub relative_path: String,
    pub source: String,
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
            AssetKind::SpriteSheet | AssetKind::AnimationClip | AssetKind::Prefab => {
                MAX_METADATA_BYTES
            }
            AssetKind::Script => MAX_SCRIPT_SOURCE_BYTES,
            AssetKind::Audio => MAX_AUDIO_SOURCE_BYTES,
            AssetKind::Ui => MAX_UI_SOURCE_BYTES,
        };
        let bytes = read_limited(&canonical, byte_limit)?;
        let record = (kind, bytes.len() as u64, fingerprint(&bytes));
        discovered.insert(relative_path, record);
    }
    Ok(())
}

fn asset_kind(path: &Path) -> Option<AssetKind> {
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if file_name.ends_with(".forgeui.json") {
        Some(AssetKind::Ui)
    } else if file_name.ends_with(".forgeprefab.json") {
        Some(AssetKind::Prefab)
    } else if file_name.ends_with(".forgeatlas.json") {
        Some(AssetKind::SpriteSheet)
    } else if file_name.ends_with(".forgeanim.json") {
        Some(AssetKind::AnimationClip)
    } else if path.extension()?.to_str()?.eq_ignore_ascii_case("png") {
        Some(AssetKind::Png)
    } else if path.extension()?.to_str()?.eq_ignore_ascii_case("luau") {
        Some(AssetKind::Script)
    } else if path.extension()?.to_str()?.eq_ignore_ascii_case("wav")
        || path.extension()?.to_str()?.eq_ignore_ascii_case("ogg")
    {
        Some(AssetKind::Audio)
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

#[cfg(test)]
mod script_asset_tests {
    use super::*;
    use std::fs;

    #[test]
    fn catalogs_and_reloads_luau_assets_without_changing_their_guid() {
        let project = tempfile::tempdir().expect("temporary project");
        let assets_dir = project.path().join("Assets");
        fs::create_dir_all(&assets_dir).expect("Assets folder");
        let script_path = assets_dir.join("Player.luau");
        fs::write(&script_path, "return { on_fixed_update = function() end }")
            .expect("write first script");

        let mut catalog = AssetCatalog::open(project.path()).expect("open asset catalog");
        let record = catalog
            .entries()
            .find(|record| record.relative_path == "Assets/Player.luau")
            .expect("script asset is indexed");
        assert_eq!(record.kind, AssetKind::Script);
        let guid = record.guid;
        let loaded = catalog.load_script(guid).expect("load script source");
        assert_eq!(loaded.guid, guid);
        assert_eq!(loaded.relative_path, "Assets/Player.luau");
        assert_eq!(loaded.source, "return { on_fixed_update = function() end }");

        fs::write(
            &script_path,
            "return { on_fixed_update = function(self, dt) end }",
        )
        .expect("write updated script");
        let changes = catalog.refresh().expect("refresh asset catalog");
        assert!(changes.iter().any(|change| change.guid == guid));
        assert_eq!(catalog.load_script(guid).expect("reload source").guid, guid);
    }
}

#[cfg(test)]
mod audio_asset_tests {
    use super::*;
    use std::fs;

    #[test]
    fn catalogs_audio_files_by_stable_guid_and_confines_the_resolved_path() {
        let project = tempfile::tempdir().expect("temporary project");
        let assets_dir = project.path().join("Assets");
        fs::create_dir_all(&assets_dir).expect("Assets folder");
        let wav_path = assets_dir.join("effect.wav");
        let mut wav = b"RIFF\x04\x00\x00\x00WAVE".to_vec();
        wav.extend_from_slice(b"fmt ");
        fs::write(&wav_path, wav).expect("write wav header");
        fs::write(assets_dir.join("music.ogg"), b"OggS\0\0\0\0").expect("write ogg header");

        let catalog = AssetCatalog::open(project.path()).expect("open asset catalog");
        let effect = catalog
            .entries()
            .find(|record| record.relative_path == "Assets/effect.wav")
            .expect("WAV should be indexed");
        assert_eq!(effect.kind, AssetKind::Audio);
        let loaded = catalog.load_audio(effect.guid).expect("load WAV metadata");
        assert_eq!(loaded.format, AudioFormat::Wav);
        assert!(
            loaded
                .path
                .starts_with(project.path().canonicalize().unwrap())
        );

        let music = catalog
            .entries()
            .find(|record| record.relative_path == "Assets/music.ogg")
            .expect("OGG should be indexed");
        assert_eq!(music.kind, AssetKind::Audio);
        assert_eq!(
            catalog
                .load_audio(music.guid)
                .expect("load OGG metadata")
                .format,
            AudioFormat::Ogg
        );
    }

    #[test]
    fn rejects_audio_with_a_mismatched_container_signature() {
        let project = tempfile::tempdir().expect("temporary project");
        let assets_dir = project.path().join("Assets");
        fs::create_dir_all(&assets_dir).expect("Assets folder");
        fs::write(assets_dir.join("broken.wav"), b"OggS not a wave file")
            .expect("write mismatched header");

        let catalog = AssetCatalog::open(project.path()).expect("open asset catalog");
        let audio = catalog
            .entries()
            .find(|record| record.relative_path == "Assets/broken.wav")
            .expect("WAV should be indexed");
        assert!(catalog.load_audio(audio.guid).is_err());
    }
}

#[cfg(test)]
mod ui_asset_tests {
    use super::*;
    use std::fs;

    #[test]
    fn catalogs_ui_documents_by_stable_guid_and_reloads_source() {
        let project = tempfile::tempdir().expect("temporary project");
        let assets_dir = project.path().join("Assets");
        fs::create_dir_all(&assets_dir).expect("Assets folder");
        let ui_path = assets_dir.join("Hud.forgeui.json");
        fs::write(&ui_path, r#"{"schema_version":1,"resource_guid":"00000000-0000-4000-8000-000000000001","name":"HUD"}"#)
            .expect("write UI document");

        let mut catalog = AssetCatalog::open(project.path()).expect("open asset catalog");
        let record = catalog
            .entries()
            .find(|record| record.relative_path == "Assets/Hud.forgeui.json")
            .expect("UI document is indexed");
        assert_eq!(record.kind, AssetKind::Ui);
        let guid = record.guid;
        let loaded = catalog.load_ui_source(guid).expect("load UI source");
        assert_eq!(loaded.guid, guid);
        assert!(loaded.source.contains("HUD"));

        fs::write(&ui_path, r#"{"schema_version":1,"resource_guid":"00000000-0000-4000-8000-000000000001","name":"HUD v2"}"#)
            .expect("write updated UI document");
        let changes = catalog.refresh().expect("refresh UI catalog");
        assert!(changes.iter().any(|change| change.guid == guid));
        assert!(
            catalog
                .load_ui_source(guid)
                .unwrap()
                .source
                .contains("HUD v2")
        );
    }
}


#[cfg(test)]
mod script_fixture_tests {
    use super::*;
    use std::fs;

    #[test]
    fn luau_asset_fixture_preserves_source_bytes_across_catalog_load() {
        let project = tempfile::tempdir().expect("temporary project");
        let assets_dir = project.path().join("Assets");
        fs::create_dir_all(&assets_dir).expect("Assets folder");
        let fixture = include_str!("../tests/fixtures/schema-v1/PlayerController.luau");
        let script_path = assets_dir.join("PlayerController.luau");
        fs::write(&script_path, fixture).expect("write script fixture");

        let catalog = AssetCatalog::open(project.path()).expect("open asset catalog");
        let asset = catalog
            .entries()
            .find(|record| record.relative_path == "Assets/PlayerController.luau")
            .expect("fixture is indexed as an asset");
        assert_eq!(asset.kind, AssetKind::Script);
        let loaded = catalog.load_script(asset.guid).expect("load script asset");
        assert_eq!(loaded.source.as_bytes(), fixture.as_bytes());
        assert_eq!(loaded.relative_path, "Assets/PlayerController.luau");
    }
}
