use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use forge_assets::{AssetCatalog, AssetChange, AssetChangeKind, AssetKind};
use forge_core::{ActionId, Guid};
use forge_editor_core::SceneEditHistory;
use forge_project::Project;
use forge_runtime::{GameRuntime, RuntimeConfig, ScriptDiagnostic};
use forge_scene::{
    AuthoredCamera, AuthoredCollider, AuthoredColliderShape, AuthoredRigidBody,
    AuthoredRigidBodyKind, AuthoredScript, AuthoredSprite, AuthoredTransform, CURRENT_SCENE_SCHEMA,
    SceneDocument, SceneObject, ScriptPropertyValue,
};
use forge_tilemap::{ChunkedTileMap, TileCell, TileCoord, TileMapDocument, TileMapHistory};

struct EditorTileMap {
    document: TileMapDocument,
    map: ChunkedTileMap,
    history: TileMapHistory,
    selected_tile: u32,
    paint_solid: bool,
    erase_mode: bool,
    fill_mode: bool,
    metadata_dirty: bool,
    resource_unsaved: bool,
    pending_stroke: BTreeMap<TileCoord, Option<TileCell>>,
}

#[derive(Clone, Copy)]
struct CanvasSprite {
    object_guid: Guid,
    asset: Option<Guid>,
    translation: [f32; 2],
    scale: [f32; 2],
    size: [f32; 2],
    tint: [f32; 4],
    uv_rect: [f32; 4],
    layer: i32,
    order: i32,
}

struct EditorApp {
    project: Project,
    scene: SceneDocument,
    scene_path: String,
    history: SceneEditHistory,
    selected: Option<Guid>,
    status: String,
    catalog: Option<AssetCatalog>,
    textures: BTreeMap<Guid, egui::TextureHandle>,
    texture_sizes: BTreeMap<Guid, [u32; 2]>,
    tilemap: Option<EditorTileMap>,
    tilemap_mode: bool,
    last_asset_scan: Option<Instant>,
    runtime: Option<GameRuntime>,
    preview_last_frame: Option<Instant>,
    preview_tick: u64,
    script_diagnostics: Vec<ScriptDiagnostic>,
    canvas_offset: Vec2,
    canvas_zoom: f32,
}

impl EditorApp {
    fn new(project: Project) -> Self {
        let scene = project.default_scene().clone();
        let scene_path = project.document().default_scene.clone();
        let selected = scene.objects.first().map(|object| object.object_guid);
        let (catalog, mut status) = match AssetCatalog::open(project.root()) {
            Ok(catalog) => (Some(catalog), String::new()),
            Err(error) => (None, format!("Asset catalog unavailable: {error}")),
        };
        let (tilemap, tilemap_error) = match load_saved_tilemap(project.root()) {
            Ok(tilemap) => (tilemap, None),
            Err(error) => (None, Some(error)),
        };
        if status.is_empty() {
            status = tilemap_error.unwrap_or_default();
        }
        Self {
            project,
            scene,
            scene_path,
            history: SceneEditHistory::new(256),
            selected,
            status,
            catalog,
            textures: BTreeMap::new(),
            texture_sizes: BTreeMap::new(),
            tilemap,
            tilemap_mode: false,
            last_asset_scan: None,
            runtime: None,
            preview_last_frame: None,
            preview_tick: 0,
            script_diagnostics: Vec::new(),
            canvas_offset: Vec2::ZERO,
            canvas_zoom: 1.0,
        }
    }

