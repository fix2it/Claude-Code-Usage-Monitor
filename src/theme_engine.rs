use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::*;

use crate::localization::LanguageId;
use crate::models::AppUsageData;
use crate::providers::{ProviderId, ProviderSet, PROVIDER_DESCRIPTORS};

pub const THEME_SCHEMA_VERSION: u32 = 1;
pub const COMPACT_FLUENT_QUAD_THEME_ID: &str = "compact-fluent-quad";

const BUILTIN_THEME_SOURCES: &[(&str, &str)] = &[(
    COMPACT_FLUENT_QUAD_THEME_ID,
    include_str!(concat!(env!("OUT_DIR"), "/compact-fluent-quad.json")),
)];

const REMOVED_BUILTIN_THEME_IDS: &[&str] = &[
    "mission-control",
    "neon-reactor",
    "rpg-party-hud",
    "quota-garden",
    "tokyo-data-skyline",
    "pixel-arcade",
    "minimal-signal",
    "quota-constellation",
    "quota-orrery",
    "terminal-ticker",
    "tactical-edge-hud",
];

fn is_builtin_theme_id(id: &str) -> bool {
    BUILTIN_THEME_SOURCES
        .iter()
        .any(|(builtin_id, _)| id == *builtin_id)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemeDocument {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    /// Runtime adapter used by the native window positioning code. The saved
    /// schema stores these values on the root `SceneObject` instead.
    #[serde(skip)]
    pub canvas: Canvas,
    #[serde(skip)]
    pub placement: Placement,
    #[serde(skip)]
    pub children: Vec<SceneObject>,
    #[serde(default)]
    pub surfaces: Vec<SceneObject>,
}

/// One visual object in a theme. Root objects are native desktop surfaces and
/// have `placement`; descendants have `parent` and use `anchor`. Content,
/// styling, layout, geometry, and the ability to contain children are otherwise
/// identical at every level.
#[derive(Clone, Debug, Serialize)]
pub struct SceneObject {
    pub id: String,
    pub name: String,
    #[serde(default = "default_render")]
    pub render: Expression,
    #[serde(default = "default_visibility")]
    pub visibility: Expression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Placement::is_default")]
    pub placement: Placement,
    #[serde(default, skip_serializing_if = "ObjectAnchor::is_default")]
    pub anchor: ObjectAnchor,
    #[serde(default, skip_serializing_if = "Expression::is_zero")]
    pub x: Expression,
    #[serde(default, skip_serializing_if = "Expression::is_zero")]
    pub y: Expression,
    #[serde(default = "default_layer_width")]
    pub width: Expression,
    #[serde(default = "default_layer_height")]
    pub height: Expression,
    #[serde(default, skip_serializing_if = "Expression::is_zero")]
    pub rotation: Expression,
    #[serde(default)]
    pub background: LayerBackground,
    #[serde(default)]
    pub border: Option<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_events: Option<MouseEvents>,
    /// Native hover popup for a root surface. Omitted by older/custom themes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<SurfaceTooltip>,
    #[serde(default)]
    pub corner_radius: Expression,
    #[serde(default)]
    pub layout: ChildLayout,
    #[serde(default)]
    pub align: ChildAlignment,
    #[serde(default)]
    pub gap: Expression,
    #[serde(default)]
    pub content: SceneContent,
    /// Descendants are stored as a flat ordered list on each root. Their
    /// `parent` ids describe the hierarchy while preserving global z-order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SceneObject>,
}

/// Resolved root dimensions used to build expression contexts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Canvas {
    #[serde(default = "default_canvas_width")]
    pub width: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_expression: Option<Expression>,
    #[serde(default = "default_canvas_height")]
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_expression: Option<Expression>,
    #[serde(default)]
    pub background: Paint,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Runtime-only layout host retained during undocking; never edits the theme.
    #[serde(skip)]
    pub host_dimensions: Option<(u32, u32)>,
    /// Runtime-only protection for drag anchoring; authored placements stay exact.
    #[serde(skip)]
    pub clamp_taskbar_drag: bool,
    #[serde(default)]
    pub reference: ReferenceTarget,
    /// Controls which native shell host owns a root surface. Older themes did
    /// not persist this value, so they are normalized from their reference
    /// region by `prepare_runtime`.
    #[serde(default = "legacy_surface_nest", alias = "layer")]
    pub nest: SurfaceNest,
    #[serde(default)]
    pub horizontal: HorizontalAnchor,
    #[serde(default)]
    pub vertical: VerticalAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_horizontal: Option<HorizontalAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_vertical: Option<VerticalAnchor>,
    #[serde(default)]
    pub offset_x: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_x_expression: Option<Expression>,
    #[serde(default)]
    pub offset_y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_y_expression: Option<Expression>,
}

