//! Project creation and loading with scene paths confined to the project root.

use std::{
    error::Error,
    fmt, fs,
    path::{Component, Path, PathBuf},
};

use forge_core::Guid;
use forge_scene::{SceneDiagnostic, SceneDocument};
use serde::{Deserialize, Serialize};

pub const CURRENT_PROJECT_SCHEMA: u32 = 1;
pub const PROJECT_FILE_NAME: &str = "project.forgeproj";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDocument {
    pub schema_version: u32,
    pub resource_guid: Guid,
    pub name: String,
    pub default_scene: String,
    pub default_scene_guid: Guid,
}

#[derive(Debug)]
pub enum ProjectError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
    Scene(Vec<SceneDiagnostic>),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "project I/O failed: {error}"),
            Self::Json(error) => write!(f, "project JSON is invalid: {error}"),
            Self::Invalid(message) => f.write_str(message),
            Self::Scene(diagnostics) => {
                write!(f, "default scene has {} diagnostic(s)", diagnostics.len())
            }
        }
    }
}

impl Error for ProjectError {}

impl From<std::io::Error> for ProjectError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProjectError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct Project {
    root: PathBuf,
    manifest_path: PathBuf,
    document: ProjectDocument,
    default_scene: SceneDocument,
}

impl Project {
    pub fn create(root: impl AsRef<Path>, name: impl Into<String>) -> Result<Self, ProjectError> {
        let root = root.as_ref();
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let scene_directory = root.join("Scenes");
        fs::create_dir_all(&scene_directory)?;

        let scene = SceneDocument {
            schema_version: forge_scene::CURRENT_SCENE_SCHEMA,
            resource_guid: Guid::new(),
            name: "Main".into(),
            dependencies: Vec::new(),
            objects: Vec::new(),
        };
        let scene_path = scene_directory.join("Main.forgescene");
        fs::write(&scene_path, scene.to_canonical_json()?)?;

        let document = ProjectDocument {
            schema_version: CURRENT_PROJECT_SCHEMA,
            resource_guid: Guid::new(),
            name: name.into(),
            default_scene: "Scenes/Main.forgescene".into(),
            default_scene_guid: scene.resource_guid,
        };
        let manifest_path = root.join(PROJECT_FILE_NAME);
        fs::write(&manifest_path, serde_json::to_vec_pretty(&document)?)?;
        Ok(Self {
            root,
            manifest_path,
            document,
            default_scene: scene,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        let manifest_path = if path.is_dir() {
            path.join(PROJECT_FILE_NAME)
        } else {
            path.to_path_buf()
        };
        let canonical_manifest = manifest_path.canonicalize()?;
        let root = canonical_manifest
            .parent()
            .ok_or_else(|| {
                ProjectError::Invalid("project manifest has no parent directory".into())
            })?
            .to_path_buf();
        let document: ProjectDocument = serde_json::from_slice(&fs::read(&canonical_manifest)?)?;
        if document.schema_version != CURRENT_PROJECT_SCHEMA {
            return Err(ProjectError::Invalid(format!(
                "project schema {} is unsupported; this build reads schema {}",
                document.schema_version, CURRENT_PROJECT_SCHEMA
            )));
        }
        if document.name.trim().is_empty() {
            return Err(ProjectError::Invalid(
                "project name must not be empty".into(),
            ));
        }
        let relative_scene = Path::new(&document.default_scene);
        if relative_scene.is_absolute()
            || relative_scene.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ProjectError::Invalid(
                "default scene path must stay within the project folder".into(),
            ));
        }
        let scene_path = root.join(relative_scene);
        let canonical_scene = scene_path.canonicalize()?;
        if !canonical_scene.starts_with(&root) {
            return Err(ProjectError::Invalid(
                "default scene resolves outside the project folder".into(),
            ));
        }
        let default_scene = SceneDocument::from_json(&fs::read_to_string(canonical_scene)?)
            .map_err(ProjectError::Scene)?;
        if default_scene.resource_guid != document.default_scene_guid {
            return Err(ProjectError::Invalid(
                "default scene GUID does not match the project manifest".into(),
            ));
        }
        Ok(Self {
            root,
            manifest_path: canonical_manifest,
            document,
            default_scene,
        })
    }

    pub fn save(&self) -> Result<(), ProjectError> {
        fs::write(
            &self.manifest_path,
            serde_json::to_vec_pretty(&self.document)?,
        )?;
        Ok(())
    }

    pub fn document(&self) -> &ProjectDocument {
        &self.document
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn default_scene(&self) -> &SceneDocument {
        &self.default_scene
    }
}
