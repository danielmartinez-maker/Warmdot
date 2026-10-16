//! Forge-owned access to runtime entities and components.
//!
//! Authored GUIDs are persisted; runtime entity handles are temporary. Third-party ECS types
//! stay behind this API so projects do not depend on a storage-library contract.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
};

use bevy_ecs::{component::Component, entity::Entity, query::QueryState, world::World as EcsWorld};
use forge_core::Guid;

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Transform2D {
    /// World coordinates use +X right and +Y down.
    pub translation: [f32; 2],
    pub rotation_radians: f32,
    pub scale: [f32; 2],
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0],
            rotation_radians: 0.0,
            scale: [1.0, 1.0],
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub asset: Option<Guid>,
    pub size: [f32; 2],
    pub tint: [f32; 4],
    /// Normalized image coordinates stored as [u, v, width, height].
    pub uv_rect: [f32; 4],
    pub layer: i32,
    pub order: i32,
}

impl Default for Sprite {
    fn default() -> Self {
        Self {
            asset: None,
            size: [1.0, 1.0],
            tint: [1.0; 4],
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            order: 0,
        }
    }
}

fn valid_transform(transform: Transform2D) -> bool {
    transform.translation.iter().all(|value| value.is_finite())
        && transform.scale.iter().all(|value| value.is_finite())
        && transform.rotation_radians.is_finite()
}

fn transform_basis(transform: Transform2D) -> ([f32; 2], [f32; 2]) {
    let (sin, cos) = transform.rotation_radians.sin_cos();
    (
        [cos * transform.scale[0], sin * transform.scale[0]],
        [-sin * transform.scale[1], cos * transform.scale[1]],
    )
}

fn decompose_basis(
    x_axis: [f32; 2],
    y_axis: [f32; 2],
    object_guid: Guid,
) -> Result<(f32, [f32; 2]), WorldError> {
    let x_length = x_axis[0].hypot(x_axis[1]);
    let y_length = y_axis[0].hypot(y_axis[1]);
    let dot = x_axis[0] * y_axis[0] + x_axis[1] * y_axis[1];
    if x_length > f32::EPSILON
        && y_length > f32::EPSILON
        && dot.abs() > 1.0e-4 * x_length * y_length
    {
        return Err(WorldError::UnsupportedParentShear(object_guid));
    }
    let determinant = x_axis[0] * y_axis[1] - x_axis[1] * y_axis[0];
    if x_length > f32::EPSILON {
        Ok((
            x_axis[1].atan2(x_axis[0]),
            [x_length, determinant / x_length],
        ))
    } else if y_length > f32::EPSILON {
        Ok((
            y_axis[1].atan2(y_axis[0]) - std::f32::consts::FRAC_PI_2,
            [0.0, y_length],
        ))
    } else {
        Ok((0.0, [0.0, 0.0]))
    }
}

fn compose_transform(
    parent: Transform2D,
    local: Transform2D,
    object_guid: Guid,
) -> Result<Transform2D, WorldError> {
    let (parent_x, parent_y) = transform_basis(parent);
    let (local_x, local_y) = transform_basis(local);
    let world_x = [
        parent_x[0] * local_x[0] + parent_y[0] * local_x[1],
        parent_x[1] * local_x[0] + parent_y[1] * local_x[1],
    ];
    let world_y = [
        parent_x[0] * local_y[0] + parent_y[0] * local_y[1],
        parent_x[1] * local_y[0] + parent_y[1] * local_y[1],
    ];
    let (rotation_radians, scale) = decompose_basis(world_x, world_y, object_guid)?;
    let transform = Transform2D {
        translation: [
            parent.translation[0] + parent_x[0] * local.translation[0]
                + parent_y[0] * local.translation[1],
            parent.translation[1] + parent_x[1] * local.translation[0]
                + parent_y[1] * local.translation[1],
        ],
        rotation_radians,
        scale,
    };
    if valid_transform(transform) {
        Ok(transform)
    } else {
        Err(WorldError::InvalidTransform(object_guid))
    }
}

