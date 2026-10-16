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
use forge_runtime::{BreakpointContinueResult, GameRuntime, RuntimeConfig, ScriptDiagnostic};
use forge_scene::{
    AuthoredCamera, AuthoredCollider, AuthoredColliderShape, AuthoredRigidBody,
    AuthoredRigidBodyKind, AuthoredScript, AuthoredSprite, AuthoredTransform,
    CURRENT_PREFAB_SCHEMA, CURRENT_SCENE_SCHEMA, PrefabDocument, PrefabInstance, SceneDocument,
    SceneObject, ScriptPropertyValue,
};
use forge_tilemap::{ChunkedTileMap, TileCell, TileCoord, TileMapDocument, TileMapHistory};

struct EditorTileMap {
    document: TileMapDocument,
    map: ChunkedTileMap,
    history: TileMapHistory,
    selected_tile: u32,
    paint_solid: bool,
    collision_only_mode: bool,
    collision_brush_solid: bool,
    erase_mode: bool,
    fill_mode: bool,
    metadata_dirty: bool,
    resource_unsaved: bool,
    pending_stroke: BTreeMap<TileCoord, Option<TileCell>>,
}

impl EditorTileMap {
    /// Returns None when the brush should leave this coordinate unchanged.
    fn brush_edit(&self, coordinate: TileCoord) -> Option<Option<TileCell>> {
        if self.collision_only_mode {
            return Some(self.map.get(coordinate).map(|mut cell| {
                cell.solid = self.collision_brush_solid;
                cell
            }));
        }
        if self.erase_mode {
            Some(None)
        } else {
            Some(Some(TileCell {
                tile_index: self.selected_tile,
                flip_x: false,
                flip_y: false,
                rotation_quarters: 0,
                solid: self.paint_solid,
            }))
        }
    }
}

#[derive(Clone, Copy)]
struct CanvasSprite {
    object_guid: Guid,
    asset: Option<Guid>,
    translation: [f32; 2],
    scale: [f32; 2],
    size: [f32; 2],
    anchor: [f32; 2],
    rotation_radians: f32,
    tint: [f32; 4],
    uv_rect: [f32; 4],
    layer: i32,
    order: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransformGizmoMode {
    Move,
    Rotate,
    Scale,
    Anchor,
}

#[derive(Clone, Copy)]
struct TransformGizmoDrag {
    object_guid: Guid,
    pointer_start: Pos2,
    pointer_center: Pos2,
    pointer_start_angle: f32,
    pointer_start_radius: f32,
    transform_start: AuthoredTransform,
    transform_preview: AuthoredTransform,
    anchor_start: [f32; 2],
    anchor_preview: [f32; 2],
    sprite_size: [f32; 2],
    mode: TransformGizmoMode,
}

struct EditorApp {
    project: Project,
    project_name_draft: String,
    project_settings_open: bool,
    scene: SceneDocument,
    scene_path: String,
    history: SceneEditHistory,
    selected: Option<Guid>,
    selected_asset: Option<Guid>,
    asset_validation: Option<(Guid, Result<(), String>)>,
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
    preview_actions: [f32; 2],
    breakpoint_line: u32,
    script_diagnostics: Vec<ScriptDiagnostic>,
    hidden_layers: BTreeSet<i32>,
    canvas_offset: Vec2,
    canvas_zoom: f32,
    transform_gizmo_drag: Option<TransformGizmoDrag>,
    gizmo_mode: TransformGizmoMode,
    snap_to_grid: bool,
}

impl EditorApp {
    fn new(project: Project) -> Self {
        let project_name_draft = project.document().name.clone();
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
            project_name_draft,
            project_settings_open: false,
            scene,
            scene_path,
            history: SceneEditHistory::new(256),
            selected,
            selected_asset: None,
            asset_validation: None,
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
            preview_actions: [0.0, 0.0],
            breakpoint_line: 1,
            script_diagnostics: Vec::new(),
            hidden_layers: BTreeSet::new(),
            canvas_offset: Vec2::ZERO,
            canvas_zoom: 1.0,
            transform_gizmo_drag: None,
            gizmo_mode: TransformGizmoMode::Move,
            snap_to_grid: false,
        }
    }