    fn add_sprite(&mut self) {
        let object = SceneObject {
            object_guid: Guid::new(),
            name: format!("Sprite {}", self.scene.objects.len() + 1),
            transform: AuthoredTransform::default(),
            sprite: Some(AuthoredSprite {
                asset: None,
                animation_clip: None,
                size: [1.0, 1.0],
                tint: [0.25, 0.72, 0.95, 1.0],
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                layer: 0,
                order: 0,
            }),
            camera: None,
            rigid_body: None,
            collider: None,
            script: None,
        };
        self.selected = Some(object.object_guid);
        match self.history.create_object(&mut self.scene, object, None) {
            Ok(()) => self.status.clear(),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn delete_selected(&mut self) {
        let Some(guid) = self.selected else {
            return;
        };
        match self.history.delete_object(&mut self.scene, guid) {
            Ok(()) => {
                self.selected = None;
                self.status.clear();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn save(&mut self) {
        let result = if self.scene_path == self.project.document().default_scene {
            self.project.save_default_scene(&self.scene)
        } else {
            self.project.save_scene(&self.scene_path, &self.scene)
        };
        match result {
            Ok(()) => {
                self.history.mark_saved();
                self.status = "Saved".into();
            }
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    fn open_scene(&mut self, relative_path: String) {
        if self.history.is_dirty() {
            self.status = "Save the current scene before opening another.".into();
            return;
        }
        match self.project.load_scene(&relative_path) {
            Ok(scene) => {
                self.selected = scene.objects.first().map(|object| object.object_guid);
                self.scene = scene;
                self.scene_path = relative_path;
                self.history = SceneEditHistory::new(256);
                self.status.clear();
            }
            Err(error) => self.status = format!("Scene open failed: {error}"),
        }
    }

    fn create_scene(&mut self) {
        if self.history.is_dirty() {
            self.status = "Save the current scene before creating another.".into();
            return;
        }
        let mut index = 2;
        let (relative_path, name) = loop {
            let path = format!("Scenes/Scene-{index}.forgescene");
            if !self.project.root().join(&path).exists() {
                break (path, format!("Scene {index}"));
            }
            index += 1;
        };
        let scene = SceneDocument {
            schema_version: CURRENT_SCENE_SCHEMA,
            resource_guid: Guid::new(),
            name,
            dependencies: Vec::new(),
            objects: Vec::new(),
        };
        match self.project.save_scene(&relative_path, &scene) {
            Ok(()) => {
                self.scene = scene;
                self.scene_path = relative_path;
                self.selected = None;
                self.history = SceneEditHistory::new(256);
                self.status = "New scene created.".into();
            }
            Err(error) => self.status = format!("Scene create failed: {error}"),
        }
    }

    fn create_tilemap(&mut self) {
        let Some(tileset_guid) = self.catalog.as_ref().and_then(|catalog| {
            catalog
                .entries()
                .find(|record| record.kind == AssetKind::Png)
                .map(|record| record.guid)
        }) else {
            self.status = "Add a PNG under Assets before creating a tilemap.".into();
            return;
        };
        let document = match TileMapDocument::empty(Guid::new(), tileset_guid, [1.0, 1.0]) {
            Ok(document) => document,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let map = match document.instantiate() {
            Ok(map) => map,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let mut history = TileMapHistory::new(256);
        history.mark_saved();
        self.tilemap = Some(EditorTileMap {
            document,
            map,
            history,
            selected_tile: 0,
            paint_solid: false,
            erase_mode: false,
            fill_mode: false,
            metadata_dirty: true,
            resource_unsaved: true,
            pending_stroke: BTreeMap::new(),
        });
        self.tilemap_mode = true;
        self.status = "New tilemap. Paint cells, then save the tilemap.".into();
    }

    fn save_tilemap(&mut self) {
        let Some(tilemap) = self.tilemap.as_ref() else {
            self.status = "Create a tilemap before saving.".into();
            return;
        };
        let mut document = match tilemap.map.to_document(
            tilemap.document.resource_guid,
            tilemap.document.tileset_guid,
            tilemap.document.tile_size,
        ) {
            Ok(document) => document,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        document.texture_tile_size = tilemap.document.texture_tile_size;
        let source = match document.to_canonical_json() {
            Ok(source) => source,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let path = match tilemap_resource_path(self.project.root()) {
            Ok(path) => path,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if let Err(error) = fs::write(path, source) {
            self.status = format!("Tilemap save failed: {error}");
            return;
        }
        let tilemap = self.tilemap.as_mut().expect("tilemap was checked above");
        tilemap.document = document;
        tilemap.history.mark_saved();
        tilemap.metadata_dirty = false;
        tilemap.resource_unsaved = false;
        self.status = "Tilemap saved.".into();
    }

    fn start_runtime(&mut self) {
        let Some(catalog) = self.catalog.as_ref() else {
            self.status = "Runtime preview needs an available asset catalog.".into();
            return;
        };
        match GameRuntime::from_scene_with_assets(&self.scene, RuntimeConfig::default(), catalog) {
            Ok(runtime) => {
                self.runtime = Some(runtime);
                self.preview_last_frame = Some(Instant::now());
                self.preview_tick = 0;
                self.script_diagnostics.clear();
                self.status = "Scene is running in the editor.".into();
            }
            Err(error) => self.status = format!("Play failed: {error}"),
        }
    }

    fn toggle_runtime_pause(&mut self) {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.set_paused(!runtime.is_paused());
            self.preview_last_frame = Some(Instant::now());
            self.status = if runtime.is_paused() {
                "Runtime paused.".into()
            } else {
                "Runtime resumed.".into()
            };
        }
    }

    fn step_runtime(&mut self) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        if let Some(frame) = runtime.step_once() {
            self.preview_tick = frame
                .first_tick
                .saturating_add(u64::from(frame.fixed_steps));
        }
        self.collect_script_diagnostics();
        self.preview_last_frame = Some(Instant::now());
    }

    fn stop_runtime(&mut self) {
        self.runtime = None;
        self.preview_last_frame = None;
        self.preview_tick = 0;
        self.status = "Runtime stopped; authored scene is unchanged.".into();
    }

    fn collect_script_diagnostics(&mut self) {
        let new_diagnostics = self
            .runtime
            .as_mut()
            .map(GameRuntime::take_script_diagnostics)
            .unwrap_or_default();
        self.script_diagnostics.extend(new_diagnostics);
        if self.script_diagnostics.len() > 256 {
            let excess = self.script_diagnostics.len() - 256;
            self.script_diagnostics.drain(..excess);
        }
    }

    fn advance_runtime(&mut self, ctx: &egui::Context) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        let now = Instant::now();
        let previous = self.preview_last_frame.replace(now).unwrap_or(now);
        let [move_x, move_y] = ctx.input(|input| {
            let left = input.key_down(egui::Key::A) || input.key_down(egui::Key::ArrowLeft);
            let right = input.key_down(egui::Key::D) || input.key_down(egui::Key::ArrowRight);
            let down = input.key_down(egui::Key::S) || input.key_down(egui::Key::ArrowDown);
            let up = input.key_down(egui::Key::W) || input.key_down(egui::Key::ArrowUp);
            [
                (if right { 1.0 } else { 0.0 }) - (if left { 1.0 } else { 0.0 }),
                (if up { 1.0 } else { 0.0 }) - (if down { 1.0 } else { 0.0 }),
            ]
        });
        let at = runtime.simulation_time();
        runtime.push_action(at, ActionId(1), move_x, move_x != 0.0);
        runtime.push_action(at, ActionId(2), move_y, move_y != 0.0);
        if !runtime.is_paused() {
            let frame = runtime.advance_frame(now.saturating_duration_since(previous));
            self.preview_tick = frame
                .first_tick
                .saturating_add(u64::from(frame.fixed_steps));
        }
        self.collect_script_diagnostics();
        if self
            .runtime
            .as_ref()
            .is_some_and(|runtime| !runtime.is_paused())
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn handle_asset_change(&mut self, change: AssetChange) {
        if self.runtime.is_none() {
            return;
        }
        if change.kind == AssetChangeKind::Removed {
            if let Some(runtime) = self.runtime.as_mut() {
                runtime.report_script_asset_issue(
                    change.guid,
                    change.relative_path,
                    "script asset was removed during play; the last loaded version remains active",
                );
            }
            return;
        }
        let updated_script = self.catalog.as_ref().and_then(|catalog| {
            let record = catalog.get(change.guid)?;
            (record.kind == AssetKind::Script).then(|| {
                (
                    record.relative_path.clone(),
                    catalog.load_script(change.guid),
                )
            })
        });
        let Some((relative_path, updated_script)) = updated_script else {
            return;
        };
        if let Some(runtime) = self.runtime.as_mut() {
            match updated_script {
                Ok(asset) => runtime.reload_script_asset(&asset),
                Err(error) => {
                    runtime.report_script_asset_issue(change.guid, relative_path, error.to_string())
                }
            }
        }
    }

    fn refresh_assets(&mut self, ctx: &egui::Context) {
        if self
            .last_asset_scan
            .is_some_and(|last| last.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        self.last_asset_scan = Some(Instant::now());

        let Some(catalog) = self.catalog.as_mut() else {
            return;
        };
        let changes = match catalog.refresh() {
            Ok(changes) => changes,
            Err(error) => {
                self.status = format!("Asset scan failed: {error}");
                return;
            }
        };
        for change in changes {
            self.textures.remove(&change.guid);
            self.texture_sizes.remove(&change.guid);
            self.handle_asset_change(change);
        }

        let mut required = self
            .scene
            .objects
            .iter()
            .filter_map(|object| object.sprite.and_then(|sprite| sprite.asset))
            .collect::<BTreeSet<_>>();
        if let Some(tilemap) = &self.tilemap {
            required.insert(tilemap.document.tileset_guid);
        }
        let mut updates = Vec::new();
        let mut errors = Vec::new();
        if let Some(catalog) = self.catalog.as_ref() {
            for guid in required {
                if self.textures.contains_key(&guid) {
                    continue;
                }
                let Some(record) = catalog.get(guid) else {
                    continue;
                };
                if record.kind != AssetKind::Png {
                    continue;
                }
                let name = record.relative_path.clone();
                match catalog.load_png(guid) {
                    Ok(texture) => {
                        updates.push((guid, name, [texture.width, texture.height], texture))
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        for (guid, name, size, decoded) in updates {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [decoded.width as usize, decoded.height as usize],
                &decoded.rgba8,
            );
            let handle = ctx.load_texture(name, image, egui::TextureOptions::NEAREST);
            self.textures.insert(guid, handle);
            self.texture_sizes.insert(guid, size);
        }
        if let Some(error) = errors.into_iter().next() {
            self.status = format!("Texture load failed: {error}");
        }
    }

    fn show_hierarchy(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scene");
        ui.label(&self.scene.name);
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for object in &self.scene.objects {
                if ui
                    .selectable_label(self.selected == Some(object.object_guid), &object.name)
                    .clicked()
                {
                    self.selected = Some(object.object_guid);
                }
            }
        });
    }

    fn show_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        let Some(guid) = self.selected else {
            ui.label("Select an object to edit it.");
            return;
        };
        let Some(index) = self
            .scene
            .objects
            .iter()
            .position(|object| object.object_guid == guid)
        else {
            self.selected = None;
            ui.label("The selected object was removed.");
            return;
        };

        let mut replacement = self.scene.objects[index].clone();
        let available_images = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .entries()
                    .filter(|record| record.kind == AssetKind::Png)
                    .map(|record| (record.guid, record.relative_path.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let available_animations = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .entries()
                    .filter(|record| record.kind == AssetKind::AnimationClip)
                    .map(|record| (record.guid, record.relative_path.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let available_scripts = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .entries()
                    .filter(|record| record.kind == AssetKind::Script)
                    .map(|record| (record.guid, record.relative_path.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut changed = false;
        let mut selected_texture_changed = false;
        let mut selected_animation_changed = false;
        ui.small(guid.to_string());
        changed |= ui.text_edit_singleline(&mut replacement.name).changed();

        ui.collapsing("Transform", |ui| {
            ui.horizontal(|ui| {
                ui.label("Position");
                changed |= ui
                    .add(egui::DragValue::new(&mut replacement.transform.translation[0]).speed(0.1))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut replacement.transform.translation[1]).speed(0.1))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Rotation");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut replacement.transform.rotation_radians)
                            .speed(0.02),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Scale");
                changed |= ui
                    .add(egui::DragValue::new(&mut replacement.transform.scale[0]).speed(0.05))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut replacement.transform.scale[1]).speed(0.05))
                    .changed();
            });
        });

        if let Some(sprite) = replacement.sprite.as_mut() {
            ui.collapsing("Sprite", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Size");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut sprite.size[0])
                                .speed(0.05)
                                .range(0.01..=10000.0),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut sprite.size[1])
                                .speed(0.05)
                                .range(0.01..=10000.0),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Texture");
                    let selected_name = sprite
                        .asset
                        .and_then(|asset| {
                            available_images
                                .iter()
                                .find(|(guid, _)| *guid == asset)
                                .map(|(_, path)| path.clone())
                        })
                        .unwrap_or_else(|| "None".into());
                    let response = egui::ComboBox::from_id_salt("sprite_texture")
                        .selected_text(selected_name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut sprite.asset, None, "None");
                            for (asset, path) in &available_images {
                                ui.selectable_value(&mut sprite.asset, Some(*asset), path);
                            }
                        });
                    selected_texture_changed |= response.response.changed();
                    changed |= response.response.changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Animation");
                    let selected_name = sprite
                        .animation_clip
                        .and_then(|clip| {
                            available_animations
                                .iter()
                                .find(|(guid, _)| *guid == clip)
                                .map(|(_, path)| path.clone())
                        })
                        .unwrap_or_else(|| "None".into());
                    let response = egui::ComboBox::from_id_salt("sprite_animation")
                        .selected_text(selected_name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut sprite.animation_clip, None, "None");
                            for (clip, path) in &available_animations {
                                ui.selectable_value(&mut sprite.animation_clip, Some(*clip), path);
                            }
                        });
                    selected_animation_changed |= response.response.changed();
                    changed |= response.response.changed();
                });
                ui.small("Place PNG images and animation metadata in the project Assets folder.");
                ui.horizontal(|ui| {
                    ui.label("Tint");
                    for channel in &mut sprite.tint {
                        changed |= ui
                            .add(egui::DragValue::new(channel).speed(0.01).range(0.0..=1.0))
                            .changed();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("UV");
                    for coordinate in &mut sprite.uv_rect {
                        changed |= ui
                            .add(
                                egui::DragValue::new(coordinate)
                                    .speed(0.01)
                                    .range(0.0..=1.0),
                            )
                            .changed();
                    }
                });
            });
        }

        ui.collapsing("Camera", |ui| {
            let mut enabled = replacement.camera.is_some();
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                replacement.camera = enabled.then_some(AuthoredCamera {
                    zoom: [1.0, 1.0],
                    pixels_per_world_unit: 32.0,
                    priority: 0,
                    active: true,
                });
                changed = true;
            }
            if let Some(camera) = replacement.camera.as_mut() {
                ui.horizontal(|ui| {
                    ui.label("Zoom");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut camera.zoom[0])
                                .speed(0.05)
                                .range(0.01..=100.0),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut camera.zoom[1])
                                .speed(0.05)
                                .range(0.01..=100.0),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Pixels / unit");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut camera.pixels_per_world_unit)
                                .speed(0.5)
                                .range(0.01..=4096.0),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Priority");
                    changed |= ui
                        .add(egui::DragValue::new(&mut camera.priority).range(-1000..=1000))
                        .changed();
                    changed |= ui.checkbox(&mut camera.active, "Active").changed();
                });
            }
        });

        ui.collapsing("Rigid body", |ui| {
            let mut enabled = replacement.rigid_body.is_some();
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                replacement.rigid_body = enabled.then_some(AuthoredRigidBody::default());
                changed = true;
            }
            if let Some(body) = replacement.rigid_body.as_mut() {
                ui.horizontal(|ui| {
                    ui.label("Kind");
                    if ui
                        .selectable_label(body.kind == AuthoredRigidBodyKind::Fixed, "Fixed")
                        .clicked()
                    {
                        body.kind = AuthoredRigidBodyKind::Fixed;
                        changed = true;
                    }
                    if ui
                        .selectable_label(body.kind == AuthoredRigidBodyKind::Dynamic, "Dynamic")
                        .clicked()
                    {
                        body.kind = AuthoredRigidBodyKind::Dynamic;
                        changed = true;
                    }
                    if ui
                        .selectable_label(
                            body.kind == AuthoredRigidBodyKind::KinematicPositionBased,
                            "Kinematic",
                        )
                        .clicked()
                    {
                        body.kind = AuthoredRigidBodyKind::KinematicPositionBased;
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Velocity");
                    changed |= ui
                        .add(egui::DragValue::new(&mut body.linear_velocity[0]).speed(0.1))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut body.linear_velocity[1]).speed(0.1))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Gravity scale");
                    changed |= ui
                        .add(egui::DragValue::new(&mut body.gravity_scale).speed(0.05))
                        .changed();
                    changed |= ui
                        .checkbox(&mut body.lock_rotation, "Lock rotation")
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Damping");
                    changed |= ui
                        .add(egui::DragValue::new(&mut body.linear_damping).speed(0.05))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut body.angular_damping).speed(0.05))
                        .changed();
                });
            }
        });

