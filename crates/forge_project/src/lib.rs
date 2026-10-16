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
                write!(f, "scene has {} diagnostic(s)", diagnostics.len())
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

    pub fn save_default_scene(&mut self, scene: &SceneDocument) -> Result<(), ProjectError> {
        scene.validate().map_err(ProjectError::Scene)?;
        if scene.resource_guid != self.document.default_scene_guid {
            return Err(ProjectError::Invalid(
                "saved scene GUID does not match the project default scene".into(),
            ));
        }
        let relative_scene = Path::new(&self.document.default_scene);
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
        let scene_path = self.root.join(relative_scene);
        let parent = scene_path
            .parent()
            .ok_or_else(|| ProjectError::Invalid("default scene has no parent directory".into()))?;
        let canonical_parent = parent.canonicalize()?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(ProjectError::Invalid(
                "default scene directory resolves outside the project folder".into(),
            ));
        }
        match fs::symlink_metadata(&scene_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ProjectError::Invalid(
                    "default scene file must not be a symbolic link".into(),
                ));
            }
            Ok(_) => {
                let canonical_scene = scene_path.canonicalize()?;
                if !canonical_scene.starts_with(&self.root) {
                    return Err(ProjectError::Invalid(
                        "default scene resolves outside the project folder".into(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ProjectError::Io(error)),
        }
        fs::write(&scene_path, scene.to_canonical_json()?)?;
        self.default_scene = scene.clone();
        Ok(())
    }

    /// Loads any scene resource whose path remains inside the project root.
    pub fn load_scene(&self, relative_path: &str) -> Result<SceneDocument, ProjectError> {
        let relative_scene = Path::new(relative_path);
        validate_scene_relative_path(relative_scene)?;
        let scene_path = self.root.join(relative_scene);
        let metadata = fs::symlink_metadata(&scene_path)?;
        if metadata.file_type().is_symlink() {
            return Err(ProjectError::Invalid(
                "scene file must not be a symbolic link".into(),
            ));
        }
        let canonical_scene = scene_path.canonicalize()?;
        if !canonical_scene.starts_with(&self.root) {
            return Err(ProjectError::Invalid(
                "scene resolves outside the project folder".into(),
            ));
        }
        SceneDocument::from_json(&fs::read_to_string(canonical_scene)?).map_err(ProjectError::Scene)
    }

    /// Saves a scene resource to a project-relative path, creating its parent folders as needed.
    pub fn save_scene(
        &self,
        relative_path: &str,
        scene: &SceneDocument,
    ) -> Result<(), ProjectError> {
        scene.validate().map_err(ProjectError::Scene)?;
        let relative_scene = Path::new(relative_path);
        validate_scene_relative_path(relative_scene)?;
        let scene_path = self.root.join(relative_scene);
        let parent = scene_path
            .parent()
            .ok_or_else(|| ProjectError::Invalid("scene path has no parent directory".into()))?;
        create_confined_scene_parent(&self.root, parent)?;
        let canonical_parent = parent.canonicalize()?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(ProjectError::Invalid(
                "scene directory resolves outside the project folder".into(),
            ));
        }
        match fs::symlink_metadata(&scene_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ProjectError::Invalid(
                    "scene file must not be a symbolic link".into(),
                ));
            }
            Ok(_) => {
                let canonical_scene = scene_path.canonicalize()?;
                if !canonical_scene.starts_with(&self.root) {
                    return Err(ProjectError::Invalid(
                        "scene resolves outside the project folder".into(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ProjectError::Io(error)),
        }
        fs::write(scene_path, scene.to_canonical_json()?)?;
        Ok(())
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

fn create_confined_scene_parent(root: &Path, parent: &Path) -> Result<(), ProjectError> {
    let relative_parent = parent
        .strip_prefix(root)
        .map_err(|_| ProjectError::Invalid("scene parent is outside the project".into()))?;
    let mut current = root.to_path_buf();
    for component in relative_parent.components() {
        let Component::Normal(name) = component else {
            return Err(ProjectError::Invalid(
                "scene parent must be a normalized project-relative directory".into(),
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ProjectError::Invalid(
                    "scene parent folders must not be symbolic links".into(),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ProjectError::Invalid(
                    "scene parent path contains a non-directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = fs::create_dir(&current) {
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(ProjectError::Io(error));
                    }
                }
            }
            Err(error) => return Err(ProjectError::Io(error)),
        }
        let canonical_current = current.canonicalize()?;
        if !canonical_current.starts_with(root) {
            return Err(ProjectError::Invalid(
                "scene parent resolves outside the project folder".into(),
            ));
        }
    }
    Ok(())
}

fn validate_scene_relative_path(path: &Path) -> Result<(), ProjectError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ProjectError::Invalid(
            "scene path must be a normalized project-relative path".into(),
        ));
    }
    Ok(())
}
