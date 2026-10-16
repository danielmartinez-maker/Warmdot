//! Forge-owned adapter for Rapier 2D rigid-body simulation.
//!
//! No Rapier handles, builders, or component types cross this crate's public API. Physics
//! settings live on Forge runtime objects; this service synchronizes them at fixed-tick barriers.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::mpsc::{self, Receiver},
};

use forge_core::Guid;
use forge_world::{
    Collider2D, ColliderShape2D, PhysicsObjectSnapshot, RigidBody2D, RigidBodyKind2D,
    RuntimeWorld, Transform2D,
};
use rapier2d::prelude::{
    ActiveEvents, ChannelEventCollector, ColliderBuilder, ColliderHandle, CollisionEvent,
    PhysicsWorld as RapierWorld, QueryFilter, Ray, RigidBodyBuilder, RigidBodyHandle, Vector,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionEvent2D {
    pub object_a: Guid,
    pub object_b: Guid,
    pub started: bool,
    pub sensor: bool,
    pub removed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RaycastHit2D {
    pub object_guid: Guid,
    pub point: [f32; 2],
    pub normal: [f32; 2],
    pub distance: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicsError {
    InvalidGravity,
    InvalidRay,
}

impl std::fmt::Display for PhysicsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGravity => f.write_str("gravity must contain only finite values"),
            Self::InvalidRay => f.write_str(
                "ray origin, direction, and distance must be finite; direction and distance must be nonzero",
            ),
        }
    }
}

impl std::error::Error for PhysicsError {}

struct BodyRecord {
    body: RigidBodyHandle,
    collider: Option<ColliderHandle>,
    settings: RigidBody2D,
    collider_settings: Option<Collider2D>,
    last_transform: Transform2D,
}

pub struct PhysicsSimulation {
    rapier: RapierWorld,
    objects: BTreeMap<Guid, BodyRecord>,
    guid_by_collider: HashMap<ColliderHandle, Guid>,
    retired_colliders: Vec<ColliderHandle>,
    event_collector: ChannelEventCollector,
    collision_events: Receiver<CollisionEvent>,
}

impl Default for PhysicsSimulation {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsSimulation {
    pub fn new() -> Self {
        let (collision_sender, collision_events) = mpsc::channel();
        let (contact_sender, _contact_receiver) = mpsc::channel();
        let (tear_sender, _tear_receiver) = mpsc::channel();
        let event_collector =
            ChannelEventCollector::new(collision_sender, contact_sender, tear_sender);
        let mut rapier = RapierWorld::default();
        rapier.gravity = Vector::new(0.0, 9.81);
        Self {
            rapier,
            objects: BTreeMap::new(),
            guid_by_collider: HashMap::new(),
            retired_colliders: Vec::new(),
            event_collector,
            collision_events,
        }
    }

    /// Forge uses +Y down, so positive Y gravity pulls objects toward the bottom of the screen.
    pub fn set_gravity(&mut self, gravity: [f32; 2]) -> Result<(), PhysicsError> {
        if !gravity.iter().all(|value| value.is_finite()) {
            return Err(PhysicsError::InvalidGravity);
        }
        self.rapier.gravity = Vector::new(gravity[0], gravity[1]);
        Ok(())
    }

    pub fn gravity(&self) -> [f32; 2] {
        [self.rapier.gravity.x, self.rapier.gravity.y]
    }

    /// Synchronizes authored physics objects, advances one fixed step, and writes body poses back.
    pub fn step(
        &mut self,
        world: &mut RuntimeWorld,
        delta_seconds: f32,
    ) -> Vec<CollisionEvent2D> {
        if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            return Vec::new();
        }

        let snapshots = world.extract_physics_objects();
        let live_guids = snapshots
            .iter()
            .map(|snapshot| snapshot.object_guid)
            .collect::<BTreeSet<_>>();
        let removed = self
            .objects
            .keys()
            .filter(|guid| !live_guids.contains(guid))
            .copied()
            .collect::<Vec<_>>();
        for guid in removed {
            self.remove_object(guid);
        }

        for snapshot in snapshots {
            self.synchronize_object(snapshot);
        }

        self.rapier.integration_parameters.dt = delta_seconds;
        self.rapier.step_with_events(&(), &self.event_collector);