    fn add_sprite(&mut self) {
        let object = SceneObject {
            object_guid: Guid::new(),
            name: format!("Sprite {}", self.scene.objects.len() + 1),
            parent_guid: None,
            prefab_instance: None,
            transform: AuthoredTransform::default(),
            sprite: Some(AuthoredSprite {
                asset: None,
                animation_clip: None,
                size: [1.0, 1.0],
                anchor: [0.5, 0.5],
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
        self.selected_asset = None;
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
                self.selected_asset = None;
                self.scene = scene;
                self.selected_asset = None;
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

    fn create_prefab_from_selected(&mut self, object_guid: Guid) {
        let Some(mut object) = self
            .scene
            .objects
            .iter()
            .find(|object| object.object_guid == object_guid)
            .cloned()
        else {
            self.status = "Selected object is missing.".into();
            return;
        };
        if self.catalog.is_none() {
            self.status = "Prefab creation needs an available asset catalog.".into();
            return;
        }
        object.parent_guid = None;
        object.prefab_instance = None;
        let prefab = PrefabDocument {
            schema_version: CURRENT_PREFAB_SCHEMA,
            resource_guid: Guid::new(),
            name: object.name.clone(),
            dependencies: object_asset_dependencies(&object),
            object,
        };
        let relative_path = format!("Assets/Prefabs/{}.forgeprefab.json", prefab.resource_guid);
        if let Err(error) = self.project.save_prefab(&relative_path, &prefab) {
            self.status = format!("Prefab save failed: {error}");
            return;
        }
        let refresh = self
            .catalog
            .as_mut()
            .expect("catalog was checked above")
            .refresh();
        if let Err(error) = refresh {
            self.status = format!("Prefab was saved but the asset catalog refresh failed: {error}");
            return;
        }
        let prefab_guid = self.catalog.as_ref().and_then(|catalog| {
            catalog
                .entries()
                .find(|record| record.relative_path == relative_path)
                .map(|record| record.guid)
        });
        if let Some(prefab_guid) = prefab_guid {
            self.selected_asset = Some(prefab_guid);
            self.selected = None;
            self.asset_validation = None;
            self.status = format!("Prefab saved as {relative_path}.");
        } else {
            self.status = "Prefab was saved but did not appear in the asset catalog.".into();
        }
    }

    fn instantiate_prefab(&mut self, prefab_guid: Guid) {
        let prefab = match self
            .catalog
            .as_ref()
            .ok_or_else(|| "asset catalog is unavailable".to_owned())
            .and_then(|catalog| {
                catalog
                    .load_prefab(prefab_guid)
                    .map_err(|error| error.to_string())
            }) {
            Ok(prefab) => prefab,
            Err(error) => {
                self.status = format!("Prefab instance failed: {error}");
                return;
            }
        };
        let previous_dependencies = self.scene.dependencies.clone();
        let mut dependencies = self
            .scene
            .dependencies
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        dependencies.insert(prefab_guid);
        dependencies.extend(prefab.dependencies.iter().copied());
        self.scene.dependencies = dependencies.into_iter().collect();

        let mut instance = prefab.object.clone();
        instance.object_guid = Guid::new();
        instance.name = format!("{} Instance", prefab.name);
        instance.parent_guid = None;
        instance.prefab_instance = Some(PrefabInstance {
            prefab_asset: prefab_guid,
            overrides: BTreeSet::new(),
        });
        let object_guid = instance.object_guid;
        match self.history.create_object(&mut self.scene, instance, None) {
            Ok(()) => {
                self.selected = Some(object_guid);
                self.selected_asset = None;
                self.status = "Prefab instance created.".into();
            }
            Err(error) => {
                self.scene.dependencies = previous_dependencies;
                self.status = format!("Prefab instance failed: {error}");
            }
        }
    }

    fn refresh_prefab_instance(&mut self, object_guid: Guid) {
        let Some(current) = self
            .scene
            .objects
            .iter()
            .find(|object| object.object_guid == object_guid)
            .cloned()
        else {
            self.status = "Selected prefab instance is missing.".into();
            return;
        };
        let Some(link) = current.prefab_instance.as_ref() else {
            self.status = "Selected object is not a prefab instance.".into();
            return;
        };
        let prefab = match self
            .catalog
            .as_ref()
            .ok_or_else(|| "asset catalog is unavailable".to_owned())
            .and_then(|catalog| {
                catalog
                    .load_prefab(link.prefab_asset)
                    .map_err(|error| error.to_string())
            }) {
            Ok(prefab) => prefab,
            Err(error) => {
                self.status = format!("Prefab refresh failed: {error}");
                return;
            }
        };
        let replacement = merge_prefab_overrides(&prefab.object, &current);
        let mut dependencies = self
            .scene
            .dependencies
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        dependencies.extend(prefab.dependencies.iter().copied());
        self.scene.dependencies = dependencies.into_iter().collect();
        match self.history.replace_object(&mut self.scene, replacement) {
            Ok(()) => {
                self.status = "Prefab instance refreshed; overridden properties were kept.".into()
            }
            Err(error) => self.status = format!("Prefab refresh failed: {error}"),
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
            collision_only_mode: false,
            collision_brush_solid: false,
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
                self.selected_asset = None;
                self.runtime = Some(runtime);
                self.preview_last_frame = Some(Instant::now());
                self.preview_tick = 0;
                self.preview_actions = [0.0, 0.0];
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

    fn continue_runtime(&mut self) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        match runtime.continue_script_breakpoint() {
            BreakpointContinueResult::Hit(hit) => {
                self.status = format!(
                    "Paused at {}:{} for object {} on tick {}.",
                    hit.source_path, hit.line, hit.object_guid, hit.tick
                );
            }
            BreakpointContinueResult::FrameComplete(frame) => {
                self.preview_tick = frame
                    .first_tick
                    .saturating_add(u64::from(frame.fixed_steps));
                self.status = "Breakpoint continued; the pending frame completed.".into();
            }
            BreakpointContinueResult::NotPaused => {
                self.status = "No script breakpoint is currently paused.".into();
            }
        }
        self.collect_script_diagnostics();
        self.preview_last_frame = Some(Instant::now());
    }

    fn set_selected_breakpoint(&mut self, enabled: bool) {
        let Some(object_guid) = self.selected else {
            self.status = "Select an object with a script first.".into();
            return;
        };
        let has_script = self
            .scene
            .objects
            .iter()
            .find(|object| object.object_guid == object_guid)
            .is_some_and(|object| object.script.is_some());
        if !has_script {
            self.status = "The selected object has no Luau script.".into();
            return;
        }
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        match runtime.set_script_breakpoint(object_guid, self.breakpoint_line, enabled) {
            Ok(Some(line)) if enabled => {
                self.status = format!(
                    "Breakpoint set at line {line} (requested line {}).",
                    self.breakpoint_line
                );
            }
            Ok(Some(line)) => self.status = format!("Breakpoint removed from line {line}."),
            Ok(None) if enabled => {
                self.status = format!(
                    "No executable script line was found at or after line {}.",
                    self.breakpoint_line
                );
            }
            Ok(None) => {
                self.status = format!("No breakpoint was found for line {}.", self.breakpoint_line);
            }
            Err(error) => self.status = format!("Breakpoint update failed: {error}"),
        }
    }

    fn stop_runtime(&mut self) {
        self.runtime = None;
        self.preview_last_frame = None;
        self.preview_tick = 0;
        self.preview_actions = [0.0, 0.0];
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
        let keyboard_captured = ctx.egui_wants_keyboard_input();
        let movement = ctx.input(|input| {
            if !input.raw.focused || keyboard_captured {
                return [0.0, 0.0];
            }
            let left = input.key_down(egui::Key::A) || input.key_down(egui::Key::ArrowLeft);
            let right = input.key_down(egui::Key::D) || input.key_down(egui::Key::ArrowRight);
            let down = input.key_down(egui::Key::S) || input.key_down(egui::Key::ArrowDown);
            let up = input.key_down(egui::Key::W) || input.key_down(egui::Key::ArrowUp);
            [
                (if right { 1.0 } else { 0.0 }) - (if left { 1.0 } else { 0.0 }),
                (if up { 1.0 } else { 0.0 }) - (if down { 1.0 } else { 0.0 }),
            ]
        });
        if !runtime.is_paused() {
            let at = runtime.simulation_time();
            if movement[0] != self.preview_actions[0] {
                runtime.push_action(at, ActionId(1), movement[0], movement[0] != 0.0);
            }
            if movement[1] != self.preview_actions[1] {
                runtime.push_action(at, ActionId(2), movement[1], movement[1] != 0.0);
            }
            self.preview_actions = movement;
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
        } else if self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.current_script_breakpoint().is_some())
        {
            ctx.request_repaint();
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
            if self
                .asset_validation
                .as_ref()
                .is_some_and(|(guid, _)| *guid == change.guid)
            {
                self.asset_validation = None;
            }
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
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                for (object_guid, name, depth) in scene_hierarchy_rows(&self.scene) {
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 14.0);
                        if ui
                            .selectable_label(self.selected == Some(object_guid), name)
                            .clicked()
                        {
                            self.selected = Some(object_guid);
                            self.selected_asset = None;
                        }
                    });
                }
            });
        if self.runtime.is_none() {
            let layers = self
                .scene
                .objects
                .iter()
                .filter_map(|object| object.sprite.map(|sprite| sprite.layer))
                .collect::<BTreeSet<_>>();
            ui.collapsing("Render Layers", |ui| {
                if layers.is_empty() {
                    ui.weak("Add a sprite to create a render layer.");
                }
                for layer in &layers {
                    let mut visible = !self.hidden_layers.contains(layer);
                    if ui
                        .checkbox(&mut visible, format!("Layer {layer}"))
                        .changed()
                    {
                        if visible {
                            self.hidden_layers.remove(layer);
                        } else {
                            self.hidden_layers.insert(*layer);
                        }
                    }
                }
            });
            let assets = self
                .catalog
                .as_ref()
                .map(|catalog| {
                    catalog
                        .entries()
                        .map(|record| (record.guid, record.relative_path.clone(), record.kind))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            ui.separator();
            ui.collapsing(format!("Assets ({})", assets.len()), |ui| {
                ui.weak(
                    "Drop PNG, Luau, sprite-sheet, animation, or prefab files into the editor to import.",
                );
                if assets.is_empty() {
                    ui.weak("The project Assets folder is empty.");
                }
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for (guid, path, kind) in &assets {
                            if ui
                                .selectable_label(self.selected_asset == Some(*guid), path)
                                .clicked()
                            {
                                self.selected_asset = Some(*guid);
                                self.selected = None;
                            }
                            ui.small(format!("{kind:?} • {guid}"));
                        }
                    });
            });
        }
    }

