//! Versioned, reviewable scene authoring data compiled into Forge runtime objects.

use std::collections::{BTreeMap, BTreeSet};

use forge_core::Guid;
use forge_world::{
    Camera2D, Collider2D, ColliderShape2D, RigidBody2D, RigidBodyKind2D, RuntimeWorld, Sprite,
    Transform2D,
};
use serde::{Deserialize, Serialize};

pub const CURRENT_SCENE_SCHEMA: u32 = 1;
pub const CURRENT_PREFAB_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDocument {
    pub schema_version: u32,
    pub resource_guid: Guid,
    pub name: String,
    #[serde(default)]
    pub dependencies: Vec<Guid>,
    #[serde(default)]
    pub objects: Vec<SceneObject>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrefabInstance {
    pub prefab_asset: Guid,
    #[serde(default)]
    pub overrides: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrefabDocument {
    pub schema_version: u32,
    pub resource_guid: Guid,
    pub name: String,
    #[serde(default)]
    pub dependencies: Vec<Guid>,
    pub object: SceneObject,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneObject {
    pub object_guid: Guid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_guid: Option<Guid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab_instance: Option<PrefabInstance>,
    #[serde(default)]
    pub transform: AuthoredTransform,
    #[serde(default)]
    pub sprite: Option<AuthoredSprite>,
    #[serde(default)]
    pub camera: Option<AuthoredCamera>,
    #[serde(default)]
    pub rigid_body: Option<AuthoredRigidBody>,
    #[serde(default)]
    pub collider: Option<AuthoredCollider>,
    #[serde(default)]
    pub script: Option<AuthoredScript>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredScript {
    pub asset: Guid,
    #[serde(default)]
    pub properties: BTreeMap<String, ScriptPropertyValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ScriptPropertyValue {
    Bool(bool),
    Number(f64),
    String(String),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredTransform {
    #[serde(default = "unit_scale")]
    pub scale: [f32; 2],
    #[serde(default)]
    pub translation: [f32; 2],
    #[serde(default)]
    pub rotation_radians: f32,
}

impl Default for AuthoredTransform {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0],
            rotation_radians: 0.0,
            scale: unit_scale(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSprite {
    #[serde(default)]
    pub asset: Option<Guid>,
    #[serde(default)]
    pub animation_clip: Option<Guid>,
    #[serde(default = "unit_scale")]
    pub size: [f32; 2],
    #[serde(default = "center_anchor")]
    pub anchor: [f32; 2],
    #[serde(default = "white_tint")]
    pub tint: [f32; 4],
    /// Normalized image coordinates stored as [u, v, width, height].
    #[serde(default = "full_uv_rect")]
    pub uv_rect: [f32; 4],
    #[serde(default)]
    pub layer: i32,
    #[serde(default)]
    pub order: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredCamera {
    #[serde(default = "unit_scale")]
    pub zoom: [f32; 2],
    #[serde(default = "unit_f32")]
    pub pixels_per_world_unit: f32,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_active")]
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRigidBody {
    pub kind: AuthoredRigidBodyKind,
    #[serde(default)]
    pub linear_velocity: [f32; 2],
    #[serde(default)]
    pub angular_velocity: f32,
    #[serde(default = "unit_f32")]
    pub gravity_scale: f32,
    #[serde(default)]
    pub linear_damping: f32,
    #[serde(default)]
    pub angular_damping: f32,
    #[serde(default)]
    pub lock_rotation: bool,
}

impl Default for AuthoredRigidBody {
    fn default() -> Self {
        Self {
            kind: AuthoredRigidBodyKind::Fixed,
            linear_velocity: [0.0, 0.0],
            angular_velocity: 0.0,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            lock_rotation: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredRigidBodyKind {
    Fixed,
    Dynamic,
    KinematicPositionBased,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredCollider {
    pub shape: AuthoredColliderShape,
    #[serde(default)]
    pub offset: [f32; 2],
    #[serde(default)]
    pub rotation_radians: f32,
    #[serde(default = "unit_f32")]
    pub density: f32,
    #[serde(default = "default_friction")]
    pub friction: f32,
    #[serde(default)]
    pub restitution: f32,
    #[serde(default)]
    pub sensor: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthoredColliderShape {
    Circle { radius: f32 },
    Box { half_extents: [f32; 2] },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SceneDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub object_guid: Option<Guid>,
}

impl std::fmt::Display for SceneDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:?} {}: {}",
            self.severity, self.code, self.message
        )?;
        if let Some(object_guid) = self.object_guid {
            write!(formatter, " (object {:?})", object_guid)?;
        }
        Ok(())
    }
}

impl SceneDiagnostic {
    fn error(code: &str, message: impl Into<String>, object_guid: Option<Guid>) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            code: code.into(),
            message: message.into(),
            object_guid,
        }
    }
}

impl SceneDocument {
    pub fn from_json(source: &str) -> Result<Self, Vec<SceneDiagnostic>> {
        let document: Self = serde_json::from_str(source).map_err(|error| {
            vec![SceneDiagnostic::error(
                "invalid_scene_json",
                error.to_string(),
                None,
            )]
        })?;
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), Vec<SceneDiagnostic>> {
        let mut diagnostics = Vec::new();
        if self.schema_version != CURRENT_SCENE_SCHEMA {
            diagnostics.push(SceneDiagnostic::error(
                "unsupported_scene_schema",
                format!(
                    "scene schema {} is unsupported; this build reads schema {}",
                    self.schema_version, CURRENT_SCENE_SCHEMA
                ),
                None,
            ));
        }
        if self.name.trim().is_empty() {
            diagnostics.push(SceneDiagnostic::error(
                "empty_scene_name",
                "scene name must not be empty",
                None,
            ));
        }

        let mut dependencies = BTreeSet::new();
        for dependency in &self.dependencies {
            if !dependencies.insert(*dependency) {
                diagnostics.push(SceneDiagnostic::error(
                    "duplicate_dependency",
                    format!("dependency {dependency} is listed more than once"),
                    None,
                ));
            }
        }

        let mut objects = BTreeSet::new();
        let mut hierarchy_valid = true;
        for object in &self.objects {
            let guid = object.object_guid;
            if !objects.insert(guid) {
                hierarchy_valid = false;
                diagnostics.push(SceneDiagnostic::error(
                    "duplicate_object_guid",
                    format!("object GUID {guid} appears more than once"),
                    Some(guid),
                ));
            }
            if object.name.trim().is_empty() {
                diagnostics.push(SceneDiagnostic::error(
                    "empty_object_name",
                    "object name must not be empty",
                    Some(guid),
                ));
            }
            if !object
                .transform
                .translation
                .iter()
                .all(|value| value.is_finite())
                || !object.transform.scale.iter().all(|value| value.is_finite())
                || !object.transform.rotation_radians.is_finite()
            {
                diagnostics.push(SceneDiagnostic::error(
                    "invalid_transform",
                    "transform values must be finite",
                    Some(guid),
                ));
            }
            if let Some(sprite) = object.sprite {
                if !sprite
                    .size
                    .iter()
                    .all(|value| value.is_finite() && *value > 0.0)
                    || !sprite.anchor.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value))
                    || !sprite.tint.iter().all(|value| value.is_finite())
                    || !sprite.uv_rect.iter().all(|value| value.is_finite())
                    || sprite.uv_rect[0] < 0.0
                    || sprite.uv_rect[1] < 0.0
                    || sprite.uv_rect[2] <= 0.0
                    || sprite.uv_rect[3] <= 0.0
                    || sprite.uv_rect[0] + sprite.uv_rect[2] > 1.0
                    || sprite.uv_rect[1] + sprite.uv_rect[3] > 1.0
                {
                    diagnostics.push(SceneDiagnostic::error(
                        "invalid_sprite",
                        "sprite size must be positive and finite, and tint values must be finite",
                        Some(guid),
                    ));
                }
                if let Some(asset) = sprite.asset {
                    if !dependencies.contains(&asset) {
                        diagnostics.push(SceneDiagnostic::error(
                            "undeclared_asset_dependency",
                            format!("sprite asset {asset} is missing from scene dependencies"),
                            Some(guid),
                        ));
                    }
                }
                if let Some(clip) = sprite.animation_clip {
                    if !dependencies.contains(&clip) {
                        diagnostics.push(SceneDiagnostic::error(
                            "undeclared_animation_dependency",
                            format!("animation clip {clip} is missing from scene dependencies"),
                            Some(guid),
                        ));
                    }
                }
            }
            if let Some(instance) = &object.prefab_instance {
                if !dependencies.contains(&instance.prefab_asset) {
                    diagnostics.push(SceneDiagnostic::error(
                        "undeclared_prefab_dependency",
                        format!(
                            "prefab asset {} is missing from scene dependencies",
                            instance.prefab_asset
                        ),
                        Some(guid),
                    ));
                }
                if instance
                    .overrides
                    .iter()
                    .any(|identifier| identifier.trim().is_empty() || identifier.len() > 128)
                {
                    diagnostics.push(SceneDiagnostic::error(
                        "invalid_prefab_override",
                        "prefab override identifiers must be non-empty and at most 128 bytes",
                        Some(guid),
                    ));
                }
            }
            if let Some(script) = &object.script {
                if !dependencies.contains(&script.asset) {
                    diagnostics.push(SceneDiagnostic::error(
                        "undeclared_script_dependency",
                        format!(
                            "script asset {} is missing from scene dependencies",
                            script.asset
                        ),
                        Some(guid),
                    ));
                }
                for (name, value) in &script.properties {
                    if name.trim().is_empty()
                        || name.len() > 128
                        || matches!(value, ScriptPropertyValue::Number(number) if !number.is_finite())
                        || matches!(value, ScriptPropertyValue::String(text) if text.len() > 16 * 1024)
                    {
                        diagnostics.push(SceneDiagnostic::error(
                            "invalid_script_property",
                            format!("script property {name:?} is empty, too large, or non-finite"),
                            Some(guid),
                        ));
                    }
                }
            }
            if let Some(camera) = object.camera {
                if !camera
                    .zoom
                    .iter()
                    .all(|value| value.is_finite() && *value > 0.0)
                    || !camera.pixels_per_world_unit.is_finite()
                    || camera.pixels_per_world_unit <= 0.0
                {
                    diagnostics.push(SceneDiagnostic::error(
                        "invalid_camera",
                        "camera zoom and pixels-per-world-unit must be positive and finite",
                        Some(guid),
                    ));
                }
            }
            if let Some(body) = object.rigid_body {
                if !body.linear_velocity.iter().all(|value| value.is_finite())
                    || !body.angular_velocity.is_finite()
                    || !body.gravity_scale.is_finite()
                    || !body.linear_damping.is_finite()
                    || body.linear_damping < 0.0
                    || !body.angular_damping.is_finite()
                    || body.angular_damping < 0.0
                {
                    diagnostics.push(SceneDiagnostic::error(
                        "invalid_rigid_body",
                        "rigid body values must be finite and damping cannot be negative",
                        Some(guid),
                    ));
                }
            }
            if let Some(collider) = object.collider {
                let valid_shape = match collider.shape {
                    AuthoredColliderShape::Circle { radius } => radius.is_finite() && radius > 0.0,
                    AuthoredColliderShape::Box { half_extents } => half_extents
                        .iter()
                        .all(|value| value.is_finite() && *value > 0.0),
                };
                if !valid_shape
                    || !collider.offset.iter().all(|value| value.is_finite())
                    || !collider.rotation_radians.is_finite()
                    || !collider.density.is_finite()
                    || collider.density <= 0.0
                    || !collider.friction.is_finite()
                    || collider.friction < 0.0
                    || !collider.restitution.is_finite()
                    || collider.restitution < 0.0
                {
                    diagnostics.push(SceneDiagnostic::error(
                        "invalid_collider",
                        "collider dimensions and material settings must be finite and non-negative",
                        Some(guid),
                    ));
                }
            }
        }

        let mut parent_by_guid = BTreeMap::new();
        for object in &self.objects {
            let Some(parent_guid) = object.parent_guid else {
                continue;
            };
            if parent_guid == object.object_guid {
                hierarchy_valid = false;
                diagnostics.push(SceneDiagnostic::error(
                    "object_is_own_parent",
                    "an object cannot be parented to itself",
                    Some(object.object_guid),
                ));
            } else if !objects.contains(&parent_guid) {
                hierarchy_valid = false;
                diagnostics.push(SceneDiagnostic::error(
                    "missing_parent_object",
                    format!("parent object {parent_guid} does not exist in this scene"),
                    Some(object.object_guid),
                ));
            } else {
                parent_by_guid.insert(object.object_guid, parent_guid);
            }
        }

        let mut reported_cycle_nodes = BTreeSet::new();
        for object in &self.objects {
            let mut visited = BTreeSet::new();
            let mut cursor = Some(object.object_guid);
            while let Some(current) = cursor {
                if !visited.insert(current) {
                    hierarchy_valid = false;
                    if reported_cycle_nodes.insert(current) {
                        diagnostics.push(SceneDiagnostic::error(
                            "scene_hierarchy_cycle",
                            "object parenting must not create a cycle",
                            Some(current),
                        ));
                    }
                    break;
                }
                cursor = parent_by_guid.get(&current).copied();
            }
        }

        if hierarchy_valid {
            for object in &self.objects {
                if object.parent_guid.is_some()
                    && self.world_transform(object.object_guid).is_none()
                {
                    hierarchy_valid = false;
                    diagnostics.push(SceneDiagnostic::error(
                        "unsupported_parent_transform",
                        "parent transform composition would introduce shear; use uniform parent scale or align child rotation",
                        Some(object.object_guid),
                    ));
                }
            }
        }

        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(diagnostics)
        }
    }

    /// Resolves an authored local transform through its parent chain.
    pub fn world_transform(&self, object_guid: Guid) -> Option<AuthoredTransform> {
        let mut resolved = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        self.resolve_world_transform(object_guid, &mut resolved, &mut visiting)
    }

    fn resolve_world_transform(
        &self,
        object_guid: Guid,
        resolved: &mut BTreeMap<Guid, AuthoredTransform>,
        visiting: &mut BTreeSet<Guid>,
    ) -> Option<AuthoredTransform> {
        if let Some(transform) = resolved.get(&object_guid) {
            return Some(*transform);
        }
        let object = self
            .objects
            .iter()
            .find(|candidate| candidate.object_guid == object_guid)?;
        if !visiting.insert(object_guid) {
            return None;
        }
        let transform = match object.parent_guid {
            Some(parent_guid) => {
                let parent = self.resolve_world_transform(parent_guid, resolved, visiting)?;
                compose_authored_transform(parent, object.transform)?
            }
            None => object.transform,
        };
        visiting.remove(&object_guid);
        resolved.insert(object_guid, transform);
        Some(transform)
    }

    pub fn instantiate(&self) -> Result<RuntimeWorld, Vec<SceneDiagnostic>> {
        self.validate()?;
        let mut world = RuntimeWorld::new();
        for object in &self.objects {
            let entity = world
                .spawn(
                    object.object_guid,
                    Transform2D {
                        translation: object.transform.translation,
                        rotation_radians: object.transform.rotation_radians,
                        scale: object.transform.scale,
                    },
                    object.sprite.map(|sprite| Sprite {
                        asset: sprite.asset,
                        size: sprite.size,
                        anchor: sprite.anchor,
                        tint: sprite.tint,
                        uv_rect: sprite.uv_rect,
                        layer: sprite.layer,
                        order: sprite.order,
                    }),
                )
                .map_err(|error| {
                    vec![SceneDiagnostic::error(
                        "runtime_scene_compile_failed",
                        error.to_string(),
                        Some(object.object_guid),
                    )]
                })?;

            if let Some(camera) = object.camera {
                world
                    .set_camera(
                        entity,
                        Camera2D {
                            zoom: camera.zoom,
                            pixels_per_world_unit: camera.pixels_per_world_unit,
                            priority: camera.priority,
                            active: camera.active,
                        },
                    )
                    .map_err(|error| {
                        vec![SceneDiagnostic::error(
                            "runtime_scene_compile_failed",
                            error.to_string(),
                            Some(object.object_guid),
                        )]
                    })?;
            }
            if let Some(body) = object.rigid_body {
                world.set_rigid_body(entity, body.into()).map_err(|error| {
                    vec![SceneDiagnostic::error(
                        "runtime_scene_compile_failed",
                        error.to_string(),
                        Some(object.object_guid),
                    )]
                })?;
            }
            if let Some(collider) = object.collider {
                world
                    .set_collider(entity, collider.into())
                    .map_err(|error| {
                        vec![SceneDiagnostic::error(
                            "runtime_scene_compile_failed",
                            error.to_string(),
                            Some(object.object_guid),
                        )]
                    })?;
            }
        }
        for object in &self.objects {
            if let Some(parent_guid) = object.parent_guid {
                let local = Transform2D {
                    translation: object.transform.translation,
                    rotation_radians: object.transform.rotation_radians,
                    scale: object.transform.scale,
                };
                world
                    .set_parent(object.object_guid, Some(parent_guid), local)
                    .map_err(|error| {
                        vec![SceneDiagnostic::error(
                            "runtime_scene_compile_failed",
                            error.to_string(),
                            Some(object.object_guid),
                        )]
                    })?;
            }
        }
        Ok(world)
    }

    pub fn to_canonical_json(&self) -> Result<String, serde_json::Error> {
        let mut canonical = self.clone();
        canonical.dependencies.sort();
        canonical.objects.sort_by_key(|object| object.object_guid);
        serde_json::to_string_pretty(&canonical)
    }
}

fn compose_authored_transform(
    parent: AuthoredTransform,
    local: AuthoredTransform,
) -> Option<AuthoredTransform> {
    let (parent_sin, parent_cos) = parent.rotation_radians.sin_cos();
    let (local_sin, local_cos) = local.rotation_radians.sin_cos();
    let parent_x = [parent_cos * parent.scale[0], parent_sin * parent.scale[0]];
    let parent_y = [-parent_sin * parent.scale[1], parent_cos * parent.scale[1]];
    let local_x = [local_cos * local.scale[0], local_sin * local.scale[0]];
    let local_y = [-local_sin * local.scale[1], local_cos * local.scale[1]];
    let world_x = [
        parent_x[0] * local_x[0] + parent_y[0] * local_x[1],
        parent_x[1] * local_x[0] + parent_y[1] * local_x[1],
    ];
    let world_y = [
        parent_x[0] * local_y[0] + parent_y[0] * local_y[1],
        parent_x[1] * local_y[0] + parent_y[1] * local_y[1],
    ];
    let x_length = world_x[0].hypot(world_x[1]);
    let y_length = world_y[0].hypot(world_y[1]);
    let dot = world_x[0] * world_y[0] + world_x[1] * world_y[1];
    if x_length > f32::EPSILON
        && y_length > f32::EPSILON
        && dot.abs() > 1.0e-4 * x_length * y_length
    {
        return None;
    }
    let determinant = world_x[0] * world_y[1] - world_x[1] * world_y[0];
    let (rotation_radians, scale) = if x_length > f32::EPSILON {
        (
            world_x[1].atan2(world_x[0]),
            [x_length, determinant / x_length],
        )
    } else if y_length > f32::EPSILON {
        (
            world_y[1].atan2(world_y[0]) - std::f32::consts::FRAC_PI_2,
            [0.0, y_length],
        )
    } else {
        (parent.rotation_radians + local.rotation_radians, [0.0, 0.0])
    };
    let translation = [
        parent.translation[0]
            + parent_x[0] * local.translation[0]
            + parent_y[0] * local.translation[1],
        parent.translation[1]
            + parent_x[1] * local.translation[0]
            + parent_y[1] * local.translation[1],
    ];
    let world = AuthoredTransform {
        translation,
        rotation_radians,
        scale,
    };
    (world.translation.iter().all(|value| value.is_finite())
        && world.scale.iter().all(|value| value.is_finite())
        && world.rotation_radians.is_finite())
    .then_some(world)
}

impl PrefabDocument {
    pub fn from_json(source: &str) -> Result<Self, Vec<SceneDiagnostic>> {
        let document: Self = serde_json::from_str(source).map_err(|error| {
            vec![SceneDiagnostic::error(
                "invalid_prefab_json",
                error.to_string(),
                None,
            )]
        })?;
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), Vec<SceneDiagnostic>> {
        let mut diagnostics = Vec::new();
        if self.schema_version != CURRENT_PREFAB_SCHEMA {
            diagnostics.push(SceneDiagnostic::error(
                "unsupported_prefab_schema",
                format!(
                    "prefab schema {} is unsupported; this build reads schema {}",
                    self.schema_version, CURRENT_PREFAB_SCHEMA
                ),
                None,
            ));
        }
        if self.name.trim().is_empty() {
            diagnostics.push(SceneDiagnostic::error(
                "empty_prefab_name",
                "prefab name must not be empty",
                None,
            ));
        }
        if self.object.prefab_instance.is_some() {
            diagnostics.push(SceneDiagnostic::error(
                "nested_prefab_template",
                "prefab templates cannot contain another prefab instance",
                Some(self.object.object_guid),
            ));
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        SceneDocument {
            schema_version: CURRENT_SCENE_SCHEMA,
            resource_guid: self.resource_guid,
            name: self.name.clone(),
            dependencies: self.dependencies.clone(),
            objects: vec![self.object.clone()],
        }
        .validate()
    }

    pub fn to_canonical_json(&self) -> Result<String, serde_json::Error> {
        let mut canonical = self.clone();
        canonical.dependencies.sort();
        serde_json::to_string_pretty(&canonical)
    }
}

impl From<AuthoredRigidBody> for RigidBody2D {
    fn from(body: AuthoredRigidBody) -> Self {
        Self {
            kind: match body.kind {
                AuthoredRigidBodyKind::Fixed => RigidBodyKind2D::Fixed,
                AuthoredRigidBodyKind::Dynamic => RigidBodyKind2D::Dynamic,
                AuthoredRigidBodyKind::KinematicPositionBased => {
                    RigidBodyKind2D::KinematicPositionBased
                }
            },
            linear_velocity: body.linear_velocity,
            angular_velocity: body.angular_velocity,
            gravity_scale: body.gravity_scale,
            linear_damping: body.linear_damping,
            angular_damping: body.angular_damping,
            lock_rotation: body.lock_rotation,
        }
    }
}

impl From<AuthoredCollider> for Collider2D {
    fn from(collider: AuthoredCollider) -> Self {
        Self {
            shape: match collider.shape {
                AuthoredColliderShape::Circle { radius } => ColliderShape2D::Circle { radius },
                AuthoredColliderShape::Box { half_extents } => {
                    ColliderShape2D::Box { half_extents }
                }
            },
            offset: collider.offset,
            rotation_radians: collider.rotation_radians,
            density: collider.density,
            friction: collider.friction,
            restitution: collider.restitution,
            sensor: collider.sensor,
        }
    }
}

const fn unit_f32() -> f32 {
    1.0
}

const fn unit_scale() -> [f32; 2] {
    [1.0, 1.0]
}

const fn center_anchor() -> [f32; 2] {
    [0.5, 0.5]
}

fn white_tint() -> [f32; 4] {
    [1.0; 4]
}

const fn default_active() -> bool {
    true
}

const fn full_uv_rect() -> [f32; 4] {
    [0.0, 0.0, 1.0, 1.0]
}

const fn default_friction() -> f32 {
    0.5
}

#[cfg(test)]
mod script_authoring_tests {
    use super::*;

    fn scene_with_script(script: AuthoredScript, dependencies: Vec<Guid>) -> SceneDocument {
        SceneDocument {
            schema_version: CURRENT_SCENE_SCHEMA,
            resource_guid: Guid::new(),
            name: "Script scene".into(),
            dependencies,
            objects: vec![SceneObject {
                object_guid: Guid::new(),
                name: "Scripted object".into(),
                parent_guid: None,
                prefab_instance: None,
                transform: AuthoredTransform::default(),
                sprite: None,
                camera: None,
                rigid_body: None,
                collider: None,
                script: Some(script),
            }],
        }
    }

    #[test]
    fn script_properties_round_trip_with_declared_asset_dependency() {
        let asset = Guid::new();
        let scene = scene_with_script(
            AuthoredScript {
                asset,
                properties: BTreeMap::from([
                    ("speed".into(), ScriptPropertyValue::Number(3.5)),
                    ("enabled".into(), ScriptPropertyValue::Bool(true)),
                    ("label".into(), ScriptPropertyValue::String("runner".into())),
                ]),
            },
            vec![asset],
        );
        scene
            .validate()
            .expect("valid script dependency and properties");
        let json = serde_json::to_string(&scene).expect("serialize scene");
        let restored = SceneDocument::from_json(&json).expect("load scene");
        assert_eq!(restored.objects[0].script, scene.objects[0].script);
    }

    #[test]
    fn old_scene_json_without_script_fields_remains_loadable() {
        let scene = SceneDocument::from_json(
            r#"{"schema_version":1,"resource_guid":"00000000-0000-4000-8000-000000000001","name":"Old scene","objects":[{"object_guid":"00000000-0000-4000-8000-000000000002","name":"Old object"}]}"#,
        ).expect("old scene stays compatible");
        assert!(scene.objects[0].script.is_none());
    }

    #[test]
    fn rejects_missing_dependency_and_non_finite_script_properties() {
        let scene = scene_with_script(
            AuthoredScript {
                asset: Guid::new(),
                properties: BTreeMap::from([
                    ("speed".into(), ScriptPropertyValue::Number(f64::NAN)),
                    ("".into(), ScriptPropertyValue::Bool(false)),
                ]),
            },
            Vec::new(),
        );
        let diagnostics = scene
            .validate()
            .expect_err("invalid script data is rejected");
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "undeclared_script_dependency")
        );
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "invalid_script_property")
        );
    }
}