        let mut states = Vec::with_capacity(self.objects.len());
        for (guid, record) in &mut self.objects {
            let Some(body) = self.rapier.bodies.get(record.body) else {
                continue;
            };
            let transform = Transform2D {
                translation: [body.translation().x, body.translation().y],
                rotation_radians: body.rotation().angle(),
                scale: record.last_transform.scale,
            };
            let linear_velocity = [body.linvel().x, body.linvel().y];
            let angular_velocity = body.angvel();
            record.last_transform = transform;
            record.settings.linear_velocity = linear_velocity;
            record.settings.angular_velocity = angular_velocity;
            states.push((*guid, transform, linear_velocity, angular_velocity));
        }
        for (guid, transform, linear_velocity, angular_velocity) in states {
            let _ = world.set_physics_state(guid, transform, linear_velocity, angular_velocity);
        }

        self.drain_collision_events()
    }

    pub fn raycast(
        &self,
        origin: [f32; 2],
        direction: [f32; 2],
        max_distance: f32,
    ) -> Result<Option<RaycastHit2D>, PhysicsError> {
        let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
        if !origin.iter().all(|value| value.is_finite())
            || !direction.iter().all(|value| value.is_finite())
            || !max_distance.is_finite()
            || max_distance <= 0.0
            || !length_squared.is_finite()
            || length_squared <= f32::EPSILON
        {
            return Err(PhysicsError::InvalidRay);
        }

        let inverse_length = length_squared.sqrt().recip();
        let unit_direction = [
            direction[0] * inverse_length,
            direction[1] * inverse_length,
        ];
        let ray = Ray::new(
            Vector::new(origin[0], origin[1]),
            Vector::new(unit_direction[0], unit_direction[1]),
        );
        let Some((collider, intersection)) = self.rapier.cast_ray_and_get_normal(
            &ray,
            max_distance,
            true,
            QueryFilter::default(),
        ) else {
            return Ok(None);
        };
        let Some(object_guid) = self.guid_by_collider.get(&collider).copied() else {
            return Ok(None);
        };

        let distance = intersection.time_of_impact;
        Ok(Some(RaycastHit2D {
            object_guid,
            point: [
                origin[0] + unit_direction[0] * distance,
                origin[1] + unit_direction[1] * distance,
            ],
            normal: [intersection.normal.x, intersection.normal.y],
            distance,
        }))
    }

    pub fn body_count(&self) -> usize {
        self.objects.len()
    }

    fn synchronize_object(&mut self, snapshot: PhysicsObjectSnapshot) {
        let settings = snapshot.body.unwrap_or_default();
        let must_rebuild = self.objects.get(&snapshot.object_guid).is_some_and(|record| {
            !same_body_settings(record.settings, settings)
                || record.collider_settings != snapshot.collider
                || record.last_transform.scale != snapshot.transform.scale
        });
        if must_rebuild {
            self.remove_object(snapshot.object_guid);
        }

        if !self.objects.contains_key(&snapshot.object_guid) {
            self.insert_object(snapshot, settings);
            return;
        }

        let record = self.objects.get_mut(&snapshot.object_guid).expect("record checked");
        let pose_changed = record.last_transform.translation != snapshot.transform.translation
            || record.last_transform.rotation_radians != snapshot.transform.rotation_radians;
        let velocity_changed = record.settings.linear_velocity != settings.linear_velocity
            || record.settings.angular_velocity != settings.angular_velocity;

        if let Some(body) = self.rapier.bodies.get_mut(record.body) {
            if pose_changed {
                let pose = rapier2d::math::Pose::new(
                    Vector::new(
                        snapshot.transform.translation[0],
                        snapshot.transform.translation[1],
                    ),
                    snapshot.transform.rotation_radians,
                );
                if settings.kind == RigidBodyKind2D::KinematicPositionBased {
                    body.set_next_kinematic_position(pose);
                } else {
                    body.set_position(pose, true);
                }
            }
            if velocity_changed {
                body.set_linvel(
                    Vector::new(settings.linear_velocity[0], settings.linear_velocity[1]),
                    true,
                );
                body.set_angvel(settings.angular_velocity, true);
            }
        }
        record.settings.linear_velocity = settings.linear_velocity;
        record.settings.angular_velocity = settings.angular_velocity;
        record.last_transform = snapshot.transform;
    }

