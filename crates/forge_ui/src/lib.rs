//! Bounded retained-mode game UI data, layout, focus, input, and draw extraction.

use std::{collections::BTreeSet, error::Error, fmt};

use forge_assets::{AssetCatalog, AssetKind};
use forge_core::Guid;
use serde::{Deserialize, Serialize};

pub const CURRENT_UI_SCHEMA: u32 = 1;
const MAX_UI_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_UI_ELEMENTS: usize = 1024;
const MAX_UI_DEPTH: usize = 32;
const MAX_UI_TEXT_BYTES: usize = 32 * 1024;
const MAX_UI_TEXT_ELEMENT_BYTES: usize = 4096;
const MAX_UI_DRAW_COMMANDS: usize = 8192;
const MAX_UI_COORDINATE: f32 = 16_384.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiDocument {
    pub schema_version: u32,
    pub resource_guid: Guid,
    pub name: String,
    #[serde(default)]
    pub dependencies: Vec<Guid>,
    #[serde(default = "default_root_layout")]
    pub root_layout: UiLayout,
    #[serde(default)]
    pub elements: Vec<UiElement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiElement {
    pub id: Guid,
    pub widget: UiWidget,
    #[serde(default)]
    pub layout: UiLayout,
    #[serde(default)]
    pub style: UiStyle,
    #[serde(default)]
    pub children: Vec<UiElement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiWidget {
    Panel,
    Image { asset: Guid },
    Text { text: String },
    Button { label: String, event: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLayoutDirection {
    Free,
    Horizontal,
    Vertical,
}

impl Default for UiLayoutDirection {
    fn default() -> Self {
        Self::Free
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiLayout {
    #[serde(default)]
    pub anchor_min: [f32; 2],
    #[serde(default)]
    pub anchor_max: [f32; 2],
    #[serde(default)]
    pub offset_min: [f32; 2],
    #[serde(default)]
    pub offset_max: [f32; 2],
    #[serde(default)]
    pub min_size: [f32; 2],
    #[serde(default = "default_preferred_size")]
    pub preferred_size: [f32; 2],
    #[serde(default)]
    pub direction: UiLayoutDirection,
    #[serde(default)]
    pub spacing: f32,
    #[serde(default)]
    pub padding: f32,
    #[serde(default)]
    pub grow: f32,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            anchor_min: [0.0, 0.0],
            anchor_max: [0.0, 0.0],
            offset_min: [0.0, 0.0],
            offset_max: [0.0, 0.0],
            min_size: [0.0, 0.0],
            preferred_size: default_preferred_size(),
            direction: UiLayoutDirection::Free,
            spacing: 0.0,
            padding: 0.0,
            grow: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiStyle {
    #[serde(default)]
    pub background_color: [f32; 4],
    #[serde(default = "default_text_color")]
    pub text_color: [f32; 4],
    #[serde(default = "default_hover_color")]
    pub hover_color: [f32; 4],
    #[serde(default = "default_focus_color")]
    pub focus_color: [f32; 4],
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
}

impl Default for UiStyle {
    fn default() -> Self {
        Self {
            background_color: [0.0, 0.0, 0.0, 0.0],
            text_color: default_text_color(),
            hover_color: default_hover_color(),
            focus_color: default_focus_color(),
            font_scale: default_font_scale(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl UiRect {
    fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.x
            && point[1] >= self.y
            && point[0] <= self.x + self.width
            && point[1] <= self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiDrawCommand {
    pub source_id: Guid,
    pub rect: UiRect,
    pub tint: [f32; 4],
    pub texture_asset: Option<Guid>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiButtonEvent {
    pub source_id: Guid,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiInputResult {
    pub consumed: bool,
    pub event: Option<UiButtonEvent>,
}

impl UiInputResult {
    fn unconsumed() -> Self {
        Self {
            consumed: false,
            event: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiKey {
    Next,
    Previous,
    Activate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiError(String);

impl fmt::Display for UiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for UiError {}

impl UiDocument {
    pub fn from_json(source: &str) -> Result<Self, UiError> {
        if source.len() > MAX_UI_DOCUMENT_BYTES {
            return Err(UiError("UI document exceeds the 1 MiB source limit".into()));
        }
        let document: Self = serde_json::from_str(source)
            .map_err(|error| UiError(format!("invalid UI document JSON: {error}")))?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_json(&self) -> Result<String, UiError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| UiError(format!("could not serialize UI document: {error}")))
    }

    pub fn validate(&self) -> Result<(), UiError> {
        if self.schema_version != CURRENT_UI_SCHEMA {
            return Err(UiError(format!(
                "UI schema {} is unsupported; this build reads schema {}",
                self.schema_version, CURRENT_UI_SCHEMA
            )));
        }
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(UiError(
                "UI document name must be non-empty and at most 256 bytes".into(),
            ));
        }
        let mut dependencies = BTreeSet::new();
        if self
            .dependencies
            .iter()
            .any(|guid| !dependencies.insert(*guid))
        {
            return Err(UiError(
                "UI document has a duplicate asset dependency".into(),
            ));
        }
        validate_layout(&self.root_layout)?;
        let mut state = ValidationState {
            ids: BTreeSet::new(),
            count: 0,
            text_bytes: 0,
            dependencies: &dependencies,
        };
        validate_elements(&self.elements, 1, &mut state)
    }
}

struct ValidationState<'a> {
    ids: BTreeSet<Guid>,
    count: usize,
    text_bytes: usize,
    dependencies: &'a BTreeSet<Guid>,
}

fn validate_elements(
    elements: &[UiElement],
    depth: usize,
    state: &mut ValidationState<'_>,
) -> Result<(), UiError> {
    if elements.is_empty() {
        return Ok(());
    }
    if depth > MAX_UI_DEPTH {
        return Err(UiError(format!(
            "UI tree exceeds the {MAX_UI_DEPTH}-level depth limit"
        )));
    }
    for element in elements {
        state.count = state.count.saturating_add(1);
        if state.count > MAX_UI_ELEMENTS {
            return Err(UiError(format!(
                "UI tree exceeds the {MAX_UI_ELEMENTS}-element limit"
            )));
        }
        if !state.ids.insert(element.id) {
            return Err(UiError(format!(
                "UI element ID {} is duplicated",
                element.id
            )));
        }
        validate_layout(&element.layout)?;
        validate_style(&element.style)?;
        match &element.widget {
            UiWidget::Panel => {}
            UiWidget::Image { asset } => {
                if !state.dependencies.contains(asset) {
                    return Err(UiError(format!(
                        "image asset {asset} is missing from UI dependencies"
                    )));
                }
            }
            UiWidget::Text { text } => validate_text(text, state)?,
            UiWidget::Button { label, event } => {
                if event.trim().is_empty() || event.len() > 128 {
                    return Err(UiError(
                        "button event name must be non-empty and at most 128 bytes".into(),
                    ));
                }
                validate_text(label, state)?;
            }
        }
        validate_elements(&element.children, depth + 1, state)?;
    }
    Ok(())
}

fn validate_text(text: &str, state: &mut ValidationState<'_>) -> Result<(), UiError> {
    if text.len() > MAX_UI_TEXT_ELEMENT_BYTES || text.contains('\0') {
        return Err(UiError(format!(
            "UI text must not contain NUL and must be at most {MAX_UI_TEXT_ELEMENT_BYTES} bytes"
        )));
    }
    state.text_bytes = state.text_bytes.saturating_add(text.len());
    if state.text_bytes > MAX_UI_TEXT_BYTES {
        return Err(UiError(
            "UI document exceeds the 32 KiB combined text limit".into(),
        ));
    }
    Ok(())
}

fn validate_layout(layout: &UiLayout) -> Result<(), UiError> {
    let valid_unit = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
    let valid_offset = |value: f32| value.is_finite() && value.abs() <= MAX_UI_COORDINATE;
    let valid_size = |value: f32| value.is_finite() && (0.0..=MAX_UI_COORDINATE).contains(&value);
    if !layout.anchor_min.iter().copied().all(valid_unit)
        || !layout.anchor_max.iter().copied().all(valid_unit)
        || layout.anchor_min[0] > layout.anchor_max[0]
        || layout.anchor_min[1] > layout.anchor_max[1]
        || !layout.offset_min.iter().copied().all(valid_offset)
        || !layout.offset_max.iter().copied().all(valid_offset)
        || !layout.min_size.iter().copied().all(valid_size)
        || !layout.preferred_size.iter().copied().all(valid_size)
        || !valid_size(layout.spacing)
        || !valid_size(layout.padding)
        || !layout.grow.is_finite()
        || !(0.0..=10_000.0).contains(&layout.grow)
    {
        return Err(UiError(
            "UI layout anchors, sizes, spacing, padding, or growth are outside supported bounds"
                .into(),
        ));
    }
    Ok(())
}

fn validate_style(style: &UiStyle) -> Result<(), UiError> {
    let valid_color = |color: &[f32; 4]| {
        color
            .iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
    };
    if !valid_color(&style.background_color)
        || !valid_color(&style.text_color)
        || !valid_color(&style.hover_color)
        || !valid_color(&style.focus_color)
        || !style.font_scale.is_finite()
        || !(1.0..=8.0).contains(&style.font_scale)
    {
        return Err(UiError(
            "UI colors must be finite RGBA values in 0..=1 and font scale must be 1..=8".into(),
        ));
    }
    Ok(())
}

pub struct UiRuntime {
    document: UiDocument,
    pointer: Option<[f32; 2]>,
    focused: Option<Guid>,
    pressed: Option<Guid>,
    viewport: [f32; 2],
    button_bounds: Vec<ButtonBounds>,
}

#[derive(Clone)]
struct ButtonBounds {
    id: Guid,
    rect: UiRect,
    event: String,
}

impl UiRuntime {
    pub fn new(document: UiDocument) -> Result<Self, UiError> {
        document.validate()?;
        Ok(Self {
            document,
            pointer: None,
            focused: None,
            pressed: None,
            viewport: [0.0, 0.0],
            button_bounds: Vec::new(),
        })
    }

    pub fn empty() -> Self {
        Self {
            document: UiDocument {
                schema_version: CURRENT_UI_SCHEMA,
                resource_guid: Guid::new(),
                name: "Empty UI".into(),
                dependencies: Vec::new(),
                root_layout: default_root_layout(),
                elements: Vec::new(),
            },
            pointer: None,
            focused: None,
            pressed: None,
            viewport: [0.0, 0.0],
            button_bounds: Vec::new(),
        }
    }

    pub fn document(&self) -> &UiDocument {
        &self.document
    }

    pub fn validate_assets(&self, assets: &AssetCatalog) -> Result<(), UiError> {
        for guid in &self.document.dependencies {
            let record = assets.get(*guid).ok_or_else(|| {
                UiError(format!(
                    "UI dependency {guid} is not in the project asset catalog"
                ))
            })?;
            if record.kind != AssetKind::Png {
                return Err(UiError(format!(
                    "UI dependency {guid} is {:?}; only PNG images are supported",
                    record.kind
                )));
            }
            assets.load_png(*guid).map_err(|error| {
                UiError(format!("UI image {guid} could not be loaded: {error}"))
            })?;
        }
        Ok(())
    }

    pub fn draw_commands(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Vec<UiDrawCommand>, UiError> {
        self.rebuild_layout(width, height, true)
    }

    pub fn route_pointer_move(
        &mut self,
        position: [f32; 2],
        width: u32,
        height: u32,
    ) -> Result<UiInputResult, UiError> {
        self.rebuild_layout(width, height, false)?;
        if !position.iter().all(|value| value.is_finite()) {
            return Err(UiError("pointer position must be finite".into()));
        }
        self.pointer = Some(position);
        Ok(UiInputResult {
            consumed: self.button_at(position).is_some(),
            event: None,
        })
    }

    pub fn route_pointer_down(
        &mut self,
        position: [f32; 2],
        width: u32,
        height: u32,
    ) -> Result<UiInputResult, UiError> {
        self.rebuild_layout(width, height, false)?;
        if !position.iter().all(|value| value.is_finite()) {
            return Err(UiError("pointer position must be finite".into()));
        }
        self.pointer = Some(position);
        let button = self.button_at(position).cloned();
        self.pressed = button.as_ref().map(|button| button.id);
        if let Some(button) = &button {
            self.focused = Some(button.id);
        }
        Ok(UiInputResult {
            consumed: button.is_some(),
            event: None,
        })
    }

    pub fn route_pointer_up(
        &mut self,
        position: [f32; 2],
        width: u32,
        height: u32,
    ) -> Result<UiInputResult, UiError> {
        self.rebuild_layout(width, height, false)?;
        if !position.iter().all(|value| value.is_finite()) {
            return Err(UiError("pointer position must be finite".into()));
        }
        self.pointer = Some(position);
        let pressed = self.pressed.take();
        let button = self.button_at(position).cloned();
        let event = match (pressed, button) {
            (Some(pressed), Some(button)) if pressed == button.id => Some(UiButtonEvent {
                source_id: button.id,
                name: button.event,
            }),
            _ => None,
        };
        Ok(UiInputResult {
            consumed: event.is_some() || pressed.is_some(),
            event,
        })
    }

    pub fn route_key(
        &mut self,
        key: UiKey,
        width: u32,
        height: u32,
    ) -> Result<UiInputResult, UiError> {
        self.rebuild_layout(width, height, false)?;
        if self.button_bounds.is_empty() {
            return Ok(UiInputResult::unconsumed());
        }
        match key {
            UiKey::Next | UiKey::Previous => {
                let current = self
                    .focused
                    .and_then(|id| self.button_bounds.iter().position(|button| button.id == id));
                let next = match (current, key) {
                    (Some(index), UiKey::Next) => (index + 1) % self.button_bounds.len(),
                    (Some(index), UiKey::Previous) => {
                        index.wrapping_sub(1) % self.button_bounds.len()
                    }
                    (_, UiKey::Next) => 0,
                    (_, UiKey::Previous) => self.button_bounds.len() - 1,
                    (_, UiKey::Activate) => unreachable!(),
                };
                self.focused = Some(self.button_bounds[next].id);
                Ok(UiInputResult {
                    consumed: true,
                    event: None,
                })
            }
            UiKey::Activate => {
                let event = self.focused.and_then(|id| {
                    self.button_bounds
                        .iter()
                        .find(|button| button.id == id)
                        .map(|button| UiButtonEvent {
                            source_id: button.id,
                            name: button.event.clone(),
                        })
                });
                Ok(UiInputResult {
                    consumed: event.is_some(),
                    event,
                })
            }
        }
    }

    pub fn clear_pointer(&mut self) {
        self.pointer = None;
        self.pressed = None;
    }

    fn button_at(&self, position: [f32; 2]) -> Option<&ButtonBounds> {
        self.button_bounds
            .iter()
            .find(|button| button.rect.contains(position))
    }

    fn rebuild_layout(
        &mut self,
        width: u32,
        height: u32,
        emit_draw: bool,
    ) -> Result<Vec<UiDrawCommand>, UiError> {
        if width == 0 || height == 0 {
            self.viewport = [0.0, 0.0];
            self.button_bounds.clear();
            return Ok(Vec::new());
        }
        if width > 32_768 || height > 32_768 {
            return Err(UiError("UI viewport exceeds the 32768-pixel limit".into()));
        }
        self.document.validate()?;
        let viewport = [width as f32, height as f32];
        let mut buttons = Vec::new();
        let mut commands = Vec::new();
        let parent = UiRect {
            x: 0.0,
            y: 0.0,
            width: viewport[0],
            height: viewport[1],
        };
        layout_children(
            &self.document.elements,
            parent,
            self.document.root_layout.direction,
            self.document.root_layout.padding,
            self.document.root_layout.spacing,
            1,
            &mut buttons,
            &mut commands,
            emit_draw,
            self.pointer,
            self.focused,
        )?;
        if self
            .focused
            .is_none_or(|id| !buttons.iter().any(|button| button.id == id))
        {
            self.focused = buttons.first().map(|button| button.id);
        }
        self.viewport = viewport;
        self.button_bounds = buttons;
        Ok(commands)
    }
}

fn layout_children(
    elements: &[UiElement],
    parent: UiRect,
    direction: UiLayoutDirection,
    padding: f32,
    spacing: f32,
    depth: usize,
    buttons: &mut Vec<ButtonBounds>,
    commands: &mut Vec<UiDrawCommand>,
    emit_draw: bool,
    pointer: Option<[f32; 2]>,
    focused: Option<Guid>,
) -> Result<(), UiError> {
    if elements.is_empty() {
        return Ok(());
    }
    if depth > MAX_UI_DEPTH {
        return Err(UiError(format!(
            "UI tree exceeds the {MAX_UI_DEPTH}-level depth limit"
        )));
    }
    let horizontal = direction == UiLayoutDirection::Horizontal;
    let vertical = direction == UiLayoutDirection::Vertical;
    let main_extent = if horizontal {
        parent.width
    } else {
        parent.height
    };
    let padding_total = padding * 2.0;
    let gaps = spacing * elements.len().saturating_sub(1) as f32;
    let available_main = (main_extent - padding_total - gaps).max(0.0);
    let base_total = elements
        .iter()
        .map(|element| {
            let axis = if horizontal { 0 } else { 1 };
            element.layout.preferred_size[axis].max(element.layout.min_size[axis])
        })
        .sum::<f32>();
    let total_grow = elements
        .iter()
        .map(|element| element.layout.grow)
        .sum::<f32>();
    let grow_space = (available_main - base_total).max(0.0);

    let mut cursor = if horizontal {
        parent.x + padding
    } else {
        parent.y + padding
    };
    for element in elements {
        let rect = if horizontal || vertical {
            let axis = if horizontal { 0 } else { 1 };
            let base = element.layout.preferred_size[axis].max(element.layout.min_size[axis]);
            let extra = if total_grow > 0.0 {
                grow_space * element.layout.grow / total_grow
            } else {
                0.0
            };
            if horizontal {
                let cross = (parent.height - padding_total).max(element.layout.min_size[1]);
                let rect = UiRect {
                    x: cursor,
                    y: parent.y + padding,
                    width: base + extra,
                    height: cross,
                };
                cursor += rect.width + spacing;
                rect
            } else {
                let cross = (parent.width - padding_total).max(element.layout.min_size[0]);
                let rect = UiRect {
                    x: parent.x + padding,
                    y: cursor,
                    width: cross,
                    height: base + extra,
                };
                cursor += rect.height + spacing;
                rect
            }
        } else {
            anchored_rect(element.layout, parent)
        };
        if rect.width > 0.0 && rect.height > 0.0 {
            if emit_draw {
                append_element_draw(element, rect, pointer, focused, commands)?;
            }
            if let UiWidget::Button { event, .. } = &element.widget {
                buttons.push(ButtonBounds {
                    id: element.id,
                    rect,
                    event: event.clone(),
                });
            }
            layout_children(
                &element.children,
                rect,
                element.layout.direction,
                element.layout.padding,
                element.layout.spacing,
                depth + 1,
                buttons,
                commands,
                emit_draw,
                pointer,
                focused,
            )?;
        }
    }
    Ok(())
}

fn anchored_rect(layout: UiLayout, parent: UiRect) -> UiRect {
    let mut position = [0.0; 2];
    let mut size = [0.0; 2];
    let parent_position = [parent.x, parent.y];
    let parent_size = [parent.width, parent.height];
    for axis in 0..2 {
        let start = parent_position[axis]
            + parent_size[axis] * layout.anchor_min[axis]
            + layout.offset_min[axis];
        if layout.anchor_min[axis] == layout.anchor_max[axis] {
            position[axis] = start;
            size[axis] = layout.preferred_size[axis].max(layout.min_size[axis]);
        } else {
            let end = parent_position[axis]
                + parent_size[axis] * layout.anchor_max[axis]
                + layout.offset_max[axis];
            position[axis] = start;
            size[axis] = (end - start).max(layout.min_size[axis]);
        }
    }
    UiRect {
        x: position[0],
        y: position[1],
        width: size[0],
        height: size[1],
    }
}

fn append_element_draw(
    element: &UiElement,
    rect: UiRect,
    pointer: Option<[f32; 2]>,
    focused: Option<Guid>,
    commands: &mut Vec<UiDrawCommand>,
) -> Result<(), UiError> {
    let add = |commands: &mut Vec<UiDrawCommand>,
               rect: UiRect,
               tint: [f32; 4],
               texture_asset: Option<Guid>| {
        if rect.width <= 0.0 || rect.height <= 0.0 || tint[3] <= 0.0 {
            return Ok(());
        }
        if commands.len() >= MAX_UI_DRAW_COMMANDS {
            return Err(UiError(format!(
                "UI draw extraction exceeds the {MAX_UI_DRAW_COMMANDS}-command limit"
            )));
        }
        commands.push(UiDrawCommand {
            source_id: element.id,
            rect,
            tint,
            texture_asset,
        });
        Ok(())
    };
    let content = UiRect {
        x: rect.x + element.layout.padding,
        y: rect.y + element.layout.padding,
        width: (rect.width - element.layout.padding * 2.0).max(0.0),
        height: (rect.height - element.layout.padding * 2.0).max(0.0),
    };
    match &element.widget {
        UiWidget::Panel => add(commands, rect, element.style.background_color, None)?,
        UiWidget::Image { asset } => add(
            commands,
            rect,
            visible_color(element.style.background_color),
            Some(*asset),
        )?,
        UiWidget::Text { text } => draw_text(
            element.id,
            text,
            content,
            element.style.text_color,
            element.style.font_scale,
            commands,
        )?,
        UiWidget::Button { label, .. } => {
            let tint = if focused == Some(element.id) {
                element.style.focus_color
            } else if pointer.is_some_and(|position| rect.contains(position)) {
                element.style.hover_color
            } else if element.style.background_color[3] > 0.0 {
                element.style.background_color
            } else {
                [0.12, 0.16, 0.22, 0.96]
            };
            add(commands, rect, tint, None)?;
            draw_text(
                element.id,
                label,
                content,
                element.style.text_color,
                element.style.font_scale,
                commands,
            )?;
        }
    }
    Ok(())
}

fn visible_color(color: [f32; 4]) -> [f32; 4] {
    if color[3] > 0.0 { color } else { [1.0; 4] }
}

fn draw_text(
    source_id: Guid,
    text: &str,
    rect: UiRect,
    color: [f32; 4],
    scale: f32,
    commands: &mut Vec<UiDrawCommand>,
) -> Result<(), UiError> {
    let mut x = rect.x;
    let mut y_origin = rect.y + ((rect.height - 7.0 * scale).max(0.0) * 0.5);
    for character in text.chars() {
        if character == '\n' {
            x = rect.x;
            y_origin += 8.0 * scale;
            if y_origin + 7.0 * scale > rect.y + rect.height {
                break;
            }
            continue;
        }
        if x + 5.0 * scale > rect.x + rect.width {
            break;
        }
        let rows = glyph(character);
        for (row_index, row) in rows.into_iter().enumerate() {
            let mut column = 0;
            while column < 5 {
                if row & (1 << (4 - column)) == 0 {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < 5 && row & (1 << (4 - column)) != 0 {
                    column += 1;
                }
                if commands.len() >= MAX_UI_DRAW_COMMANDS {
                    return Err(UiError(format!(
                        "UI draw extraction exceeds the {MAX_UI_DRAW_COMMANDS}-command limit"
                    )));
                }
                commands.push(UiDrawCommand {
                    source_id,
                    rect: UiRect {
                        x: x + start as f32 * scale,
                        y: y_origin + row_index as f32 * scale,
                        width: (column - start) as f32 * scale,
                        height: scale,
                    },
                    tint: color,
                    texture_asset: None,
                });
            }
        }
        x += 6.0 * scale;
    }
    Ok(())
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        ';' => [0, 4, 4, 0, 4, 4, 8],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 4, 4, 8],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '=' => [0, 31, 0, 31, 0, 0, 0],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        ' ' => [0; 7],
        _ => [31, 17, 1, 2, 4, 0, 4],
    }
}

fn default_root_layout() -> UiLayout {
    UiLayout {
        anchor_max: [1.0, 1.0],
        preferred_size: [0.0, 0.0],
        ..UiLayout::default()
    }
}

fn default_preferred_size() -> [f32; 2] {
    [120.0, 40.0]
}

fn default_text_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_hover_color() -> [f32; 4] {
    [0.22, 0.36, 0.58, 1.0]
}

fn default_focus_color() -> [f32; 4] {
    [0.16, 0.3, 0.5, 1.0]
}

fn default_font_scale() -> f32 {
    2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(event: &str) -> UiElement {
        UiElement {
            id: Guid::new(),
            widget: UiWidget::Button {
                label: "START".into(),
                event: event.into(),
            },
            layout: UiLayout {
                preferred_size: [160.0, 48.0],
                ..UiLayout::default()
            },
            style: UiStyle::default(),
            children: Vec::new(),
        }
    }

    fn document(elements: Vec<UiElement>) -> UiDocument {
        UiDocument {
            schema_version: CURRENT_UI_SCHEMA,
            resource_guid: Guid::new(),
            name: "Test UI".into(),
            dependencies: Vec::new(),
            root_layout: UiLayout::default(),
            elements,
        }
    }

    #[test]
    fn serializes_and_loads_a_versioned_retained_tree() {
        let document = document(vec![button("start")]);
        let json = document.to_json().expect("serialize UI document");
        let loaded = UiDocument::from_json(&json).expect("load UI document");
        assert_eq!(loaded.schema_version, CURRENT_UI_SCHEMA);
        assert_eq!(loaded.elements.len(), 1);
        assert!(matches!(
            &loaded.elements[0].widget,
            UiWidget::Button { .. }
        ));
    }

    #[test]
    fn rejects_duplicate_element_ids_and_undeclared_image_dependencies() {
        let mut first = button("one");
        let mut second = button("two");
        second.id = first.id;
        assert!(document(vec![first.clone(), second]).validate().is_err());

        first.widget = UiWidget::Image { asset: Guid::new() };
        assert!(document(vec![first]).validate().is_err());
    }

    #[test]
    fn routes_pointer_and_keyboard_activation_to_bounded_button_events() {
        let mut first = button("start");
        first.layout.anchor_min = [0.0, 0.0];
        first.layout.anchor_max = [0.0, 0.0];
        first.layout.offset_min = [12.0, 16.0];
        let id = first.id;
        let mut runtime = UiRuntime::new(document(vec![first])).expect("create UI runtime");
        let down = runtime
            .route_pointer_down([24.0, 24.0], 640, 360)
            .expect("route pointer down");
        assert!(down.consumed);
        let click = runtime
            .route_pointer_up([24.0, 24.0], 640, 360)
            .expect("route pointer up");
        assert_eq!(
            click.event,
            Some(UiButtonEvent {
                source_id: id,
                name: "start".into()
            })
        );

        let keyboard = runtime
            .route_key(UiKey::Activate, 640, 360)
            .expect("route keyboard activation");
        assert!(keyboard.consumed);
        assert_eq!(keyboard.event.expect("button event").name, "start");
    }

    #[test]
    fn anchored_layout_scales_to_viewport_and_extracts_screen_space_quads() {
        let mut panel = UiElement {
            id: Guid::new(),
            widget: UiWidget::Panel,
            layout: UiLayout {
                anchor_min: [0.0, 0.0],
                anchor_max: [1.0, 0.0],
                offset_min: [8.0, 8.0],
                offset_max: [-8.0, 0.0],
                min_size: [0.0, 48.0],
                ..UiLayout::default()
            },
            style: UiStyle {
                background_color: [0.0, 0.0, 0.0, 0.8],
                ..UiStyle::default()
            },
            children: vec![UiElement {
                id: Guid::new(),
                widget: UiWidget::Text {
                    text: "HP 3/3".into(),
                },
                layout: UiLayout {
                    preferred_size: [120.0, 24.0],
                    ..UiLayout::default()
                },
                style: UiStyle::default(),
                children: Vec::new(),
            }],
        };
        panel.layout.direction = UiLayoutDirection::Free;
        let mut runtime = UiRuntime::new(document(vec![panel])).expect("create UI runtime");
        let commands = runtime.draw_commands(1280, 720).expect("extract UI quads");
        assert!(!commands.is_empty());
        assert_eq!(commands[0].rect.width, 1264.0);
        assert!(commands.iter().all(|command| command.rect.x >= 0.0));
    }

    #[test]
    fn enforces_document_size_and_tree_depth_limits() {
        assert!(UiDocument::from_json(&" ".repeat(MAX_UI_DOCUMENT_BYTES + 1)).is_err());
        let mut nested = button("deep");
        for _ in 0..MAX_UI_DEPTH {
            nested = UiElement {
                id: Guid::new(),
                widget: UiWidget::Panel,
                layout: UiLayout::default(),
                style: UiStyle::default(),
                children: vec![nested],
            };
        }
        assert!(document(vec![nested]).validate().is_err());
    }
}
