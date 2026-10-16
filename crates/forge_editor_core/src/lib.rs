//! Validated scene editing operations with bounded undo/redo history.

use std::{error::Error, fmt};

use forge_core::Guid;
use forge_scene::{SceneDiagnostic, SceneDocument, SceneObject};

#[derive(Debug)]
pub enum EditorError {
    Scene(Vec<SceneDiagnostic>),
    ObjectNotFound(Guid),
    DuplicateObject(Guid),
    InvalidCommand(&'static str),
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(diagnostics) => {
                write!(
                    f,
                    "scene edit produced {} validation diagnostic(s)",
                    diagnostics.len()
                )
            }
            Self::ObjectNotFound(guid) => write!(f, "scene object {guid} was not found"),
            Self::DuplicateObject(guid) => write!(f, "scene already contains object {guid}"),
            Self::InvalidCommand(message) => f.write_str(message),
        }
    }
}

impl Error for EditorError {}

#[derive(Clone, Debug)]
enum SceneCommand {
    Insert {
        index: usize,
        object: SceneObject,
    },
    Remove {
        index: usize,
        object: SceneObject,
    },
    Replace {
        index: usize,
        before: SceneObject,
        after: SceneObject,
    },
}

impl SceneCommand {
    fn apply(&self, scene: &mut SceneDocument) -> Result<(), EditorError> {
        match self {
            Self::Insert { index, object } => {
                if scene
                    .objects
                    .iter()
                    .any(|item| item.object_guid == object.object_guid)
                {
                    return Err(EditorError::DuplicateObject(object.object_guid));
                }
                if *index > scene.objects.len() {
                    return Err(EditorError::InvalidCommand(
                        "insert position is outside the scene",
                    ));
                }
                scene.objects.insert(*index, object.clone());
                validate_or_restore(scene, |scene| {
                    scene.objects.remove(*index);
                })
            }
            Self::Remove { index, object } => {
                ensure_object_at(scene, *index, object.object_guid)?;
                let removed = scene.objects.remove(*index);
                validate_or_restore(scene, |scene| {
                    scene.objects.insert(*index, removed);
                })
            }
            Self::Replace {
                index,
                before,
                after,
            } => {
                ensure_object_at(scene, *index, before.object_guid)?;
                if before.object_guid != after.object_guid {
                    return Err(EditorError::InvalidCommand(
                        "replacement must preserve the authored object GUID",
                    ));
                }
                let original = std::mem::replace(&mut scene.objects[*index], after.clone());
                validate_or_restore(scene, |scene| {
                    scene.objects[*index] = original;
                })
            }
        }
    }

    fn revert(&self, scene: &mut SceneDocument) -> Result<(), EditorError> {
        match self {
            Self::Insert { index, object } => {
                ensure_object_at(scene, *index, object.object_guid)?;
                let removed = scene.objects.remove(*index);
                validate_or_restore(scene, || {
                    scene.objects.insert(*index, removed);
                })
            }
            Self::Remove { index, object } => {
                if scene
                    .objects
                    .iter()
                    .any(|item| item.object_guid == object.object_guid)
                {
                    return Err(EditorError::DuplicateObject(object.object_guid));
                }
                if *index > scene.objects.len() {
                    return Err(EditorError::InvalidCommand(
                        "restore position is outside the scene",
                    ));
                }
                scene.objects.insert(*index, object.clone());
                validate_or_restore(scene, || {
                    scene.objects.remove(*index);
                })
            }
            Self::Replace {
                index,
                before,
                after,
            } => {
                ensure_object_at(scene, *index, after.object_guid)?;
                let original = std::mem::replace(&mut scene.objects[*index], before.clone());
                validate_or_restore(scene, || {
                    scene.objects[*index] = original;
                })
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SceneEditHistory {
    commands: Vec<SceneCommand>,
    state_ids: Vec<u64>,
    cursor: usize,
    saved_state: Option<u64>,
    next_state_id: u64,
    max_entries: usize,
}

impl SceneEditHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            commands: Vec::new(),
            state_ids: vec![0],
            cursor: 0,
            saved_state: Some(0),
            next_state_id: 1,
            max_entries: max_entries.max(1),
        }
    }

    pub fn create_object(
        &mut self,
        scene: &mut SceneDocument,
        object: SceneObject,
        index: Option<usize>,
    ) -> Result<(), EditorError> {
        let index = index.unwrap_or(scene.objects.len());
        let command = SceneCommand::Insert { index, object };
        command.apply(scene)?;
        self.record(command);
        Ok(())
    }

    pub fn delete_object(
        &mut self,
        scene: &mut SceneDocument,
        guid: Guid,
    ) -> Result<(), EditorError> {
        let index = scene
            .objects
            .iter()
            .position(|object| object.object_guid == guid)
            .ok_or(EditorError::ObjectNotFound(guid))?;
        let command = SceneCommand::Remove {
            index,
            object: scene.objects[index].clone(),
        };
        command.apply(scene)?;
        self.record(command);
        Ok(())
    }

    pub fn replace_object(
        &mut self,
        scene: &mut SceneDocument,
        replacement: SceneObject,
    ) -> Result<(), EditorError> {
        let guid = replacement.object_guid;
        let index = scene
            .objects
            .iter()
            .position(|object| object.object_guid == guid)
            .ok_or(EditorError::ObjectNotFound(guid))?;
        let command = SceneCommand::Replace {
            index,
            before: scene.objects[index].clone(),
            after: replacement,
        };
        command.apply(scene)?;
        self.record(command);
        Ok(())
    }

    pub fn undo(&mut self, scene: &mut SceneDocument) -> Result<bool, EditorError> {
        if self.cursor == 0 {
            return Ok(false);
        }
        self.commands[self.cursor - 1].revert(scene)?;
        self.cursor -= 1;
        Ok(true)
    }

    pub fn redo(&mut self, scene: &mut SceneDocument) -> Result<bool, EditorError> {
        if self.cursor == self.commands.len() {
            return Ok(false);
        }
        self.commands[self.cursor].apply(scene)?;
        self.cursor += 1;
        Ok(true)
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
        self.commands.len() - self.cursor
    }

    fn record(&mut self, command: SceneCommand) {
        self.commands.truncate(self.cursor);
        self.state_ids.truncate(self.cursor + 1);
        self.commands.push(command);
        self.cursor += 1;
        let state_id = self.next_state_id;
        self.next_state_id = self.next_state_id.saturating_add(1);
        self.state_ids.push(state_id);
        if self.commands.len() > self.max_entries {
            self.commands.remove(0);
            self.state_ids.remove(0);
            self.cursor -= 1;
        }
    }
}

fn ensure_object_at(
    scene: &SceneDocument,
    index: usize,
    expected_guid: Guid,
) -> Result<(), EditorError> {
    if scene
        .objects
        .get(index)
        .is_some_and(|object| object.object_guid == expected_guid)
    {
        Ok(())
    } else {
        Err(EditorError::InvalidCommand(
            "scene changed outside the recorded edit history",
        ))
    }
}

fn validate_or_restore(
    scene: &mut SceneDocument,
    restore: impl FnOnce(&mut SceneDocument),
) -> Result<(), EditorError> {
    if let Err(diagnostics) = scene.validate() {
        restore(scene);
        return Err(EditorError::Scene(diagnostics));
    }
    Ok(())
}