    fn insert_object(&mut self, snapshot: PhysicsObjectSnapshot, settings: RigidBody2D) {
        let body_builder = build_body(settings, snapshot.transform);
        let (body, collider) = if let Some(collider) = snapshot.collider {
            let collider_builder = build_collider(collider, snapshot.transform.scale);
            let (body, collider) = self.rapier.insert(body_builder, collider_builder);
            (body, Some(collider))
        } else {
            (self.rapier.insert_body(body_builder), None)
        };
        if let Some(collider) = collider {
            self.guid_by_collider.insert(collider, snapshot.object_guid);
        }
        self.objects.insert(
            snapshot.object_guid,
            BodyRecord {
                body,
                collider,
                settings,
                collider_settings: snapshot.collider,
                last_transform: snapshot.transform,
            },
        );
    }

    fn remove_object(&mut self, guid: Guid) {
        if let Some(record) = self.objects.remove(&guid) {
            if let Some(collider) = record.collider {
                self.retired_colliders.push(collider);
            }
            let _ = self.rapier.remove_body(record.body);
        }
    }

    fn drain_collision_events(&mut self) -> Vec<CollisionEvent2D> {
        let mut result = Vec::new();
        while let Ok(event) = self.collision_events.try_recv() {
            let Some(object_a) = self.guid_by_collider.get(&event.collider1()).copied() else {
                continue;
            };
            let Some(object_b) = self.guid_by_collider.get(&event.collider2()).copied() else {
                continue;
            };
            let (object_a, object_b) = if object_a <= object_b {
                (object_a, object_b)
            } else {
                (object_b, object_a)
            };
            result.push(CollisionEvent2D {
                object_a,
                object_b,
                started: event.started(),
                sensor: event.sensor(),
                removed: event.removed(),
            });
        }
        result.sort_by_key(|event| (event.object_a, event.object_b, event.started));
        for collider in self.retired_colliders.drain(..) {
            self.guid_by_collider.remove(&collider);
        }
        result
    }
}
fn same_body_settings(left: RigidBody2D, right: RigidBody2D) -> bool {
    left.kind == right.kind
        && left.gravity_scale == right.gravity_scale
        && left.linear_damping == right.linear_damping
        && left.angular_damping == right.angular_damping
        && left.lock_rotation == right.lock_rotation
}

fn build_body(settings: RigidBody2D, transform: Transform2D) -> RigidBodyBuilder {
    let builder = match settings.kind {
        RigidBodyKind2D::Fixed => RigidBodyBuilder::fixed(),
        RigidBodyKind2D::Dynamic => RigidBodyBuilder::dynamic(),
        RigidBodyKind2D::KinematicPositionBased => RigidBodyBuilder::kinematic_position_based(),
    };
    let mut builder = builder
        .translation(Vector::new(
            transform.translation[0],
            transform.translation[1],
        ))
        .rotation(transform.rotation_radians)
        .linvel(Vector::new(
            settings.linear_velocity[0],
            settings.linear_velocity[1],
        ))
        .angvel(settings.angular_velocity)
        .gravity_scale(settings.gravity_scale)
        .linear_damping(settings.linear_damping)
        .angular_damping(settings.angular_damping);
    if settings.lock_rotation {
        builder = builder.lock_rotations();
    }
    builder
}

fn build_collider(settings: Collider2D, scale: [f32; 2]) -> ColliderBuilder {
    let scale_x = scale[0].abs().max(0.0001);
    let scale_y = scale[1].abs().max(0.0001);
    let mut builder = match settings.shape {
        ColliderShape2D::Circle { radius } => {
            ColliderBuilder::ball(radius * scale_x.max(scale_y))
        }
        ColliderShape2D::Box { half_extents } => {
            ColliderBuilder::cuboid(half_extents[0] * scale_x, half_extents[1] * scale_y)
        }
    };
    builder = builder
        .translation(Vector::new(settings.offset[0], settings.offset[1]))
        .rotation(settings.rotation_radians)
        .density(settings.density)
        .friction(settings.friction)
        .restitution(settings.restitution)
        .sensor(settings.sensor)
        .active_events(ActiveEvents::COLLISION_EVENTS);
    builder
}