fn relative_transform(
    parent: Transform2D,
    world: Transform2D,
    object_guid: Guid,
) -> Result<Transform2D, WorldError> {
    if parent.scale.iter().any(|value| value.abs() <= f32::EPSILON) {
        return Err(WorldError::InvalidTransform(object_guid));
    }
    let (sin, cos) = parent.rotation_radians.sin_cos();
    let world_delta = [
        world.translation[0] - parent.translation[0],
        world.translation[1] - parent.translation[1],
    ];
    let local_translation = [
        (cos * world_delta[0] + sin * world_delta[1]) / parent.scale[0],
        (-sin * world_delta[0] + cos * world_delta[1]) / parent.scale[1],
    ];
    let (world_x, world_y) = transform_basis(world);
    let local_x = [
        (cos * world_x[0] + sin * world_x[1]) / parent.scale[0],
        (-sin * world_x[0] + cos * world_x[1]) / parent.scale[1],
    ];
    let local_y = [
        (cos * world_y[0] + sin * world_y[1]) / parent.scale[0],
        (-sin * world_y[0] + cos * world_y[1]) / parent.scale[1],
    ];
    let (rotation_radians, scale) = decompose_basis(local_x, local_y, object_guid)?;
    let transform = Transform2D {
        translation: local_translation,
        rotation_radians,
        scale,
    };
    if valid_transform(transform) {
        Ok(transform)
    } else {
        Err(WorldError::InvalidTransform(object_guid))
    }
}