    fn show_project_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Project Settings");
        ui.label(format!("Project folder: {}", self.project.root().display()));
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.project_name_draft);
        });
        if ui.button("Save project name").clicked() {
            match self.project.update_name(self.project_name_draft.clone()) {
                Ok(()) => {
                    self.project_name_draft = self.project.document().name.clone();
                    self.status = "Project settings saved.".into();
                }
                Err(error) => self.status = format!("Project settings failed: {error}"),
            }
        }
        ui.separator();
        ui.label("Default scene");
        ui.label(&self.project.document().default_scene);
        let can_change_default = !self.history.is_dirty();
        if ui
            .add_enabled(
                can_change_default,
                egui::Button::new("Set current scene as default"),
            )
            .clicked()
        {
            match self.project.set_default_scene(&self.scene_path) {
                Ok(()) => self.status = "Default scene updated.".into(),
                Err(error) => self.status = format!("Default scene update failed: {error}"),
            }
        }
        if !can_change_default {
            ui.weak("Save the current scene before changing the default scene.");
        }
    }

    fn show_asset_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Asset Inspector");
        let Some(guid) = self.selected_asset else {
            ui.label("Select an asset from the browser.");
            return;
        };
        let asset_info = self.catalog.as_ref().and_then(|catalog| {
            catalog
                .get(guid)
                .map(|record| (record.relative_path.clone(), record.kind))
        });
        let Some((path, kind)) = asset_info else {
            ui.colored_label(
                Color32::RED,
                "This asset is missing from the project catalog.",
            );
            ui.label(format!("GUID: {guid}"));
            return;
        };
        ui.label(&path);
        ui.label(format!("Type: {kind:?}"));
        ui.label(format!("GUID: {guid}"));
        if ui.button("Validate resource").clicked() {
            let result = self.validate_asset(guid);
            self.asset_validation = Some((guid, result));
        }
        if kind == AssetKind::Prefab && ui.button("Create instance in scene").clicked() {
            self.instantiate_prefab(guid);
        }
        if let Some((validated_guid, result)) = &self.asset_validation {
            if *validated_guid == guid {
                match result {
                    Ok(()) => {
                        ui.label("Resource loaded successfully.");
                    }
                    Err(error) => {
                        ui.colored_label(Color32::RED, format!("Import diagnostic: {error}"));
                    }
                }
            }
        }
    }

    fn import_dropped_assets(&mut self, ui: &egui::Ui) {
        if self.runtime.is_some() {
            return;
        }
        let (drop_count, paths) = ui.input(|input| {
            let files = &input.raw.dropped_files;
            (
                files.len(),
                files
                    .iter()
                    .filter_map(|file| {
                        let path = file.path();
                        (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
                    })
                    .collect::<Vec<_>>(),
            )
        });
        if drop_count == 0 {
            return;
        }

        let mut imported = Vec::new();
        let mut issues = Vec::new();
        for source in paths {
            match import_asset_file(self.project.root(), &source) {
                Ok(relative_path) => imported.push(relative_path),
                Err(error) => issues.push(error),
            }
        }
        self.last_asset_scan = None;
        let pathless_drops = drop_count.saturating_sub(imported.len() + issues.len());
        if pathless_drops > 0 {
            issues.push(format!(
                "{pathless_drops} dropped item(s) did not provide a file path"
            ));
        }
        if imported.is_empty() {
            self.status = format!("Asset import failed: {}", issues.join("; "));
        } else if issues.is_empty() {
            self.status = format!(
                "Imported {} asset(s); the catalog will assign stable GUIDs.",
                imported.len()
            );
        } else {
            self.status = format!(
                "Imported {} asset(s); {} issue(s): {}",
                imported.len(),
                issues.len(),
                issues.join("; ")
            );
        }
    }

    fn validate_asset(&self, guid: Guid) -> Result<(), String> {
        let catalog = self
            .catalog
            .as_ref()
            .ok_or_else(|| "asset catalog is unavailable".to_owned())?;
        let record = catalog
            .get(guid)
            .ok_or_else(|| format!("asset {guid} is missing from the project catalog"))?;
        match record.kind {
            AssetKind::Png => catalog.load_png(guid).map(|_| ()),
            AssetKind::SpriteSheet => catalog.load_sprite_sheet(guid).map(|_| ()),
            AssetKind::AnimationClip => catalog.load_animation_clip(guid).map(|_| ()),
            AssetKind::Script => catalog.load_script(guid).map(|_| ()),
            AssetKind::Prefab => catalog.load_prefab(guid).map(|_| ()),
        }
        .map_err(|error| error.to_string())
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
        if replacement.prefab_instance.is_some() {
            if ui.button("Refresh from prefab").clicked() {
                self.refresh_prefab_instance(guid);
                return;
            }
        } else if ui.button("Save as prefab").clicked() {
            self.create_prefab_from_selected(guid);
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
        ui.horizontal(|ui| {
            ui.label("Parent");
            let selected_parent = replacement
                .parent_guid
                .and_then(|parent_guid| {
                    self.scene
                        .objects
                        .iter()
                        .find(|object| object.object_guid == parent_guid)
                        .map(|object| object.name.clone())
                })
                .unwrap_or_else(|| "None".into());
            let response = egui::ComboBox::from_id_salt(("parent_object", guid))
                .selected_text(selected_parent)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut replacement.parent_guid, None, "None");
                    for object in self
                        .scene
                        .objects
                        .iter()
                        .filter(|object| object.object_guid != guid)
                    {
                        ui.selectable_value(
                            &mut replacement.parent_guid,
                            Some(object.object_guid),
                            &object.name,
                        );
                    }
                });
            changed |= response.response.changed();
        });

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

        if replacement.sprite.is_none() && ui.button("Add Sprite component").clicked() {
            replacement.sprite = Some(AuthoredSprite {
                asset: None,
                animation_clip: None,
                size: [1.0, 1.0],
                anchor: [0.5, 0.5],
                tint: [1.0; 4],
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                layer: 0,
                order: 0,
            });
            changed = true;
        }
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
                    ui.label("Anchor");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut sprite.anchor[0])
                                .speed(0.01)
                                .range(0.0..=1.0),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut sprite.anchor[1])
                                .speed(0.01)
                                .range(0.0..=1.0),
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
                ui.horizontal(|ui| {
                    ui.label("Render layer");
                    changed |= ui
                        .add(egui::DragValue::new(&mut sprite.layer).range(-10_000..=10_000))
                        .changed();
                    ui.label("Order");
                    changed |= ui
                        .add(egui::DragValue::new(&mut sprite.order).range(-10_000..=10_000))
                        .changed();
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
            if let Some(before) = self.scene.objects.get(index) {
                record_prefab_overrides(before, &mut replacement);
            }
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
            if let Some(instance) = &replacement.prefab_instance {
                if !self.scene.dependencies.contains(&instance.prefab_asset) {
                    self.scene.dependencies.push(instance.prefab_asset);
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
        let collision_mode = ui.checkbox(
            &mut tilemap.collision_only_mode,
            "Collision-only brush (preserve tile art)",
        );
        if collision_mode.changed() && tilemap.collision_only_mode {
            tilemap.fill_mode = false;
            tilemap.erase_mode = false;
        }
        if tilemap.collision_only_mode {
            ui.checkbox(
                &mut tilemap.collision_brush_solid,
                "Mark brushed cells as solid",
            );
            ui.label("Left drag changes collision on occupied cells. Right drag erases tiles.");
        } else {
            ui.checkbox(&mut tilemap.paint_solid, "New tiles are solid");
            ui.checkbox(&mut tilemap.erase_mode, "Erase with left drag");
            ui.checkbox(&mut tilemap.fill_mode, "Fill connected region on click");
            if tilemap.fill_mode {
                ui.label("Fill replaces the connected occupied region, up to 65,536 cells.");
            } else {
                ui.label("Left drag paints. Right drag erases.");
            }
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
                if primary_down && (!tilemap.fill_mode || tilemap.collision_only_mode) {
                    if let Some(cell) = tilemap.brush_edit(coordinate) {
                        tilemap.pending_stroke.insert(coordinate, cell);
                    }
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
                if tilemap.fill_mode && !tilemap.collision_only_mode {
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
                } else if let Some(cell) = tilemap.brush_edit(coordinate) {
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
        if self.runtime.is_some() {
            self.transform_gizmo_drag = None;
        }
        let pixels_per_unit = 24.0 * self.canvas_zoom;
        let origin_before_drag = rect.center() + self.canvas_offset;
        if response.drag_started() && self.runtime.is_none() {
            let target = response.interact_pointer_pos().and_then(|pointer| {
                self.scene
                    .objects
                    .iter()
                    .filter_map(|object| {
                        let sprite = object.sprite?;
                        if self.hidden_layers.contains(&sprite.layer) {
                            return None;
                        }
                        let transform = self.scene.world_transform(object.object_guid)?;
                        let pivot = origin_before_drag
                            + Vec2::new(
                                transform.translation[0] * pixels_per_unit,
                                transform.translation[1] * pixels_per_unit,
                            );
                        let corners = sprite_canvas_corners(
                            origin_before_drag,
                            pixels_per_unit,
                            transform.translation,
                            transform.rotation_radians,
                            transform.scale,
                            sprite.size,
                            sprite.anchor,
                        );
                        let object_rect = Rect::from_points(&corners);
                        let is_selected = self.selected == Some(object.object_guid);
                        let hit = match self.gizmo_mode {
                            TransformGizmoMode::Move => object_rect.contains(pointer),
                            TransformGizmoMode::Rotate if is_selected => {
                                let radius = object_rect.size().length() * 0.55 + 12.0;
                                ((pointer - pivot).length() - radius).abs() <= 8.0
                            }
                            TransformGizmoMode::Scale if is_selected => {
                                let handle = object_rect.right_bottom() + Vec2::splat(8.0);
                                Rect::from_center_size(handle, Vec2::splat(20.0)).contains(pointer)
                            }
                            TransformGizmoMode::Anchor if is_selected => {
                                pointer.distance(pivot) <= 10.0
                            }
                            _ => false,
                        };
                        hit.then_some((
                            (sprite.layer, sprite.order, object.object_guid),
                            (
                                object.object_guid,
                                transform,
                                pivot,
                                sprite.anchor,
                                sprite.size,
                            ),
                        ))
                    })
                    .max_by_key(|(draw_order, _)| *draw_order)
                    .map(|(_, target)| target)
            });
            self.transform_gizmo_drag = target.map(
                |(object_guid, transform, pointer_center, anchor, sprite_size)| {
                    self.selected = Some(object_guid);
                    self.selected_asset = None;
                    let pointer_start = response
                        .interact_pointer_pos()
                        .unwrap_or(origin_before_drag);
                    let pointer_offset = pointer_start - pointer_center;
                    TransformGizmoDrag {
                        object_guid,
                        pointer_start,
                        pointer_center,
                        pointer_start_angle: pointer_offset.y.atan2(pointer_offset.x),
                        pointer_start_radius: pointer_offset.length().max(1.0),
                        transform_start: transform,
                        transform_preview: transform,
                        anchor_start: anchor,
                        anchor_preview: anchor,
                        sprite_size,
                        mode: self.gizmo_mode,
                    }
                },
            );
        }
        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                if let Some(drag) = self.transform_gizmo_drag.as_mut() {
                    let mut preview = drag.transform_start;
                    match drag.mode {
                        TransformGizmoMode::Move => {
                            let delta = (pointer - drag.pointer_start) / pixels_per_unit;
                            preview.translation = [
                                drag.transform_start.translation[0] + delta.x,
                                drag.transform_start.translation[1] + delta.y,
                            ];
                            if self.snap_to_grid {
                                for value in &mut preview.translation {
                                    *value = (*value / 0.5).round() * 0.5;
                                }
                            }
                        }
                        TransformGizmoMode::Rotate => {
                            let pointer_offset = pointer - drag.pointer_center;
                            let angle = pointer_offset.y.atan2(pointer_offset.x);
                            preview.rotation_radians = drag.transform_start.rotation_radians
                                + wrap_radians(angle - drag.pointer_start_angle);
                            if self.snap_to_grid {
                                let step = std::f32::consts::PI / 12.0;
                                preview.rotation_radians =
                                    (preview.rotation_radians / step).round() * step;
                            }
                        }
                        TransformGizmoMode::Scale => {
                            let pointer_offset = pointer - drag.pointer_center;
                            let factor = (pointer_offset.length() / drag.pointer_start_radius)
                                .clamp(0.01, 100.0);
                            for value in &mut preview.scale {
                                let sign = value.signum();
                                *value = (value.abs() * factor).clamp(0.01, 10_000.0) * sign;
                                if self.snap_to_grid {
                                    *value = (*value * 10.0).round() / 10.0;
                                }
                            }
                        }
                        TransformGizmoMode::Anchor => {
                            let screen_delta = (pointer - drag.pointer_start) / pixels_per_unit;
                            let (sin, cos) = drag.transform_start.rotation_radians.sin_cos();
                            let local_delta = Vec2::new(
                                cos * screen_delta.x + sin * screen_delta.y,
                                -sin * screen_delta.x + cos * screen_delta.y,
                            );
                            let size = Vec2::new(
                                drag.sprite_size[0] * drag.transform_start.scale[0],
                                drag.sprite_size[1] * drag.transform_start.scale[1],
                            );
                            let mut anchor = drag.anchor_start;
                            for (index, delta) in
                                [local_delta.x, local_delta.y].into_iter().enumerate()
                            {
                                let divisor = if size[index].abs() < 0.0001 {
                                    0.0001_f32.copysign(size[index])
                                } else {
                                    size[index]
                                };
                                anchor[index] =
                                    (drag.anchor_start[index] + delta / divisor).clamp(0.0, 1.0);
                                if self.snap_to_grid {
                                    anchor[index] = (anchor[index] * 20.0).round() / 20.0;
                                }
                            }
                            let anchor_delta = Vec2::new(
                                anchor[0] - drag.anchor_start[0],
                                anchor[1] - drag.anchor_start[1],
                            );
                            let local_offset =
                                Vec2::new(anchor_delta.x * size.x, anchor_delta.y * size.y);
                            preview.translation[0] += cos * local_offset.x - sin * local_offset.y;
                            preview.translation[1] += sin * local_offset.x + cos * local_offset.y;
                            drag.anchor_preview = anchor;
                        }
                    }
                    drag.transform_preview = preview;
                } else {
                    self.canvas_offset += ui.input(|input| input.pointer.delta());
                }
            }
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
                    anchor: sprite.anchor,
                    rotation_radians: sprite.rotation_radians,
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
                    let transform = self.scene.world_transform(object.object_guid)?;
                    object.sprite.map(|sprite| CanvasSprite {
                        object_guid: object.object_guid,
                        asset: sprite.asset,
                        translation: transform.translation,
                        scale: transform.scale,
                        size: sprite.size,
                        anchor: sprite.anchor,
                        rotation_radians: transform.rotation_radians,
                        tint: sprite.tint,
                        uv_rect: sprite.uv_rect,
                        layer: sprite.layer,
                        order: sprite.order,
                    })
                })
                .collect::<Vec<_>>()
        };
        objects.sort_by_key(|sprite| (sprite.layer, sprite.order, sprite.object_guid));
        if self.runtime.is_none() {
            objects.retain(|sprite| !self.hidden_layers.contains(&sprite.layer));
        }
        if let Some(drag) = self.transform_gizmo_drag {
            if let Some(sprite) = objects
                .iter_mut()
                .find(|sprite| sprite.object_guid == drag.object_guid)
            {
                sprite.translation = drag.transform_preview.translation;
                sprite.scale = drag.transform_preview.scale;
                sprite.rotation_radians = drag.transform_preview.rotation_radians;
                sprite.anchor = drag.anchor_preview;
            }
        }
        for sprite in &objects {
            let corners = sprite_canvas_corners(
                origin,
                pixels_per_unit,
                sprite.translation,
                sprite.rotation_radians,
                sprite.scale,
                sprite.size,
                sprite.anchor,
            );
            let object_rect = Rect::from_points(&corners);
            let pivot = origin
                + Vec2::new(
                    sprite.translation[0] * pixels_per_unit,
                    sprite.translation[1] * pixels_per_unit,
                );
            let sprite_center = Pos2::new(
                corners.iter().map(|point| point.x).sum::<f32>() / 4.0,
                corners.iter().map(|point| point.y).sum::<f32>() / 4.0,
            );
            let extent = Vec2::new(
                (sprite.size[0] * sprite.scale[0]).abs() * pixels_per_unit,
                (sprite.size[1] * sprite.scale[1]).abs() * pixels_per_unit,
            )
            .max(Vec2::splat(2.0));
            let local_rect = Rect::from_center_size(sprite_center, extent);
            let (sin, cos) = sprite.rotation_radians.sin_cos();
            if let Some(texture) = sprite.asset.and_then(|asset| self.textures.get(&asset)) {
                let [u, v, width, height] = sprite.uv_rect;
                let mut mesh = egui::Mesh::with_texture(texture.id());
                mesh.add_rect_with_uv(
                    local_rect,
                    Rect::from_min_max(Pos2::new(u, v), Pos2::new(u + width, v + height)),
                    tint_color(sprite.tint),
                );
                for vertex in &mut mesh.vertices {
                    let offset = vertex.pos - sprite_center;
                    vertex.pos = sprite_center
                        + Vec2::new(
                            cos * offset.x - sin * offset.y,
                            sin * offset.x + cos * offset.y,
                        );
                }
                painter.add(egui::Shape::mesh(mesh));
            } else {
                painter.add(egui::Shape::convex_polygon(
                    corners.to_vec(),
                    tint_color(sprite.tint),
                    Stroke::NONE,
                ));
            }
        }
        if let Some(sprite) = objects
            .iter()
            .find(|sprite| self.selected == Some(sprite.object_guid))
        {
            let corners = sprite_canvas_corners(
                origin,
                pixels_per_unit,
                sprite.translation,
                sprite.rotation_radians,
                sprite.scale,
                sprite.size,
                sprite.anchor,
            );
            let object_rect = Rect::from_points(&corners);
            let pivot = origin
                + Vec2::new(
                    sprite.translation[0] * pixels_per_unit,
                    sprite.translation[1] * pixels_per_unit,
                );
            for index in 0..4 {
                painter.line_segment(
                    [corners[index], corners[(index + 1) % 4]],
                    Stroke::new(2.0, Color32::YELLOW),
                );
            }
            if self.runtime.is_none() {
                match self.gizmo_mode {
                    TransformGizmoMode::Move => {
                        painter.line_segment(
                            [pivot, pivot + Vec2::new(30.0, 0.0)],
                            Stroke::new(2.5, Color32::from_rgb(240, 90, 80)),
                        );
                        painter.line_segment(
                            [pivot, pivot + Vec2::new(0.0, 30.0)],
                            Stroke::new(2.5, Color32::from_rgb(90, 220, 130)),
                        );
                    }
                    TransformGizmoMode::Rotate => {
                        let radius = object_rect.size().length() * 0.55 + 12.0;
                        painter.circle_stroke(
                            pivot,
                            radius,
                            Stroke::new(2.0, Color32::from_rgb(100, 170, 255)),
                        );
                        painter.circle_filled(
                            pivot + Vec2::new(0.0, -radius),
                            5.0,
                            Color32::from_rgb(100, 170, 255),
                        );
                    }
                    TransformGizmoMode::Scale => {
                        painter.rect_stroke(
                            object_rect.expand(8.0),
                            0.0,
                            Stroke::new(2.0, Color32::from_rgb(255, 180, 70)),
                            egui::StrokeKind::Middle,
                        );
                        painter.rect_filled(
                            Rect::from_center_size(
                                object_rect.right_bottom() + Vec2::splat(8.0),
                                Vec2::splat(10.0),
                            ),
                            0.0,
                            Color32::from_rgb(255, 180, 70),
                        );
                    }
                    TransformGizmoMode::Anchor => {
                        painter.line_segment(
                            [pivot - Vec2::new(9.0, 0.0), pivot + Vec2::new(9.0, 0.0)],
                            Stroke::new(2.0, Color32::from_rgb(255, 180, 70)),
                        );
                        painter.line_segment(
                            [pivot - Vec2::new(0.0, 9.0), pivot + Vec2::new(0.0, 9.0)],
                            Stroke::new(2.0, Color32::from_rgb(255, 180, 70)),
                        );
                        painter.circle_stroke(
                            pivot,
                            6.0,
                            Stroke::new(2.0, Color32::from_rgb(255, 180, 70)),
                        );
                    }
                }
                painter.circle_filled(pivot, 3.0, Color32::WHITE);
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
                "{:.0}%  •  drag the selected object to {:?}, empty space to pan",
                self.canvas_zoom * 100.0,
                self.gizmo_mode
            ),
            egui::FontId::proportional(13.0),
            Color32::LIGHT_GRAY,
        );

        if response.clicked() {
            self.selected_asset = None;
            if let Some(pointer) = response.interact_pointer_pos() {
                self.selected = objects
                    .iter()
                    .rev()
                    .find(|sprite| {
                        let corners = sprite_canvas_corners(
                            origin,
                            pixels_per_unit,
                            sprite.translation,
                            sprite.rotation_radians,
                            sprite.scale,
                            sprite.size,
                            sprite.anchor,
                        );
                        Rect::from_points(&corners).contains(pointer)
                    })
                    .map(|sprite| sprite.object_guid);
            }
        }
        if response.drag_stopped() {
            if let Some(drag) = self.transform_gizmo_drag.take() {
                if drag.transform_preview != drag.transform_start
                    || drag.anchor_preview != drag.anchor_start
                {
                    if let Some(mut replacement) = self
                        .scene
                        .objects
                        .iter()
                        .find(|object| object.object_guid == drag.object_guid)
                        .cloned()
                    {
                        let before = replacement.clone();
                        if let Some(local_transform) = world_transform_to_local(
                            &self.scene,
                            drag.object_guid,
                            drag.transform_preview,
                        ) {
                            replacement.transform = local_transform;
                            if let Some(sprite) = replacement.sprite.as_mut() {
                                sprite.anchor = drag.anchor_preview;
                            }
                            record_prefab_overrides(&before, &mut replacement);
                            if let Err(error) =
                                self.history.replace_object(&mut self.scene, replacement)
                            {
                                self.status = error.to_string();
                            }
                        } else {
                            self.status =
                                "This transform cannot be represented in the selected parent."
                                    .into();
                        }
                    }
                }
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

fn sprite_canvas_corners(
    origin: Pos2,
    pixels_per_unit: f32,
    translation: [f32; 2],
    rotation_radians: f32,
    scale: [f32; 2],
    size: [f32; 2],
    anchor: [f32; 2],
) -> [Pos2; 4] {
    let (sin, cos) = rotation_radians.sin_cos();
    let point = |u: f32, v: f32| {
        let local_x = (u - anchor[0]) * size[0] * scale[0];
        let local_y = (v - anchor[1]) * size[1] * scale[1];
        origin
            + Vec2::new(
                translation[0] * pixels_per_unit
                    + (cos * local_x - sin * local_y) * pixels_per_unit,
                translation[1] * pixels_per_unit
                    + (sin * local_x + cos * local_y) * pixels_per_unit,
            )
    };
    [
        point(0.0, 0.0),
        point(1.0, 0.0),
        point(1.0, 1.0),
        point(0.0, 1.0),
    ]
}

fn scene_hierarchy_rows(scene: &SceneDocument) -> Vec<(Guid, String, usize)> {
    let mut children = BTreeMap::<Option<Guid>, Vec<(Guid, String)>>::new();
    for object in &scene.objects {
        children
            .entry(object.parent_guid)
            .or_default()
            .push((object.object_guid, object.name.clone()));
    }
    fn append_rows(
        parent_guid: Option<Guid>,
        depth: usize,
        children: &BTreeMap<Option<Guid>, Vec<(Guid, String)>>,
        rows: &mut Vec<(Guid, String, usize)>,
    ) {
        if let Some(objects) = children.get(&parent_guid) {
            for (object_guid, name) in objects {
                rows.push((*object_guid, name.clone(), depth));
                append_rows(Some(*object_guid), depth + 1, children, rows);
            }
        }
    }
    let mut rows = Vec::with_capacity(scene.objects.len());
    append_rows(None, 0, &children, &mut rows);
    rows
}

fn world_transform_to_local(
    scene: &SceneDocument,
    object_guid: Guid,
    world: AuthoredTransform,
) -> Option<AuthoredTransform> {
    let object = scene
        .objects
        .iter()
        .find(|object| object.object_guid == object_guid)?;
    let Some(parent_guid) = object.parent_guid else {
        return Some(world);
    };
    let parent = scene.world_transform(parent_guid)?;
    if parent.scale.iter().any(|value| value.abs() <= f32::EPSILON) {
        return None;
    }
    let (parent_sin, parent_cos) = parent.rotation_radians.sin_cos();
    let delta = [
        world.translation[0] - parent.translation[0],
        world.translation[1] - parent.translation[1],
    ];
    let translation = [
        (parent_cos * delta[0] + parent_sin * delta[1]) / parent.scale[0],
        (-parent_sin * delta[0] + parent_cos * delta[1]) / parent.scale[1],
    ];
    let (world_sin, world_cos) = world.rotation_radians.sin_cos();
    let world_x = [world_cos * world.scale[0], world_sin * world.scale[0]];
    let world_y = [-world_sin * world.scale[1], world_cos * world.scale[1]];
    let local_x = [
        (parent_cos * world_x[0] + parent_sin * world_x[1]) / parent.scale[0],
        (-parent_sin * world_x[0] + parent_cos * world_x[1]) / parent.scale[1],
    ];
    let local_y = [
        (parent_cos * world_y[0] + parent_sin * world_y[1]) / parent.scale[0],
        (-parent_sin * world_y[0] + parent_cos * world_y[1]) / parent.scale[1],
    ];
    let x_length = local_x[0].hypot(local_x[1]);
    let y_length = local_y[0].hypot(local_y[1]);
    let dot = local_x[0] * local_y[0] + local_x[1] * local_y[1];
    if x_length > f32::EPSILON
        && y_length > f32::EPSILON
        && dot.abs() > 1.0e-4 * x_length * y_length
    {
        return None;
    }
    let determinant = local_x[0] * local_y[1] - local_x[1] * local_y[0];
    let (rotation_radians, scale) = if x_length > f32::EPSILON {
        (
            local_x[1].atan2(local_x[0]),
            [x_length, determinant / x_length],
        )
    } else if y_length > f32::EPSILON {
        (
            local_y[1].atan2(local_y[0]) - std::f32::consts::FRAC_PI_2,
            [0.0, y_length],
        )
    } else {
        (0.0, [0.0, 0.0])
    };
    let local = AuthoredTransform {
        translation,
        rotation_radians,
        scale,
    };
    (local.translation.iter().all(|value| value.is_finite())
        && local.scale.iter().all(|value| value.is_finite())
        && local.rotation_radians.is_finite())
    .then_some(local)
}

fn wrap_radians(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn object_asset_dependencies(object: &SceneObject) -> Vec<Guid> {
    let mut dependencies = BTreeSet::new();
    if let Some(sprite) = object.sprite {
        dependencies.extend([sprite.asset, sprite.animation_clip].into_iter().flatten());
    }
    if let Some(script) = &object.script {
        dependencies.insert(script.asset);
    }
    dependencies.into_iter().collect()
}

fn record_prefab_overrides(before: &SceneObject, after: &mut SceneObject) {
    let Some(instance) = after.prefab_instance.as_mut() else {
        return;
    };
    let overrides = &mut instance.overrides;
    if before.name != after.name {
        overrides.insert("object.name".into());
    }
    if before.parent_guid != after.parent_guid {
        overrides.insert("object.parent".into());
    }
    for (changed, identifier) in [
        (
            before.transform.translation[0] != after.transform.translation[0],
            "transform.translation.x",
        ),
        (
            before.transform.translation[1] != after.transform.translation[1],
            "transform.translation.y",
        ),
        (
            before.transform.rotation_radians != after.transform.rotation_radians,
            "transform.rotation",
        ),
        (
            before.transform.scale[0] != after.transform.scale[0],
            "transform.scale.x",
        ),
        (
            before.transform.scale[1] != after.transform.scale[1],
            "transform.scale.y",
        ),
    ] {
        if changed {
            overrides.insert(identifier.into());
        }
    }
    if before.sprite != after.sprite {
        match (&before.sprite, &after.sprite) {
            (Some(before), Some(after)) => {
                for (changed, identifier) in [
                    (before.asset != after.asset, "component.sprite.asset"),
                    (
                        before.animation_clip != after.animation_clip,
                        "component.sprite.animation_clip",
                    ),
                    (before.size[0] != after.size[0], "component.sprite.size.x"),
                    (before.size[1] != after.size[1], "component.sprite.size.y"),
                    (
                        before.anchor[0] != after.anchor[0],
                        "component.sprite.anchor.x",
                    ),
                    (
                        before.anchor[1] != after.anchor[1],
                        "component.sprite.anchor.y",
                    ),
                    (before.tint[0] != after.tint[0], "component.sprite.tint.r"),
                    (before.tint[1] != after.tint[1], "component.sprite.tint.g"),
                    (before.tint[2] != after.tint[2], "component.sprite.tint.b"),
                    (before.tint[3] != after.tint[3], "component.sprite.tint.a"),
                    (
                        before.uv_rect[0] != after.uv_rect[0],
                        "component.sprite.uv_rect.x",
                    ),
                    (
                        before.uv_rect[1] != after.uv_rect[1],
                        "component.sprite.uv_rect.y",
                    ),
                    (
                        before.uv_rect[2] != after.uv_rect[2],
                        "component.sprite.uv_rect.width",
                    ),
                    (
                        before.uv_rect[3] != after.uv_rect[3],
                        "component.sprite.uv_rect.height",
                    ),
                    (before.layer != after.layer, "component.sprite.layer"),
                    (before.order != after.order, "component.sprite.order"),
                ] {
                    if changed {
                        overrides.insert(identifier.into());
                    }
                }
            }
            _ => {
                overrides.insert("component.sprite".into());
            }
        }
    }
    for (changed, identifier) in [
        (before.camera != after.camera, "component.camera"),
        (
            before.rigid_body != after.rigid_body,
            "component.rigid_body",
        ),
        (before.collider != after.collider, "component.collider"),
        (before.script != after.script, "component.script"),
    ] {
        if changed {
            overrides.insert(identifier.into());
        }
    }
}

fn merge_prefab_overrides(template: &SceneObject, instance: &SceneObject) -> SceneObject {
    let Some(link) = instance.prefab_instance.as_ref() else {
        return instance.clone();
    };
    let mut merged = template.clone();
    merged.object_guid = instance.object_guid;
    merged.prefab_instance = Some(link.clone());
    if link.overrides.contains("object.parent") {
        merged.parent_guid = instance.parent_guid;
    }
    for (identifier, field) in [
        ("object.name", 0),
        ("transform.translation.x", 1),
        ("transform.translation.y", 2),
        ("transform.rotation", 3),
        ("transform.scale.x", 4),
        ("transform.scale.y", 5),
    ] {
        if !link.overrides.contains(identifier) {
            continue;
        }
        match field {
            0 => merged.name = instance.name.clone(),
            1 => merged.transform.translation[0] = instance.transform.translation[0],
            2 => merged.transform.translation[1] = instance.transform.translation[1],
            3 => merged.transform.rotation_radians = instance.transform.rotation_radians,
            4 => merged.transform.scale[0] = instance.transform.scale[0],
            5 => merged.transform.scale[1] = instance.transform.scale[1],
            _ => unreachable!(),
        }
    }
    if link.overrides.contains("component.sprite") {
        merged.sprite = instance.sprite;
    } else if let (Some(sprite), Some(instance_sprite)) = (merged.sprite.as_mut(), instance.sprite)
    {
        for (identifier, field) in [
            ("component.sprite.asset", 0),
            ("component.sprite.animation_clip", 1),
            ("component.sprite.size.x", 2),
            ("component.sprite.size.y", 3),
            ("component.sprite.anchor.x", 4),
            ("component.sprite.anchor.y", 5),
            ("component.sprite.tint.r", 6),
            ("component.sprite.tint.g", 7),
            ("component.sprite.tint.b", 8),
            ("component.sprite.tint.a", 9),
            ("component.sprite.uv_rect.x", 10),
            ("component.sprite.uv_rect.y", 11),
            ("component.sprite.uv_rect.width", 12),
            ("component.sprite.uv_rect.height", 13),
            ("component.sprite.layer", 14),
            ("component.sprite.order", 15),
        ] {
            if !link.overrides.contains(identifier) {
                continue;
            }
            match field {
                0 => sprite.asset = instance_sprite.asset,
                1 => sprite.animation_clip = instance_sprite.animation_clip,
                2 => sprite.size[0] = instance_sprite.size[0],
                3 => sprite.size[1] = instance_sprite.size[1],
                4 => sprite.anchor[0] = instance_sprite.anchor[0],
                5 => sprite.anchor[1] = instance_sprite.anchor[1],
                6 => sprite.tint[0] = instance_sprite.tint[0],
                7 => sprite.tint[1] = instance_sprite.tint[1],
                8 => sprite.tint[2] = instance_sprite.tint[2],
                9 => sprite.tint[3] = instance_sprite.tint[3],
                10 => sprite.uv_rect[0] = instance_sprite.uv_rect[0],
                11 => sprite.uv_rect[1] = instance_sprite.uv_rect[1],
                12 => sprite.uv_rect[2] = instance_sprite.uv_rect[2],
                13 => sprite.uv_rect[3] = instance_sprite.uv_rect[3],
                14 => sprite.layer = instance_sprite.layer,
                15 => sprite.order = instance_sprite.order,
                _ => unreachable!(),
            }
        }
    }
    if link.overrides.contains("component.camera") {
        merged.camera = instance.camera;
    }
    if link.overrides.contains("component.rigid_body") {
        merged.rigid_body = instance.rigid_body;
    }
    if link.overrides.contains("component.collider") {
        merged.collider = instance.collider;
    }
    if link.overrides.contains("component.script") {
        merged.script = instance.script.clone();
    }
    merged
}

fn import_asset_file(project_root: &Path, source: &Path) -> Result<String, String> {
    let source_metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Cannot inspect {}: {error}", source.display()))?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err(format!("{} is not a regular file", source.display()));
    }
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has an invalid filename", source.display()))?;
    let lower_name = file_name.to_ascii_lowercase();
    let suffix = [
        ".forgeprefab.json",
        ".forgeatlas.json",
        ".forgeanim.json",
        ".png",
        ".luau",
    ]
    .into_iter()
    .find(|suffix| lower_name.ends_with(suffix))
    .ok_or_else(|| format!("{} has an unsupported asset type", source.display()))?;
    let size_limit = match suffix {
        ".png" => 128 * 1024 * 1024,
        ".luau" => 1024 * 1024,
        _ => 8 * 1024 * 1024,
    };
    if source_metadata.len() > size_limit {
        return Err(format!(
            "{} exceeds the {} byte import limit",
            source.display(),
            size_limit
        ));
    }

    let project_root = project_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve project root: {error}"))?;
    let assets_root = project_root.join("Assets");
    match fs::symlink_metadata(&assets_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("Project Assets folder must not be a symbolic link".into());
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err("Project Assets path is not a directory".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&assets_root)
                .map_err(|error| format!("Cannot create project Assets folder: {error}"))?;
        }
        Err(error) => return Err(format!("Cannot inspect project Assets folder: {error}")),
    }
    let assets_root = assets_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve project Assets folder: {error}"))?;
    if !assets_root.starts_with(&project_root) {
        return Err("Project Assets folder resolves outside the project".into());
    }

    let source = source
        .canonicalize()
        .map_err(|error| format!("Cannot resolve {}: {error}", source.display()))?;
    if source.starts_with(&assets_root) {
        return Err("This file is already inside the project Assets folder".into());
    }
    let stem = &file_name[..file_name.len() - suffix.len()];
    let lower_suffix = suffix;
    let mut source_file = fs::File::open(&source)
        .map_err(|error| format!("Cannot open {}: {error}", source.display()))?;
    for index in 1..=10_000 {
        let candidate_name = if index == 1 {
            format!("{stem}{lower_suffix}")
        } else {
            format!("{stem} ({index}){lower_suffix}")
        };
        let target = assets_root.join(candidate_name);
        let mut target_file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Cannot create imported asset: {error}")),
        };
        if let Err(error) = std::io::copy(&mut source_file, &mut target_file) {
            let _ = fs::remove_file(&target);
            return Err(format!("Cannot copy {}: {error}", source.display()));
        }
        let relative_path = target
            .strip_prefix(&project_root)
            .map_err(|_| "Imported asset escaped the project folder".to_owned())?
            .to_string_lossy()
            .replace('\\', "/");
        return Ok(relative_path);
    }
    Err("Could not find an unused filename in the project Assets folder".into())
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
        collision_only_mode: false,
        collision_brush_solid: false,
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
        self.import_dropped_assets(ui);
        self.refresh_assets(ui.ctx());
        self.advance_runtime(ui.ctx());
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let can_edit_scene = self.runtime.is_none();
                ui.add_enabled_ui(can_edit_scene, |ui| {
                    ui.selectable_value(&mut self.tilemap_mode, false, "Scene");
                    ui.selectable_value(&mut self.tilemap_mode, true, "Tilemap");
                });
                if ui
                    .add_enabled(can_edit_scene, egui::Button::new("Project Settings"))
                    .clicked()
                {
                    self.project_settings_open = true;
                }
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
                    ui.selectable_value(&mut self.gizmo_mode, TransformGizmoMode::Move, "Move");
                    ui.selectable_value(&mut self.gizmo_mode, TransformGizmoMode::Rotate, "Rotate");
                    ui.selectable_value(&mut self.gizmo_mode, TransformGizmoMode::Scale, "Scale");
                    ui.selectable_value(&mut self.gizmo_mode, TransformGizmoMode::Anchor, "Anchor");
                    ui.checkbox(&mut self.snap_to_grid, "Snap");
                }
                ui.separator();
                if !self.tilemap_mode {
                    if self.runtime.is_none() {
                        if ui.button("Play").clicked() {
                            self.start_runtime();
                        }
                    } else {
                        let breakpoint_hit = self
                            .runtime
                            .as_ref()
                            .and_then(GameRuntime::current_script_breakpoint);
                        if let Some(hit) = breakpoint_hit {
                            if ui.button("Continue").clicked() {
                                self.continue_runtime();
                            }
                            ui.label(format!(
                                "Paused at {}:{}  •  object {}  •  tick {}",
                                hit.source_path, hit.line, hit.object_guid, hit.tick
                            ));
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
                        }
                        let selected_script = self
                            .selected
                            .and_then(|guid| {
                                self.scene
                                    .objects
                                    .iter()
                                    .find(|object| object.object_guid == guid)
                            })
                            .is_some_and(|object| object.script.is_some());
                        if selected_script {
                            ui.add(
                                egui::DragValue::new(&mut self.breakpoint_line)
                                    .range(1..=100_000)
                                    .prefix("Line "),
                            );
                            if ui.button("Set breakpoint").clicked() {
                                self.set_selected_breakpoint(true);
                            }
                            if ui.button("Remove breakpoint").clicked() {
                                self.set_selected_breakpoint(false);
                            }
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
                } else if self.selected_asset.is_some() {
                    self.show_asset_inspector(ui);
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
        if self.project_settings_open {
            let mut open = self.project_settings_open;
            egui::Window::new("Project Settings")
                .open(&mut open)
                .show(ui, |ui| self.show_project_settings(ui));
            self.project_settings_open = open;
        }
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
    use super::{ChunkedTileMap, TileCell, TileCoord, TileMapHistory, connected_tile_fill};

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
        assert_eq!(
            map.get(TileCoord { x: 2, y: 0 }),
            Some(TileCell {
                solid: true,
                ..seed
            })
        );
        assert!(history.undo(&mut map));
        assert_eq!(map.get(TileCoord { x: 0, y: 0 }), Some(seed));
    }
}
