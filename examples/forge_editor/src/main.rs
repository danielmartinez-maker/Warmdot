use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    path::PathBuf,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use forge_assets::{AssetCatalog, AssetChangeKind, AssetKind};
use forge_core::Guid;
use forge_editor_core::SceneEditHistory;
use forge_project::Project;
use forge_scene::{AuthoredSprite, AuthoredTransform, SceneDocument, SceneObject};

struct EditorApp {
    project: Project,
    scene: SceneDocument,
    history: SceneEditHistory,
    selected: Option<Guid>,
    status: String,
    catalog: Option<AssetCatalog>,
    textures: BTreeMap<Guid, egui::TextureHandle>,
    last_asset_scan: Option<Instant>,
}

impl EditorApp {
    fn new(project: Project) -> Self {
        let scene = project.default_scene().clone();
        let selected = scene.objects.first().map(|object| object.object_guid);
        let (catalog, status) = match AssetCatalog::open(project.root()) {
            Ok(catalog) => (Some(catalog), String::new()),
            Err(error) => (None, format!("Asset catalog unavailable: {error}")),
        };
        Self {
            project,
            scene,
            history: SceneEditHistory::new(256),
            selected,
            status,
            catalog,
            textures: BTreeMap::new(),
            last_asset_scan: None,
        }
    }

    fn add_sprite(&mut self) {
        let object = SceneObject {
            object_guid: Guid::new(),
            name: format!("Sprite {}", self.scene.objects.len() + 1),
            transform: AuthoredTransform::default(),
            sprite: Some(AuthoredSprite {
                asset: None,
                size: [1.0, 1.0],
                tint: [0.25, 0.72, 0.95, 1.0],
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                layer: 0,
                order: 0,
            }),
            camera: None,
            rigid_body: None,
            collider: None,
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
        match self.project.save_default_scene(&self.scene) {
            Ok(()) => {
                self.history.mark_saved();
                self.status = "Saved".into();
            }
            Err(error) => self.status = format!("Save failed: {error}"),
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
        }

        let required = self
            .scene
            .objects
            .iter()
            .filter_map(|object| object.sprite.and_then(|sprite| sprite.asset))
            .collect::<BTreeSet<_>>();
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
                    Ok(texture) => updates.push((guid, name, texture)),
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        for (guid, name, decoded) in updates {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [decoded.width as usize, decoded.height as usize],
                &decoded.rgba8,
            );
            let handle = ctx.load_texture(name, image, egui::TextureOptions::NEAREST);
            self.textures.insert(guid, handle);
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
        let mut changed = false;
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
                    changed |= egui::ComboBox::from_id_salt("sprite_texture")
                        .selected_text(selected_name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut sprite.asset, None, "None");
                            for (asset, path) in &available_images {
                                ui.selectable_value(&mut sprite.asset, Some(*asset), path);
                            }
                        })
                        .response
                        .changed();
                });
                ui.small("Place PNG images in the project Assets folder.");
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

        if changed {
            if let Some(asset) = replacement.sprite.and_then(|sprite| sprite.asset) {
                if !self.scene.dependencies.contains(&asset) {
                    self.scene.dependencies.push(asset);
                }
            }
            self.last_asset_scan = None;
            match self.history.replace_object(&mut self.scene, replacement) {
                Ok(()) => self.status.clear(),
                Err(error) => self.status = error.to_string(),
            }
        }
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 24, 31));
        painter.line_segment(
            [
                Pos2::new(rect.center().x, rect.top()),
                Pos2::new(rect.center().x, rect.bottom()),
            ],
            Stroke::new(1.0, Color32::from_gray(55)),
        );
        painter.line_segment(
            [
                Pos2::new(rect.left(), rect.center().y),
                Pos2::new(rect.right(), rect.center().y),
            ],
            Stroke::new(1.0, Color32::from_gray(55)),
        );

        let mut objects = self.scene.objects.iter().collect::<Vec<_>>();
        objects.sort_by_key(|object| {
            (
                object.sprite.map_or(0, |sprite| sprite.layer),
                object.sprite.map_or(0, |sprite| sprite.order),
                object.object_guid,
            )
        });
        let pixels_per_unit = 24.0;
        for object in &objects {
            let Some(sprite) = object.sprite else {
                continue;
            };
            let center = rect.center()
                + Vec2::new(
                    object.transform.translation[0] * pixels_per_unit,
                    -object.transform.translation[1] * pixels_per_unit,
                );
            let extent = Vec2::new(
                (sprite.size[0] * object.transform.scale[0]).abs() * pixels_per_unit,
                (sprite.size[1] * object.transform.scale[1]).abs() * pixels_per_unit,
            )
            .max(Vec2::splat(2.0));
            let object_rect = Rect::from_center_size(center, extent);
            if self.selected == Some(object.object_guid) {
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
            "2D Scene Preview",
            egui::FontId::proportional(16.0),
            Color32::LIGHT_GRAY,
        );

        if response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.selected = objects
                    .iter()
                    .rev()
                    .find(|object| {
                        let Some(sprite) = object.sprite else {
                            return false;
                        };
                        let center = rect.center()
                            + Vec2::new(
                                object.transform.translation[0] * pixels_per_unit,
                                -object.transform.translation[1] * pixels_per_unit,
                            );
                        let extent = Vec2::new(
                            (sprite.size[0] * object.transform.scale[0]).abs() * pixels_per_unit,
                            (sprite.size[1] * object.transform.scale[1]).abs() * pixels_per_unit,
                        )
                        .max(Vec2::splat(2.0));
                        Rect::from_center_size(center, extent).contains(pointer)
                    })
                    .map(|object| object.object_guid);
            }
        }
    }
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
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add sprite").clicked() {
                    self.add_sprite();
                }
                if ui.button("Delete").clicked() {
                    self.delete_selected();
                }
                ui.separator();
                if ui
                    .add_enabled(self.history.undo_depth() > 0, egui::Button::new("Undo"))
                    .clicked()
                {
                    match self.history.undo(&mut self.scene) {
                        Ok(_) => self.status.clear(),
                        Err(error) => self.status = error.to_string(),
                    }
                }
                if ui
                    .add_enabled(self.history.redo_depth() > 0, egui::Button::new("Redo"))
                    .clicked()
                {
                    match self.history.redo(&mut self.scene) {
                        Ok(_) => self.status.clear(),
                        Err(error) => self.status = error.to_string(),
                    }
                }
                ui.separator();
                if ui.button("Save").clicked() {
                    self.save();
                }
                ui.label(if self.history.is_dirty() {
                    "Unsaved changes"
                } else {
                    "Saved"
                });
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
            .show(ui, |ui| self.show_inspector(ui));
        egui::CentralPanel::default().show(ui, |ui| self.show_canvas(ui));
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
