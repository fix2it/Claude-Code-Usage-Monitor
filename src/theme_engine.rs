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
    /// Indexed by [`SplitPool`].
    pub split_hover_phases: [f64; SplitPool::ALL.len()],
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
            split_hover_phases: [0.0; SplitPool::ALL.len()],
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
            split_hover_phases: [0.0; SplitPool::ALL.len()],
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

    /// "Pools on one widget" for Claude, Codex and agy.
    pub fn with_pool_layouts(mut self, claude: bool, codex: bool, agy: bool) -> Self {
        self.claude_pools_one_widget = claude;
        self.codex_pools_one_widget = codex;
        self.antigravity_pools_one_widget = agy;
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

    /// Hover phases of the split second-pool tiles, indexed by [`SplitPool`].
    pub fn with_split_hover_phases(mut self, phases: [f64; SplitPool::ALL.len()]) -> Self {
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

impl ThemeDocument {
    /// Splitting is an all-provider capability: every required widget must
    /// exist on the primary surface before any saved split choice takes effect.
    pub fn supports_pool_splitting(&self) -> bool {
        self.surfaces.first().is_some_and(|surface| {
            SplitPool::ALL.iter().all(|pool| {
                surface.children.iter().any(|object| {
                    object.id == pool.object_id()
                        && object.parent.as_deref() == Some("provider-row")
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

mod theme_mouse;
pub use theme_mouse::*;
mod theme_widgets;
pub use theme_widgets::*;
mod theme_metrics;
pub use theme_metrics::*;
mod theme_data_context;

mod theme_validation;
pub use theme_validation::*;

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