        ui.collapsing("Collider", |ui| {
            let mut enabled = replacement.collider.is_some();
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                replacement.collider = enabled.then_some(AuthoredCollider {
                    shape: AuthoredColliderShape::Box {
                        half_extents: [0.5, 0.5],
                    },
                    offset: [0.0, 0.0],
                    rotation_radians: 0.0,
                    density: 1.0,
                    friction: 0.5,
                    restitution: 0.0,
                    sensor: false,
                });
                changed = true;
            }
            if let Some(collider) = replacement.collider.as_mut() {
                ui.horizontal(|ui| {
                    ui.label("Shape");
                    if ui
                        .selectable_label(
                            matches!(collider.shape, AuthoredColliderShape::Box { .. }),
                            "Box",
                        )
                        .clicked()
                    {
                        collider.shape = AuthoredColliderShape::Box {
                            half_extents: [0.5, 0.5],
                        };
                        changed = true;
                    }
                    if ui
                        .selectable_label(
                            matches!(collider.shape, AuthoredColliderShape::Circle { .. }),
                            "Circle",
                        )
                        .clicked()
                    {
                        collider.shape = AuthoredColliderShape::Circle { radius: 0.5 };
                        changed = true;
                    }
                });
                match &mut collider.shape {
                    AuthoredColliderShape::Circle { radius } => {
                        ui.horizontal(|ui| {
                            ui.label("Radius");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(radius)
                                        .speed(0.05)
                                        .range(0.01..=10000.0),
                                )
                                .changed();
                        });
                    }
                    AuthoredColliderShape::Box { half_extents } => {
                        ui.horizontal(|ui| {
                            ui.label("Half extents");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut half_extents[0])
                                        .speed(0.05)
                                        .range(0.01..=10000.0),
                                )
                                .changed();
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut half_extents[1])
                                        .speed(0.05)
                                        .range(0.01..=10000.0),
                                )
                                .changed();
                        });
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Offset");
                    changed |= ui
                        .add(egui::DragValue::new(&mut collider.offset[0]).speed(0.05))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut collider.offset[1]).speed(0.05))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Material");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut collider.density)
                                .speed(0.05)
                                .range(0.01..=10000.0),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut collider.friction)
                                .speed(0.05)
                                .range(0.0..=10.0),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut collider.restitution)
                                .speed(0.05)
                                .range(0.0..=10.0),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut collider.rotation_radians).speed(0.02))
                        .changed();
                    changed |= ui.checkbox(&mut collider.sensor, "Sensor").changed();
                });
            }
        });

        let mut remove_script = false;
        if replacement.script.is_none() {
            if ui.button("Add Luau Script").clicked() {
                if let Some((asset, _)) = available_scripts.first() {
                    replacement.script = Some(AuthoredScript {
                        asset: *asset,
                        properties: BTreeMap::new(),
                    });
                    changed = true;
                } else {
                    self.status = "Add a .luau file under Assets before attaching a script.".into();
                }
            }
        }
        if let Some(script) = replacement.script.as_mut() {
            ui.collapsing("Luau Script", |ui| {
                let selected_name = available_scripts
                    .iter()
                    .find(|(guid, _)| *guid == script.asset)
                    .map(|(_, path)| path.clone())
                    .unwrap_or_else(|| format!("Missing asset {}", script.asset));
                let response = egui::ComboBox::from_id_salt(("script_asset", guid))
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for (asset, path) in &available_scripts {
                            ui.selectable_value(&mut script.asset, *asset, path);
                        }
                    });
                changed |= response.response.changed();
                if available_scripts.is_empty() {
                    ui.small("Place .luau scripts in the project's Assets folder.");
                }

                let names = script.properties.keys().cloned().collect::<Vec<_>>();
                let mut removed_property = None;
                for name in names {
                    ui.horizontal(|ui| {
                        ui.label(&name);
                        if let Some(value) = script.properties.get_mut(&name) {
                            match value {
                                ScriptPropertyValue::Bool(value) => {
                                    changed |= ui.checkbox(value, "").changed();
                                }
                                ScriptPropertyValue::Number(value) => {
                                    changed |=
                                        ui.add(egui::DragValue::new(value).speed(0.1)).changed();
                                }
                                ScriptPropertyValue::String(value) => {
                                    changed |= ui.text_edit_singleline(value).changed();
                                }
                            }
                        }
                        if ui.small_button("Remove").clicked() {
                            removed_property = Some(name);
                        }
                    });
                }
                if let Some(name) = removed_property {
                    script.properties.remove(&name);
                    changed = true;
                }
                if ui.button("Add property").clicked() {
                    let mut name = "property".to_owned();
                    let mut suffix = 2_u32;
                    while script.properties.contains_key(&name) {
                        name = format!("property_{suffix}");
                        suffix += 1;
                    }
                    script
                        .properties
                        .insert(name, ScriptPropertyValue::Number(0.0));
                    changed = true;
                }
                if ui.button("Remove script component").clicked() {
                    remove_script = true;
                }
            });
        }
        if remove_script {
            replacement.script = None;
            changed = true;
        }

        if selected_texture_changed {
            if let Some(sprite) = replacement.sprite.as_mut() {
                sprite.animation_clip = None;
            }
        }
        if selected_animation_changed {
            if let Some(clip_guid) = replacement.sprite.and_then(|sprite| sprite.animation_clip) {
                if let Some((texture_guid, uv_rect)) = self.catalog.as_ref().and_then(|catalog| {
                    let clip = catalog.load_animation_clip(clip_guid).ok()?;
                    let sheet = catalog.load_sprite_sheet(clip.sprite_sheet).ok()?;
                    let texture = catalog.load_png(sheet.texture).ok()?;
                    let frame = clip.frames.first()?;
                    let uv_rect = sheet
                        .uv_rect_for_frame(&frame.sprite_frame, texture.width, texture.height)
                        .ok()?;
                    Some((texture.guid, uv_rect))
                }) {
                    if let Some(sprite) = replacement.sprite.as_mut() {
                        sprite.asset = Some(texture_guid);
                        sprite.uv_rect = uv_rect;
                    }
                }
            }
        }
        if changed {
            if let Some(sprite) = replacement.sprite {
                for dependency in [sprite.asset, sprite.animation_clip].into_iter().flatten() {
                    if !self.scene.dependencies.contains(&dependency) {
                        self.scene.dependencies.push(dependency);
                    }
                }
            }
            if let Some(script) = &replacement.script {
                if !self.scene.dependencies.contains(&script.asset) {
                    self.scene.dependencies.push(script.asset);
                }
            }
            self.last_asset_scan = None;
            match self.history.replace_object(&mut self.scene, replacement) {
                Ok(()) => self.status.clear(),
                Err(error) => self.status = error.to_string(),
            }
        }
    }

    fn show_runtime_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runtime Inspector");
        let Some(guid) = self.selected else {
            ui.label("Select an object to inspect its script properties.");
            return;
        };
        let Some(mut object) = self
            .scene
            .objects
            .iter()
            .find(|object| object.object_guid == guid)
            .cloned()
        else {
            ui.label("Selected object is not in this scene.");
            return;
        };
        let Some(mut script) = object.script.clone() else {
            ui.label("Selected object has no attached script.");
            return;
        };
        ui.label(format!("Script asset: {}", script.asset));
        let mut changed = false;
        for (name, value) in &mut script.properties {
            ui.horizontal(|ui| {
                ui.label(name);
                changed |= match value {
                    ScriptPropertyValue::Bool(value) => ui.checkbox(value, "").changed(),
                    ScriptPropertyValue::Number(value) => {
                        ui.add(egui::DragValue::new(value).speed(0.1)).changed()
                    }
                    ScriptPropertyValue::String(value) => ui.text_edit_singleline(value).changed(),
                };
            });
        }
        if ui.button("Add runtime property").clicked() {
            let mut suffix = 1_u32;
            let name = loop {
                let candidate = format!("property_{suffix}");
                if !script.properties.contains_key(&candidate) {
                    break candidate;
                }
                suffix = suffix.saturating_add(1);
            };
            script
                .properties
                .insert(name, ScriptPropertyValue::Number(0.0));
            changed = true;
        }
        if !changed {
            return;
        }
        object.script = Some(script.clone());
        match self.history.replace_object(&mut self.scene, object) {
            Ok(()) => {
                let result = self
                    .runtime
                    .as_mut()
                    .map(|runtime| runtime.update_script_properties(guid, script.properties));
                match result {
                    Some(Ok(())) => self.status = "Live script properties updated.".into(),
                    Some(Err(error)) => {
                        let _ = self.history.undo(&mut self.scene);
                        self.status = format!("Live property update rejected: {error}");
                    }
                    None => self.status = "Runtime stopped before the property update.".into(),
                }
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn show_tilemap_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tilemap");
        if self.tilemap.is_none() {
            ui.label("Create a map from one of the project's PNG tilesets.");
            if ui.button("Create tilemap").clicked() {
                self.create_tilemap();
            }
            return;
        }

        let available_images = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .entries()
                    .filter(|record| record.kind == AssetKind::Png)
                    .map(|record| (record.guid, record.relative_path.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let texture_guid = self
            .tilemap
            .as_ref()
            .map(|tilemap| tilemap.document.tileset_guid)
            .expect("tilemap is present");
        let texture_size = self
            .texture_sizes
            .get(&texture_guid)
            .copied()
            .unwrap_or([0, 0]);
        let source_tile_size = self
            .tilemap
            .as_ref()
            .map(|tilemap| tilemap.document.texture_tile_size)
            .expect("tilemap is present");
        let tile_count = if source_tile_size[0] == 0 || source_tile_size[1] == 0 {
            0
        } else {
            (texture_size[0] / source_tile_size[0])
                .saturating_mul(texture_size[1] / source_tile_size[1])
        };

        let tilemap = self.tilemap.as_mut().expect("tilemap is present");
        let current_name = available_images
            .iter()
            .find(|(guid, _)| *guid == tilemap.document.tileset_guid)
            .map(|(_, path)| path.clone())
            .unwrap_or_else(|| "Missing PNG tileset".into());
        let response = egui::ComboBox::from_id_salt("tilemap_tileset")
            .selected_text(current_name)
            .show_ui(ui, |ui| {
                for (guid, path) in &available_images {
                    ui.selectable_value(&mut tilemap.document.tileset_guid, *guid, path);
                }
            });
        if response.response.changed() {
            tilemap.metadata_dirty = true;
            tilemap.selected_tile = 0;
        }

        ui.horizontal(|ui| {
            ui.label("Cell size");
            let changed_x = ui
                .add(
                    egui::DragValue::new(&mut tilemap.document.tile_size[0])
                        .speed(0.05)
                        .range(0.01..=10000.0),
                )
                .changed();
            let changed_y = ui
                .add(
                    egui::DragValue::new(&mut tilemap.document.tile_size[1])
                        .speed(0.05)
                        .range(0.01..=10000.0),
                )
                .changed();
            tilemap.metadata_dirty |= changed_x || changed_y;
        });
        ui.horizontal(|ui| {
            ui.label("PNG tile px");
            let changed_x = ui
                .add(
                    egui::DragValue::new(&mut tilemap.document.texture_tile_size[0])
                        .speed(1)
                        .range(1..=4096),
                )
                .changed();
            let changed_y = ui
                .add(
                    egui::DragValue::new(&mut tilemap.document.texture_tile_size[1])
                        .speed(1)
                        .range(1..=4096),
                )
                .changed();
            tilemap.metadata_dirty |= changed_x || changed_y;
        });

        if tile_count > 0 {
            tilemap.selected_tile = tilemap.selected_tile.min(tile_count - 1);
            ui.horizontal(|ui| {
                ui.label("Tile index");
                ui.add(egui::DragValue::new(&mut tilemap.selected_tile).range(0..=tile_count - 1));
            });
            ui.label(format!("{tile_count} tiles in the selected PNG."));
        } else {
            ui.label("No complete tiles fit the PNG tile size.");
        }
        ui.checkbox(&mut tilemap.paint_solid, "Paint cells as solid");
        ui.checkbox(&mut tilemap.erase_mode, "Erase with left drag");
        ui.checkbox(&mut tilemap.fill_mode, "Fill connected region on click");
        if tilemap.fill_mode {
            ui.label("Fill replaces the connected occupied region, up to 65,536 cells.");
        } else {
            ui.label("Left drag paints. Right drag erases.");
        }
        ui.separator();
        ui.label(format!("{} occupied cells", tilemap.map.tile_count()));
        ui.label(format!(
            "{} undo and {} redo strokes",
            tilemap.history.undo_depth(),
            tilemap.history.redo_depth()
        ));
    }

    fn show_tilemap_canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 24, 31));
        let Some(tilemap_state) = self.tilemap.as_ref() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Create a tilemap and choose a PNG tileset in the inspector.",
                egui::FontId::proportional(16.0),
                Color32::LIGHT_GRAY,
            );
            return;
        };
        let tileset_guid = tilemap_state.document.tileset_guid;
        let texture_id = self.textures.get(&tileset_guid).map(|texture| texture.id());
        let texture_size = self
            .texture_sizes
            .get(&tileset_guid)
            .copied()
            .unwrap_or([0, 0]);
        let tilemap = self.tilemap.as_mut().expect("tilemap was checked above");
        let cell_size = tilemap.document.tile_size;
        let texture_tile_size = tilemap.document.texture_tile_size;
        let pixels_per_unit = 24.0;
        let cell_pixels = Vec2::new(
            cell_size[0] * pixels_per_unit,
            cell_size[1] * pixels_per_unit,
        );
        if cell_pixels.x > 1.0 && cell_pixels.y > 1.0 {
            for index in -64..=64 {
                let offset_x = index as f32 * cell_pixels.x;
                let offset_y = index as f32 * cell_pixels.y;
                let x = rect.center().x + offset_x;
                let y = rect.center().y + offset_y;
                if x >= rect.left() && x <= rect.right() {
                    painter.line_segment(
                        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                        Stroke::new(1.0, Color32::from_gray(37)),
                    );
                }
                if y >= rect.top() && y <= rect.bottom() {
                    painter.line_segment(
                        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                        Stroke::new(1.0, Color32::from_gray(37)),
                    );
                }
            }
        }
        let cell_rect = |coordinate: TileCoord| {
            let center = rect.center()
                + Vec2::new(
                    (coordinate.x as f32 + 0.5) * cell_pixels.x,
                    (coordinate.y as f32 + 0.5) * cell_pixels.y,
                );
            Rect::from_center_size(center, cell_pixels)
        };
        let columns = if texture_tile_size[0] == 0 {
            0
        } else {
            texture_size[0] / texture_tile_size[0]
        };
        let rows = if texture_tile_size[1] == 0 {
            0
        } else {
            texture_size[1] / texture_tile_size[1]
        };
        let tile_count = columns.saturating_mul(rows);
        for (coordinate, cell) in tilemap.map.iter() {
            let tile_rect = cell_rect(coordinate);
            if !tile_rect.intersects(rect) {
                continue;
            }
            if let Some(texture_id) = texture_id.filter(|_| tile_count > 0) {
                let tile_index = cell.tile_index.min(tile_count - 1);
                let column = tile_index % columns;
                let row = tile_index / columns;
                let uv_min = Pos2::new(
                    (column * texture_tile_size[0]) as f32 / texture_size[0] as f32,
                    (row * texture_tile_size[1]) as f32 / texture_size[1] as f32,
                );
                let uv_max = Pos2::new(
                    ((column + 1) * texture_tile_size[0]) as f32 / texture_size[0] as f32,
                    ((row + 1) * texture_tile_size[1]) as f32 / texture_size[1] as f32,
                );
                painter.image(
                    texture_id,
                    tile_rect,
                    Rect::from_min_max(uv_min, uv_max),
                    Color32::WHITE,
                );
            } else {
                let fill = if cell.solid {
                    Color32::from_rgb(170, 116, 50)
                } else {
                    Color32::from_rgb(70, 115, 150)
                };
                painter.rect_filled(tile_rect.shrink(1.0), 0.0, fill);
            }
            if cell.solid {
                let color = Color32::from_rgba_unmultiplied(255, 190, 70, 160);
                painter.line_segment(
                    [tile_rect.left_top(), tile_rect.right_top()],
                    Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [tile_rect.right_top(), tile_rect.right_bottom()],
                    Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [tile_rect.right_bottom(), tile_rect.left_bottom()],
                    Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [tile_rect.left_bottom(), tile_rect.left_top()],
                    Stroke::new(2.0, color),
                );
            }
        }
        for (coordinate, cell) in &tilemap.pending_stroke {
            let tile_rect = cell_rect(*coordinate);
            if !tile_rect.intersects(rect) {
                continue;
            }
            if let Some(cell) = cell {
                painter.rect_filled(
                    tile_rect.shrink(2.0),
                    0.0,
                    if cell.solid {
                        Color32::from_rgb(170, 116, 50)
                    } else {
                        Color32::from_rgb(70, 115, 150)
                    },
                );
            } else {
                painter.rect_filled(
                    tile_rect.shrink(2.0),
                    0.0,
                    Color32::from_rgba_unmultiplied(200, 70, 70, 150),
                );
            }
        }
        painter.text(
            rect.left_top() + Vec2::new(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            "Tilemap paint canvas",
            egui::FontId::proportional(16.0),
            Color32::LIGHT_GRAY,
        );

        let mut finish_stroke = false;
        let pointer = response.interact_pointer_pos();
        if response.hovered() {
            if let Some(pointer) = pointer {
                let coordinate = TileCoord {
                    x: ((pointer.x - rect.center().x) / cell_pixels.x).floor() as i32,
                    y: ((pointer.y - rect.center().y) / cell_pixels.y).floor() as i32,
                };
                let primary_down =
                    ui.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
                let secondary_down =
                    ui.input(|input| input.pointer.button_down(egui::PointerButton::Secondary));
                if primary_down && !tilemap.fill_mode {
                    let cell = if tilemap.erase_mode {
                        None
                    } else {
                        Some(TileCell {
                            tile_index: tilemap.selected_tile,
                            flip_x: false,
                            flip_y: false,
                            rotation_quarters: 0,
                            solid: tilemap.paint_solid,
                        })
                    };
                    tilemap.pending_stroke.insert(coordinate, cell);
                } else if secondary_down {
                    tilemap.pending_stroke.insert(coordinate, None);
                }
            }
        }
        if response.clicked_by(egui::PointerButton::Primary) {
            if let Some(pointer) = pointer {
                let coordinate = TileCoord {
                    x: ((pointer.x - rect.center().x) / cell_pixels.x).floor() as i32,
                    y: ((pointer.y - rect.center().y) / cell_pixels.y).floor() as i32,
                };
                if tilemap.fill_mode {
                    let replacement = if tilemap.erase_mode {
                        None
                    } else {
                        Some(TileCell {
                            tile_index: tilemap.selected_tile,
                            flip_x: false,
                            flip_y: false,
                            rotation_quarters: 0,
                            solid: tilemap.paint_solid,
                        })
                    };
                    if tilemap.map.get(coordinate).is_none() {
                        self.status = "Connected fill must start on an occupied cell.".into();
                    } else {
                        match connected_tile_fill(&tilemap.map, coordinate, replacement) {
                            Ok(cells) => {
                                tilemap.history.execute_stroke(&mut tilemap.map, cells);
                            }
                            Err(limit) => {
                                self.status = format!(
                                    "Connected fill exceeds the {limit}-cell limit; no cells changed."
                                );
                            }
                        }
                    }
                } else {
                    let cell = if tilemap.erase_mode {
                        None
                    } else {
                        Some(TileCell {
                            tile_index: tilemap.selected_tile,
                            flip_x: false,
                            flip_y: false,
                            rotation_quarters: 0,
                            solid: tilemap.paint_solid,
                        })
                    };
                    tilemap.pending_stroke.insert(coordinate, cell);
                }
            }
            finish_stroke = true;
        }
        if response.clicked_by(egui::PointerButton::Secondary) {
            if let Some(pointer) = pointer {
                let coordinate = TileCoord {
                    x: ((pointer.x - rect.center().x) / cell_pixels.x).floor() as i32,
                    y: ((pointer.y - rect.center().y) / cell_pixels.y).floor() as i32,
                };
                tilemap.pending_stroke.insert(coordinate, None);
            }
            finish_stroke = true;
        }
        if response.drag_stopped() {
            finish_stroke = true;
        }
        if finish_stroke && !tilemap.pending_stroke.is_empty() {
            let cells = std::mem::take(&mut tilemap.pending_stroke);
            tilemap.history.execute_stroke(&mut tilemap.map, cells);
        }
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if response.dragged() {
            self.canvas_offset += ui.input(|input| input.pointer.delta());
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.canvas_zoom = (self.canvas_zoom * (scroll * 0.001).exp()).clamp(0.25, 8.0);
            }
        }
        let origin = rect.center() + self.canvas_offset;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 24, 31));
        painter.line_segment(
            [
                Pos2::new(origin.x, rect.top()),
                Pos2::new(origin.x, rect.bottom()),
            ],
            Stroke::new(1.0, Color32::from_gray(55)),
        );
        painter.line_segment(
            [
                Pos2::new(rect.left(), origin.y),
                Pos2::new(rect.right(), origin.y),
            ],
            Stroke::new(1.0, Color32::from_gray(55)),
        );

        let mut objects = if let Some(runtime) = self.runtime.as_mut() {
            runtime
                .world_mut()
                .extract_sprites()
                .into_iter()
                .map(|sprite| CanvasSprite {
                    object_guid: sprite.object_guid,
                    asset: sprite.asset,
                    translation: sprite.translation,
                    scale: sprite.scale,
                    size: sprite.size,
                    tint: sprite.tint,
                    uv_rect: sprite.uv_rect,
                    layer: sprite.layer,
                    order: sprite.order,
                })
                .collect::<Vec<_>>()
        } else {
            self.scene
                .objects
                .iter()
                .filter_map(|object| {
                    object.sprite.map(|sprite| CanvasSprite {
                        object_guid: object.object_guid,
                        asset: sprite.asset,
                        translation: object.transform.translation,
                        scale: object.transform.scale,
                        size: sprite.size,
                        tint: sprite.tint,
                        uv_rect: sprite.uv_rect,
                        layer: sprite.layer,
                        order: sprite.order,
                    })
                })
                .collect::<Vec<_>>()
        };
        objects.sort_by_key(|sprite| (sprite.layer, sprite.order, sprite.object_guid));
        let pixels_per_unit = 24.0 * self.canvas_zoom;
        for sprite in &objects {
            let center = origin
                + Vec2::new(
                    sprite.translation[0] * pixels_per_unit,
                    sprite.translation[1] * pixels_per_unit,
                );
            let extent = Vec2::new(
                (sprite.size[0] * sprite.scale[0]).abs() * pixels_per_unit,
                (sprite.size[1] * sprite.scale[1]).abs() * pixels_per_unit,
            )
            .max(Vec2::splat(2.0));
            let object_rect = Rect::from_center_size(center, extent);
            if self.selected == Some(sprite.object_guid) {
                painter.rect_filled(object_rect.expand(4.0), 0.0, Color32::YELLOW);
            }
            if let Some(texture) = sprite.asset.and_then(|asset| self.textures.get(&asset)) {
                let [u, v, width, height] = sprite.uv_rect;
                painter.image(
                    texture.id(),
                    object_rect,
                    Rect::from_min_max(Pos2::new(u, v), Pos2::new(u + width, v + height)),
                    tint_color(sprite.tint),
                );
            } else {
                painter.rect_filled(object_rect, 0.0, tint_color(sprite.tint));
            }
        }
        painter.text(
            rect.left_top() + Vec2::new(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            if self.runtime.is_some() {
                format!("Play in Editor  •  tick {}", self.preview_tick)
            } else {
                "2D Scene Preview".into()
            },
            egui::FontId::proportional(16.0),
            Color32::LIGHT_GRAY,
        );
        painter.text(
            rect.right_top() + Vec2::new(-12.0, 12.0),
            egui::Align2::RIGHT_TOP,
            format!(
                "{:.0}%  •  drag to pan, wheel to zoom",
                self.canvas_zoom * 100.0
            ),
            egui::FontId::proportional(13.0),
            Color32::LIGHT_GRAY,
        );

        if response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.selected = objects
                    .iter()
                    .rev()
                    .find(|sprite| {
                        let center = origin
                            + Vec2::new(
                                sprite.translation[0] * pixels_per_unit,
                                sprite.translation[1] * pixels_per_unit,
                            );
                        let extent = Vec2::new(
                            (sprite.size[0] * sprite.scale[0]).abs() * pixels_per_unit,
                            (sprite.size[1] * sprite.scale[1]).abs() * pixels_per_unit,
                        )
                        .max(Vec2::splat(2.0));
                        Rect::from_center_size(center, extent).contains(pointer)
                    })
                    .map(|sprite| sprite.object_guid);
            }
        }
    }
}