impl Placement {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceTarget {
    #[serde(default)]
    pub region: ReferenceRegion,
    #[serde(default)]
    pub display: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRegion {
    Monitor,
    Taskbar,
    #[default]
    SystemTray,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceNest {
    /// Transitional value used only while loading themes saved before native
    /// surface hosting was introduced.
    #[default]
    Auto,
    Taskbar,
    /// A genuine Windows notification-area icon. Explorer owns its position,
    /// overflow state, DPI scaling, and drag ordering; the root object supplies
    /// the icon artwork.
    TrayIcon,
    Desktop,
    Floating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceTooltip {
    UsageSummary,
}

impl SurfaceNest {
    pub fn resolve(self, reference: ReferenceRegion) -> Self {
        match self {
            Self::Auto => match reference {
                ReferenceRegion::Taskbar | ReferenceRegion::SystemTray => Self::Taskbar,
                ReferenceRegion::Monitor => Self::Floating,
            },
            nest => nest,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAnchor {
    Left,
    Center,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAnchor {
    Top,
    Center,
    #[default]
    Bottom,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneContent {
    #[default]
    None,
    Text {
        #[serde(default)]
        template: String,
        #[serde(default = "default_font_family")]
        font_family: String,
        #[serde(default = "default_font_size")]
        font_size: Expression,
        #[serde(default)]
        weight: FontWeight,
        #[serde(default)]
        rendering: FontRendering,
        #[serde(default = "default_font_contrast")]
        contrast: Expression,
        #[serde(default)]
        align: TextAlign,
        #[serde(default = "default_text_paint")]
        color: Paint,
    },
    Progress {
        #[serde(default = "default_progress_value")]
        value: Expression,
        #[serde(default)]
        direction: ProgressDirection,
        #[serde(default = "default_accent_paint")]
        fill: Paint,
        #[serde(default = "default_track_paint")]
        track: Paint,
        #[serde(default)]
        corner_radius: Expression,
        #[serde(default)]
        segments: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        segments_expression: Option<Expression>,
        #[serde(default = "default_segment_gap")]
        segment_gap: Expression,
    },
}

impl SceneObject {
    pub fn object(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            render: default_render(),
            visibility: default_visibility(),
            parent: None,
            placement: Placement::default(),
            anchor: ObjectAnchor::default(),
            x: 0.0.into(),
            y: 0.0.into(),
            width: default_layer_width(),
            height: default_layer_height(),
            rotation: 0.0.into(),
            background: LayerBackground::default(),
            border: None,
            mouse_events: None,
            tooltip: None,
            corner_radius: 0.0.into(),
            layout: ChildLayout::Freeform,
            align: ChildAlignment::Start,
            gap: 0.0.into(),
            content: SceneContent::None,
            children: Vec::new(),
        }
    }

    pub fn root(
        id: impl Into<String>,
        name: impl Into<String>,
        width: Expression,
        height: Expression,
        placement: Placement,
    ) -> Self {
        let mut object = Self::object(id, name);
        object.width = width;
        object.height = height;
        object.placement = placement;
        object
    }
}

/// Optional, side-effecting mouse handlers for a scene object. Handler text is
/// parsed as a small action language rather than by the numeric expression
/// evaluator, keeping ordinary theme expressions deterministic and read-only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseEvents {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub click: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub double_click: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub right_click: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mouse_enter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mouse_leave: String,
}

impl MouseEvents {
    pub fn is_empty(&self) -> bool {
        self.click.trim().is_empty()
            && self.double_click.trim().is_empty()
            && self.right_click.trim().is_empty()
            && self.mouse_enter.trim().is_empty()
            && self.mouse_leave.trim().is_empty()
    }

    pub fn handler(&self, event: MouseEventKind) -> &str {
        match event {
            MouseEventKind::Click => &self.click,
            MouseEventKind::DoubleClick => &self.double_click,
            MouseEventKind::RightClick => &self.right_click,
            MouseEventKind::MouseEnter => &self.mouse_enter,
            MouseEventKind::MouseLeave => &self.mouse_leave,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    Click,
    DoubleClick,
    RightClick,
    MouseEnter,
    MouseLeave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseActionProperty {
    Render,
    Visibility,
    X,
    Y,
    Width,
    Height,
    Rotation,
}

impl MouseActionProperty {
    pub fn name(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Visibility => "visibility",
            Self::X => "x",
            Self::Y => "y",
            Self::Width => "width",
            Self::Height => "height",
            Self::Rotation => "rotation",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "render" => Some(Self::Render),
            "visibility" => Some(Self::Visibility),
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "width" => Some(Self::Width),
            "height" => Some(Self::Height),
            "rotation" => Some(Self::Rotation),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseActionTarget {
    SelfObject,
    Object(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseAction {
    ShowDashboard,
    ToggleDashboard,
    OpenUrl {
        url: String,
    },
    ShowContextMenu {
        menu: Option<String>,
    },
    Set {
        target: MouseActionTarget,
        property: MouseActionProperty,
        value: Expression,
    },
    Toggle {
        target: MouseActionTarget,
        property: MouseActionProperty,
    },
    Increase {
        target: MouseActionTarget,
        property: MouseActionProperty,
        amount: Expression,
    },
    Decrease {
        target: MouseActionTarget,
        property: MouseActionProperty,
        amount: Expression,
    },
    Reset {
        target: MouseActionTarget,
        property: MouseActionProperty,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MouseActionOverrideKey {
    pub surface_index: usize,
    pub object_id: String,
    pub property: MouseActionProperty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseActionEffect {
    ShowDashboard,
    ToggleDashboard,
    OpenUrl(String),
    ShowContextMenu(Option<String>),
}

/// Parse the deliberately small, line-or-semicolon separated action language.
/// Supported forms include `show_dashboard()`, `toggle_dashboard()`,
/// `open_url("https://example.com")`, `show_context_menu()`,
/// `show_context_menu("menu-id")`, `set(self.render, false)`,
/// `increase("layer-id", height, 10)`, `decrease(self.width, 5)`,
/// `toggle(self.render)`, and `reset("layer-id", height)`.
pub fn parse_mouse_actions(source: &str) -> Result<Vec<MouseAction>, String> {
    let statements = split_action_statements(source)?;
    let mut actions = Vec::with_capacity(statements.len());
    for statement in statements {
        let open = statement
            .find('(')
            .ok_or_else(|| format!("Expected an action call: {statement}"))?;
        if !statement.ends_with(')') {
            return Err(format!("Expected ')' at the end of: {statement}"));
        }
        let name = statement[..open].trim().to_ascii_lowercase();
        let args = split_action_arguments(&statement[open + 1..statement.len() - 1])?;
        match name.as_str() {
            "show_dashboard" if args.is_empty() => actions.push(MouseAction::ShowDashboard),
            "show_dashboard" => return Err("show_dashboard() does not take arguments".into()),
            "toggle_dashboard" if args.is_empty() => actions.push(MouseAction::ToggleDashboard),
            "toggle_dashboard" => return Err("toggle_dashboard() does not take arguments".into()),
            "open_url" if args.len() == 1 => {
                let url = parse_quoted_action_string(&args[0], "URL")?;
                if !crate::context_menu::supported_url(&url) {
                    return Err("open_url URL must start with http:// or https://".into());
                }
                actions.push(MouseAction::OpenUrl { url });
            }
            "open_url" => return Err("open_url expects one quoted URL".into()),
            "show_context_menu" if args.is_empty() => {
                actions.push(MouseAction::ShowContextMenu { menu: None })
            }
            "show_context_menu" if args.len() == 1 => {
                actions.push(MouseAction::ShowContextMenu {
                    menu: Some(parse_quoted_action_string(
                        &args[0],
                        "context menu id or name",
                    )?),
                });
            }
            "show_context_menu" => {
                return Err("show_context_menu expects zero or one quoted menu id or name".into())
            }
            "set" | "increase" | "decrease" => {
                let (target, property, value) = match args.as_slice() {
                    [target_property, value] => {
                        let (target, property) = parse_target_property(target_property)?;
                        (target, property, value.as_str())
                    }
                    [target, property, value] => (
                        parse_action_target(target)?,
                        parse_action_property(property)?,
                        value.as_str(),
                    ),
                    _ => return Err(
                        format!(
                            "{name} expects {name}(target.property, value) or {name}(target, property, value)"
                        ),
                    ),
                };
                if value.trim().is_empty() {
                    return Err(format!("{name} requires a value expression"));
                }
                if name != "set" && property == MouseActionProperty::Render {
                    return Err(format!(
                        "{name} requires a numeric property; use toggle(..., render) for Render"
                    ));
                }
                let value = Expression(value.trim().to_string());
                match name.as_str() {
                    "set" => actions.push(MouseAction::Set {
                        target,
                        property,
                        value,
                    }),
                    "increase" => actions.push(MouseAction::Increase {
                        target,
                        property,
                        amount: value,
                    }),
                    "decrease" => actions.push(MouseAction::Decrease {
                        target,
                        property,
                        amount: value,
                    }),
                    _ => unreachable!(),
                }
            }
            "toggle" | "reset" => {
                let (target, property) = match args.as_slice() {
                    [target_property] => parse_target_property(target_property)?,
                    [target, property] => (
                        parse_action_target(target)?,
                        parse_action_property(property)?,
                    ),
                    _ => {
                        return Err(format!(
                            "{name} expects {name}(target.property) or {name}(target, property)"
                        ))
                    }
                };
                if name == "toggle" {
                    if property != MouseActionProperty::Render {
                        return Err("toggle currently supports the render property only".into());
                    }
                    actions.push(MouseAction::Toggle { target, property });
                } else {
                    actions.push(MouseAction::Reset { target, property });
                }
            }
            _ => return Err(format!("Unknown action: {name}")),
        }
    }
    Ok(actions)
}

fn split_action_statements(source: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for character in source.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if quoted && character == '\\' {
            current.push(character);
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
            current.push(character);
            continue;
        }
        if !quoted {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| "Unexpected ')' in action script".to_string())?
                }
                ';' | '\n' | '\r' if depth == 0 => {
                    if !current.trim().is_empty() {
                        result.push(current.trim().to_string());
                    }
                    current.clear();
                    continue;
                }
                _ => {}
            }
        }
        current.push(character);
    }
    if quoted {
        return Err("Unterminated quoted layer id in action script".into());
    }
    if depth != 0 {
        return Err("Unclosed '(' in action script".into());
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    Ok(result)
}

fn split_action_arguments(source: &str) -> Result<Vec<String>, String> {
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for character in source.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if quoted && character == '\\' {
            current.push(character);
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
            current.push(character);
            continue;
        }
        if !quoted {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| "Unexpected ')' in action arguments".to_string())?
                }
                ',' if depth == 0 => {
                    result.push(current.trim().to_string());
                    current.clear();
                    continue;
                }
                _ => {}
            }
        }
        current.push(character);
    }
    if quoted || depth != 0 {
        return Err("Unclosed quote or parenthesis in action arguments".into());
    }
    result.push(current.trim().to_string());
    Ok(result)
}

fn parse_target_property(source: &str) -> Result<(MouseActionTarget, MouseActionProperty), String> {
    let (target, property) = source
        .rsplit_once('.')
        .ok_or_else(|| format!("Expected target.property, got: {source}"))?;
    Ok((
        parse_action_target(target)?,
        parse_action_property(property)?,
    ))
}

fn parse_action_target(source: &str) -> Result<MouseActionTarget, String> {
    let source = source.trim();
    if source.eq_ignore_ascii_case("self") {
        return Ok(MouseActionTarget::SelfObject);
    }
    if source.len() >= 2 && source.starts_with('"') && source.ends_with('"') {
        let value = source[1..source.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
        if value.trim().is_empty() {
            return Err("Layer id cannot be empty".into());
        }
        return Ok(MouseActionTarget::Object(value));
    }
    Err(format!("Use self or a quoted layer id, got: {source}"))
}

fn parse_quoted_action_string(source: &str, label: &str) -> Result<String, String> {
    let source = source.trim();
    if source.len() >= 2 && source.starts_with('"') && source.ends_with('"') {
        let value = source[1..source.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    Err(format!("Use a non-empty quoted {label}, got: {source}"))
}

fn parse_action_property(source: &str) -> Result<MouseActionProperty, String> {
    MouseActionProperty::parse(source).ok_or_else(|| {
        format!(
            "Unknown layer property '{}'; use render, visibility, x, y, width, height, or rotation",
            source.trim()
        )
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildLayout {
    #[default]
    Freeform,
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectHorizontalAnchor {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectVerticalAnchor {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectAnchor {
    #[serde(default)]
    pub horizontal: ObjectHorizontalAnchor,
    #[serde(default)]
    pub vertical: ObjectVerticalAnchor,
}

impl ObjectAnchor {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    Contain,
    Cover,
    #[default]
    Stretch,
    Original,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerBackground {
    #[default]
    None,
    Colour {
        colour: Paint,
    },
    Gradient {
        start: Paint,
        end: Paint,
        #[serde(default)]
        angle: Expression,
    },
    Image {
        path: String,
        #[serde(default)]
        fit: ImageFit,
    },
}

impl LayerBackground {
    fn from_legacy_paint(paint: Paint) -> Self {
        if paint.color.trim().eq_ignore_ascii_case("#00000000") {
            Self::None
        } else {
            Self::Colour { colour: paint }
        }
    }

    fn canvas_paint(&self) -> Paint {
        match self {
            Self::Colour { colour } => colour.clone(),
            Self::None | Self::Gradient { .. } | Self::Image { .. } => Paint::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LayerBackgroundWire {
    Current(LayerBackgroundCurrent),
    Legacy(Paint),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LayerBackgroundCurrent {
    None,
    Colour {
        colour: Paint,
    },
    Gradient {
        start: Paint,
        end: Paint,
        #[serde(default)]
        angle: Expression,
    },
    Image {
        path: String,
        #[serde(default)]
        fit: ImageFit,
    },
}

impl<'de> Deserialize<'de> for LayerBackground {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match LayerBackgroundWire::deserialize(deserializer)? {
            LayerBackgroundWire::Current(LayerBackgroundCurrent::None) => Self::None,
            LayerBackgroundWire::Current(LayerBackgroundCurrent::Colour { colour }) => {
                Self::Colour { colour }
            }
            LayerBackgroundWire::Current(LayerBackgroundCurrent::Gradient {
                start,
                end,
                angle,
            }) => Self::Gradient { start, end, angle },
            LayerBackgroundWire::Current(LayerBackgroundCurrent::Image { path, fit }) => {
                Self::Image { path, fit }
            }
            LayerBackgroundWire::Legacy(paint) => Self::from_legacy_paint(paint),
        })
    }
}

#[derive(Deserialize)]
struct SceneObjectWire {
    id: String,
    name: String,
    #[serde(default = "default_render")]
    render: Expression,
    #[serde(default = "default_visibility")]
    visibility: Expression,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    placement: Placement,
    #[serde(default)]
    anchor: ObjectAnchor,
    #[serde(default)]
    x: Expression,
    #[serde(default)]
    y: Expression,
    #[serde(default = "default_layer_width")]
    width: Expression,
    #[serde(default = "default_layer_height")]
    height: Expression,
    #[serde(default)]
    rotation: Expression,
    #[serde(default)]
    background: LayerBackground,
    #[serde(default)]
    border: Option<Stroke>,
    #[serde(default)]
    mouse_events: Option<MouseEvents>,
    #[serde(default)]
    tooltip: Option<SurfaceTooltip>,
    #[serde(default)]
    corner_radius: Expression,
    #[serde(default)]
    layout: ChildLayout,
    #[serde(default)]
    align: ChildAlignment,
    #[serde(default)]
    gap: Expression,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    children: Vec<SceneObject>,
}

#[derive(Deserialize)]
struct LegacyImageContent {
    path: String,
    #[serde(default)]
    fit: ImageFit,
}

#[derive(Deserialize)]
struct LegacyShapeContent {
    #[serde(default = "default_accent_paint")]
    fill: Paint,
    #[serde(default)]
    stroke: Option<Stroke>,
    #[serde(default)]
    corner_radius: Expression,
}

impl<'de> Deserialize<'de> for SceneObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut wire = SceneObjectWire::deserialize(deserializer)?;
        let content_type = wire
            .content
            .as_ref()
            .and_then(|content| content.get("type"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let content = match (content_type.as_deref(), wire.content.take()) {
            (Some("image"), Some(value)) => {
                let legacy: LegacyImageContent =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                wire.background = LayerBackground::Image {
                    path: legacy.path,
                    fit: legacy.fit,
                };
                SceneContent::None
            }
            (Some("shape"), Some(value)) => {
                let legacy: LegacyShapeContent =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                wire.background = LayerBackground::from_legacy_paint(legacy.fill);
                if wire.border.is_none() {
                    wire.border = legacy.stroke;
                }
                if wire.corner_radius.is_zero() {
                    wire.corner_radius = legacy.corner_radius;
                }
                SceneContent::None
            }
            (_, Some(serde_json::Value::Null) | None) => SceneContent::None,
            (_, Some(value)) => serde_json::from_value(value).map_err(serde::de::Error::custom)?,
        };
        Ok(Self {
            id: wire.id,
            name: wire.name,
            render: wire.render,
            visibility: wire.visibility,
            parent: wire.parent,
            placement: wire.placement,
            anchor: wire.anchor,
            x: wire.x,
            y: wire.y,
            width: wire.width,
            height: wire.height,
            rotation: wire.rotation,
            background: wire.background,
            border: wire.border,
            mouse_events: wire.mouse_events,
            tooltip: wire.tooltip,
            corner_radius: wire.corner_radius,
            layout: wire.layout,
            align: wire.align,
            gap: wire.gap,
            content,
            children: wire.children,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressDirection {
    #[default]
    LeftToRight,
    RightToLeft,
    BottomToTop,
    TopToBottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    Light,
    #[default]
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl FontWeight {
    pub fn gdi_weight(self) -> i32 {
        match self {
            Self::Light => 300,
            Self::Regular => 400,
            Self::Medium => 500,
            Self::Semibold => 600,
            Self::Bold => 700,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRendering {
    #[default]
    Antialiased,
    ClearType,
    Aliased,
}

impl FontRendering {
    fn gdi_quality(self) -> u32 {
        match self {
            Self::Antialiased => ANTIALIASED_QUALITY.0 as u32,
            Self::ClearType => CLEARTYPE_QUALITY.0 as u32,
            Self::Aliased => NONANTIALIASED_QUALITY.0 as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    #[serde(default = "default_text_paint")]
    pub color: Paint,
    #[serde(default = "default_stroke_width")]
    pub width: Expression,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paint {
    #[serde(default = "default_paint_color")]
    pub color: String,
    #[serde(default = "default_opacity")]
    pub opacity: Expression,
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            color: "#00000000".to_string(),
            opacity: Expression::from(1.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Expression(pub String);

impl Default for Expression {
    fn default() -> Self {
        Self("0".to_string())
    }
}

impl Expression {
    fn is_zero(&self) -> bool {
        self.0.trim().parse::<f64>().is_ok_and(|value| value == 0.0)
    }
}

impl From<f64> for Expression {
    fn from(value: f64) -> Self {
        Self(format_number(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Clone, Debug)]
pub struct ResolvedObject<'a> {
    pub source: &'a SceneObject,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub parent_width: f64,
    pub parent_height: f64,
    pub gap: f64,
    pub opacity: f64,
    pub rotation: f64,
    pub clip: Vec<ClipRegion>,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipRegion {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rotation: f64,
}

#[derive(Clone, Debug)]
pub struct RenderedTheme {
    pub width: u32,
    pub height: u32,
    /// Top-down, premultiplied BGRA pixels, represented as 0xAARRGGBB.
    pub pixels: Vec<u32>,
    #[cfg(test)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DataContext {
    values: Arc<HashMap<String, f64>>,
    strings: Arc<HashMap<String, String>>,
    geometry_values: HashMap<String, f64>,
    force_segoe_ui: bool,
    display_metrics: Option<DisplayMetrics>,
}

/// Runtime environment used by theme expressions and layout. Provider state is
/// deliberately independent of polling availability so a temporary provider
/// error never causes the widget to jump or resize.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeRuntime {
    providers: ProviderSet,
    pub poll_ok: bool,
    pub has_error: bool,
    pub language: LanguageId,
    /// Present each allowance as what is left rather than what is spent. Only
    /// the `.display` values and summaries using them follow this; `.percentage`
    /// always means consumption so severity thresholds keep their meaning.
    pub countdown: bool,
    pub use_windows_default_font: bool,
    pub show_pace: bool,
    /// Opt-in visibility for the Claude "Fable" promo quota column.
    pub show_fable: bool,
    pub show_fable_x: bool,
    pub claude_pools_one_widget: bool,
    pub codex_pools_one_widget: bool,
    pub pool_split_supported: bool,
    pub antigravity_pools_one_widget: bool,
    /// Selected widget view mode (1..3) and its continuous animation phase
    /// (0.0 = mode 1, 1.0 = mode 2, 2.0 = mode 3).
    pub view_mode: u8,
    pub mode_phase: f64,
    widget_mode_phases: [Option<f64>; WIDGET_SLOTS],
    pub surface_nest: SurfaceNest,
    pub floating_card_opacity: u8,
    pub antigravity_gemini_enabled: bool,
    pub antigravity_external_enabled: bool,
    /// The second (OpenCode) agy pool is switched on.
    pub antigravity_x_enabled: bool,
    /// The first agy pool (Antigravity's own login) is switched on.
    pub antigravity_a_enabled: bool,
    /// Pool o's External models toggle (pool a's is
    /// `antigravity_external_enabled` while both pools are on).
    pub antigravity_external_x_enabled: bool,
    pub crossfade_opacity: f64,
    pub claude_crossfade_opacity: f64,
    pub claude_hovered: bool,
    pub claude_main_enabled: bool,
    pub claude_x_enabled: bool,
    pub codex_crossfade_opacity: f64,
    pub codex_hovered: bool,
    pub codex_main_enabled: bool,
    pub codex_x_enabled: bool,
    pub opencode_crossfade_opacity: f64,
    pub qoder_crossfade_opacity: f64,
    /// Hover progress (0 = normal, 1 = hovered) of the agy external-models
    /// widget, which has its own hover area.
    pub agy_external_crossfade_opacity: f64,
    pub split_hover_phases: [f64; 4],
    host_width: u32,
    host_height: u32,
}

impl Default for ThemeRuntime {
    fn default() -> Self {
        Self {
            providers: ProviderSet::default(),
            poll_ok: true,
            has_error: false,
            language: LanguageId::English,
            countdown: false,
            use_windows_default_font: true,
            show_pace: true,
            show_fable: false,
            show_fable_x: false,
            claude_pools_one_widget: true,
            codex_pools_one_widget: true,
            pool_split_supported: true,
            antigravity_pools_one_widget: true,
            view_mode: 1,
            mode_phase: 0.0,
            widget_mode_phases: [None; WIDGET_SLOTS],
            surface_nest: SurfaceNest::Taskbar,
            floating_card_opacity: 85,
            antigravity_gemini_enabled: true,
            antigravity_external_enabled: false,
            antigravity_x_enabled: false,
            antigravity_a_enabled: true,
            antigravity_external_x_enabled: false,
            crossfade_opacity: 1.0,
            claude_crossfade_opacity: 0.0,
            claude_hovered: false,
            claude_main_enabled: true,
            claude_x_enabled: false,
            codex_crossfade_opacity: 0.0,
            codex_hovered: false,
            codex_main_enabled: true,
            codex_x_enabled: false,
            opencode_crossfade_opacity: 0.0,
            qoder_crossfade_opacity: 0.0,
            agy_external_crossfade_opacity: 0.0,
            split_hover_phases: [0.0; 4],
            host_width: default_canvas_width(),
            host_height: default_canvas_height(),
        }
    }
}

impl ThemeRuntime {
    #[cfg(test)]
    pub fn new(claude_enabled: bool, codex_enabled: bool, antigravity_enabled: bool) -> Self {
        let providers = ProviderSet::from_enabled(
            [
                (ProviderId::Claude, claude_enabled),
                (ProviderId::Codex, codex_enabled),
                (ProviderId::Antigravity, antigravity_enabled),
            ]
            .into_iter()
            .filter_map(|(provider, enabled)| enabled.then_some(provider)),
        );
        Self::from_providers(providers)
    }

    pub fn from_providers(providers: ProviderSet) -> Self {
        let agy = providers.contains(ProviderId::Antigravity);
        Self {
            providers,
            poll_ok: true,
            has_error: false,
            language: LanguageId::English,
            countdown: false,
            use_windows_default_font: true,
            show_pace: true,
            show_fable: false,
            show_fable_x: false,
            claude_pools_one_widget: true,
            codex_pools_one_widget: true,
            pool_split_supported: true,
            antigravity_pools_one_widget: true,
            view_mode: 1,
            mode_phase: 0.0,
            widget_mode_phases: [None; WIDGET_SLOTS],
            surface_nest: SurfaceNest::Taskbar,
            floating_card_opacity: 85,
            antigravity_gemini_enabled: agy,
            antigravity_external_enabled: false,
            antigravity_x_enabled: false,
            antigravity_a_enabled: true,
            antigravity_external_x_enabled: false,
            crossfade_opacity: 1.0,
            claude_crossfade_opacity: 0.0,
            claude_hovered: false,
            claude_main_enabled: true,
            claude_x_enabled: false,
            codex_crossfade_opacity: 0.0,
            codex_hovered: false,
            codex_main_enabled: true,
            codex_x_enabled: false,
            opencode_crossfade_opacity: 0.0,
            qoder_crossfade_opacity: 0.0,
            agy_external_crossfade_opacity: 0.0,
            split_hover_phases: [0.0; 4],
            host_width: default_canvas_width(),
            host_height: default_canvas_height(),
        }
    }

    pub fn with_claude_state(
        mut self,
        hovered: bool,
        main_enabled: bool,
        x_enabled: bool,
        crossfade_opacity: f64,
    ) -> Self {
        self.claude_hovered = hovered;
        self.claude_main_enabled = main_enabled;
        self.claude_x_enabled = x_enabled;
        self.claude_crossfade_opacity = crossfade_opacity;
        self
    }

    pub fn with_codex_state(
        mut self,
        hovered: bool,
        main_enabled: bool,
        x_enabled: bool,
        crossfade_opacity: f64,
    ) -> Self {
        self.codex_hovered = hovered;
        self.codex_main_enabled = main_enabled;
        self.codex_x_enabled = x_enabled;
        self.codex_crossfade_opacity = crossfade_opacity;
        self
    }

    pub fn with_antigravity_pools(
        mut self,
        gemini: bool,
        external: bool,
        crossfade_opacity: f64,
    ) -> Self {
        self.antigravity_gemini_enabled = gemini;
        self.antigravity_external_enabled = gemini && external;
        self.crossfade_opacity = crossfade_opacity;
        self
    }

    /// The OpenCode pool follows its parent: it is on only while agy is.
    pub fn with_antigravity_x(mut self, enabled: bool) -> Self {
        self.antigravity_x_enabled = enabled;
        self
    }

    /// Pool o's own External models toggle.
    pub fn with_antigravity_external_x(mut self, enabled: bool) -> Self {
        self.antigravity_external_x_enabled = enabled;
        self
    }

    /// Pool A can be off while pool O is on; agy then shows pool O alone.
    pub fn with_antigravity_a(mut self, enabled: bool) -> Self {
        self.antigravity_a_enabled = enabled;
        self
    }

    /// Only pool O is shown: it fills the single-pool agy layout.
    pub(crate) fn antigravity_o_only(self) -> bool {
        self.antigravity_x_enabled && !self.antigravity_a_enabled
    }

    pub fn with_nest(mut self, nest: SurfaceNest) -> Self {
        self.surface_nest = nest;
        self
    }

    pub fn with_floating_card_opacity(mut self, opacity: u8) -> Self {
        self.floating_card_opacity = opacity;
        self
    }

    pub fn with_poll_state(mut self, poll_ok: bool, has_error: bool) -> Self {
        self.poll_ok = poll_ok;
        self.has_error = has_error;
        self
    }

    pub fn with_language(mut self, language: LanguageId) -> Self {
        self.language = language;
        self
    }

    /// Count each allowance down towards its limit instead of up from zero.
    pub fn with_countdown(mut self, countdown: bool) -> Self {
        self.countdown = countdown;
        self
    }

    pub fn with_windows_default_font(mut self, enabled: bool) -> Self {
        self.use_windows_default_font = enabled;
        self
    }

    pub fn with_show_pace(mut self, show_pace: bool) -> Self {
        self.show_pace = show_pace;
        self
    }

    pub fn with_show_fable(mut self, show_fable: bool) -> Self {
        self.show_fable = show_fable;
        self
    }

    pub fn with_show_fable_x(mut self, show_fable_x: bool) -> Self {
        self.show_fable_x = show_fable_x;
        self
    }

    pub fn with_pool_layouts(mut self, claude: bool, agy: bool) -> Self {
        self.claude_pools_one_widget = claude;
        self.antigravity_pools_one_widget = agy;
        self
    }

    pub fn with_codex_pool_layout(mut self, combined: bool) -> Self {
        self.codex_pools_one_widget = combined;
        self
    }

    /// Unsupported themes keep the existing combined layout while preserving
    /// the user's persisted choices for a later supporting theme.
    pub fn with_pool_split_support(mut self, supported: bool) -> Self {
        self.pool_split_supported = supported;
        if !supported {
            self.claude_pools_one_widget = true;
            self.codex_pools_one_widget = true;
            self.antigravity_pools_one_widget = true;
        }
        self
    }

    pub fn with_view_mode(mut self, view_mode: u8) -> Self {
        self.view_mode = view_mode.clamp(1, 3);
        self
    }

    pub fn with_mode_phase(mut self, phase: f64) -> Self {
        self.mode_phase = phase.clamp(0.0, 2.0);
        self
    }

    /// Override the phase for a Compact provider or the separate Fable column.
    pub fn with_widget_mode_phase(mut self, key: &str, phase: f64) -> Self {
        if let Some(index) = widget_mode_index(key) {
            self.widget_mode_phases[index] = Some(phase.clamp(0.0, 2.0));
        } else {
            debug_assert!(false, "unknown Compact widget key {key}");
        }
        self
    }

    fn widget_mode_phase(&self, index: usize) -> f64 {
        self.widget_mode_phases[index]
            .unwrap_or(self.mode_phase)
            .clamp(0.0, 2.0)
    }

    pub fn with_opencode_state(mut self, opacity: f64) -> Self {
        self.opencode_crossfade_opacity = opacity;
        self
    }

    pub fn with_qoder_state(mut self, opacity: f64) -> Self {
        self.qoder_crossfade_opacity = opacity;
        self
    }

    pub fn with_agy_external_state(mut self, opacity: f64) -> Self {
        self.agy_external_crossfade_opacity = opacity;
        self
    }

    pub fn with_split_hover_phases(mut self, phases: [f64; 4]) -> Self {
        self.split_hover_phases = phases;
        self
    }

    /// Supply the selected native host's 96-DPI logical dimensions. Theme
    /// expressions consume these as `host.width` and `host.height`.
    pub fn with_host_dimensions(mut self, width: u32, height: u32) -> Self {
        self.host_width = width.max(1);
        self.host_height = height.max(1);
        self
    }

    pub fn host_dimensions(self) -> (u32, u32) {
        (self.host_width, self.host_height)
    }

    pub fn provider_count(self) -> usize {
        self.providers.len().max(1)
    }

    pub fn provider_enabled(self, provider: ProviderId) -> bool {
        self.providers.contains(provider)
    }
}

const FIVE_HOURS: std::time::Duration = std::time::Duration::from_secs(5 * 3600);
const WEEK: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
/// Fallback length for the 30-day billing window when a cached reading has
/// no period start. A live reading uses the real `startsAt..endsAt` length.
const MONTH: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

/// Percent of a window of `length` that has passed by now, given when it
/// resets; `None` when the reset is not within one window from now.
/// Long name for a one-letter pool marker (Qoder `u`/`c`), for themes with
/// room to spell it out; any other label is returned as it is.
fn pool_name(label: &str) -> String {
    match label {
        "u" => "Ultima".into(),
        "c" => "Credits".into(),
        other => other.into(),
    }
}

fn expected_used(resets_at: std::time::SystemTime, length: std::time::Duration) -> Option<f64> {
    let left = resets_at
        .duration_since(std::time::SystemTime::now())
        .ok()?;
    // A minute of slack for poll latency right after a reset.
    (!length.is_zero() && left <= length + std::time::Duration::from_secs(60))
        .then(|| (1.0 - left.min(length).as_secs_f64() / length.as_secs_f64()) * 100.0)
}

/// Computes asymmetric crossfade opacities `(normal_opacity, hover_opacity)`
/// for hover transitions.
///
/// Progress `p` runs from 0.0 (unhovered/normal state) to 1.0 (hovered state).
/// - The outgoing normal layer fades out over the first 45%:
///   `normal_opacity = (1.0 - p / 0.45).clamp(0.0, 1.0)`.
/// - The incoming hover layer starts at 10% and reaches full opacity at 65%:
///   `hover_opacity = ((p - 0.10) / 0.55).clamp(0.0, 1.0)`.
///
/// The original ramp (0.35 out / 0.25 in) left the middle of the transition
/// almost blank: combined opacity bottomed out near 0.21 and the incoming
/// text was only at ~33% at `p=0.5`. Labels stay at fixed coordinates and
/// substitute in place, so a wider overlap only blends the same text box and
/// does not produce a positional ghost. Combined opacity now stays above
/// ~0.63, and the two layers are still never both fully opaque (their sum
/// never exceeds 1.0).
pub fn asymmetric_crossfade_opacities(progress_to_hover: f64) -> (f64, f64) {
    let p = progress_to_hover.clamp(0.0, 1.0);
    let normal_opacity = (1.0 - p / 0.45).clamp(0.0, 1.0);
    let hover_opacity = ((p - 0.10) / 0.55).clamp(0.0, 1.0);
    (normal_opacity, hover_opacity)
}

/// Dimensionless decomposition of the continuous mode phase.
/// `text_collapse` drives the `1 -> 2` transition (the numeric text leaves),
/// `bar_collapse` drives `2 -> 3` (the bar shuts towards the centre and the
/// percent badge appears). Pure so the choreography is unit-testable without
/// Win32 or a theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModePhases {
    pub text_collapse: f64,
    pub bar_collapse: f64,
}

pub fn decompose_mode_phase(phase: f64) -> ModePhases {
    let p = phase.clamp(0.0, 2.0);
    ModePhases {
        text_collapse: p.min(1.0),
        bar_collapse: (p - 1.0).clamp(0.0, 1.0),
    }
}

/// Row geometry mirrored from the Compact theme. Stage 2 replaces the theme's
/// own expressions with the `display.*` bindings below, which makes these the
/// single source of truth; until then the bindings are simply unused.
const COMPACT_BAR_SEGMENTS: f64 = 10.0;
/// A bar never shows more cubes than this, whatever the widget count.
const COMPACT_MAX_CUBES: f64 = 5.0;
const COMPACT_TEXT_WIDTH: f64 = 62.0;
const COMPACT_TEXT_WIDTH_WIDE: f64 = 90.0;
const COMPACT_MARKER_WIDTH: f64 = 10.0;
const COMPACT_ROW_GAP: f64 = 4.0;

/// Cubes per bar: `min(5, ceil(10 / max(1, widgets)))`. The theme's
/// `segments_expression` uses the same formula over `display.widget_count`.
fn compact_bar_cubes(widget_count: usize) -> f64 {
    (COMPACT_BAR_SEGMENTS / widget_count.max(1) as f64)
        .ceil()
        .min(COMPACT_MAX_CUBES)
}

/// `widget_count` is providers plus one when the Fable column is shown.
fn compact_bar_full_width(widget_count: usize) -> f64 {
    compact_bar_cubes(widget_count) * 11.0 - 1.0
}

/// Count enabled providers and the separate two-pool Fable column.
pub(crate) fn widget_count(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> usize {
    runtime.provider_count()
        + (fable_visible(data, runtime) && claude_dual(runtime)) as usize
        + antigravity_split(data, runtime) as usize
        + claude_separate(runtime) as usize
        + codex_separate(runtime) as usize
        + agy_separate(data, runtime) as usize
}

fn claude_separate(runtime: ThemeRuntime) -> bool {
    runtime.provider_enabled(ProviderId::Claude)
        && runtime.claude_main_enabled
        && runtime.claude_x_enabled
        && !runtime.claude_pools_one_widget
}

fn codex_separate(runtime: ThemeRuntime) -> bool {
    runtime.provider_enabled(ProviderId::Codex)
        && runtime.codex_main_enabled
        && runtime.codex_x_enabled
        && !runtime.codex_pools_one_widget
}

fn agy_separate(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    antigravity_pool_o(data, runtime) && !runtime.antigravity_pools_one_widget
}

/// External models on with pool O shown: the external models get their own
/// widget next to the Gemini one, each with a row per pool (A and O).
pub(crate) fn antigravity_split(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    antigravity_pool_o(data, runtime)
        && runtime.antigravity_pools_one_widget
        && (runtime.antigravity_external_enabled || runtime.antigravity_external_x_enabled)
}

/// The OpenCode agy pool has its own row: it is switched on (with agy) and is
/// not the same login as pool A. Until the first poll the row is kept, so the
/// widget does not jump; a known duplicate hides it.
pub(crate) fn antigravity_pool_o(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    runtime.provider_enabled(ProviderId::Antigravity)
        && runtime.antigravity_gemini_enabled
        && runtime.antigravity_x_enabled
        && runtime.antigravity_a_enabled
        && !data
            .and_then(AppUsageData::antigravity_x)
            .is_some_and(|account| account.error == Some(crate::poller::PollError::NoCredentials))
}

/// Both Claude pools are on: the Fable column then shows one row per pool.
fn claude_dual(runtime: ThemeRuntime) -> bool {
    runtime.claude_main_enabled && runtime.claude_x_enabled && runtime.claude_pools_one_widget
}

/// Whether the Fable column is drawn.
pub(crate) fn fable_visible(_data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    let (main, x) = fable_pools(runtime);
    main || x
}

/// Per-pool Fable rows: each pool's own toggle, while Claude and that pool are on.
fn fable_pools(runtime: ThemeRuntime) -> (bool, bool) {
    let claude = runtime.provider_enabled(ProviderId::Claude);
    (
        claude && runtime.show_fable && runtime.claude_main_enabled,
        claude && runtime.show_fable_x && runtime.claude_x_enabled,
    )
}

/// Geometry and visibility for one widget row at a given mode phase. Pure:
/// the caller supplies the row's own widths, so no theme constants are baked
/// into the formulas themselves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeBindings {
    pub bar_width: f64,
    pub bar_x: f64,
    pub text_width: f64,
    pub text_slide_x: f64,
    pub text_opacity: f64,
    pub percent_opacity: f64,
    pub marker_opacity: f64,
    pub provider_width: f64,
}

pub fn mode_row_bindings(
    phase: f64,
    bar_full_width: f64,
    text_full_width: f64,
    marker_width: f64,
    gap: f64,
) -> ModeBindings {
    let phases = decompose_mode_phase(phase);
    let bar_width = bar_full_width * (1.0 - phases.bar_collapse);
    let text_width = text_full_width * (1.0 - phases.text_collapse);
    ModeBindings {
        bar_width,
        bar_x: bar_full_width * phases.bar_collapse / 2.0,
        text_width,
        text_slide_x: -text_full_width * phases.text_collapse,
        text_opacity: 100.0 * (1.0 - phases.text_collapse),
        percent_opacity: 100.0 * phases.bar_collapse,
        // Markers stay at their fixed positions in every mode.
        marker_opacity: 100.0,
        provider_width: bar_width + text_width + gap + marker_width,
    }
}

/// Provider keys that carry an adaptive Compact column, in the same order the
/// bindings are read by the theme.
pub const DISPLAY_PROVIDER_KEYS: [&str; 8] = [
    "cursor",
    "claude",
    "codex",
    "antigravity",
    "opencode",
    "qoder",
    "grok",
    "copilot",
];

fn widget_mode_index(key: &str) -> Option<usize> {
    widget_keys().position(|candidate| candidate == key)
}

/// Every widget key in slot order: the providers, the Fable column, then the
/// agy external-models widget.
pub(crate) fn widget_keys() -> impl Iterator<Item = &'static str> {
    DISPLAY_PROVIDER_KEYS.into_iter().chain([
        FABLE_KEY,
        AGY_EXTERNAL_KEY,
        "claude_x",
        "antigravity_o",
        "agy_external_o",
        "codex_x",
    ])
}

const COMPACT_PERCENT_WIDTH: f64 = 30.0;
const COMPACT_CURSOR_TEXT_WIDTH: f64 = 80.0;
const DISPLAY_METRICS_CACHE_CAPACITY: usize = 8;

/// One provider's adaptive column. `text_slide` and `row_width` are already
/// resolved for the runtime's mode phase, so the theme never recomputes them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProviderMetrics {
    pub text_box: f64,
    pub percent_box: f64,
    pub marker_box: f64,
    pub origin: f64,
    pub text_slide: f64,
    pub row_width: f64,
}

/// Measured Compact geometry for the current data snapshot and mode phase.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayMetrics {
    providers: [ProviderMetrics; 8],
    pub providers_width: f64,
    /// Providers plus the Fable column when shown; sizes every bar.
    pub widget_count: usize,
    /// The Fable column, measured like a provider. Its `text_box` is 0 when
    /// the line cannot be measured, so the theme's `max(bar, text_box.fable)`
    /// falls back to the bar width.
    fable: ProviderMetrics,
    /// The agy external-models widget is shown.
    agy_split: bool,
    /// The agy external-models widget, measured and moded on its own. It is
    /// only shown with pool O, so it always has a marker column.
    agy_external: ProviderMetrics,
    claude_x: ProviderMetrics,
    codex_x: ProviderMetrics,
    antigravity_o: ProviderMetrics,
    agy_external_o: ProviderMetrics,
    claude_separate: bool,
    codex_separate: bool,
    agy_separate: bool,
}

impl DisplayMetrics {
    #[cfg(test)]
    pub fn provider(&self, key: &str) -> Option<ProviderMetrics> {
        DISPLAY_PROVIDER_KEYS
            .iter()
            .position(|candidate| *candidate == key)
            .map(|index| self.providers[index])
    }

    /// Legacy reserves, used whenever a line cannot be measured and as the
    /// base context that theme validation and Studio build before data exists.
    pub fn legacy(runtime: ThemeRuntime) -> Self {
        Self::legacy_with_widgets(runtime, runtime.provider_count())
    }

    pub(crate) fn legacy_with_widgets(runtime: ThemeRuntime, widget_count: usize) -> Self {
        let bar = compact_bar_full_width(widget_count);
        let mut metrics = Self {
            providers: [ProviderMetrics::default(); 8],
            providers_width: 0.0,
            widget_count,
            fable: ProviderMetrics::default(),
            agy_split: antigravity_split(None, runtime),
            agy_external: ProviderMetrics::default(),
            claude_x: ProviderMetrics::default(),
            codex_x: ProviderMetrics::default(),
            antigravity_o: ProviderMetrics::default(),
            agy_external_o: ProviderMetrics::default(),
            claude_separate: claude_separate(runtime),
            codex_separate: codex_separate(runtime),
            agy_separate: agy_separate(None, runtime),
        };
        let dual = claude_dual(runtime);
        metrics.fable = fable_column(
            bar,
            if dual { COMPACT_TEXT_WIDTH } else { 0.0 },
            COMPACT_PERCENT_WIDTH,
            COMPACT_MARKER_WIDTH,
            COMPACT_ORIGIN_LEGACY,
            decompose_mode_phase(runtime.widget_mode_phase(FABLE_METRICS_INDEX)),
            dual,
        );
        for (index, key) in DISPLAY_PROVIDER_KEYS.iter().enumerate() {
            let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
            let text_box = if *key == "cursor" {
                COMPACT_CURSOR_TEXT_WIDTH
            } else {
                COMPACT_TEXT_WIDTH
            };
            let (marker_box, origin) = match *key {
                "claude" | "codex" | "opencode" | "qoder" => {
                    (COMPACT_MARKER_WIDTH, COMPACT_ORIGIN_LEGACY)
                }
                _ => (0.0, 0.0),
            };
            metrics.providers[index] = ProviderMetrics {
                text_box,
                percent_box: COMPACT_PERCENT_WIDTH,
                marker_box,
                origin,
                text_slide: -text_box * phases.text_collapse,
                row_width: compact_row_width(bar, text_box, COMPACT_PERCENT_WIDTH, origin, phases),
            };
        }
        metrics.agy_external = agy_external_column(
            bar,
            COMPACT_TEXT_WIDTH,
            COMPACT_PERCENT_WIDTH,
            COMPACT_ORIGIN_LEGACY,
            decompose_mode_phase(runtime.widget_mode_phase(AGY_EXTERNAL_INDEX)),
            COMPACT_MARKER_WIDTH,
        );
        metrics.claude_x = metrics.providers[1];
        metrics.codex_x = metrics.providers[2];
        metrics.antigravity_o = metrics.providers[3];
        metrics.agy_external_o = metrics.agy_external;
        metrics.claude_x.origin = 0.0;
        metrics.codex_x.origin = 0.0;
        metrics.antigravity_o.origin = 0.0;
        metrics = metrics_for_phase(metrics, runtime);
        metrics.providers_width = metrics.providers_width(runtime);
        metrics
    }

    /// The agy external-models widget's geometry.
    #[cfg(test)]
    pub fn agy_external(&self) -> ProviderMetrics {
        self.agy_external
    }

    /// Enabled provider columns plus the agy external-models widget, sized
    /// by its own measurement and mode.
    fn providers_width(&self, runtime: ThemeRuntime) -> f64 {
        let agy = self.agy_external.row_width;
        compact_providers_width(
            DISPLAY_PROVIDER_KEYS
                .iter()
                .enumerate()
                .filter_map(|(index, key)| {
                    runtime
                        .provider_enabled(display_provider_id(key))
                        .then_some(self.providers[index].row_width)
                })
                .chain(self.agy_split.then_some(agy))
                .chain(self.claude_separate.then_some(self.claude_x.row_width))
                .chain(self.codex_separate.then_some(self.codex_x.row_width))
                .chain(self.agy_separate.then_some(self.antigravity_o.row_width)),
            runtime.provider_count()
                + self.agy_split as usize
                + self.claude_separate as usize
                + self.codex_separate as usize
                + self.agy_separate as usize,
        )
    }
}

const COMPACT_ORIGIN_LEGACY: f64 = 12.0;

/// The agy external-models widget: a provider-style column with a pool
/// marker column (`a`/`o`) before the bars.
fn agy_external_column(
    bar_full_width: f64,
    text_box: f64,
    percent_box: f64,
    origin: f64,
    phases: ModePhases,
    marker_box: f64,
) -> ProviderMetrics {
    ProviderMetrics {
        text_box,
        percent_box,
        marker_box,
        origin,
        text_slide: -text_box * phases.text_collapse,
        row_width: compact_row_width(bar_full_width, text_box, percent_box, origin, phases),
    }
}

fn display_provider_id(key: &str) -> ProviderId {
    match key {
        "claude" => ProviderId::Claude,
        "codex" => ProviderId::Codex,
        "antigravity" => ProviderId::Antigravity,
        "opencode" => ProviderId::OpenCode,
        "cursor" => ProviderId::Cursor,
        "grok" => ProviderId::Grok,
        "copilot" => ProviderId::Copilot,
        "qoder" => ProviderId::Qoder,
        other => {
            debug_assert!(false, "unknown Compact provider key {other}");
            ProviderId::Claude
        }
    }
}

/// `display.row_width.<key>` for one row: the visible bar plus whatever text,
/// percent badge and marker column the current mode phase keeps on screen.
pub fn compact_row_width(
    bar_full_width: f64,
    text_box: f64,
    percent_box: f64,
    origin: f64,
    phases: ModePhases,
) -> f64 {
    bar_full_width * (1.0 - phases.bar_collapse)
        + COMPACT_ROW_GAP * (1.0 - phases.text_collapse)
        + text_box * (1.0 - phases.text_collapse)
        + percent_box * phases.bar_collapse
        + origin
}

/// The Fable column: a marker, then the bar stacked above its text line. The
/// bar still sets the minimum width, so the stacked text extent is expressed
/// through `compact_row_width` as `max(bar, text) - bar - gap`.
fn fable_column(
    bar_full_width: f64,
    text_box: f64,
    percent_box: f64,
    marker_box: f64,
    origin: f64,
    phases: ModePhases,
    dual: bool,
) -> ProviderMetrics {
    if dual {
        // Two pool rows, each with its text beside the bar like a provider.
        return ProviderMetrics {
            text_box,
            percent_box,
            marker_box,
            origin,
            text_slide: -text_box * phases.text_collapse,
            row_width: compact_row_width(bar_full_width, text_box, percent_box, origin, phases),
        };
    }
    let stacked_text = bar_full_width.max(text_box) - bar_full_width - COMPACT_ROW_GAP;
    ProviderMetrics {
        text_box,
        percent_box,
        marker_box,
        origin,
        text_slide: -text_box * phases.text_collapse,
        row_width: compact_row_width(bar_full_width, stacked_text, percent_box, origin, phases),
    }
}

/// Sum of the enabled rows plus the 6 px inter-widget gap. `count` is already
/// clamped to one row exactly like `providers.count`.
pub fn compact_providers_width(rows: impl IntoIterator<Item = f64>, count: usize) -> f64 {
    let rows: f64 = rows.into_iter().sum();
    rows + count.saturating_sub(1) as f64 * 6.0
}

/// Container of the agy external-models widget shown next to the agy one.
pub(crate) const ANTIGRAVITY_EXTERNAL_WIDGET: &str = "antigravity-external-provider";

/// Slot after the eight providers: the Fable column's text measurement.
const FABLE_METRICS_INDEX: usize = DISPLAY_PROVIDER_KEYS.len();
const FABLE_KEY: &str = "fable";
/// Widget key (mode slot, settings key and `display.*.<key>` bindings) of the
/// agy external-models widget; its container is `ANTIGRAVITY_EXTERNAL_WIDGET`.
pub(crate) const AGY_EXTERNAL_KEY: &str = "agy_external";
/// Slot after the Fable column: the agy external-models widget.
const AGY_EXTERNAL_INDEX: usize = FABLE_METRICS_INDEX + 1;
/// Providers, the Fable column and the agy external-models widget.
pub(crate) const WIDGET_SLOTS: usize = AGY_EXTERNAL_INDEX + 5;

fn display_provider_key(provider_id: &str) -> Option<&'static str> {
    // The agy external-models widget is measured as its own widget.
    if provider_id.eq_ignore_ascii_case(ANTIGRAVITY_EXTERNAL_WIDGET) {
        return Some(AGY_EXTERNAL_KEY);
    }
    widget_keys().find(|key| provider_id.eq_ignore_ascii_case(&format!("{key}-provider")))
}

fn scene_provider_key<'a>(
    object: &'a SceneObject,
    by_id: &HashMap<&'a str, &'a SceneObject>,
) -> Option<&'static str> {
    let mut current = object.parent.as_deref();
    while let Some(parent_id) = current {
        if let Some(key) = display_provider_key(parent_id) {
            return Some(key);
        }
        current = by_id
            .get(parent_id)
            .and_then(|parent| parent.parent.as_deref());
    }
    None
}

fn scene_object_renders(
    object: &SceneObject,
    by_id: &HashMap<&str, &SceneObject>,
    context: &DataContext,
) -> bool {
    let mut current = Some(object);
    while let Some(candidate) = current {
        if !evaluate(&candidate.render.0, context)
            .is_ok_and(|value| value.is_finite() && value != 0.0)
        {
            return false;
        }
        current = candidate
            .parent
            .as_deref()
            .and_then(|parent_id| by_id.get(parent_id).copied());
    }
    true
}

/// Mirrors `format_template`'s token walk and reports whether a usage
/// substitution produced a placeholder. Cursor prefixes (`int `, `ext `) mean
/// the whole string must never be compared against `--`.
fn template_has_placeholder_usage(template: &str, context: &DataContext) -> bool {
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '{' && index + 1 < chars.len() && chars[index + 1] == '{' {
            index += 2;
            continue;
        }
        if chars[index] != '{' {
            index += 1;
            continue;
        }
        let Some(relative_end) = chars[index + 1..].iter().position(|c| *c == '}') else {
            index += 1;
            continue;
        };
        let end = index + 1 + relative_end;
        let token: String = chars[index + 1..end].iter().collect();
        let (expression, format) = split_template_token(&token);
        let expression = expression.trim();
        let format = format.trim();
        let value = if !format.eq_ignore_ascii_case("usage_line")
            && !format.eq_ignore_ascii_case("usage_badge")
        {
            None
        } else if let Some(value) = context.get_string(expression) {
            Some(value.to_string())
        } else if format.eq_ignore_ascii_case("usage_line") {
            Some(format_usage_line(expression, context).unwrap_or_else(|| "--".into()))
        } else if format.eq_ignore_ascii_case("usage_badge") {
            Some(format_usage_badge(expression, context).unwrap_or_else(|| "--".into()))
        } else {
            None
        };
        if let Some(value) = value {
            if value.is_empty() || value == "--" || value == "!" {
                return true;
            }
        }
        index = end + 1;
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
enum MeasuredLineKind {
    Text,
    Percent,
    Marker,
}

struct MeasuredLine {
    provider_index: usize,
    kind: MeasuredLineKind,
    text: String,
    family: String,
    font_size: f64,
    weight: i32,
}

/// Build the eight extremes the theme can show: normal/hover and the text and
/// percent mode phases, each in both system themes. `visibility` is ignored
/// because it follows the mode animations; `render` is not.
fn measurement_contexts(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> Vec<DataContext> {
    let mut contexts = Vec::with_capacity(8);
    for dark in [0u8, 1] {
        for (phase, view_mode) in [(0.0, 1), (2.0, 3)] {
            for hover in [false, true] {
                let mut variant = runtime;
                variant.mode_phase = phase;
                variant.widget_mode_phases = [None; WIDGET_SLOTS];
                variant.view_mode = view_mode;
                let (claude, codex, antigravity, opencode, qoder) = if hover {
                    (1.0, 1.0, 0.0, 1.0, 1.0)
                } else {
                    (0.0, 0.0, 1.0, 0.0, 0.0)
                };
                variant.claude_hovered = hover;
                variant.codex_hovered = hover;
                variant.claude_crossfade_opacity = claude;
                variant.codex_crossfade_opacity = codex;
                variant.crossfade_opacity = antigravity;
                variant.opencode_crossfade_opacity = opencode;
                variant.qoder_crossfade_opacity = qoder;
                variant.agy_external_crossfade_opacity = claude;
                variant.split_hover_phases = [claude; 4];
                let mut context =
                    DataContext::from_usage_with_runtime(data, &Canvas::default(), variant);
                context.insert("system.dark", f64::from(dark));
                contexts.push(context);
            }
        }
    }
    contexts
}

fn display_metrics_cache_key(
    theme: &ThemeDocument,
    runtime: ThemeRuntime,
    data: Option<&AppUsageData>,
    surface: &SceneObject,
    fingerprint: &str,
) -> String {
    let base = DataContext::from_usage_with_runtime(data, &Canvas::default(), runtime);
    let mut key = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|",
        theme.id,
        runtime.countdown,
        runtime.use_windows_default_font,
        base.get("display.widget_count").unwrap_or(0.0),
        DISPLAY_PROVIDER_KEYS
            .iter()
            .filter(|key| runtime.provider_enabled(display_provider_id(key)))
            .copied()
            .collect::<Vec<_>>()
            .join(","),
        base.get("claude.dual_mode").unwrap_or(0.0),
        base.get("codex.dual_mode").unwrap_or(0.0),
        base.get("qoder.session.available").unwrap_or(0.0),
        base.get("qoder.week.available").unwrap_or(0.0),
        base.get("antigravity.pool_o").unwrap_or(0.0),
        base.get("display.agy_split").unwrap_or(0.0),
    );
    key.push_str(&surface.id);
    key.push('|');
    key.push_str(fingerprint);
    key
}

fn cached_display_metrics(key: &str) -> Option<DisplayMetrics> {
    let cache = DISPLAY_METRICS_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let cache = cache.lock().ok()?;
    cache
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, metrics)| metrics.clone())
}

fn store_display_metrics(key: String, metrics: &DisplayMetrics) {
    let cache = DISPLAY_METRICS_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    if cache.len() >= DISPLAY_METRICS_CACHE_CAPACITY {
        cache.pop_front();
    }
    cache.push_back((key, metrics.clone()));
}

static DISPLAY_METRICS_CACHE: OnceLock<Mutex<VecDeque<(String, DisplayMetrics)>>> = OnceLock::new();

/// Measure every Full/percent/marker line the Compact surface can show.
/// Repeated calls in one frame reuse the cached result through the theme,
/// formatted-string and runtime fingerprint.
pub(crate) fn display_metrics(
    theme: &ThemeDocument,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
) -> DisplayMetrics {
    #[cfg(test)]
    METRICS_COMPUTATION_COUNT.with(|count| count.set(count.get() + 1));
    let widgets = widget_count(data, runtime);
    let Some(surface) = theme.surfaces.first() else {
        return DisplayMetrics::legacy_with_widgets(runtime, widgets);
    };
    let by_id: HashMap<&str, &SceneObject> = surface
        .children
        .iter()
        .map(|object| (object.id.as_str(), object))
        .collect();
    let contexts = measurement_contexts(data, runtime);
    let mut fingerprint = String::new();
    let mut lines = Vec::new();
    for object in &surface.children {
        let SceneContent::Text {
            template,
            font_family,
            font_size,
            weight,
            ..
        } = &object.content
        else {
            continue;
        };
        let Some(key) = scene_provider_key(object, &by_id) else {
            continue;
        };
        let provider_index = widget_mode_index(key).unwrap_or(0);
        let kind = if object.id.contains("-marker-") {
            MeasuredLineKind::Marker
        } else if object.id.ends_with("-pct") {
            MeasuredLineKind::Percent
        } else {
            MeasuredLineKind::Text
        };
        fingerprint.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|",
            object.id,
            template,
            font_family,
            font_size.0,
            weight.gdi_weight(),
            object.render.0
        ));
        for context in &contexts {
            if !scene_object_renders(object, &by_id, context) {
                fingerprint.push_str("0;");
                continue;
            }
            let text = format_template(template, context);
            if kind != MeasuredLineKind::Marker && template_has_placeholder_usage(template, context)
            {
                fingerprint.push_str("p;");
                continue;
            }
            fingerprint.push_str("1:");
            fingerprint.push_str(&text);
            fingerprint.push(';');
            if text.is_empty() {
                continue;
            }
            let font_size = evaluate(&font_size.0, context).unwrap_or(13.0);
            let family = if context.force_segoe_ui {
                crate::widget_font::segoe_ui_family(weight.gdi_weight()).to_string()
            } else {
                font_family.clone()
            };
            fingerprint.push_str(&format!("font:{family}:{font_size};"));
            lines.push(MeasuredLine {
                provider_index,
                kind,
                text,
                family,
                font_size,
                weight: weight.gdi_weight(),
            });
        }
    }
    let key = display_metrics_cache_key(theme, runtime, data, surface, &fingerprint);
    if let Some(metrics) = cached_display_metrics(&key) {
        return metrics_for_phase(metrics, runtime);
    }

    let base = DataContext::from_usage_with_runtime(data, &Canvas::default(), runtime);
    let mut text_advance = [None::<f64>; WIDGET_SLOTS];
    let mut percent_advance = [None::<f64>; WIDGET_SLOTS];
    let mut marker_advance = [None::<f64>; WIDGET_SLOTS];
    let mut advances = HashMap::new();
    for line in &lines {
        let advance = *advances
            .entry((
                &line.text,
                &line.family,
                line.font_size.to_bits(),
                line.weight,
            ))
            .or_insert_with(|| {
                #[cfg(test)]
                MEASUREMENT_COUNT.with(|count| count.set(count.get() + 1));
                text_measure::measure_text(&line.text, &line.family, line.font_size, line.weight)
            });
        let Some(advance) = advance else {
            continue;
        };
        let slot = match line.kind {
            MeasuredLineKind::Text => &mut text_advance[line.provider_index],
            MeasuredLineKind::Percent => &mut percent_advance[line.provider_index],
            MeasuredLineKind::Marker => &mut marker_advance[line.provider_index],
        };
        *slot = Some((*slot).map_or(advance, |current: f64| current.max(advance)));
    }

    let legacy = DisplayMetrics::legacy_with_widgets(runtime, widgets);
    let bar = compact_bar_full_width(widgets);
    let claude_dual = base.get("claude.dual_mode").unwrap_or(0.0) != 0.0;
    let codex_dual = base.get("codex.dual_mode").unwrap_or(0.0) != 0.0;
    let qoder_pool = base.get("qoder.session.available").unwrap_or(0.0) != 0.0
        || base.get("qoder.week.available").unwrap_or(0.0) != 0.0;
    let agy_pool_o = base.get("antigravity.pool_o").unwrap_or(0.0) != 0.0;
    let mut metrics = legacy.clone();
    metrics.agy_split = base.get("display.agy_split").unwrap_or(0.0) != 0.0;
    metrics.agy_separate = agy_separate(data, runtime);
    let (fable_marker_box, fable_origin) =
        marker_column(marker_advance[FABLE_METRICS_INDEX], legacy.fable);
    metrics.fable = fable_column(
        bar,
        text_advance[FABLE_METRICS_INDEX].map_or(legacy.fable.text_box, |advance| advance + 2.0),
        percent_advance[FABLE_METRICS_INDEX].map_or(COMPACT_PERCENT_WIDTH, |advance| advance + 2.0),
        fable_marker_box,
        fable_origin,
        decompose_mode_phase(runtime.widget_mode_phase(FABLE_METRICS_INDEX)),
        claude_dual,
    );
    let (external_marker_box, external_origin) =
        marker_column(marker_advance[AGY_EXTERNAL_INDEX], legacy.agy_external);
    metrics.agy_external = agy_external_column(
        bar,
        text_advance[AGY_EXTERNAL_INDEX]
            .map_or(legacy.agy_external.text_box, |advance| advance + 2.0),
        percent_advance[AGY_EXTERNAL_INDEX]
            .map_or(legacy.agy_external.percent_box, |advance| advance + 2.0),
        external_origin,
        decompose_mode_phase(runtime.widget_mode_phase(AGY_EXTERNAL_INDEX)),
        external_marker_box,
    );
    for (index, key) in DISPLAY_PROVIDER_KEYS.iter().enumerate() {
        let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
        let fallback = legacy.providers[index];
        let text_box = text_advance[index].map_or(fallback.text_box, |advance| advance + 2.0);
        let percent_box =
            percent_advance[index].map_or(fallback.percent_box, |advance| advance + 2.0);
        let (marker_box, origin) = match *key {
            "claude" if !claude_dual => (0.0, 0.0),
            "codex" if !codex_dual => (0.0, 0.0),
            "claude" | "codex" => marker_column(marker_advance[index], fallback),
            "opencode" => (fallback.marker_box, fallback.origin),
            "qoder" if qoder_pool => marker_column(marker_advance[index], fallback),
            "qoder" => (fallback.marker_box, fallback.origin),
            "antigravity" if agy_pool_o => marker_column(
                marker_advance[index],
                ProviderMetrics {
                    marker_box: COMPACT_MARKER_WIDTH,
                    origin: COMPACT_ORIGIN_LEGACY,
                    ..fallback
                },
            ),
            _ => (0.0, 0.0),
        };
        metrics.providers[index] = ProviderMetrics {
            text_box,
            percent_box,
            marker_box,
            origin,
            text_slide: -text_box * phases.text_collapse,
            row_width: compact_row_width(bar, text_box, percent_box, origin, phases),
        };
    }
    for (index, target) in [
        (AGY_EXTERNAL_INDEX + 1, &mut metrics.claude_x),
        (AGY_EXTERNAL_INDEX + 2, &mut metrics.antigravity_o),
        (AGY_EXTERNAL_INDEX + 3, &mut metrics.agy_external_o),
        (AGY_EXTERNAL_INDEX + 4, &mut metrics.codex_x),
    ] {
        let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
        target.text_box = text_advance[index].map_or(target.text_box, |advance| advance + 2.0);
        target.percent_box =
            percent_advance[index].map_or(target.percent_box, |advance| advance + 2.0);
        if index != AGY_EXTERNAL_INDEX + 3 {
            target.origin = 0.0;
        }
        target.text_slide = -target.text_box * phases.text_collapse;
        target.row_width = compact_row_width(
            bar,
            target.text_box,
            target.percent_box,
            target.origin,
            phases,
        );
    }
    metrics.providers_width = metrics.providers_width(runtime);
    store_display_metrics(key, &metrics);
    metrics_for_phase(metrics, runtime)
}

pub(crate) fn metrics_for_phase(
    mut metrics: DisplayMetrics,
    runtime: ThemeRuntime,
) -> DisplayMetrics {
    let bar = compact_bar_full_width(metrics.widget_count);
    for (index, provider) in metrics.providers.iter_mut().enumerate() {
        let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
        provider.text_slide = -provider.text_box * phases.text_collapse;
        provider.row_width = compact_row_width(
            bar,
            provider.text_box,
            provider.percent_box,
            provider.origin,
            phases,
        );
    }
    metrics.fable = fable_column(
        bar,
        metrics.fable.text_box,
        metrics.fable.percent_box,
        metrics.fable.marker_box,
        metrics.fable.origin,
        decompose_mode_phase(runtime.widget_mode_phase(FABLE_METRICS_INDEX)),
        claude_dual(runtime),
    );
    let external = metrics.agy_external;
    metrics.agy_external = agy_external_column(
        bar,
        external.text_box,
        external.percent_box,
        external.origin,
        decompose_mode_phase(runtime.widget_mode_phase(AGY_EXTERNAL_INDEX)),
        external.marker_box,
    );
    for (index, target) in [
        (AGY_EXTERNAL_INDEX + 1, &mut metrics.claude_x),
        (AGY_EXTERNAL_INDEX + 2, &mut metrics.antigravity_o),
        (AGY_EXTERNAL_INDEX + 3, &mut metrics.agy_external_o),
        (AGY_EXTERNAL_INDEX + 4, &mut metrics.codex_x),
    ] {
        let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
        target.text_slide = -target.text_box * phases.text_collapse;
        target.row_width = compact_row_width(
            bar,
            target.text_box,
            target.percent_box,
            target.origin,
            phases,
        );
    }
    metrics.providers_width = metrics.providers_width(runtime);
    metrics
}

/// Measured marker column: widest available glyph plus a 1 px frame and a
/// 2 px gap to the bar. Falls back to the legacy 10/12 when GDI refused.
fn marker_column(advance: Option<f64>, fallback: ProviderMetrics) -> (f64, f64) {
    match advance {
        Some(advance) => {
            let marker_box = advance + 1.0;
            (marker_box, marker_box + 2.0)
        }
        None => (fallback.marker_box, fallback.origin),
    }
}

fn insert_display_metrics(context: &mut DataContext, metrics: &DisplayMetrics) {
    context.display_metrics = Some(metrics.clone());
}

fn display_metric(metrics: &DisplayMetrics, name: &str) -> Option<f64> {
    if name.eq_ignore_ascii_case("display.providers_width") {
        return Some(metrics.providers_width);
    }
    let (prefix, key) = name.rsplit_once('.')?;
    let provider = if key.eq_ignore_ascii_case(FABLE_KEY) {
        metrics.fable
    } else if key.eq_ignore_ascii_case(AGY_EXTERNAL_KEY) {
        metrics.agy_external
    } else if key.eq_ignore_ascii_case("claude_x") {
        metrics.claude_x
    } else if key.eq_ignore_ascii_case("codex_x") {
        metrics.codex_x
    } else if key.eq_ignore_ascii_case("antigravity_o") {
        metrics.antigravity_o
    } else if key.eq_ignore_ascii_case("agy_external_o") {
        metrics.agy_external_o
    } else {
        let index = DISPLAY_PROVIDER_KEYS
            .iter()
            .position(|candidate| key.eq_ignore_ascii_case(candidate))?;
        metrics.providers[index]
    };
    if prefix.eq_ignore_ascii_case("display.text_box") {
        Some(provider.text_box)
    } else if prefix.eq_ignore_ascii_case("display.percent_box") {
        Some(provider.percent_box)
    } else if prefix.eq_ignore_ascii_case("display.marker_box") {
        Some(provider.marker_box)
    } else if prefix.eq_ignore_ascii_case("display.origin") {
        Some(provider.origin)
    } else if prefix.eq_ignore_ascii_case("display.text_slide") {
        Some(provider.text_slide)
    } else if prefix.eq_ignore_ascii_case("display.row_width") {
        Some(provider.row_width)
    } else {
        None
    }
}

/// Insert every binding derived from the animation state. Called once per
/// render, so the theme never duplicates the formulas.
fn insert_animation_bindings(context: &mut DataContext, runtime: &ThemeRuntime, widgets: usize) {
    let phase = runtime.mode_phase.clamp(0.0, 2.0);
    let phases = decompose_mode_phase(phase);
    context.insert("display.view_mode", f64::from(runtime.view_mode));
    context.insert("display.mode_phase", phase);
    context.insert("display.mode_text_progress", phases.text_collapse);
    context.insert("display.mode_bar_progress", phases.bar_collapse);

    for (index, key) in widget_keys().enumerate() {
        let phase = runtime.widget_mode_phase(index);
        let phases = decompose_mode_phase(phase);
        context.insert(&format!("display.mode_phase.{key}"), phase);
        context.insert(
            &format!("display.mode_text_progress.{key}"),
            phases.text_collapse,
        );
        context.insert(
            &format!("display.mode_bar_progress.{key}"),
            phases.bar_collapse,
        );
    }

    let bar = compact_bar_full_width(widgets);
    context.insert("display.widget_count", widgets as f64);
    context.insert("display.bar_full_width", bar);
    let narrow = mode_row_bindings(
        phase,
        bar,
        COMPACT_TEXT_WIDTH,
        COMPACT_MARKER_WIDTH,
        COMPACT_ROW_GAP,
    );
    let wide = mode_row_bindings(
        phase,
        bar,
        COMPACT_TEXT_WIDTH_WIDE,
        COMPACT_MARKER_WIDTH,
        COMPACT_ROW_GAP,
    );
    context.insert("display.bar_width", narrow.bar_width);
    context.insert("display.bar_x", narrow.bar_x);
    context.insert("display.text_width", narrow.text_width);
    context.insert("display.text_slide_x", narrow.text_slide_x);
    context.insert("display.text_opacity", narrow.text_opacity);
    context.insert("display.percent_opacity", narrow.percent_opacity);
    context.insert("display.marker_opacity", narrow.marker_opacity);
    context.insert("display.provider_width", narrow.provider_width);
    // Rows carrying a window label reserve the wider text block.
    context.insert("display.text_width_wide", wide.text_width);
    context.insert("display.text_slide_x_wide", wide.text_slide_x);
    context.insert("display.provider_width_wide", wide.provider_width);
    // Per-widget row bindings follow each widget's own mode phase.
    for (index, key) in widget_keys().enumerate() {
        let row = mode_row_bindings(
            runtime.widget_mode_phase(index),
            bar,
            COMPACT_TEXT_WIDTH,
            COMPACT_MARKER_WIDTH,
            COMPACT_ROW_GAP,
        );
        context.insert(&format!("display.bar_width.{key}"), row.bar_width);
        context.insert(&format!("display.text_opacity.{key}"), row.text_opacity);
        context.insert(
            &format!("display.percent_opacity.{key}"),
            row.percent_opacity,
        );
    }

    // Hover progress per provider. Antigravity's crossfade runs in the
    // opposite direction (1.0 = normal, 0.0 = hover), so invert it here to
    // give every provider the same 0..1 hover phase.
    let clamp = |value: f64| value.clamp(0.0, 1.0);
    context.insert(
        "claude.hover_phase",
        clamp(runtime.claude_crossfade_opacity),
    );
    context.insert("codex.hover_phase", clamp(runtime.codex_crossfade_opacity));
    context.insert(
        "antigravity.hover_phase",
        1.0 - clamp(runtime.crossfade_opacity),
    );
    context.insert(
        "opencode.hover_phase",
        clamp(runtime.opencode_crossfade_opacity),
    );
    context.insert("qoder.hover_phase", clamp(runtime.qoder_crossfade_opacity));
    context.insert(
        "agy_external.hover_phase",
        clamp(runtime.agy_external_crossfade_opacity),
    );

    // Adaptive Compact columns. Legacy reserves are the base every context
    // sees; theme-aware resolvers overwrite them with measured values.
    let metrics = DisplayMetrics::legacy_with_widgets(*runtime, widgets);
    insert_display_metrics(context, &metrics);
}

impl DataContext {
    /// Evaluate a surface render expression only when it does not require its
    /// resolved size. Unknown values leave the decision to the normal renderer.
    pub fn surface_render_without_size(
        expression: &Expression,
        data: Option<&AppUsageData>,
        canvas: &Canvas,
        runtime: ThemeRuntime,
    ) -> Option<bool> {
        let mut context = Self::from_usage_with_runtime(data, canvas, runtime);
        context.display_metrics = None;
        for name in [
            "canvas.width",
            "canvas.height",
            "parent.width",
            "parent.height",
        ] {
            Arc::make_mut(&mut context.values).remove(name);
            context.geometry_values.remove(name);
        }
        evaluate(&expression.0, &context)
            .ok()
            .map(|value| value.is_finite() && value != 0.0)
    }

    pub fn from_usage(data: Option<&AppUsageData>, canvas: &Canvas) -> Self {
        Self::from_usage_with_runtime(data, canvas, ThemeRuntime::default())
    }

    pub fn from_usage_with_runtime(
        data: Option<&AppUsageData>,
        canvas: &Canvas,
        runtime: ThemeRuntime,
    ) -> Self {
        let mut context = Self {
            force_segoe_ui: !runtime.use_windows_default_font,
            ..Self::default()
        };
        context.insert("canvas.width", canvas.width as f64);
        context.insert("canvas.height", canvas.height as f64);
        context.insert("parent.width", canvas.width as f64);
        context.insert("parent.height", canvas.height as f64);
        context.insert("host.width", runtime.host_width as f64);
        context.insert("host.height", runtime.host_height as f64);
        context.insert("pi", std::f64::consts::PI);
        context.insert("e", std::f64::consts::E);
        context.insert("true", 1.0);
        context.insert("false", 0.0);
        context.insert_string("app.version", env!("CARGO_PKG_VERSION"));
        let mut version_parts = env!("CARGO_PKG_VERSION")
            .split(['.', '-', '+'])
            .filter_map(|part| part.parse::<f64>().ok());
        context.insert("app.version.major", version_parts.next().unwrap_or(0.0));
        context.insert("app.version.minor", version_parts.next().unwrap_or(0.0));
        context.insert("app.version.patch", version_parts.next().unwrap_or(0.0));
        context.insert("system.dark", crate::theme::is_dark_mode() as u8 as f64);
        context.insert("data.poll_ok", runtime.poll_ok as u8 as f64);
        context.insert("data.has_error", runtime.has_error as u8 as f64);
        context.insert(
            "data.loading",
            (!runtime.poll_ok && !runtime.has_error) as u8 as f64,
        );
        let strings = runtime.language.strings();
        context.insert_string("i18n.locale", runtime.language.code());
        context.insert_string("i18n.session_window", strings.session_window);
        context.insert_string("i18n.weekly_window", strings.weekly_window);
        context.insert_string("i18n.cursor_auto_window", strings.cursor_auto_window);
        context.insert_string("i18n.cursor_api_window", strings.cursor_api_window);
        context.insert_string("i18n.now", strings.now);
        context.insert_string("i18n.day_suffix", strings.day_suffix);
        context.insert_string("i18n.hour_suffix", strings.hour_suffix);
        context.insert_string("i18n.minute_suffix", strings.minute_suffix);
        context.insert_string("i18n.second_suffix", strings.second_suffix);
        context.insert(
            "providers.count",
            (runtime.provider_count()
                + claude_separate(runtime) as usize
                + codex_separate(runtime) as usize
                + agy_separate(data, runtime) as usize) as f64,
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs_f64())
            .unwrap_or(0.0);
        context.insert("time.now.unix", now);
        context.insert("time.now.milliseconds", now * 1000.0);
        for (zone, local) in [("local", true), ("utc", false)] {
            if let Some(value) = timestamp_parts(now, local) {
                for (component, value) in [
                    ("year", value.year),
                    ("month", value.month),
                    ("day", value.day),
                    ("weekday", value.weekday),
                    ("hour", value.hour),
                    ("minute", value.minute),
                    ("second", value.second),
                ] {
                    context.insert(&format!("time.{zone}.{component}"), f64::from(value));
                }
            }
        }
        for descriptor in PROVIDER_DESCRIPTORS {
            context.insert_string(&format!("{}.account.name", descriptor.key), "");
            context.insert(
                &format!("providers.{}.enabled", descriptor.key),
                runtime.provider_enabled(descriptor.id) as u8 as f64,
            );
        }
        context.insert("display.countdown", runtime.countdown as u8 as f64);
        context.insert("display.show_pace", runtime.show_pace as u8 as f64);
        context.insert(
            "display.pool_split_supported",
            runtime.pool_split_supported as u8 as f64,
        );
        context.insert("display.show_fable", runtime.show_fable as u8 as f64);
        let (fable_main, fable_x) = fable_pools(runtime);
        context.insert("display.fable_main", fable_main as u8 as f64);
        context.insert("display.fable_x", fable_x as u8 as f64);
        context.insert(
            "display.fable_visible",
            fable_visible(data, runtime) as u8 as f64,
        );
        context.insert(
            "display.fable_separate",
            (fable_visible(data, runtime) && claude_dual(runtime)) as u8 as f64,
        );
        context.insert(
            "display.fable_row",
            (if claude_separate(runtime) {
                runtime.show_fable
            } else {
                fable_visible(data, runtime) && !claude_dual(runtime)
            }) as u8 as f64,
        );
        context.insert(
            "display.fable_x_row",
            (claude_separate(runtime) && runtime.show_fable_x) as u8 as f64,
        );
        context.insert(
            "display.claude_split",
            claude_separate(runtime) as u8 as f64,
        );
        context.insert("display.codex_split", codex_separate(runtime) as u8 as f64);
        context.insert(
            "display.agy_pool_split",
            agy_separate(data, runtime) as u8 as f64,
        );
        insert_animation_bindings(&mut context, &runtime, widget_count(data, runtime));
        context.insert(
            "antigravity.gemini.enabled",
            runtime.antigravity_gemini_enabled as u8 as f64,
        );
        context.insert(
            "antigravity.external.enabled",
            runtime.antigravity_external_enabled as u8 as f64,
        );
        let pool_o = antigravity_pool_o(data, runtime) && !agy_separate(data, runtime);
        context.insert(
            "antigravity.x.enabled",
            runtime.antigravity_x_enabled as u8 as f64,
        );
        context.insert("antigravity.pool_o", pool_o as u8 as f64);
        context.insert(
            "antigravity.a.enabled",
            runtime.antigravity_a_enabled as u8 as f64,
        );
        context.insert(
            "display.agy_split",
            antigravity_split(data, runtime) as u8 as f64,
        );
        context.insert("display.agy_o_external", 0.0);
        // Two rows: Gemini and external models (pool A), or pools A and O.
        let agy_dual_mode =
            (runtime.antigravity_gemini_enabled && runtime.antigravity_external_enabled) || pool_o;
        context.insert("antigravity.dual_mode", agy_dual_mode as u8 as f64);
        let crossfade = runtime.crossfade_opacity.clamp(0.0, 1.0);
        context.insert("antigravity.crossfade_opacity", crossfade);
        let (agy_normal_opacity, agy_hover_opacity) =
            asymmetric_crossfade_opacities(1.0 - crossfade);
        context.insert("antigravity.normal.opacity", agy_normal_opacity);
        context.insert("antigravity.hover.opacity", agy_hover_opacity);
        // crossfade 1.0 shows the primary pool: Gemini in dual mode, otherwise
        // the single enabled pool; hover fades toward the other one.
        let (gemini_opacity, external_opacity) = match (
            runtime.antigravity_gemini_enabled,
            runtime.antigravity_external_enabled,
        ) {
            (false, true) => (agy_hover_opacity, agy_normal_opacity),
            (false, false) => (0.0, 0.0),
            _ => (agy_normal_opacity, agy_hover_opacity),
        };
        let only_gemini_a = agy_separate(data, runtime) && !runtime.antigravity_external_enabled;
        context.insert(
            "antigravity.gemini.opacity",
            if only_gemini_a { 1.0 } else { gemini_opacity },
        );
        context.insert(
            "antigravity.external.opacity",
            if only_gemini_a { 0.0 } else { external_opacity },
        );
        context.insert("claude.hovered", runtime.claude_hovered as u8 as f64);
        context.insert_hover_crossfade("claude", runtime.claude_crossfade_opacity);
        context.insert(
            "claude.main.enabled",
            runtime.claude_main_enabled as u8 as f64,
        );
        context.insert("claude.x.enabled", runtime.claude_x_enabled as u8 as f64);
        context.insert("claude.dual_mode", claude_dual(runtime) as u8 as f64);
        context.insert("codex.hovered", runtime.codex_hovered as u8 as f64);
        context.insert_hover_crossfade("codex", runtime.codex_crossfade_opacity);
        context.insert(
            "codex.main.enabled",
            runtime.codex_main_enabled as u8 as f64,
        );
        context.insert("codex.x.enabled", runtime.codex_x_enabled as u8 as f64);
        context.insert(
            "codex.dual_mode",
            (runtime.codex_main_enabled
                && runtime.codex_x_enabled
                && runtime.codex_pools_one_widget) as u8 as f64,
        );
        context.insert_hover_crossfade("opencode", runtime.opencode_crossfade_opacity);
        // Qoder's hover only swaps in a second pool. If a poll removes that
        // pool mid-hover, rest at phase 0 so the single bottom row stays drawn.
        let qoder_phase = if data
            .and_then(|data| data.get(ProviderId::Qoder))
            .is_some_and(|usage| usage.week.is_some())
        {
            runtime.qoder_crossfade_opacity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        context.insert("qoder.hover_phase", qoder_phase);
        context.insert_hover_crossfade("qoder", qoder_phase);
        context.insert_hover_crossfade(AGY_EXTERNAL_KEY, runtime.agy_external_crossfade_opacity);
        if let Some(data) = data {
            for account in &data.accounts {
                let key = format!(
                    "accounts.{}.{}",
                    account.provider.descriptor().key,
                    account.profile.id
                );
                context.insert_string(&format!("{key}.name"), &account.profile.name);
                context.insert(&format!("{key}.selected"), account.selected as u8 as f64);
                context.insert(
                    &format!("{key}.has_error"),
                    account.error.is_some() as u8 as f64,
                );
                context.insert_provider(
                    &key,
                    account.usage.as_ref(),
                    account.provider == ProviderId::Codex,
                    runtime.countdown,
                );
            }
            if !data
                .accounts
                .iter()
                .any(|a| a.provider == ProviderId::Claude && a.profile.id == "default")
            {
                // Without any Claude account the legacy entry is the default
                // profile. Otherwise it may belong to another profile, so the
                // fallback keeps its windows but never borrows its limits.
                let has_claude_account = data
                    .accounts
                    .iter()
                    .any(|a| a.provider == ProviderId::Claude);
                let claude_usage = data.get(ProviderId::Claude).map(|usage| {
                    let mut usage = usage.clone();
                    if has_claude_account {
                        usage.limits.clear();
                    }
                    usage
                });
                context.insert_provider(
                    "accounts.claude.default",
                    claude_usage.as_ref(),
                    false,
                    runtime.countdown,
                );
            }
            if !data
                .accounts
                .iter()
                .any(|a| a.provider == ProviderId::Claude && a.profile.id == "claude_x")
            {
                // A missing second profile keeps the legacy fallback for its
                // windows, but never borrows the default profile's limits.
                let claude_usage =
                    data.get(ProviderId::Claude)
                        .map(|usage| crate::models::UsageData {
                            limits: Vec::new(),
                            ..usage.clone()
                        });
                context.insert_provider(
                    "accounts.claude.claude_x",
                    claude_usage.as_ref(),
                    false,
                    runtime.countdown,
                );
            }
            if !data
                .accounts
                .iter()
                .any(|a| a.provider == ProviderId::Codex && a.profile.id == "default")
            {
                let codex_usage = data.get(ProviderId::Codex);
                context.insert_provider(
                    "accounts.codex.default",
                    codex_usage,
                    true,
                    runtime.countdown,
                );
            }
            if !data
                .accounts
                .iter()
                .any(|a| a.provider == ProviderId::Codex && a.profile.id == "codex_x")
            {
                let codex_usage = data.get(ProviderId::Codex);
                context.insert_provider(
                    "accounts.codex.codex_x",
                    codex_usage,
                    true,
                    runtime.countdown,
                );
            }
            for descriptor in PROVIDER_DESCRIPTORS {
                if let Some(account) = data
                    .accounts
                    .iter()
                    .find(|account| account.provider == descriptor.id && account.selected)
                {
                    context.insert(
                        &format!("{}.has_error", descriptor.key),
                        account.error.is_some() as u8 as f64,
                    );
                }
                context.insert_string(
                    &format!("{}.account.name", descriptor.key),
                    data.selected_account_name(descriptor.id).unwrap_or(""),
                );
                context.insert_provider(
                    descriptor.key,
                    data.get(descriptor.id),
                    descriptor.id == ProviderId::Codex,
                    runtime.countdown,
                );
            }
            let pool_o_usage = data
                .antigravity_x()
                .and_then(|account| account.usage.as_ref());
            // With pool A off, pool O fills the single-pool layout alone.
            let pool_a = if runtime.antigravity_o_only() {
                pool_o_usage
            } else {
                // Never pool O's stand-in reading under pool A's label.
                data.antigravity_pool_a_reading()
            };
            context.insert_antigravity_rows(pool_a, pool_o_usage, pool_o, runtime);
            if let Some(agy) = pool_a {
                // Antigravity answered this poll; pool-level gaps are "no limit
                // data" ("--"), not a provider error.
                context.insert("antigravity.reported", 1.0);
                context.insert_provider("antigravity.gemini", Some(agy), false, runtime.countdown);
                // A missing external pool stays empty ("--"); never show Gemini figures in its place.
                context.insert_provider(
                    "antigravity.external",
                    agy.external_pool.as_deref(),
                    false,
                    runtime.countdown,
                );
            } else {
                context.insert_provider("antigravity.gemini", None, false, runtime.countdown);
                context.insert_provider("antigravity.external", None, false, runtime.countdown);
            }
            let active = ProviderId::ALL
                .into_iter()
                .find_map(|provider| data.get(provider).map(|usage| (provider, usage)));
            context.insert_provider(
                "active",
                active.map(|(_, usage)| usage),
                active.is_some_and(|(provider, _)| provider == ProviderId::Codex),
                runtime.countdown,
            );
        } else {
            for descriptor in PROVIDER_DESCRIPTORS {
                context.insert_provider(
                    descriptor.key,
                    None,
                    descriptor.id == ProviderId::Codex,
                    runtime.countdown,
                );
            }
            context.insert_provider("antigravity.gemini", None, false, runtime.countdown);
            context.insert_provider("antigravity.external", None, false, runtime.countdown);
            context.insert_antigravity_rows(None, None, pool_o, runtime);
            context.insert_provider("active", None, false, runtime.countdown);
        }
        // Theme validation evaluates every expression even when its parent is
        // hidden, so the split namespaces must exist in single-pool contexts.
        context.copy_prefix("claude.", "claude_x.");
        context.copy_prefix("codex.", "codex_x.");
        context.copy_prefix("antigravity.", "antigravity_o.");
        context.copy_prefix("agy_external.", "agy_external_o.");
        if claude_separate(runtime) {
            for owner in ["claude", "claude_x"] {
                for field in [
                    "session.",
                    "five_hour.",
                    "weekly.",
                    "model.",
                    "limits.",
                    "scoped.",
                ] {
                    let prefix = format!("{owner}.{field}");
                    Arc::make_mut(&mut context.values).retain(|key, _| !key.starts_with(&prefix));
                    Arc::make_mut(&mut context.strings).retain(|key, _| !key.starts_with(&prefix));
                }
            }
            context.copy_prefix("accounts.claude.default.", "claude.");
            context.copy_prefix("accounts.claude.claude_x.", "claude_x.");
            let phase = runtime.split_hover_phases[0].clamp(0.0, 1.0);
            context.insert("claude_x.hover_phase", phase);
            context.insert("claude_x.hovered", (phase > 0.0) as u8 as f64);
            context.insert_hover_crossfade("claude_x", phase);
        }
        if codex_separate(runtime) {
            if data.is_some() {
                for owner in ["codex", "codex_x"] {
                    for field in [
                        "session.",
                        "five_hour.",
                        "weekly.",
                        "model.",
                        "limits.",
                        "scoped.",
                        "credits.",
                    ] {
                        let prefix = format!("{owner}.{field}");
                        Arc::make_mut(&mut context.values)
                            .retain(|key, _| !key.starts_with(&prefix));
                        Arc::make_mut(&mut context.strings)
                            .retain(|key, _| !key.starts_with(&prefix));
                    }
                }
                context.copy_prefix("accounts.codex.default.", "codex.");
                context.copy_prefix("accounts.codex.codex_x.", "codex_x.");
            }
            let phase = runtime.split_hover_phases[3].clamp(0.0, 1.0);
            context.insert("codex_x.hover_phase", phase);
            context.insert("codex_x.hovered", (phase > 0.0) as u8 as f64);
            context.insert_hover_crossfade("codex_x", phase);
        }
        if agy_separate(data, runtime) {
            // Build pool O through the same provider insertion path as A,
            // without rebuilding every unrelated provider and animation binding.
            let account = data.and_then(AppUsageData::antigravity_x);
            let pool = account.and_then(|account| account.usage.as_ref());
            let external = pool.and_then(|usage| usage.external_pool.as_deref());
            for (key, usage) in [
                ("antigravity_o", pool),
                ("antigravity_o.gemini", pool),
                ("antigravity_o.external", external),
                ("antigravity_o.row1h", pool),
                ("antigravity_o.row2", external),
                ("antigravity_o.row2h", external),
            ] {
                context.insert_provider(key, usage, false, runtime.countdown);
            }
            context.insert("antigravity_o.reported", pool.is_some() as u8 as f64);
            Arc::make_mut(&mut context.values).remove("antigravity_o.has_error");
            if let Some(account) = account {
                context.insert(
                    "antigravity_o.has_error",
                    account.error.is_some() as u8 as f64,
                );
            }
            let phase = runtime.split_hover_phases[1].clamp(0.0, 1.0);
            context.insert("antigravity_o.hover_phase", phase);
            let (normal, hover) = context.insert_hover_crossfade("antigravity_o", phase);
            context.insert(
                "antigravity_o.external.enabled",
                runtime.antigravity_external_x_enabled as u8 as f64,
            );
            context.insert(
                "antigravity_o.gemini.opacity",
                if runtime.antigravity_external_x_enabled {
                    normal
                } else {
                    1.0
                },
            );
            context.insert(
                "antigravity_o.external.opacity",
                if runtime.antigravity_external_x_enabled {
                    hover
                } else {
                    0.0
                },
            );
            context.insert(
                "antigravity_o.dual_mode",
                runtime.antigravity_external_x_enabled as u8 as f64,
            );
            context.insert(
                "antigravity.external_a.shown",
                runtime.antigravity_external_enabled as u8 as f64,
            );
            context.insert("antigravity.external_o.shown", 0.0);
            context.insert(
                "antigravity_o.external_a.shown",
                runtime.antigravity_external_x_enabled as u8 as f64,
            );
            context.insert("antigravity_o.external_o.shown", 0.0);
            context.copy_prefix("agy_external.", "agy_external_o.");
            let phase = runtime.split_hover_phases[2].clamp(0.0, 1.0);
            context.insert("agy_external_o.hover_phase", phase);
            context.insert_hover_crossfade("agy_external_o", phase);
        }
        context
    }

    fn copy_prefix(&mut self, source: &str, target: &str) {
        let values: Vec<_> = self
            .values
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(source)
                    .map(|suffix| (format!("{target}{suffix}"), *value))
            })
            .collect();
        let strings: Vec<_> = self
            .strings
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(source)
                    .map(|suffix| (format!("{target}{suffix}"), value.clone()))
            })
            .collect();
        Arc::make_mut(&mut self.values).extend(values);
        Arc::make_mut(&mut self.strings).extend(strings);
    }

    /// Sources of the agy two-row layout. Row 1 is always pool A's Gemini
    /// (`antigravity.gemini`). Without pool O row 2 is pool A's external
    /// models, exactly as before; with pool O it is pool O's Gemini, and the
    /// external models move to their own widget (`display.agy_split`).
    fn insert_antigravity_rows(
        &mut self,
        pool_a: Option<&crate::models::UsageData>,
        pool_o: Option<&crate::models::UsageData>,
        pool_o_on: bool,
        runtime: ThemeRuntime,
    ) {
        let a_external = pool_a.and_then(|usage| usage.external_pool.as_deref());
        let o_external = pool_o.and_then(|usage| usage.external_pool.as_deref());
        let row2 = if pool_o_on { pool_o } else { a_external };
        self.insert_provider("antigravity.row2", row2, false, runtime.countdown);
        self.insert_provider("antigravity.row1h", pool_a, false, runtime.countdown);
        self.insert_provider("antigravity.row2h", row2, false, runtime.countdown);
        // Pool O's row in the external-models widget. A missing or failed
        // reading leaves the row in place with no data ("--", empty gauge),
        // like any provider without data, so the widget is never empty.
        self.insert_provider(
            "antigravity.external_o",
            o_external,
            false,
            runtime.countdown,
        );
        // One external-models row per pool whose own toggle is on.
        let split = pool_o_on
            && (runtime.antigravity_external_enabled || runtime.antigravity_external_x_enabled);
        self.insert(
            "antigravity.external_a.shown",
            (split && runtime.antigravity_external_enabled) as u8 as f64,
        );
        self.insert(
            "antigravity.external_o.shown",
            (split && runtime.antigravity_external_x_enabled) as u8 as f64,
        );
    }

    pub fn insert_hover_crossfade(&mut self, prefix: &str, progress_to_hover: f64) -> (f64, f64) {
        let p = progress_to_hover.clamp(0.0, 1.0);
        self.insert(&format!("{prefix}.crossfade_opacity"), p);
        let (normal, hover) = asymmetric_crossfade_opacities(p);
        self.insert(&format!("{prefix}.normal.opacity"), normal);
        self.insert(&format!("{prefix}.hover.opacity"), hover);
        (normal, hover)
    }

    fn insert_provider(
        &mut self,
        name: &str,
        usage: Option<&crate::models::UsageData>,
        codex_compatibility: bool,
        countdown: bool,
    ) {
        // What a gauge or a badge should show. `percentage` stays the share
        // that has been spent so warning thresholds keep working, while
        // `display` follows the countdown setting.
        self.insert_limits(name, usage, countdown);
        let display = |percentage: f64| {
            if countdown {
                100.0 - percentage
            } else {
                percentage
            }
        };
        let weekly_label = usage
            .and_then(|usage| usage.weekly_label.as_deref())
            .or_else(|| self.get_string("i18n.weekly_window"))
            .unwrap_or("7d")
            .to_string();
        self.insert_string(&format!("{name}.weekly.label"), weekly_label);
        let session_label = usage
            .and_then(|usage| usage.session_label.as_deref())
            .or_else(|| self.get_string("i18n.session_window"))
            .unwrap_or("5h")
            .to_string();
        self.insert_string(&format!("{name}.session.name"), pool_name(&session_label));
        self.insert_string(&format!("{name}.session.label"), session_label);
        let (five_hour, weekly) = usage
            .map(|usage| (usage.session.percentage, usage.weekly.percentage))
            .unwrap_or((0.0, 0.0));
        // Before Codex windows were classified by duration, a weekly-only
        // response arrived in the primary slot and was exposed to themes as
        // `codex.session`. Keep that established binding working for existing
        // custom themes, while `codex.five_hour` always means the real window.
        let use_codex_session_fallback = codex_compatibility
            && usage.is_some_and(|usage| !usage.session.available && usage.weekly.available);
        let session = if use_codex_session_fallback {
            weekly
        } else {
            five_hour
        };
        let (session_available, weekly_available) = usage
            .map(|usage| {
                (
                    usage.session.available || use_codex_session_fallback,
                    usage.weekly.available,
                )
            })
            .unwrap_or((false, false));
        self.insert(
            &format!("{name}.session.available"),
            session_available as u8 as f64,
        );
        self.insert(
            &format!("{name}.five_hour.available"),
            usage.is_some_and(|usage| usage.session.available) as u8 as f64,
        );
        self.insert(
            &format!("{name}.weekly.available"),
            weekly_available as u8 as f64,
        );
        self.insert(&format!("{name}.session.percentage"), session);
        self.insert(&format!("{name}.session.remaining"), 100.0 - session);
        // An Antigravity pool window that was not reported draws an empty
        // gauge in either usage direction instead of a full "remaining" one.
        let shown_session = if name.starts_with("antigravity") && !session_available {
            0.0
        } else {
            display(session)
        };
        self.insert(&format!("{name}.session.display"), shown_session);
        self.insert(&format!("{name}.five_hour.percentage"), five_hour);
        self.insert(&format!("{name}.five_hour.remaining"), 100.0 - five_hour);
        // An Antigravity pool window that was not reported draws an empty
        // gauge in either usage direction instead of a full "remaining" one.
        let shown_five_hour = if name.starts_with("antigravity")
            && !usage.is_some_and(|usage| usage.session.available)
        {
            0.0
        } else {
            display(five_hour)
        };
        self.insert(&format!("{name}.five_hour.display"), shown_five_hour);
        self.insert(&format!("{name}.weekly.percentage"), weekly);
        self.insert(&format!("{name}.weekly.remaining"), 100.0 - weekly);
        // An Antigravity pool window that was not reported draws an empty
        // gauge in either usage direction instead of a full "remaining" one.
        let shown_weekly = if name.starts_with("antigravity") && !weekly_available {
            0.0
        } else {
            display(weekly)
        };
        self.insert(&format!("{name}.weekly.display"), shown_weekly);
        // Pace: the share a window would have spent by now at an even rate.
        // `expected_display` follows the countdown setting like `display`.
        // Providers with a non-standard session slot (Qoder credit packs)
        // give its start; everyone else uses the 5-hour window.
        let session_length = usage
            .and_then(|usage| usage.session_started_at.zip(usage.session.resets_at))
            .and_then(|(start, end)| end.duration_since(start).ok())
            .filter(|length| !length.is_zero())
            .unwrap_or(FIVE_HOURS);
        let five_hour_section = usage
            .filter(|usage| usage.session.available)
            .map(|usage| &usage.session);
        let weekly_section = usage
            .filter(|usage| usage.weekly.available)
            .map(|usage| &usage.weekly);
        let session_section = if use_codex_session_fallback {
            weekly_section.map(|section| (section, WEEK))
        } else {
            five_hour_section.map(|section| (section, session_length))
        };
        for (window, section) in [
            ("session", session_section),
            (
                "five_hour",
                five_hour_section.map(|section| (section, FIVE_HOURS)),
            ),
            ("weekly", weekly_section.map(|section| (section, WEEK))),
        ] {
            let expected =
                section.and_then(|(section, length)| expected_used(section.resets_at?, length));
            self.insert(
                &format!("{name}.{window}.expected_available"),
                expected.is_some() as u8 as f64,
            );
            let expected = expected.unwrap_or(0.0);
            self.insert(&format!("{name}.{window}.expected"), expected);
            self.insert(
                &format!("{name}.{window}.expected_display"),
                display(expected),
            );
        }
        // The true 7-day window (OpenCode Go). `weekly` above may instead
        // hold the more constrained 30-day window, so its pace bindings must
        // come from `week` directly rather than from the auto-selected slot.
        let week = usage.and_then(|usage| usage.week.as_ref());
        let week_percentage = week.map(|week| week.percentage).unwrap_or(0.0);
        let week_label = usage
            .and_then(|usage| usage.week_label.as_deref())
            .unwrap_or("7d")
            .to_string();
        self.insert_string(&format!("{name}.week.name"), pool_name(&week_label));
        self.insert_string(&format!("{name}.week.label"), week_label);
        let week_length = usage
            .and_then(|usage| {
                usage
                    .week_started_at
                    .zip(week.and_then(|week| week.resets_at))
            })
            .and_then(|(start, end)| end.duration_since(start).ok())
            .filter(|length| !length.is_zero())
            .unwrap_or(WEEK);
        self.insert(
            &format!("{name}.week.available"),
            week.is_some() as u8 as f64,
        );
        self.insert(&format!("{name}.week.percentage"), week_percentage);
        self.insert(&format!("{name}.week.remaining"), 100.0 - week_percentage);
        let shown_week = if name == "opencode" && week.is_none() {
            0.0
        } else {
            display(week_percentage)
        };
        self.insert(&format!("{name}.week.display"), shown_week);
        let week_expected = week
            .filter(|week| week.available)
            .and_then(|week| expected_used(week.resets_at?, week_length));
        self.insert(
            &format!("{name}.week.expected_available"),
            week_expected.is_some() as u8 as f64,
        );
        let week_expected = week_expected.unwrap_or(0.0);
        self.insert(&format!("{name}.week.expected"), week_expected);
        self.insert(
            &format!("{name}.week.expected_display"),
            display(week_expected),
        );
        let monthly = usage.and_then(|usage| usage.monthly.as_ref());
        let monthly_percentage = monthly.map(|m| m.percentage).unwrap_or(0.0);
        self.insert_string(&format!("{name}.monthly.label"), "30d");
        self.insert(&format!("{name}.monthly.percentage"), monthly_percentage);
        self.insert(
            &format!("{name}.monthly.remaining"),
            100.0 - monthly_percentage,
        );
        let shown_monthly = if name == "opencode" && monthly.is_none() {
            0.0
        } else {
            display(monthly_percentage)
        };
        self.insert(&format!("{name}.monthly.display"), shown_monthly);
        self.insert(
            &format!("{name}.monthly.available"),
            monthly.is_some() as u8 as f64,
        );
        // The 30-day window's real length is the billing period. Cached
        // readings without `startsAt` fall back to a fixed 30-day window.
        let monthly_length = usage
            .and_then(|usage| usage.monthly_started_at)
            .zip(monthly.and_then(|monthly| monthly.resets_at))
            .and_then(|(start, end)| end.duration_since(start).ok())
            .filter(|length| !length.is_zero())
            .unwrap_or(MONTH);
        let monthly_expected = monthly
            .filter(|monthly| monthly.available)
            .and_then(|monthly| expected_used(monthly.resets_at?, monthly_length));
        self.insert(
            &format!("{name}.monthly.expected_available"),
            monthly_expected.is_some() as u8 as f64,
        );
        let monthly_expected = monthly_expected.unwrap_or(0.0);
        self.insert(&format!("{name}.monthly.expected"), monthly_expected);
        self.insert(
            &format!("{name}.monthly.expected_display"),
            display(monthly_expected),
        );
        self.insert(&format!("{name}.available"), usage.is_some() as u8 as f64);
        // Carried over from an earlier poll: real figures, not current ones.
        self.insert(
            &format!("{name}.stale"),
            usage.is_some_and(|usage| usage.stale) as u8 as f64,
        );
        // Credits are absent for most accounts, so `credits.available` is what
        // a theme should gate the overlay on rather than `available`.
        let credits = usage.and_then(|usage| usage.credits.as_ref());
        let credits_percentage = credits.map(|credits| credits.percentage).unwrap_or(0.0);
        self.insert(&format!("{name}.credits.percentage"), credits_percentage);
        self.insert(
            &format!("{name}.credits.remaining"),
            100.0 - credits_percentage,
        );
        self.insert(
            &format!("{name}.credits.display"),
            display(credits_percentage),
        );
        // Currency, unlike the percentages either side of it.
        self.insert(
            &format!("{name}.credits.balance"),
            credits.map(|credits| credits.remaining).unwrap_or(0.0),
        );
        self.insert(
            &format!("{name}.credits.total"),
            credits.map(|credits| credits.total).unwrap_or(0.0),
        );
        self.insert(
            &format!("{name}.credits.available"),
            credits.is_some() as u8 as f64,
        );
        // The single figure a badge should show: whatever is closest to its
        // limit. A provider can switch a window off entirely -- Codex has its
        // five-hour window disabled -- so binding a badge to one window alone
        // leaves it reporting 0% while another allowance is spent.
        let headline = match credits {
            Some(credits) => credits.percentage,
            None => five_hour.max(weekly),
        };
        self.insert(&format!("{name}.headline.percentage"), headline);
        self.insert(&format!("{name}.headline.remaining"), 100.0 - headline);
        self.insert(&format!("{name}.headline.display"), display(headline));
        let reset_value = |reset: Option<std::time::SystemTime>| {
            let unix = reset
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs_f64())
                .unwrap_or(0.0);
            let seconds = reset
                .and_then(|value| value.duration_since(std::time::SystemTime::now()).ok())
                .map(|value| value.as_secs_f64())
                .unwrap_or(0.0);
            (unix, seconds)
        };
        let (five_hour_unix, five_hour_seconds) =
            reset_value(usage.and_then(|value| value.session.resets_at));
        let (weekly_unix, weekly_seconds) =
            reset_value(usage.and_then(|value| value.weekly.resets_at));
        let (session_unix, session_seconds) = if use_codex_session_fallback {
            (weekly_unix, weekly_seconds)
        } else {
            (five_hour_unix, five_hour_seconds)
        };
        let five_hour_available = usage.is_some_and(|value| value.session.available);
        let weekly_available = usage.is_some_and(|value| value.weekly.available);
        let session_available = if use_codex_session_fallback {
            weekly_available
        } else {
            five_hour_available
        };
        // Presence is independent of percentage and reset times. Monthly
        // availability remains based on its explicitly optional section.
        for (window, available) in [
            ("session", session_available),
            ("five_hour", five_hour_available),
            ("weekly", weekly_available),
        ] {
            self.insert(
                &format!("{name}.{window}.available"),
                available as u8 as f64,
            );
        }
        let (monthly_unix, monthly_seconds) =
            reset_value(monthly.and_then(|value| value.resets_at));
        let (week_unix, week_seconds) = reset_value(week.and_then(|value| value.resets_at));
        for (window, unix, seconds) in [
            ("session", session_unix, session_seconds),
            ("five_hour", five_hour_unix, five_hour_seconds),
            ("weekly", weekly_unix, weekly_seconds),
            ("week", week_unix, week_seconds),
            ("monthly", monthly_unix, monthly_seconds),
        ] {
            self.insert(&format!("{name}.{window}.reset.unix"), unix);
            self.insert(&format!("{name}.{window}.reset.seconds"), seconds);
            self.insert(&format!("{name}.{window}.reset.minutes"), seconds / 60.0);
            self.insert(&format!("{name}.{window}.reset.hours"), seconds / 3600.0);
            self.insert(&format!("{name}.{window}.reset.days"), seconds / 86400.0);
        }
    }

    pub fn insert(&mut self, name: &str, value: f64) {
        let key = name.to_ascii_lowercase();
        if key.starts_with("parent.") || key.starts_with("this.") || key.starts_with("object.") {
            self.geometry_values.insert(key, value);
        } else {
            Arc::make_mut(&mut self.values).insert(key, value);
        }
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        if let Some(value) = self.geometry_values.get(name).copied() {
            return Some(value);
        }
        if let Some(value) = self.values.get(name).copied() {
            return Some(value);
        }
        if let Some(value) = self
            .display_metrics
            .as_ref()
            .and_then(|metrics| display_metric(metrics, name))
        {
            return Some(value);
        }
        let name = name.to_ascii_lowercase();
        self.geometry_values
            .get(&name)
            .copied()
            .or_else(|| self.values.get(&name).copied())
            .or_else(|| self.limit_default(&name))
            .or_else(|| {
                let key = Self::account_default_key(&name)?;
                let value = *Self::account_defaults().values.get(&key)?;
                Some(
                    if key.ends_with(".display")
                        && self.values.get("display.countdown").copied().unwrap_or(0.0) != 0.0
                    {
                        100.0 - value
                    } else {
                        value
                    },
                )
            })
    }

    pub fn insert_string(&mut self, name: &str, value: impl Into<String>) {
        Arc::make_mut(&mut self.strings).insert(name.to_ascii_lowercase(), value.into());
    }

    pub fn get_string(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.strings
            .get(&name)
            .map(String::as_str)
            .or_else(|| Self::limit_string_default(&name))
            .or_else(|| {
                let key = Self::account_default_key(&name)?;
                Self::account_defaults()
                    .strings
                    .get(&key)
                    .map(String::as_str)
            })
    }

    // Account IDs are dynamic, but their fields use the same schema as provider
    // bindings. Supply typed defaults during validation, startup and removal;
    // unknown fields must still report typos instead of silently becoming zero.
    fn account_default_key(name: &str) -> Option<String> {
        let (provider, rest) = name.strip_prefix("accounts.")?.split_once('.')?;
        let (id, field) = rest.split_once('.')?;
        (matches!(provider, "claude" | "codex")
            && !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
        .then(|| format!("account.{field}"))
    }

    fn account_defaults() -> &'static Self {
        static DEFAULTS: OnceLock<DataContext> = OnceLock::new();
        DEFAULTS.get_or_init(|| {
            let mut context = Self::default();
            context.insert_provider("account", None, false, false);
            context.insert_string("account.name", "");
            context.insert("account.selected", 0.0);
            context.insert("account.has_error", 0.0);
            context
        })
    }

    pub fn with_object(mut self, object: &ResolvedObject<'_>) -> Self {
        self.insert("object.x", object.x);
        self.insert("object.y", object.y);
        self.insert("object.width", object.width);
        self.insert("object.height", object.height);
        self.insert("this.x", object.x);
        self.insert("this.y", object.y);
        self.insert("this.width", object.width);
        self.insert("this.height", object.height);
        self.insert("this.gap", object.gap);
        self.insert("parent.width", object.parent_width);
        self.insert("parent.height", object.parent_height);
        self
    }
}

fn action_target_id<'a>(target: &'a MouseActionTarget, self_id: &'a str) -> &'a str {
    match target {
        MouseActionTarget::SelfObject => self_id,
        MouseActionTarget::Object(id) => id,
    }
}

fn surface_object_by_id<'a>(surface: &'a SceneObject, id: &str) -> Option<&'a SceneObject> {
    std::iter::once(surface)
        .chain(surface.children.iter())
        .find(|object| object.id.eq_ignore_ascii_case(id))
}

fn surface_object_by_id_mut<'a>(
    surface: &'a mut SceneObject,
    id: &str,
) -> Option<&'a mut SceneObject> {
    if surface.id.eq_ignore_ascii_case(id) {
        return Some(surface);
    }
    surface
        .children
        .iter_mut()
        .find(|object| object.id.eq_ignore_ascii_case(id))
}

fn mouse_action_target<'a>(
    theme: &'a ThemeDocument,
    source_surface_index: usize,
    self_id: &str,
    target: &MouseActionTarget,
) -> Result<(usize, &'a SceneObject), String> {
    let object_id = action_target_id(target, self_id);
    let source_surface = theme
        .surfaces
        .get(source_surface_index)
        .ok_or_else(|| format!("Surface {source_surface_index} does not exist"))?;
    if matches!(target, MouseActionTarget::SelfObject) {
        let object = surface_object_by_id(source_surface, object_id)
            .ok_or_else(|| format!("Layer '{object_id}' does not exist"))?;
        return Ok((source_surface_index, object));
    }
    if let Some(object) = surface_object_by_id(source_surface, object_id) {
        return Ok((source_surface_index, object));
    }
    let mut matches = theme
        .surfaces
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != source_surface_index)
        .filter_map(|(index, surface)| {
            surface_object_by_id(surface, object_id).map(|object| (index, object))
        });
    let Some(found) = matches.next() else {
        return Err(format!("Layer '{object_id}' does not exist in this theme"));
    };
    if matches.next().is_some() {
        return Err(format!(
            "Layer id '{object_id}' is ambiguous across multiple root surfaces"
        ));
    }
    Ok(found)
}

fn mouse_property_expression(object: &SceneObject, property: MouseActionProperty) -> &Expression {
    match property {
        MouseActionProperty::Render => &object.render,
        MouseActionProperty::Visibility => &object.visibility,
        MouseActionProperty::X => &object.x,
        MouseActionProperty::Y => &object.y,
        MouseActionProperty::Width => &object.width,
        MouseActionProperty::Height => &object.height,
        MouseActionProperty::Rotation => &object.rotation,
    }
}

fn set_mouse_property_expression(
    object: &mut SceneObject,
    property: MouseActionProperty,
    expression: Expression,
) {
    *match property {
        MouseActionProperty::Render => &mut object.render,
        MouseActionProperty::Visibility => &mut object.visibility,
        MouseActionProperty::X => &mut object.x,
        MouseActionProperty::Y => &mut object.y,
        MouseActionProperty::Width => &mut object.width,
        MouseActionProperty::Height => &mut object.height,
        MouseActionProperty::Rotation => &mut object.rotation,
    } = expression;
}

pub fn validate_mouse_action_script(
    source: &str,
    theme: &ThemeDocument,
    source_surface_index: usize,
    self_id: &str,
    context: &DataContext,
) -> Vec<String> {
    let actions = match parse_mouse_actions(source) {
        Ok(actions) => actions,
        Err(error) => return vec![error],
    };
    let mut errors = Vec::new();
    for action in actions {
        let (target, property, value) = match action {
            MouseAction::ShowDashboard
            | MouseAction::ToggleDashboard
            | MouseAction::OpenUrl { .. }
            | MouseAction::ShowContextMenu { .. } => continue,
            MouseAction::Set {
                target,
                property,
                value,
            } => (target, property, Some(value)),
            MouseAction::Increase {
                target,
                property,
                amount,
            }
            | MouseAction::Decrease {
                target,
                property,
                amount,
            } => (target, property, Some(amount)),
            MouseAction::Toggle { target, property } | MouseAction::Reset { target, property } => {
                (target, property, None)
            }
        };
        let (target_surface_index, target_object) =
            match mouse_action_target(theme, source_surface_index, self_id, &target) {
                Ok(target) => target,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
        let target_is_root = theme
            .surfaces
            .get(target_surface_index)
            .is_some_and(|surface| surface.id.eq_ignore_ascii_case(&target_object.id));
        if target_is_root && matches!(property, MouseActionProperty::X | MouseActionProperty::Y) {
            errors.push(format!(
                "{}.{} cannot be changed because root layers use screen placement",
                target_object.name,
                property.name()
            ));
        }
        if let Some(value) = value {
            if let Err(error) = evaluate(&value.0, context) {
                errors.push(format!(
                    "{}.{} value: {error}",
                    target_object.name,
                    property.name()
                ));
            }
        }
    }
    errors
}

pub fn mouse_event_script<'a>(
    theme: &'a ThemeDocument,
    surface_index: usize,
    object_id: &str,
    event: MouseEventKind,
) -> Option<&'a str> {
    let surface = theme.surfaces.get(surface_index)?;
    let object = surface_object_by_id(surface, object_id)?;
    object
        .mouse_events
        .as_ref()
        .map(|events| events.handler(event).trim())
        .filter(|handler| !handler.is_empty())
}

pub fn apply_mouse_action_overrides(
    theme: &ThemeDocument,
    overrides: &HashMap<MouseActionOverrideKey, Expression>,
) -> ThemeDocument {
    let mut effective = theme.clone();
    for (key, expression) in overrides {
        let Some(surface) = effective.surfaces.get_mut(key.surface_index) else {
            continue;
        };
        let is_root = surface.id.eq_ignore_ascii_case(&key.object_id);
        if is_root
            && matches!(
                key.property,
                MouseActionProperty::X | MouseActionProperty::Y
            )
        {
            continue;
        }
        if let Some(object) = surface_object_by_id_mut(surface, &key.object_id) {
            set_mouse_property_expression(object, key.property, expression.clone());
        }
    }
    effective.prepare_runtime();
    effective
}

fn mouse_action_object_context(
    theme: &ThemeDocument,
    surface_index: usize,
    object_id: &str,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
    cached: Option<&DisplayMetrics>,
) -> Result<DataContext, String> {
    let surface = theme
        .surfaces
        .get(surface_index)
        .ok_or_else(|| format!("Surface {surface_index} does not exist"))?;
    let metrics = cached.map_or_else(
        || display_metrics(theme, data, runtime),
        |metrics| metrics_for_phase(metrics.clone(), runtime),
    );
    let (width, height) =
        resolve_surface_content_size_with_metrics(theme, surface_index, data, runtime, &metrics);
    let canvas = Canvas {
        width,
        width_expression: Some(surface.width.clone()),
        height,
        height_expression: Some(surface.height.clone()),
        background: surface.background.canvas_paint(),
    };
    let mut context = DataContext::from_usage_with_runtime(data, &canvas, runtime);
    insert_display_metrics(&mut context, &metrics);
    if surface.id.eq_ignore_ascii_case(object_id) {
        return Ok(context);
    }
    let (resolved, _) = resolve_objects_for_with_metrics(
        surface,
        &canvas,
        &surface.children,
        data,
        runtime,
        &metrics,
    );
    Ok(resolved
        .iter()
        .find(|object| object.source.id.eq_ignore_ascii_case(object_id))
        .map_or(context.clone(), |object| context.with_object(object)))
}

struct MouseActionExecutionContext<'a> {
    theme: &'a ThemeDocument,
    surface_index: usize,
    self_id: &'a str,
    data: Option<&'a AppUsageData>,
    runtime: ThemeRuntime,
}

impl MouseActionExecutionContext<'_> {
    fn adjust_property(
        &self,
        target: &MouseActionTarget,
        property: MouseActionProperty,
        amount: &Expression,
        direction: f64,
        cached: Option<&DisplayMetrics>,
        overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
    ) -> Result<(), String> {
        let effective = apply_mouse_action_overrides(self.theme, overrides);
        let (target_surface_index, object) =
            mouse_action_target(&effective, self.surface_index, self.self_id, target)?;
        let surface = &effective.surfaces[target_surface_index];
        let object_id = object.id.clone();
        if object.id == surface.id
            && matches!(property, MouseActionProperty::X | MouseActionProperty::Y)
        {
            return Err(format!(
                "{}.{} cannot be changed because root layers use screen placement",
                object.name,
                property.name()
            ));
        }
        let context = mouse_action_object_context(
            &effective,
            target_surface_index,
            &object_id,
            self.data,
            self.runtime,
            cached,
        )?;
        let current = evaluate(&mouse_property_expression(object, property).0, &context)?;
        let amount = evaluate(&amount.0, &context)?;
        let adjusted = current + amount * direction;
        if !adjusted.is_finite() {
            return Err(format!(
                "{}.{} adjustment did not produce a finite value",
                object.name,
                property.name()
            ));
        }
        overrides.insert(
            MouseActionOverrideKey {
                surface_index: target_surface_index,
                object_id,
                property,
            },
            Expression(format_number(adjusted)),
        );
        Ok(())
    }
}

#[cfg(test)]
pub fn execute_mouse_actions(
    theme: &ThemeDocument,
    surface_index: usize,
    self_id: &str,
    source: &str,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
    overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
) -> Result<Vec<MouseActionEffect>, String> {
    execute_mouse_actions_with_metrics(
        theme,
        surface_index,
        self_id,
        source,
        data,
        runtime,
        overrides,
        None,
    )
}

pub(crate) fn execute_mouse_actions_with_metrics(
    theme: &ThemeDocument,
    surface_index: usize,
    self_id: &str,
    source: &str,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
    overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
    mut cached: Option<&DisplayMetrics>,
) -> Result<Vec<MouseActionEffect>, String> {
    let actions = parse_mouse_actions(source)?;
    let execution = MouseActionExecutionContext {
        theme,
        surface_index,
        self_id,
        data,
        runtime,
    };
    let mut effects = Vec::new();
    for action in actions {
        match action {
            MouseAction::ShowDashboard => effects.push(MouseActionEffect::ShowDashboard),
            MouseAction::ToggleDashboard => effects.push(MouseActionEffect::ToggleDashboard),
            MouseAction::OpenUrl { url } => effects.push(MouseActionEffect::OpenUrl(url)),
            MouseAction::ShowContextMenu { menu } => {
                effects.push(MouseActionEffect::ShowContextMenu(menu))
            }
            MouseAction::Set {
                target,
                property,
                value,
            } => {
                let (target_surface_index, object) =
                    mouse_action_target(theme, surface_index, self_id, &target)?;
                let surface = &theme.surfaces[target_surface_index];
                let object_id = object.id.clone();
                if object.id == surface.id
                    && matches!(property, MouseActionProperty::X | MouseActionProperty::Y)
                {
                    return Err(format!(
                        "{}.{} cannot be changed because root layers use screen placement",
                        object.name,
                        property.name()
                    ));
                }
                overrides.insert(
                    MouseActionOverrideKey {
                        surface_index: target_surface_index,
                        object_id,
                        property,
                    },
                    value,
                );
                cached = None;
            }
            MouseAction::Reset { target, property } => {
                let (target_surface_index, object) =
                    mouse_action_target(theme, surface_index, self_id, &target)?;
                overrides.remove(&MouseActionOverrideKey {
                    surface_index: target_surface_index,
                    object_id: object.id.clone(),
                    property,
                });
                cached = None;
            }
            MouseAction::Increase {
                target,
                property,
                amount,
            } => {
                execution.adjust_property(&target, property, &amount, 1.0, cached, overrides)?;
                cached = None;
            }
            MouseAction::Decrease {
                target,
                property,
                amount,
            } => {
                execution.adjust_property(&target, property, &amount, -1.0, cached, overrides)?;
                cached = None;
            }
            MouseAction::Toggle { target, property } => {
                let effective = apply_mouse_action_overrides(theme, overrides);
                let (target_surface_index, object) =
                    mouse_action_target(&effective, surface_index, self_id, &target)?;
                let surface = &effective.surfaces[target_surface_index];
                let object_id = object.id.clone();
                let metrics = cached.map_or_else(
                    || display_metrics(&effective, data, runtime),
                    |metrics| metrics_for_phase(metrics.clone(), runtime),
                );
                let (width, height) = resolve_surface_content_size_with_metrics(
                    &effective,
                    target_surface_index,
                    data,
                    runtime,
                    &metrics,
                );
                let canvas = Canvas {
                    width,
                    width_expression: Some(surface.width.clone()),
                    height,
                    height_expression: Some(surface.height.clone()),
                    background: surface.background.canvas_paint(),
                };
                let mut context = DataContext::from_usage_with_runtime(data, &canvas, runtime);
                insert_display_metrics(&mut context, &metrics);
                let current = evaluate(&mouse_property_expression(object, property).0, &context)
                    .unwrap_or(0.0);
                overrides.insert(
                    MouseActionOverrideKey {
                        surface_index: target_surface_index,
                        object_id,
                        property,
                    },
                    Expression(if current == 0.0 { "1" } else { "0" }.into()),
                );
                cached = None;
            }
        }
    }
    Ok(effects)
}

impl ThemeDocument {
    /// Splitting is an all-provider capability: every required widget must
    /// exist on the primary surface before any saved split choice takes effect.
    pub fn supports_pool_splitting(&self) -> bool {
        self.surfaces.first().is_some_and(|surface| {
            [
                "claude_x-provider",
                "codex_x-provider",
                "antigravity_o-provider",
                "agy_external_o-provider",
            ]
            .iter()
            .all(|id| {
                surface.children.iter().any(|object| {
                    object.id == *id && object.parent.as_deref() == Some("provider-row")
                })
            })
        })
    }
    /// How often a live clock can visibly change. Minute-only clocks avoid the
    /// much more expensive second-by-second surface rendering path.
    pub fn current_time_refresh_interval(&self) -> Option<Duration> {
        current_time_refresh_interval_for(self)
    }

    pub fn surface_current_time_refresh_interval(&self, surface_index: usize) -> Option<Duration> {
        current_time_refresh_interval_for(self.surfaces.get(surface_index)?)
    }

    pub fn is_builtin(&self) -> bool {
        is_builtin_theme_id(&self.id)
    }

    /// Identifies the short-lived first Studio approximation so it can be
    /// upgraded without replacing themes users have renamed or repurposed.
    pub fn is_obsolete_studio_starter(&self) -> bool {
        (self.id == "classic-segments" && self.name == "Classic Segments")
            || (self.id == "midnight-glass" && self.name == "Midnight Glass")
    }

    pub fn starter() -> Self {
        let mut theme: Self = serde_json::from_str(BUILTIN_THEME_SOURCES[0].1)
            .expect("built-in Compact Fluent Quad theme must be valid JSON");
        theme.prepare_runtime();
        theme
    }

    /// Create the writable one-time upgrade target for pre-theme installs.
    /// Only the primary taskbar surface inherits legacy placement and
    /// visibility; notification-area roots remain registered with Explorer.
    pub fn migrated_from_legacy(placement: Option<(usize, i32)>, widget_visible: bool) -> Self {
        let mut theme = Self::starter();
        theme.id = "migrated-theme".into();
        theme.name = "Migrated Theme".into();
        if let Some(surface) = theme.surfaces.first_mut() {
            surface.render = (if widget_visible { 1.0 } else { 0.0 }).into();
            if let Some((display, offset_x)) = placement {
                surface.placement.reference.display = display;
                surface.placement.offset_x = offset_x;
                surface.placement.offset_x_expression = None;
            }
        }
        theme
    }

    pub fn prepare_runtime(&mut self) {
        // The empty-surface path supports documents assembled through the
        // runtime adapter fields before their root surface is materialized.
        if self.surfaces.is_empty() {
            let width = self
                .canvas
                .width_expression
                .clone()
                .unwrap_or_else(|| (self.canvas.width as f64).into());
            let height = self
                .canvas
                .height_expression
                .clone()
                .unwrap_or_else(|| (self.canvas.height as f64).into());
            let mut root = SceneObject::root(
                "main",
                "Main surface",
                width,
                height,
                self.placement.clone(),
            );
            root.background = LayerBackground::from_legacy_paint(self.canvas.background.clone());
            root.children = std::mem::take(&mut self.children);
            self.surfaces.push(root);
        }
        for surface in &mut self.surfaces {
            surface.placement.nest = surface
                .placement
                .nest
                .resolve(surface.placement.reference.region);
        }
        if let Some(surface) = self.surfaces.first() {
            let (width, height) =
                resolve_object_size(surface, None, ThemeRuntime::default(), &mut Vec::new());
            self.canvas = Canvas {
                width,
                width_expression: Some(surface.width.clone()),
                height,
                height_expression: Some(surface.height.clone()),
                background: surface.background.canvas_paint(),
            };
            self.placement = surface.placement.clone();
            self.children = surface.children.clone();
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema_version != THEME_SCHEMA_VERSION {
            errors.push(format!(
                "Theme schema {} is not supported; expected {}",
                self.schema_version, THEME_SCHEMA_VERSION
            ));
        }
        if self.surfaces.is_empty() {
            errors.push("A theme needs at least one root object".into());
            return errors;
        }
        let mut root_ids = std::collections::HashSet::new();
        for (surface_index, root) in self.surfaces.iter().enumerate() {
            if root.parent.is_some() {
                errors.push(format!(
                    "{}.parent: root objects cannot have a parent",
                    root.name
                ));
            }
            if root.id.trim().is_empty() || !root_ids.insert(root.id.to_ascii_lowercase()) {
                errors.push(format!("Root object '{}' needs a unique id", root.name));
            }
            if !root.anchor.is_default() || !root.x.is_zero() || !root.y.is_zero() {
                errors.push(format!(
                    "{}: root objects use screen placement rather than parent anchors or X/Y offsets",
                    root.name
                ));
            }
            let (width, height) =
                resolve_object_size(root, None, ThemeRuntime::default(), &mut errors);
            let canvas = Canvas {
                width,
                width_expression: Some(root.width.clone()),
                height,
                height_expression: Some(root.height.clone()),
                background: root.background.canvas_paint(),
            };
            let context = DataContext::from_usage(None, &canvas);
            validate_scene_object(&mut errors, &context, root);
            validate_scene_mouse_events(&mut errors, &context, self, surface_index, root);

            let mut ids = std::collections::HashSet::new();
            ids.insert(root.id.to_ascii_lowercase());
            for object in &root.children {
                if object.id.trim().is_empty() {
                    errors.push(format!("{}: every object needs an id", root.name));
                } else if !ids.insert(object.id.to_ascii_lowercase()) {
                    errors.push(format!("{}: duplicate object id: {}", root.name, object.id));
                }
                if !object.children.is_empty() {
                    errors.push(format!(
                        "{}.children: descendants must be stored in their root's ordered child list",
                        object.name
                    ));
                }
                if !object.placement.is_default() {
                    errors.push(format!(
                        "{}.placement: screen placement is only available on root objects",
                        object.name
                    ));
                }
                validate_scene_object(&mut errors, &context, object);
                validate_scene_mouse_events(&mut errors, &context, self, surface_index, object);
            }
            let by_id: HashMap<&str, &SceneObject> = std::iter::once(root)
                .chain(root.children.iter())
                .map(|object| (object.id.as_str(), object))
                .collect();
            for object in &root.children {
                let mut cursor = object.parent.as_deref().or(Some(root.id.as_str()));
                let mut visited = std::collections::HashSet::new();
                while let Some(parent_id) = cursor {
                    if !visited.insert(parent_id) {
                        errors.push(format!(
                            "{}.parent: hierarchy contains a cycle",
                            object.name
                        ));
                        break;
                    }
                    let Some(parent) = by_id.get(parent_id).copied() else {
                        errors.push(format!(
                            "{}.parent: '{}' does not exist in root '{}'",
                            object.name, parent_id, root.name
                        ));
                        break;
                    };
                    cursor = parent.parent.as_deref();
                }
            }
        }
        errors
    }
}

fn current_time_refresh_interval_for(value: &impl Serialize) -> Option<Duration> {
    let source = serde_json::to_string(value).ok()?.to_ascii_lowercase();
    let uses_clock =
        source.contains("time.now") || source.contains("time.local") || source.contains("time.utc");
    if !uses_clock {
        return None;
    }

    let needs_seconds = source.contains("time.now.milliseconds")
        || source.contains("time.local.second")
        || source.contains("time.utc.second")
        || unix_clock_requires_second_refresh(&source);
    Some(Duration::from_secs(if needs_seconds { 1 } else { 60 }))
}

fn unix_clock_requires_second_refresh(source: &str) -> bool {
    let mut remaining = source;
    while let Some(index) = remaining.find("time.now.unix") {
        let after_clock = &remaining[index + "time.now.unix".len()..];
        let Some(token_end) = after_clock.find('}') else {
            return true;
        };
        let token_tail = &after_clock[..token_end];
        let Some((_, format)) = token_tail.rsplit_once(':') else {
            return true;
        };
        if !is_minute_or_slower_clock_format(format) {
            return true;
        }
        remaining = &after_clock[token_end + 1..];
    }
    false
}

fn is_minute_or_slower_clock_format(format: &str) -> bool {
    let format = format.trim().strip_prefix("utc_").unwrap_or(format.trim());
    matches!(
        format,
        "weekday_2"
            | "weekday_short"
            | "weekday_long"
            | "day"
            | "day_2"
            | "month"
            | "month_2"
            | "month_short"
            | "month_long"
            | "year_2"
            | "year"
            | "date"
            | "date_short"
            | "date_long"
            | "time"
            | "time_short"
            | "time_24"
            | "time_12"
            | "am_pm"
            | "datetime_short"
            | "iso_date"
    )
}

fn validate_scene_mouse_events(
    errors: &mut Vec<String>,
    context: &DataContext,
    theme: &ThemeDocument,
    surface_index: usize,
    object: &SceneObject,
) {
    let Some(events) = &object.mouse_events else {
        return;
    };
    for (name, source) in [
        ("click", &events.click),
        ("double_click", &events.double_click),
        ("right_click", &events.right_click),
        ("mouse_enter", &events.mouse_enter),
        ("mouse_leave", &events.mouse_leave),
    ] {
        if source.trim().is_empty() {
            continue;
        }
        for error in validate_mouse_action_script(source, theme, surface_index, &object.id, context)
        {
            errors.push(format!("{}.mouse_events.{name}: {error}", object.name));
        }
    }
}

fn validate_scene_object(errors: &mut Vec<String>, context: &DataContext, object: &SceneObject) {
    let mut object_context = context.clone();
    let gap = evaluate(&object.gap.0, context)
        .ok()
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .max(0.0);
    object_context.insert("this.gap", gap);
    for (property, expression) in [
        ("render", &object.render),
        ("visibility", &object.visibility),
        ("x", &object.x),
        ("y", &object.y),
        ("width", &object.width),
        ("height", &object.height),
        ("rotation", &object.rotation),
        ("corner_radius", &object.corner_radius),
        ("gap", &object.gap),
    ] {
        validate_expression(
            errors,
            if property == "gap" {
                context
            } else {
                &object_context
            },
            &format!("{}.{}", object.name, property),
            expression,
        );
    }
    match &object.background {
        LayerBackground::None => {}
        LayerBackground::Colour { colour } => validate_paint(
            errors,
            &object_context,
            &format!("{}.background.colour", object.name),
            colour,
        ),
        LayerBackground::Gradient { start, end, angle } => {
            validate_paint(
                errors,
                &object_context,
                &format!("{}.background.gradient.start", object.name),
                start,
            );
            validate_paint(
                errors,
                &object_context,
                &format!("{}.background.gradient.end", object.name),
                end,
            );
            validate_expression(
                errors,
                &object_context,
                &format!("{}.background.gradient.angle", object.name),
                angle,
            );
        }
        LayerBackground::Image { path, .. } => {
            if path.trim().is_empty() {
                errors.push(format!("{}.background.image: choose an image", object.name));
            }
        }
    }
    if let Some(border) = &object.border {
        validate_paint(
            errors,
            &object_context,
            &format!("{}.border", object.name),
            &border.color,
        );
        validate_expression(
            errors,
            &object_context,
            &format!("{}.border.width", object.name),
            &border.width,
        );
    }
    match &object.content {
        SceneContent::None => {}
        SceneContent::Text {
            template,
            font_size,
            contrast,
            color,
            ..
        } => {
            validate_expression(
                errors,
                &object_context,
                &format!("{}.font_size", object.name),
                font_size,
            );
            validate_expression(
                errors,
                &object_context,
                &format!("{}.contrast", object.name),
                contrast,
            );
            validate_paint(
                errors,
                &object_context,
                &format!("{}.color", object.name),
                color,
            );
            for error in validate_template(template, &object_context) {
                errors.push(format!("{}.template: {error}", object.name));
            }
        }
        SceneContent::Progress {
            value,
            fill,
            track,
            corner_radius,
            segment_gap,
            segments_expression,
            ..
        } => {
            for (name, expression) in [
                ("value", value),
                ("content.corner_radius", corner_radius),
                ("segment_gap", segment_gap),
            ] {
                validate_expression(
                    errors,
                    &object_context,
                    &format!("{}.{}", object.name, name),
                    expression,
                );
            }
            if let Some(expression) = segments_expression {
                validate_expression(
                    errors,
                    &object_context,
                    &format!("{}.segments", object.name),
                    expression,
                );
            }
            validate_paint(
                errors,
                &object_context,
                &format!("{}.fill", object.name),
                fill,
            );
            validate_paint(
                errors,
                &object_context,
                &format!("{}.track", object.name),
                track,
            );
        }
    }
}

fn validate_expression(
    errors: &mut Vec<String>,
    context: &DataContext,
    label: &str,
    expression: &Expression,
) {
    if let Err(error) = evaluate(&expression.0, context) {
        errors.push(format!("{label}: {error}"));
    }
}

fn validate_paint(errors: &mut Vec<String>, context: &DataContext, label: &str, paint: &Paint) {
    if parse_color(&paint.color).is_none() {
        errors.push(format!(
            "{label}: '{}' is not #RRGGBB or #AARRGGBB",
            paint.color
        ));
    }
    validate_expression(errors, context, &format!("{label}.opacity"), &paint.opacity);
}

pub fn validate_template(template: &str, context: &DataContext) -> Vec<String> {
    let mut errors = Vec::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        remaining = &remaining[start + 1..];
        if remaining.starts_with('{') {
            remaining = &remaining[1..];
            continue;
        }
        let Some(end) = remaining.find('}') else {
            errors.push("missing closing brace".into());
            break;
        };
        let token = &remaining[..end];
        let (expression, format) = split_template_token(token);
        let expression = expression.trim();
        let format = format.trim();
        if context.get_string(expression).is_some()
            || (format.eq_ignore_ascii_case("usage_line")
                && format_usage_line(expression, context).is_some())
            || (format.eq_ignore_ascii_case("usage_badge")
                && format_usage_badge(expression, context).is_some())
        {
            remaining = &remaining[end + 1..];
            continue;
        }
        if let Err(error) = evaluate_value(expression, context) {
            errors.push(error);
        }
        remaining = &remaining[end + 1..];
    }
    errors
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: default_canvas_width(),
            width_expression: None,
            height: default_canvas_height(),
            height_expression: None,
            background: Paint::default(),
        }
    }
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            host_dimensions: None,
            clamp_taskbar_drag: false,
            reference: ReferenceTarget::default(),
            nest: SurfaceNest::Taskbar,
            horizontal: HorizontalAnchor::Left,
            vertical: VerticalAnchor::Center,
            surface_horizontal: Some(HorizontalAnchor::Right),
            surface_vertical: Some(VerticalAnchor::Center),
            offset_x: -12,
            offset_x_expression: None,
            offset_y: 0,
            offset_y_expression: None,
        }
    }
}

fn legacy_surface_nest() -> SurfaceNest {
    SurfaceNest::Auto
}

impl Paint {
    pub fn new(color: &str) -> Self {
        Self {
            color: color.to_string(),
            opacity: 1.0.into(),
        }
    }
    pub fn resolve(&self, context: &DataContext) -> Rgba {
        let mut rgba = parse_color(&self.color).unwrap_or(Rgba {
            r: 255,
            g: 0,
            b: 255,
            a: 255,
        });
        let opacity = evaluate(&self.opacity.0, context)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        rgba.a = ((rgba.a as f64) * opacity).round() as u8;
        rgba
    }
}

mod theme_storage;
pub use theme_storage::*;

/// Hit the topmost rendered provider container, including its empty space.
pub(crate) fn hit_test_widget_with_metrics(
    theme: &ThemeDocument,
    surface_index: usize,
    x: f64,
    y: f64,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
    metrics: &DisplayMetrics,
) -> Option<String> {
    let surface = theme.surfaces.get(surface_index)?;
    if !surface_should_render_with_metrics(theme, surface_index, data, runtime, metrics) {
        return None;
    }
    let (width, height) =
        resolve_surface_content_size_with_metrics(theme, surface_index, data, runtime, metrics);
    let canvas = Canvas {
        width,
        height,
        width_expression: Some(surface.width.clone()),
        height_expression: Some(surface.height.clone()),
        background: surface.background.canvas_paint(),
    };
    let (resolved, _) = theme_rendering::resolve_objects_for_with_metrics(
        surface,
        &canvas,
        &surface.children,
        data,
        runtime,
        metrics,
    );
    let point = (
        x - surface_horizontal_padding(theme, surface_index, runtime) as f64,
        y,
    );
    resolved
        .into_iter()
        .rev()
        .find(|object| {
            (object.source.id == ANTIGRAVITY_EXTERNAL_WIDGET
                || crate::app_settings::widget_mode_keys()
                    .any(|key| object.source.id == format!("{key}-provider")))
                && object.opacity > 0.0
                && object.width > 0.0
                && object.height > 0.0
                && theme_rendering::point_in_clip(
                    point,
                    ClipRegion {
                        x: object.x,
                        y: object.y,
                        width: object.width,
                        height: object.height,
                        rotation: object.rotation,
                    },
                )
                && object
                    .clip
                    .iter()
                    .all(|clip| theme_rendering::point_in_clip(point, *clip))
        })
        .map(|object| object.source.id.clone())
}

mod theme_rendering;
pub use theme_rendering::*;

mod theme_expression;
pub use theme_expression::*;

#[cfg(windows)]
mod text_measure;

mod theme_datetime;
mod theme_limits;
use theme_datetime::*;
fn schema_version() -> u32 {
    THEME_SCHEMA_VERSION
}
pub(crate) fn default_render() -> Expression {
    1.0.into()
}
fn default_visibility() -> Expression {
    100.0.into()
}
fn default_canvas_width() -> u32 {
    292
}
fn default_canvas_height() -> u32 {
    104
}
fn default_layer_width() -> Expression {
    100.0.into()
}
fn default_layer_height() -> Expression {
    32.0.into()
}
fn default_opacity() -> Expression {
    1.0.into()
}
fn default_font_size() -> Expression {
    13.0.into()
}
fn default_font_contrast() -> Expression {
    1.4.into()
}
fn default_stroke_width() -> Expression {
    1.0.into()
}
fn default_segment_gap() -> Expression {
    2.0.into()
}
fn default_progress_value() -> Expression {
    Expression("claude.session.percentage".into())
}
fn default_font_family() -> String {
    "Segoe UI Variable Text".into()
}
fn default_paint_color() -> String {
    "#00000000".into()
}
fn default_accent_paint() -> Paint {
    Paint::new("#D97757")
}
fn default_track_paint() -> Paint {
    Paint::new("#FFFFFF24")
}
fn default_text_paint() -> Paint {
    Paint::new("#FFFFFFFF")
}
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}
fn safe_file_stem(id: &str) -> String {
    let value: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if value.is_empty() {
        "custom-theme".into()
    } else {
        value
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    pub(crate) static METRICS_COMPUTATION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MEASUREMENT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod adaptive_metrics_tests {
    use super::*;

    #[test]
    fn placement_and_render_use_supplied_metrics() {
        let mut theme: ThemeDocument =
            serde_json::from_str(include_str!("themes/compact-fluent-quad.json")).unwrap();
        let runtime = ThemeRuntime::default();
        let mut metrics = DisplayMetrics::legacy(runtime);
        metrics.providers_width = 1234.0;
        theme.surfaces[0].placement.offset_x_expression =
            Some(Expression("display.providers_width".into()));
        theme.surfaces[0].render = Expression("display.providers_width == 1234".into());
        assert_eq!(
            resolve_surface_placement_with_metrics(&theme, 0, None, runtime, &metrics).offset_x,
            1234
        );
        assert!(surface_should_render_with_metrics(
            &theme, 0, None, runtime, &metrics
        ));
        metrics.providers_width = 1235.0;
        assert!(!surface_should_render_with_metrics(
            &theme, 0, None, runtime, &metrics
        ));
    }

    #[test]
    fn precheck_defers_adaptive_bindings() {
        let runtime = ThemeRuntime::default();
        let canvas = Canvas::default();
        for expression in ["display.providers_width > 0", "display.text_box.claude > 0"] {
            assert_eq!(
                DataContext::surface_render_without_size(
                    &Expression(expression.into()),
                    None,
                    &canvas,
                    runtime
                ),
                None
            );
        }
        assert_eq!(
            DataContext::surface_render_without_size(
                &Expression("false".into()),
                None,
                &canvas,
                runtime
            ),
            Some(false)
        );
    }

    #[test]
    fn render_and_hit_test_compute_metrics_once() {
        let theme: ThemeDocument =
            serde_json::from_str(include_str!("themes/compact-fluent-quad.json")).unwrap();
        let runtime = ThemeRuntime::from_providers(ProviderSet::from_enabled([ProviderId::Cursor]));
        METRICS_COMPUTATION_COUNT.with(|count| count.set(0));
        render_theme_surface_with_runtime(&theme, 0, None, runtime);
        assert_eq!(METRICS_COMPUTATION_COUNT.with(|count| count.get()), 1);
        METRICS_COMPUTATION_COUNT.with(|count| count.set(0));
        let metrics = display_metrics(&theme, None, runtime);
        surface_should_render_with_metrics(&theme, 0, None, runtime, &metrics);
        resolve_surface_size_with_metrics(&theme, 0, None, runtime, &metrics);
        resolve_surface_placement_with_metrics(&theme, 0, None, runtime, &metrics);
        object_bounds_with_metrics(&theme, 0, None, runtime, "cursor-provider", &metrics);
        render_theme_surface_with_metrics_at_scale(&theme, 0, None, runtime, 1.0, &metrics);
        assert_eq!(METRICS_COMPUTATION_COUNT.with(|count| count.get()), 1);
        METRICS_COMPUTATION_COUNT.with(|count| count.set(0));
        hit_test_mouse_event(&theme, 0, 1.0, 1.0, None, runtime);
        assert_eq!(METRICS_COMPUTATION_COUNT.with(|count| count.get()), 1);
    }

    #[test]
    fn fable_widget_count_adds_no_metrics_computation() {
        let theme: ThemeDocument =
            serde_json::from_str(include_str!("themes/compact-fluent-quad.json")).unwrap();
        let usage = AppUsageData::from_iter([(
            ProviderId::Claude,
            crate::models::UsageData {
                limits: vec![crate::models::UsageLimit {
                    key: "weekly_scoped_fable".into(),
                    kind: "weekly_scoped".into(),
                    label: "Fable".into(),
                    model: Some("Fable".into()),
                    model_id: None,
                    scope: None,
                    is_active: true,
                    usage: crate::models::UsageSection {
                        available: true,
                        percentage: 43.0,
                        resets_at: None,
                    },
                }],
                ..Default::default()
            },
        )]);
        for fable in [false, true] {
            let runtime =
                ThemeRuntime::from_providers(ProviderSet::from_enabled([ProviderId::Claude]))
                    .with_show_fable(fable);
            METRICS_COMPUTATION_COUNT.with(|count| count.set(0));
            render_theme_surface_with_runtime(&theme, 0, Some(&usage), runtime);
            assert_eq!(METRICS_COMPUTATION_COUNT.with(|count| count.get()), 1);
            METRICS_COMPUTATION_COUNT.with(|count| count.set(0));
            hit_test_mouse_event(&theme, 0, 1.0, 1.0, Some(&usage), runtime);
            assert_eq!(METRICS_COMPUTATION_COUNT.with(|count| count.get()), 1);
        }
    }

    #[test]
    fn animation_phase_reuses_measurements() {
        let theme: ThemeDocument =
            serde_json::from_str(include_str!("themes/compact-fluent-quad.json")).unwrap();
        let mut runtime =
            ThemeRuntime::from_providers(ProviderSet::from_enabled([ProviderId::Cursor]));
        runtime.language = LanguageId::English;
        render_theme_surface_with_runtime(&theme, 0, None, runtime);
        let before = MEASUREMENT_COUNT.with(|count| count.get());
        hit_test_mouse_event(&theme, 0, 1.0, 1.0, None, runtime);
        render_theme_surface_with_runtime(&theme, 0, None, runtime);
        assert_eq!(MEASUREMENT_COUNT.with(|count| count.get()), before);
        runtime.mode_phase = 0.5;
        display_metrics(&theme, None, runtime);
        assert_eq!(MEASUREMENT_COUNT.with(|count| count.get()), before);
    }

    #[test]
    fn hover_dependent_text_is_measured_for_both_states() {
        let mut theme: ThemeDocument =
            serde_json::from_str(include_str!("themes/compact-fluent-quad.json")).unwrap();
        let child = theme.surfaces[0]
            .children
            .iter_mut()
            .find(|object| {
                object.id.starts_with("claude-")
                    && matches!(object.content, SceneContent::Text { .. })
            })
            .unwrap();
        child.render = Expression("claude.hovered && codex.hovered".into());
        if let SceneContent::Text { template, .. } = &mut child.content {
            *template = "A very long hover dependent label {claude.hovered} {codex.hovered}".into();
        }
        let mut runtime =
            ThemeRuntime::from_providers(ProviderSet::from_enabled([ProviderId::Claude]));
        let normal = display_metrics(&theme, None, runtime)
            .provider("claude")
            .unwrap()
            .text_box;
        runtime.claude_hovered = true;
        runtime.codex_hovered = true;
        let hovered = display_metrics(&theme, None, runtime)
            .provider("claude")
            .unwrap()
            .text_box;
        assert_eq!(normal, hovered);
        assert!(normal > 100.0, "hover text was not measured: {normal}");
    }

    #[test]
    fn row_width_and_no_data_baselines() {
        let totals = |count: usize, origins: &[f64]| -> Vec<f64> {
            [0.0, 1.0, 2.0]
                .into_iter()
                .map(|phase| {
                    let p = decompose_mode_phase(phase);
                    compact_providers_width(
                        origins.iter().map(|origin| {
                            compact_row_width(compact_bar_full_width(count), 62.0, 30.0, *origin, p)
                        }),
                        count,
                    ) + 4.0
                })
                .collect()
        };
        assert_eq!(
            totals(4, &[10.0, 10.0, 0.0, 12.0]),
            vec![446.0, 182.0, 174.0]
        );
        assert_eq!(totals(4, &[0.0, 0.0, 0.0, 12.0]), vec![426.0, 162.0, 154.0]);
        assert_eq!(totals(2, &[0.0, 12.0]), vec![262.0, 130.0, 82.0]);
        let phase = decompose_mode_phase(0.5);
        assert_eq!(compact_row_width(32.0, 62.0, 30.0, 10.0, phase), 75.0);
    }

    #[test]
    fn cursor_loading_preserves_legacy_text_box() {
        let theme: ThemeDocument =
            serde_json::from_str(include_str!("themes/compact-fluent-quad.json")).unwrap();
        let mut runtime =
            ThemeRuntime::from_providers(ProviderSet::from_enabled([ProviderId::Cursor]));
        runtime.poll_ok = false;
        let context = DataContext::from_usage_with_runtime(None, &Canvas::default(), runtime);
        assert_eq!(context.get("data.loading"), Some(1.0));
        assert!(template_has_placeholder_usage(
            "int {cursor.api:usage_line}",
            &context
        ));
        assert_eq!(
            display_metrics(&theme, None, runtime)
                .provider("cursor")
                .unwrap()
                .text_box,
            80.0
        );
    }
}