fn valid_uv_rect([u, v, width, height]: [f32; 4]) -> bool {
    [u, v, width, height].iter().all(|value| value.is_finite())
        && u >= 0.0
        && v >= 0.0
        && width > 0.0
        && height > 0.0
        && u + width <= 1.0
        && v + height <= 1.0
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Camera2D {
    pub zoom: [f32; 2],
    pub pixels_per_world_unit: f32,
    pub priority: i32,
    pub active: bool,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            zoom: [1.0, 1.0],
            pixels_per_world_unit: 1.0,
            priority: 0,
            active: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RigidBodyKind2D {
    Fixed,
    Dynamic,
    KinematicPositionBased,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RigidBody2D {
    pub kind: RigidBodyKind2D,
    pub linear_velocity: [f32; 2],
    pub angular_velocity: f32,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub lock_rotation: bool,
}

impl Default for RigidBody2D {
    fn default() -> Self {
        Self {
            kind: RigidBodyKind2D::Fixed,
            linear_velocity: [0.0, 0.0],
            angular_velocity: 0.0,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            lock_rotation: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColliderShape2D {
    Circle { radius: f32 },
    Box { half_extents: [f32; 2] },
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Collider2D {
    pub shape: ColliderShape2D,
    pub offset: [f32; 2],
    pub rotation_radians: f32,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub sensor: bool,
}

impl Default for Collider2D {
    fn default() -> Self {
        Self {
            shape: ColliderShape2D::Box {
                half_extents: [0.5, 0.5],
            },
            offset: [0.0, 0.0],
            rotation_radians: 0.0,
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            sensor: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeEntity(Entity);

impl RuntimeEntity {
    /// Diagnostic-only index. Never write this value to an authored file.
    pub fn debug_index(self) -> u32 {
        self.0.index_u32()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtractedSprite {
    pub object_guid: Guid,
    pub asset: Option<Guid>,
    pub translation: [f32; 2],
    pub rotation_radians: f32,
    pub scale: [f32; 2],
    pub size: [f32; 2],
    pub tint: [f32; 4],
    pub uv_rect: [f32; 4],
    pub layer: i32,
    pub order: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtractedCamera {
    pub object_guid: Guid,
    pub translation: [f32; 2],
    pub rotation_radians: f32,
    pub zoom: [f32; 2],
    pub pixels_per_world_unit: f32,
    pub priority: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsObjectSnapshot {
    pub object_guid: Guid,
    pub transform: Transform2D,
    pub body: Option<RigidBody2D>,
    pub collider: Option<Collider2D>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldError {
    DuplicateGuid(Guid),
    StaleEntity,
    MissingSprite,
    InvalidSprite(&'static str),
    InvalidPhysicsConfiguration(&'static str),
    MissingParent(Guid),
    ParentCycle(Guid),
    UnsupportedParentShear(Guid),
    InvalidTransform(Guid),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGuid(guid) => {
                write!(f, "object GUID {guid} already exists in this world")
            }
            Self::StaleEntity => f.write_str("runtime entity is no longer alive"),
            Self::MissingSprite => f.write_str("runtime entity has no sprite component"),
            Self::InvalidSprite(message) => f.write_str(message),
            Self::InvalidPhysicsConfiguration(message) => f.write_str(message),
            Self::MissingParent(guid) => write!(f, "parent object {guid} is missing"),
            Self::ParentCycle(guid) => write!(f, "parenting would create a cycle at object {guid}"),
            Self::UnsupportedParentShear(guid) => write!(
                f,
                "parent transform would introduce unsupported shear for object {guid}"
            ),
            Self::InvalidTransform(guid) => {
                write!(f, "object {guid} has a non-finite or singular transform")
            }
        }
    }
}

impl std::error::Error for WorldError {}

#[derive(Component, Clone, Copy)]
struct AuthoredIdentity(Guid);

pub struct RuntimeWorld {
    ecs: EcsWorld,
    by_guid: BTreeMap<Guid, Entity>,
    guid_by_entity: HashMap<Entity, Guid>,
    local_transforms: BTreeMap<Guid, Transform2D>,
    parent_by_guid: BTreeMap<Guid, Guid>,
    sprite_query: QueryState<(
        &'static AuthoredIdentity,
        &'static Transform2D,
        &'static Sprite,
    )>,
    camera_query: QueryState<(
        &'static AuthoredIdentity,
        &'static Transform2D,
        &'static Camera2D,
    )>,
    physics_query: QueryState<(
        &'static AuthoredIdentity,
        &'static Transform2D,
        Option<&'static RigidBody2D>,
        Option<&'static Collider2D>,
    )>,
}

impl Default for RuntimeWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeWorld {
    pub fn new() -> Self {
        let mut ecs = EcsWorld::new();
        let sprite_query = ecs.query::<(&AuthoredIdentity, &Transform2D, &Sprite)>();
        let camera_query = ecs.query::<(&AuthoredIdentity, &Transform2D, &Camera2D)>();
        let physics_query = ecs.query::<(
            &AuthoredIdentity,
            &Transform2D,
            Option<&RigidBody2D>,
            Option<&Collider2D>,
        )>();
        Self {
            ecs,
            by_guid: BTreeMap::new(),
            guid_by_entity: HashMap::new(),
            local_transforms: BTreeMap::new(),
            parent_by_guid: BTreeMap::new(),
            sprite_query,
            camera_query,
            physics_query,
        }
    }

    pub fn spawn(
        &mut self,
        object_guid: Guid,
        transform: Transform2D,
        sprite: Option<Sprite>,
    ) -> Result<RuntimeEntity, WorldError> {
        if self.by_guid.contains_key(&object_guid) {
            return Err(WorldError::DuplicateGuid(object_guid));
        }
        if sprite.is_some_and(|sprite| !valid_uv_rect(sprite.uv_rect)) {
            return Err(WorldError::InvalidSprite(
                "sprite UV rectangle must be finite, positive, and inside the image",
            ));
        }
        let mut entity = self.ecs.spawn((AuthoredIdentity(object_guid), transform));
        if let Some(sprite) = sprite {
            entity.insert(sprite);
        }
        let runtime_entity = entity.id();
        self.by_guid.insert(object_guid, runtime_entity);
        self.guid_by_entity.insert(runtime_entity, object_guid);
        self.local_transforms.insert(object_guid, transform);
        Ok(RuntimeEntity(runtime_entity))
    }

    /// Sets the authored parent and local transform, then propagates world transforms.
    pub fn set_parent(
        &mut self,
        object_guid: Guid,
        parent_guid: Option<Guid>,
        local_transform: Transform2D,
    ) -> Result<(), WorldError> {
        if !self.by_guid.contains_key(&object_guid) {
            return Err(WorldError::StaleEntity);
        }
        if !valid_transform(local_transform) {
            return Err(WorldError::InvalidTransform(object_guid));
        }
        if let Some(parent_guid) = parent_guid {
            if !self.by_guid.contains_key(&parent_guid) {
                return Err(WorldError::MissingParent(parent_guid));
            }
            let mut cursor = Some(parent_guid);
            let mut visited = BTreeSet::new();
            while let Some(current) = cursor {
                if current == object_guid || !visited.insert(current) {
                    return Err(WorldError::ParentCycle(object_guid));
                }
                cursor = self.parent_by_guid.get(&current).copied();
            }
            self.parent_by_guid.insert(object_guid, parent_guid);
        } else {
            self.parent_by_guid.remove(&object_guid);
        }
        self.local_transforms.insert(object_guid, local_transform);
        self.propagate_parent_transforms()
    }

    /// Rebuilds world transforms from stable parent GUIDs and authored local transforms.
    pub fn propagate_parent_transforms(&mut self) -> Result<(), WorldError> {
        let mut resolved = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        let object_guids = self.by_guid.keys().copied().collect::<Vec<_>>();
        for object_guid in object_guids {
            self.resolve_world_transform(object_guid, &mut resolved, &mut visiting)?;
        }
        for (object_guid, transform) in resolved {
            let Some(entity) = self.by_guid.get(&object_guid).copied() else {
                return Err(WorldError::StaleEntity);
            };
            let Some(mut current) = self.ecs.get_mut::<Transform2D>(entity) else {
                return Err(WorldError::StaleEntity);
            };
            *current = transform;
        }
        Ok(())
    }

    fn resolve_world_transform(
        &self,
        object_guid: Guid,
        resolved: &mut BTreeMap<Guid, Transform2D>,
        visiting: &mut BTreeSet<Guid>,
    ) -> Result<Transform2D, WorldError> {
        if let Some(transform) = resolved.get(&object_guid) {
            return Ok(*transform);
        }
        if !visiting.insert(object_guid) {
            return Err(WorldError::ParentCycle(object_guid));
        }
        let local = *self
            .local_transforms
            .get(&object_guid)
            .ok_or(WorldError::StaleEntity)?;
        let transform = if let Some(parent_guid) = self.parent_by_guid.get(&object_guid).copied() {
            let parent = self.resolve_world_transform(parent_guid, resolved, visiting)?;
            compose_transform(parent, local, object_guid)?
        } else {
            local
        };
        visiting.remove(&object_guid);
        if !valid_transform(transform) {
            return Err(WorldError::InvalidTransform(object_guid));
        }
        resolved.insert(object_guid, transform);
        Ok(transform)
    }

    fn set_world_transform(
        &mut self,
        object_guid: Guid,
        transform: Transform2D,
    ) -> Result<(), WorldError> {
        if !valid_transform(transform) {
            return Err(WorldError::InvalidTransform(object_guid));
        }
        let local = if let Some(parent_guid) = self.parent_by_guid.get(&object_guid).copied() {
            let parent_entity = self
                .by_guid
                .get(&parent_guid)
                .copied()
                .ok_or(WorldError::MissingParent(parent_guid))?;
            let parent = self
                .ecs
                .get::<Transform2D>(parent_entity)
                .copied()
                .ok_or(WorldError::StaleEntity)?;
            relative_transform(parent, transform, object_guid)?
        } else {
            transform
        };
        self.local_transforms.insert(object_guid, local);
        let entity = self
            .by_guid
            .get(&object_guid)
            .copied()
            .ok_or(WorldError::StaleEntity)?;
        let Some(mut current) = self.ecs.get_mut::<Transform2D>(entity) else {
            return Err(WorldError::StaleEntity);
        };
        *current = transform;
        drop(current);
        self.propagate_parent_transforms()
    }

    pub fn entity_for_guid(&self, object_guid: Guid) -> Option<RuntimeEntity> {
        self.by_guid.get(&object_guid).copied().map(RuntimeEntity)
    }

    pub fn contains(&self, entity: RuntimeEntity) -> bool {
        self.guid_by_entity.contains_key(&entity.0)
    }

    pub fn despawn(&mut self, entity: RuntimeEntity) -> bool {
        let Some(object_guid) = self.guid_by_entity.remove(&entity.0) else {
            return false;
        };
        let children = self
            .parent_by_guid
            .iter()
            .filter_map(|(child, parent)| (*parent == object_guid).then_some(*child))
            .collect::<Vec<_>>();
        for child in children {
            self.parent_by_guid.remove(&child);
            if let Some(child_entity) = self.by_guid.get(&child).copied() {
                if let Some(world_transform) = self.ecs.get::<Transform2D>(child_entity).copied() {
                    self.local_transforms.insert(child, world_transform);
                }
            }
        }
        self.parent_by_guid.remove(&object_guid);
        self.local_transforms.remove(&object_guid);
        self.by_guid.remove(&object_guid);
        let removed = self.ecs.despawn(entity.0);
        let _ = self.propagate_parent_transforms();
        removed
    }

    pub fn set_transform(
        &mut self,
        entity: RuntimeEntity,
        transform: Transform2D,
    ) -> Result<(), WorldError> {
        let object_guid = self
            .guid_by_entity
            .get(&entity.0)
            .copied()
            .ok_or(WorldError::StaleEntity)?;
        self.set_world_transform(object_guid, transform)
    }

    pub fn transform(&self, entity: RuntimeEntity) -> Option<Transform2D> {
        self.ecs.get::<Transform2D>(entity.0).copied()
    }

    pub fn set_sprite(&mut self, entity: RuntimeEntity, sprite: Sprite) -> Result<(), WorldError> {
        if !valid_uv_rect(sprite.uv_rect) {
            return Err(WorldError::InvalidSprite(
                "sprite UV rectangle must be finite, positive, and inside the image",
            ));
        }
        let Ok(mut current) = self.ecs.get_entity_mut(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        current.insert(sprite);
        Ok(())
    }

    pub fn set_sprite_asset(
        &mut self,
        entity: RuntimeEntity,
        asset: Option<Guid>,
    ) -> Result<(), WorldError> {
        let mut current = self
            .ecs
            .get_entity_mut(entity.0)
            .map_err(|_| WorldError::StaleEntity)?;
        let Some(mut sprite) = current.get_mut::<Sprite>() else {
            return Err(WorldError::MissingSprite);
        };
        sprite.asset = asset;
        Ok(())
    }

    pub fn set_sprite_uv_rect(
        &mut self,
        entity: RuntimeEntity,
        uv_rect: [f32; 4],
    ) -> Result<(), WorldError> {
        if !valid_uv_rect(uv_rect) {
            return Err(WorldError::InvalidSprite(
                "sprite UV rectangle must be finite, positive, and inside the image",
            ));
        }
        let mut current = self
            .ecs
            .get_entity_mut(entity.0)
            .map_err(|_| WorldError::StaleEntity)?;
        let Some(mut sprite) = current.get_mut::<Sprite>() else {
            return Err(WorldError::MissingSprite);
        };
        sprite.uv_rect = uv_rect;
        Ok(())
    }

    pub fn remove_sprite(&mut self, entity: RuntimeEntity) -> Result<bool, WorldError> {
        let Ok(mut current) = self.ecs.get_entity_mut(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        let had_sprite = current.contains::<Sprite>();
        current.remove::<Sprite>();
        Ok(had_sprite)
    }

    pub fn set_camera(
        &mut self,
        entity: RuntimeEntity,
        camera: Camera2D,
    ) -> Result<(), WorldError> {
        let Ok(mut current) = self.ecs.get_entity_mut(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        current.insert(camera);
        Ok(())
    }

    pub fn set_rigid_body(
        &mut self,
        entity: RuntimeEntity,
        body: RigidBody2D,
    ) -> Result<(), WorldError> {
        if !body.linear_velocity.iter().all(|value| value.is_finite())
            || !body.angular_velocity.is_finite()
            || !body.gravity_scale.is_finite()
            || !body.linear_damping.is_finite()
            || body.linear_damping < 0.0
            || !body.angular_damping.is_finite()
            || body.angular_damping < 0.0
        {
            return Err(WorldError::InvalidPhysicsConfiguration(
                "rigid body velocities, gravity scale, and non-negative damping must be finite",
            ));
        }
        let Ok(mut current) = self.ecs.get_entity_mut(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        current.insert(body);
        Ok(())
    }

    pub fn set_collider(
        &mut self,
        entity: RuntimeEntity,
        collider: Collider2D,
    ) -> Result<(), WorldError> {
        let valid_shape = match collider.shape {
            ColliderShape2D::Circle { radius } => radius.is_finite() && radius > 0.0,
            ColliderShape2D::Box { half_extents } => half_extents
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
            return Err(WorldError::InvalidPhysicsConfiguration(
                "collider dimensions, offset, rotation, density, friction, or restitution are invalid",
            ));
        }
        let Ok(mut current) = self.ecs.get_entity_mut(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        current.insert(collider);
        Ok(())
    }

    pub fn remove_physics(&mut self, entity: RuntimeEntity) -> Result<(), WorldError> {
        let Ok(mut current) = self.ecs.get_entity_mut(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        current.remove::<RigidBody2D>();
        current.remove::<Collider2D>();
        Ok(())
    }

    pub fn set_physics_state(
        &mut self,
        object_guid: Guid,
        transform: Transform2D,
        linear_velocity: [f32; 2],
        angular_velocity: f32,
    ) -> Result<(), WorldError> {
        if !transform.translation.iter().all(|value| value.is_finite())
            || !transform.scale.iter().all(|value| value.is_finite())
            || !transform.rotation_radians.is_finite()
            || !linear_velocity.iter().all(|value| value.is_finite())
            || !angular_velocity.is_finite()
        {
            return Err(WorldError::InvalidPhysicsConfiguration(
                "physics state must contain only finite values",
            ));
        }
        let Some(entity) = self.by_guid.get(&object_guid).copied() else {
            return Err(WorldError::StaleEntity);
        };
        self.set_world_transform(object_guid, transform)?;
        if let Some(mut body) = self.ecs.get_mut::<RigidBody2D>(entity) {
            body.linear_velocity = linear_velocity;
            body.angular_velocity = angular_velocity;
        }
        Ok(())
    }

    pub fn extract_physics_objects(&mut self) -> Vec<PhysicsObjectSnapshot> {
        let mut extracted = self
            .physics_query
            .iter(&self.ecs)
            .filter_map(|(identity, transform, body, collider)| {
                (body.is_some() || collider.is_some()).then_some(PhysicsObjectSnapshot {
                    object_guid: identity.0,
                    transform: *transform,
                    body: body.copied(),
                    collider: collider.copied(),
                })
            })
            .collect::<Vec<_>>();
        extracted.sort_by_key(|object| object.object_guid);
        extracted
    }

    pub fn extract_sprites(&mut self) -> Vec<ExtractedSprite> {
        let mut extracted = self
            .sprite_query
            .iter(&self.ecs)
            .map(|(identity, transform, sprite)| ExtractedSprite {
                object_guid: identity.0,
                asset: sprite.asset,
                translation: transform.translation,
                rotation_radians: transform.rotation_radians,
                scale: transform.scale,
                size: sprite.size,
                tint: sprite.tint,
                uv_rect: sprite.uv_rect,
                layer: sprite.layer,
                order: sprite.order,
            })
            .collect::<Vec<_>>();
        extracted.sort_by_key(|sprite| (sprite.layer, sprite.order, sprite.object_guid));
        extracted
    }

    pub fn extract_camera(&mut self) -> Option<ExtractedCamera> {
        let mut cameras = self
            .camera_query
            .iter(&self.ecs)
            .filter(|(_, _, camera)| camera.active)
            .map(|(identity, transform, camera)| ExtractedCamera {
                object_guid: identity.0,
                translation: transform.translation,
                rotation_radians: transform.rotation_radians,
                zoom: camera.zoom,
                pixels_per_world_unit: camera.pixels_per_world_unit,
                priority: camera.priority,
            })
            .collect::<Vec<_>>();
        cameras.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.object_guid.cmp(&right.object_guid))
        });
        cameras.into_iter().next()
    }

    pub fn object_count(&self) -> usize {
        self.by_guid.len()
    }
}