fn project_scene_paths(project_root: &Path) -> Vec<String> {
    let Ok(project_root) = project_root.canonicalize() else {
        return Vec::new();
    };
    let scenes_root = project_root.join("Scenes");
    let Ok(metadata) = fs::symlink_metadata(&scenes_root) else {
        return Vec::new();
    };
    if metadata.file_type().is_symlink() {
        return Vec::new();
    }
    let Ok(scenes_root) = scenes_root.canonicalize() else {
        return Vec::new();
    };
    if !scenes_root.starts_with(&project_root) {
        return Vec::new();
    }
    let mut paths = Vec::new();
    collect_scene_paths(&project_root, &scenes_root, &mut paths);
    paths.sort();
    paths
}

fn collect_scene_paths(project_root: &Path, directory: &Path, paths: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_scene_paths(project_root, &path, paths);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("forgescene"))
        {
            if let Ok(relative) = path.strip_prefix(project_root) {
                if let Some(relative) = relative.to_str() {
                    paths.push(relative.replace('\\', "/"));
                }
            }
        }
    }
}

const MAX_CONNECTED_TILE_FILL_CELLS: usize = 65_536;

fn connected_tile_fill(
    map: &ChunkedTileMap,
    seed: TileCoord,
    replacement: Option<TileCell>,
) -> Result<Vec<(TileCoord, Option<TileCell>)>, usize> {
    let Some(target) = map.get(seed) else {
        return Ok(Vec::new());
    };
    let mut pending = vec![seed];
    let mut visited = BTreeSet::new();
    let mut changes = Vec::new();
    while let Some(coordinate) = pending.pop() {
        if !visited.insert(coordinate) {
            continue;
        }
        let Some(current) = map.get(coordinate) else {
            continue;
        };
        if current.tile_index != target.tile_index || current.solid != target.solid {
            continue;
        }
        changes.push((coordinate, replacement));
        if changes.len() > MAX_CONNECTED_TILE_FILL_CELLS {
            return Err(MAX_CONNECTED_TILE_FILL_CELLS);
        }
        pending.extend([
            TileCoord {
                x: coordinate.x.saturating_sub(1),
                y: coordinate.y,
            },
            TileCoord {
                x: coordinate.x.saturating_add(1),
                y: coordinate.y,
            },
            TileCoord {
                x: coordinate.x,
                y: coordinate.y.saturating_sub(1),
            },
            TileCoord {
                x: coordinate.x,
                y: coordinate.y.saturating_add(1),
            },
        ]);
    }
    Ok(changes)
}

