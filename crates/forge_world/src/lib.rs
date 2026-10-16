//! Forge-owned access to runtime entities and components.
//!
//! Authored GUIDs are persisted; runtime entity handles are temporary. Third-party ECS types
//! stay behind this API so projects do not depend on a storage-library contract.

use std::{
    collections::{BTreeMap, HashMap},
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
        Ok(RuntimeEntity(runtime_entity))
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
        self.by_guid.remove(&object_guid);
        self.ecs.despawn(entity.0)
    }

    pub fn set_transform(
        &mut self,
        entity: RuntimeEntity,
        transform: Transform2D,
    ) -> Result<(), WorldError> {
        let Some(mut current) = self.ecs.get_mut::<Transform2D>(entity.0) else {
            return Err(WorldError::StaleEntity);
        };
        *current = transform;
        Ok(())
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
        if let Some(mut current) = self.ecs.get_mut::<Transform2D>(entity) {
            *current = transform;
        } else {
            return Err(WorldError::StaleEntity);
        }
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