fn tilemap_resource_path(project_root: &Path) -> Result<PathBuf, String> {
    let project_root = project_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve project root: {error}"))?;
    let assets_root = project_root.join("Assets");
    fs::create_dir_all(&assets_root)
        .map_err(|error| format!("Cannot create project Assets folder: {error}"))?;
    let assets_root = assets_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve project Assets folder: {error}"))?;
    if !assets_root.starts_with(&project_root) {
        return Err("Project Assets folder resolves outside the project.".into());
    }
    let path = assets_root.join("Default.forgetilemap.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("Tilemap file must not be a symbolic link.".into());
        }
        Ok(_) => {
            let canonical_path = path
                .canonicalize()
                .map_err(|error| format!("Cannot resolve tilemap file: {error}"))?;
            if !canonical_path.starts_with(&assets_root) {
                return Err("Tilemap file resolves outside the Assets folder.".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot inspect tilemap file: {error}")),
    }
    Ok(path)
}

fn load_saved_tilemap(project_root: &Path) -> Result<Option<EditorTileMap>, String> {
    let path = tilemap_resource_path(project_root)?;
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read tilemap: {error}")),
    };
    let document = TileMapDocument::from_json(&source)
        .map_err(|error| format!("Tilemap is invalid: {error}"))?;
    let map = document
        .instantiate()
        .map_err(|error| format!("Tilemap cannot be opened: {error}"))?;
    let mut history = TileMapHistory::new(256);
    history.mark_saved();
    Ok(Some(EditorTileMap {
        document,
        map,
        history,
        selected_tile: 0,
        paint_solid: false,
        erase_mode: false,
        fill_mode: false,
        metadata_dirty: false,
        resource_unsaved: false,
        pending_stroke: BTreeMap::new(),
    }))
}

fn tint_color(tint: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (tint[0].clamp(0.0, 1.0) * 255.0) as u8,
        (tint[1].clamp(0.0, 1.0) * 255.0) as u8,
        (tint[2].clamp(0.0, 1.0) * 255.0) as u8,
        (tint[3].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.refresh_assets(ui.ctx());
        self.advance_runtime(ui.ctx());
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let can_edit_scene = self.runtime.is_none();
                ui.add_enabled_ui(can_edit_scene, |ui| {
                    ui.selectable_value(&mut self.tilemap_mode, false, "Scene");
                    ui.selectable_value(&mut self.tilemap_mode, true, "Tilemap");
                });
                ui.separator();
                if !self.tilemap_mode {
                    let scene_paths = project_scene_paths(self.project.root());
                    let mut selected_scene = self.scene_path.clone();
                    ui.add_enabled_ui(can_edit_scene, |ui| {
                        egui::ComboBox::from_id_salt("scene_selector")
                            .selected_text(&selected_scene)
                            .show_ui(ui, |ui| {
                                for path in &scene_paths {
                                    ui.selectable_value(&mut selected_scene, path.clone(), path);
                                }
                            });
                        if ui.button("New scene").clicked() {
                            self.create_scene();
                        }
                    });
                    if can_edit_scene && selected_scene != self.scene_path {
                        self.open_scene(selected_scene);
                    }
                }
                if self.tilemap_mode {
                    if ui.button("New tilemap").clicked() {
                        self.create_tilemap();
                    }
                    if ui
                        .add_enabled(
                            can_edit_scene && self.tilemap.is_some(),
                            egui::Button::new("Save tilemap"),
                        )
                        .clicked()
                    {
                        self.save_tilemap();
                    }
                } else {
                    if ui
                        .add_enabled(can_edit_scene, egui::Button::new("Add sprite"))
                        .clicked()
                    {
                        self.add_sprite();
                    }
                    if ui
                        .add_enabled(can_edit_scene, egui::Button::new("Delete"))
                        .clicked()
                    {
                        self.delete_selected();
                    }
                    if ui.button("Reset view").clicked() {
                        self.canvas_offset = Vec2::ZERO;
                        self.canvas_zoom = 1.0;
                    }
                }
                ui.separator();
                if !self.tilemap_mode {
                    if self.runtime.is_none() {
                        if ui.button("Play").clicked() {
                            self.start_runtime();
                        }
                    } else {
                        if ui
                            .button(
                                if self.runtime.as_ref().is_some_and(GameRuntime::is_paused) {
                                    "Resume"
                                } else {
                                    "Pause"
                                },
                            )
                            .clicked()
                        {
                            self.toggle_runtime_pause();
                        }
                        if ui
                            .add_enabled(
                                self.runtime.as_ref().is_some_and(GameRuntime::is_paused),
                                egui::Button::new("Step"),
                            )
                            .clicked()
                        {
                            self.step_runtime();
                        }
                        if ui.button("Stop").clicked() {
                            self.stop_runtime();
                        }
                    }
                    ui.separator();
                }
                let can_undo = self.runtime.is_none()
                    && if self.tilemap_mode {
                        self.tilemap
                            .as_ref()
                            .is_some_and(|tilemap| tilemap.history.undo_depth() > 0)
                    } else {
                        self.history.undo_depth() > 0
                    };
                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo"))
                    .clicked()
                {
                    if self.tilemap_mode {
                        if let Some(tilemap) = self.tilemap.as_mut() {
                            tilemap.history.undo(&mut tilemap.map);
                        }
                    } else {
                        match self.history.undo(&mut self.scene) {
                            Ok(_) => self.status.clear(),
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                }
                let can_redo = self.runtime.is_none()
                    && if self.tilemap_mode {
                        self.tilemap
                            .as_ref()
                            .is_some_and(|tilemap| tilemap.history.redo_depth() > 0)
                    } else {
                        self.history.redo_depth() > 0
                    };
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo"))
                    .clicked()
                {
                    if self.tilemap_mode {
                        if let Some(tilemap) = self.tilemap.as_mut() {
                            tilemap.history.redo(&mut tilemap.map);
                        }
                    } else {
                        match self.history.redo(&mut self.scene) {
                            Ok(_) => self.status.clear(),
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                }
                ui.separator();
                let changed = if self.tilemap_mode {
                    self.tilemap.as_ref().is_some_and(|tilemap| {
                        tilemap.resource_unsaved
                            || tilemap.metadata_dirty
                            || tilemap.history.is_dirty()
                    })
                } else {
                    self.history.is_dirty()
                };
                if !self.tilemap_mode
                    && ui
                        .add_enabled(can_edit_scene && changed, egui::Button::new("Save scene"))
                        .clicked()
                {
                    self.save();
                }
                ui.label(if changed { "Unsaved changes" } else { "Saved" });
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        });
        egui::Panel::left("hierarchy")
            .default_size(220.0)
            .show(ui, |ui| self.show_hierarchy(ui));
        egui::Panel::right("inspector")
            .default_size(320.0)
            .show(ui, |ui| {
                if self.runtime.is_some() {
                    self.show_runtime_inspector(ui);
                } else if self.tilemap_mode {
                    self.show_tilemap_inspector(ui);
                } else {
                    self.show_inspector(ui);
                }
            });
        egui::Panel::bottom("script_console")
            .default_size(130.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Script Console");
                    if ui.button("Clear").clicked() {
                        self.script_diagnostics.clear();
                    }
                    ui.label(format!("{} diagnostic(s)", self.script_diagnostics.len()));
                });
                egui::ScrollArea::vertical()
                    .max_height(100.0)
                    .show(ui, |ui| {
                        if self.script_diagnostics.is_empty() {
                            ui.weak("Play the scene to inspect Luau runtime diagnostics.");
                        } else {
                            for diagnostic in self.script_diagnostics.iter().rev() {
                                ui.label(format!(
                                    "{}  •  {}  •  tick {}  •  {}",
                                    diagnostic.source_path,
                                    diagnostic.object_guid,
                                    diagnostic.tick,
                                    diagnostic.message
                                ));
                            }
                        }
                    });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.tilemap_mode && self.runtime.is_none() {
                self.show_tilemap_canvas(ui);
            } else {
                self.show_canvas(ui);
            }
        });
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let project = match args.as_slice() {
        [path] => Project::open(PathBuf::from(path.as_str()))?,
        [flag, path, name] if flag == "--new" => {
            Project::create(PathBuf::from(path.as_str()), name.as_str())?
        }
        _ => return Err(
            "usage: forge-editor <project directory or .forgeproj path> | --new <directory> <name>"
                .into(),
        ),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Forge 2D Editor",
        options,
        Box::new(|_| Ok(Box::new(EditorApp::new(project)))),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{connected_tile_fill, ChunkedTileMap, TileCell, TileCoord, TileMapHistory};

    #[test]
    fn connected_tile_fill_respects_solid_boundaries_and_is_undoable() {
        let mut map = ChunkedTileMap::new();
        let seed = TileCell {
            tile_index: 2,
            flip_x: false,
            flip_y: false,
            rotation_quarters: 0,
            solid: false,
        };
        for x in [-1, 0, 1] {
            let _ = map.set_tile(TileCoord { x, y: 0 }, Some(seed));
        }
        let _ = map.set_tile(
            TileCoord { x: 2, y: 0 },
            Some(TileCell {
                solid: true,
                ..seed
            }),
        );
        let _ = map.set_tile(TileCoord { x: 10, y: 10 }, Some(seed));
        let replacement = TileCell {
            tile_index: 7,
            solid: true,
            ..seed
        };
        let changes = connected_tile_fill(&map, TileCoord { x: 0, y: 0 }, Some(replacement))
            .expect("bounded connected region");

        assert_eq!(changes.len(), 3);
        let mut history = TileMapHistory::new(8);
        assert!(history.execute_stroke(&mut map, changes));
        assert_eq!(map.get(TileCoord { x: -1, y: 0 }), Some(replacement));
        assert_eq!(map.get(TileCoord { x: 2, y: 0 }), Some(TileCell { solid: true, ..seed }));
        assert!(history.undo(&mut map));
        assert_eq!(map.get(TileCoord { x: 0, y: 0 }), Some(seed));
    }
}

