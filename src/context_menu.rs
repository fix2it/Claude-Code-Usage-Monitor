use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::providers::ProviderId;
use crate::theme_engine::{self, DataContext, Expression};

const CHILD_INDENT: &str = "    ";

pub const CONTEXT_MENU_SCHEMA_VERSION: u32 = 1;
pub const CLASSIC_CONTEXT_MENU_ID: &str = "classic-v1";
pub const DASHBOARD_V2_CONTEXT_MENU_ID: &str = "dashboard-v2";
pub const LEGACY_CLASSIC_CONTEXT_MENU_ID: &str = "classic-v1-4-9";
pub const LEGACY_DASHBOARD_V2_CONTEXT_MENU_ID: &str = "dashboard-and-exit";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMenuDocument {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<ContextMenuItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMenuItem {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "theme_engine::default_render")]
    pub render: Expression,
    #[serde(flatten)]
    pub kind: ContextMenuItemKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextMenuItemKind {
    Action {
        action: ContextMenuAction,
    },
    /// A non-clickable informational row. Its label is evaluated as a theme
    /// text template whenever the menu opens.
    Text,
    Separator,
    Submenu {
        items: Vec<ContextMenuItem>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextMenuAction {
    OpenDashboard,
    Refresh,
    SetUpdateFrequency {
        #[serde(
            alias = "milliseconds",
            deserialize_with = "deserialize_update_frequency_seconds"
        )]
        seconds: u32,
    },
    ToggleProvider {
        provider: ContextMenuProvider,
    },
    ToggleAntigravityGemini,
    ToggleAntigravityExternal,
    /// First agy pool (Antigravity's own login), switchable on its own.
    ToggleAntigravityA,
    /// Second agy pool, read from OpenCode's login.
    ToggleAntigravityX,
    /// External models of the second (OpenCode) agy pool.
    ToggleAntigravityExternalX,
    ToggleClaudeMain,
    ToggleClaudeX,
    ToggleCodexMain,
    ToggleCodexX,
    ToggleStartup,
    ToggleTaskbarLock,
    ToggleShowPace,
    ToggleShowFable,
    ToggleShowFableX,
    ToggleClaudePoolsOneWidget,
    ToggleCodexPoolsOneWidget,
    ToggleAntigravityPoolsOneWidget,
    ToggleWidget,
    /// Accepted only so menus saved by older versions can be loaded and
    /// cleaned up. New menus cannot create or execute this legacy action.
    #[serde(rename = "reset_position")]
    LegacyResetPosition,
    SetLanguage {
        language: String,
    },
    CheckForUpdates,
    ToggleLayerRender {
        target: String,
    },
    LayerActions {
        actions: String,
    },
    OpenUrl {
        url: String,
    },
    Exit,
}

fn deserialize_update_frequency_seconds<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    Ok(match value {
        crate::app_settings::POLL_1_MIN
        | crate::app_settings::POLL_5_MIN
        | crate::app_settings::POLL_15_MIN
        | crate::app_settings::POLL_1_HOUR => value / 1_000,
        _ => value,
    })
}

pub type ContextMenuProvider = ProviderId;

#[derive(Clone, Debug)]
pub struct ContextMenuDescriptor {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub built_in: bool,
}

impl ContextMenuItem {
    /// Invalid conditions hide the item, including a submenu's entire subtree.
    pub fn should_render(&self, context: &DataContext) -> bool {
        theme_engine::evaluate(&self.render.0, context)
            .is_ok_and(|value| value.is_finite() && value != 0.0)
    }

    pub fn action(id: &str, label: &str, action: ContextMenuAction) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Action { action },
        }
    }

    pub fn separator(id: &str) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Separator,
        }
    }

    #[cfg(test)]
    pub fn text(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Text,
        }
    }

    pub fn submenu(id: &str, label: &str, items: Vec<Self>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Submenu { items },
        }
    }
}

impl ContextMenuDocument {
    #[cfg(test)]
    pub fn blank(name: &str) -> Self {
        let id = unique_document_id(name);
        Self {
            schema_version: CONTEXT_MENU_SCHEMA_VERSION,
            id,
            name: name.into(),
            items: vec![
                ContextMenuItem::action(
                    "open-dashboard",
                    "Open Dashboard",
                    ContextMenuAction::OpenDashboard,
                ),
                ContextMenuItem::separator("separator-1"),
                ContextMenuItem::action("exit", "Exit", ContextMenuAction::Exit),
            ],
        }
    }

    pub fn is_builtin(&self) -> bool {
        self.id.eq_ignore_ascii_case(CLASSIC_CONTEXT_MENU_ID)
            || self.id.eq_ignore_ascii_case(DASHBOARD_V2_CONTEXT_MENU_ID)
            || self.id.eq_ignore_ascii_case(LEGACY_CLASSIC_CONTEXT_MENU_ID)
            || self
                .id
                .eq_ignore_ascii_case(LEGACY_DASHBOARD_V2_CONTEXT_MENU_ID)
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema_version != CONTEXT_MENU_SCHEMA_VERSION {
            errors.push(format!(
                "Context menu schema {} is not supported; expected {}",
                self.schema_version, CONTEXT_MENU_SCHEMA_VERSION
            ));
        }
        if self.id.trim().is_empty() {
            errors.push("Context menu id cannot be empty".into());
        }
        if self.name.trim().is_empty() {
            errors.push("Context menu name cannot be empty".into());
        }
        if self.items.is_empty() {
            errors.push("A context menu needs at least one item".into());
        }
        let mut ids = std::collections::HashSet::new();
        validate_items(&self.items, 0, &mut ids, &mut errors);
        errors
    }
}

fn validate_items(
    items: &[ContextMenuItem],
    depth: usize,
    ids: &mut std::collections::HashSet<String>,
    errors: &mut Vec<String>,
) {
    if depth > 5 {
        errors.push("Context menus support at most six nested levels".into());
        return;
    }
    let context = DataContext::from_usage(None, &theme_engine::Canvas::default());
    for item in items {
        if item.id.trim().is_empty() || !ids.insert(item.id.to_ascii_lowercase()) {
            errors.push(format!("Menu item '{}' needs a unique id", item.label));
        }
        match theme_engine::evaluate(&item.render.0, &context) {
            Ok(value) if value.is_finite() => {}
            Ok(_) => errors.push(format!("{}.render did not produce a finite value", item.id)),
            Err(error) => errors.push(format!("{}.render: {error}", item.id)),
        }
        match &item.kind {
            ContextMenuItemKind::Separator => {}
            ContextMenuItemKind::Text => validate_item_label(item, "Menu text", errors),
            ContextMenuItemKind::Submenu { items } => {
                validate_item_label(item, "Submenu", errors);
                if items.is_empty() {
                    errors.push(format!("Submenu '{}' needs at least one item", item.label));
                }
                validate_items(items, depth + 1, ids, errors);
            }
            ContextMenuItemKind::Action { action } => {
                validate_item_label(item, "Menu action", errors);
                match action {
                    ContextMenuAction::SetUpdateFrequency { seconds }
                        if !matches!(
                            *seconds,
                            crate::app_settings::POLL_1_MIN_SECONDS
                                | crate::app_settings::POLL_5_MIN_SECONDS
                                | crate::app_settings::POLL_15_MIN_SECONDS
                                | crate::app_settings::POLL_1_HOUR_SECONDS
                        ) =>
                    {
                        errors.push(format!(
                            "{}.seconds must be a supported update frequency",
                            item.id
                        ));
                    }
                    ContextMenuAction::SetLanguage { language } if language.trim().is_empty() => {
                        errors.push(format!("{}.language cannot be empty", item.id));
                    }
                    ContextMenuAction::ToggleLayerRender { target } if target.trim().is_empty() => {
                        errors.push(format!("{}.target cannot be empty", item.id));
                    }
                    ContextMenuAction::LayerActions { actions } => {
                        if actions.trim().is_empty() {
                            errors.push(format!("{}.actions cannot be empty", item.id));
                        } else if let Err(error) = crate::theme_engine::parse_mouse_actions(actions)
                        {
                            errors.push(format!("{}.actions: {error}", item.id));
                        }
                    }
                    ContextMenuAction::OpenUrl { url } if !supported_url(url) => errors.push(
                        format!("{}.url must start with http:// or https://", item.id),
                    ),
                    _ => {}
                }
            }
        }
    }
}

fn validate_item_label(item: &ContextMenuItem, kind: &str, errors: &mut Vec<String>) {
    if item.label.trim().is_empty() {
        errors.push(format!("{kind} '{}' needs a label", item.id));
        return;
    }
    let context =
        crate::theme_engine::DataContext::from_usage(None, &crate::theme_engine::Canvas::default());
    for error in crate::theme_engine::validate_template(&item.label, &context) {
        errors.push(format!("{}.label: {error}", item.id));
    }
}

pub fn supported_url(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    url.starts_with("https://") || url.starts_with("http://")
}

pub fn rendered_label(
    language: crate::localization::LanguageId,
    label: &str,
    context: &crate::theme_engine::DataContext,
) -> String {
    let (indent, label) = match label.strip_prefix(CHILD_INDENT) {
        Some(label) => (CHILD_INDENT, label),
        None => ("", label),
    };
    let translated = match label {
        "Settings" => language.text("Settings"),
        "Update frequency" => language.text("Update frequency"),
        "Start with Windows" => language.text("Start with Windows"),
        "Language" => language.text("Language"),
        "System default" => language.text("System default"),
        "Refresh" => language.text("Refresh"),
        "Exit" => language.text("Exit"),
        "Claude Code" => language.text("Claude Code"),
        "Claude Code (Main)" => language.text("Claude Code (Main)"),
        "Claude Code X" => language.text("Claude Code X"),
        "Codex" => language.text("Codex"),
        "Codex (Main)" => language.text("Codex (Main)"),
        "Codex X" => language.text("Codex X"),
        "Antigravity" => language.text("Antigravity"),
        "OpenCode" => language.text("OpenCode"),
        "Cursor" => language.text("Cursor"),
        "Grok" => language.text("Grok"),
        "Copilot" => language.text("Copilot"),
        "Qoder" => language.text("Qoder"),
        "Open Dashboard" => language.text("Open Dashboard"),
        "Every minute" => language.text("Every minute"),
        "Every 5 minutes" => language.text("Every 5 minutes"),
        "Every 15 minutes" => language.text("Every 15 minutes"),
        "Every hour" => language.text("Every hour"),
        "Providers" => language.text("Providers"),
        "Check for updates" => language.text("Check for updates"),
        "Lock in taskbar" => language.text("Lock in taskbar"),
        "Show widget" => language.text("Show widget"),
        "Show pace" => language.text("Show pace"),
        "Fable" => language.text("Fable"),
        "External models" => language.text("External models"),
        "Antigravity (Main)" => language.text("Antigravity (Main)"),
        "Antigravity (OpenCode)" => language.text("Antigravity (OpenCode)"),
        "Pools on one widget" => language.text("Pools on one widget"),
        _ => label,
    };
    format!(
        "{indent}{}",
        crate::theme_engine::format_template(translated, context)
    )
}

pub fn classic_context_menu() -> ContextMenuDocument {
    use ContextMenuAction as Action;

    let frequency = classic_frequency_menu();
    let providers = classic_providers_menu();
    let settings = classic_settings_menu();

    ContextMenuDocument {
        schema_version: CONTEXT_MENU_SCHEMA_VERSION,
        id: CLASSIC_CONTEXT_MENU_ID.into(),
        name: "Classic v1".into(),
        items: vec![
            ContextMenuItem::action("refresh", "Refresh", Action::Refresh),
            frequency,
            providers,
            settings,
            ContextMenuItem::action("toggle-widget", "Show widget", Action::ToggleWidget),
            ContextMenuItem::action("open-dashboard", "Open Dashboard", Action::OpenDashboard),
            ContextMenuItem::separator("root-separator"),
            ContextMenuItem::action("exit", "Exit", Action::Exit),
        ],
    }
}

fn classic_frequency_menu() -> ContextMenuItem {
    use ContextMenuAction as Action;

    ContextMenuItem::submenu(
        "update-frequency",
        "Update frequency",
        vec![
            ContextMenuItem::action(
                "frequency-1-minute",
                "Every minute",
                Action::SetUpdateFrequency {
                    seconds: crate::app_settings::POLL_1_MIN_SECONDS,
                },
            ),
            ContextMenuItem::action(
                "frequency-5-minutes",
                "Every 5 minutes",
                Action::SetUpdateFrequency {
                    seconds: crate::app_settings::POLL_5_MIN_SECONDS,
                },
            ),
            ContextMenuItem::action(
                "frequency-15-minutes",
                "Every 15 minutes",
                Action::SetUpdateFrequency {
                    seconds: crate::app_settings::POLL_15_MIN_SECONDS,
                },
            ),
            ContextMenuItem::action(
                "frequency-1-hour",
                "Every hour",
                Action::SetUpdateFrequency {
                    seconds: crate::app_settings::POLL_1_HOUR_SECONDS,
                },
            ),
        ],
    )
}

fn classic_providers_menu() -> ContextMenuItem {
    use ContextMenuAction as Action;
    use ContextMenuProvider as Provider;

    let child = |id, label, action, render: &str| {
        let mut item = ContextMenuItem::action(id, &format!("{CHILD_INDENT}{label}"), action);
        item.render = Expression(render.into());
        item
    };
    // Shown while both pools of `provider` are on and the theme can split them.
    let pools_one_widget = |id, action, provider: &str, first: &str, second: &str| {
        child(
            id,
            "Pools on one widget",
            action,
            &format!(
                "display.pool_split_supported && providers.{provider}.enabled && {provider}.{first}.enabled && {provider}.{second}.enabled"
            ),
        )
    };
    ContextMenuItem::submenu(
        "providers",
        "Providers",
        vec![
            ContextMenuItem::action(
                "provider-claude-main",
                "Claude Code (Main)",
                Action::ToggleClaudeMain,
            ),
            child(
                "show-fable",
                "Fable",
                Action::ToggleShowFable,
                "providers.claude.enabled && claude.main.enabled",
            ),
            ContextMenuItem::action("provider-claude-x", "Claude Code X", Action::ToggleClaudeX),
            child(
                "show-fable-x",
                "Fable",
                Action::ToggleShowFableX,
                "providers.claude.enabled && claude.x.enabled",
            ),
            pools_one_widget(
                "claude-pools-one-widget",
                Action::ToggleClaudePoolsOneWidget,
                "claude",
                "main",
                "x",
            ),
            ContextMenuItem::action(
                "provider-codex-main",
                "Codex (Main)",
                Action::ToggleCodexMain,
            ),
            ContextMenuItem::action("provider-codex-x", "Codex X", Action::ToggleCodexX),
            pools_one_widget(
                "codex-pools-one-widget",
                Action::ToggleCodexPoolsOneWidget,
                "codex",
                "main",
                "x",
            ),
            ContextMenuItem::action(
                "provider-antigravity-a",
                "Antigravity (Main)",
                Action::ToggleAntigravityA,
            ),
            child(
                "provider-antigravity-external",
                "External models",
                Action::ToggleAntigravityExternal,
                "providers.antigravity.enabled && antigravity.a.enabled",
            ),
            ContextMenuItem::action(
                "provider-antigravity-x",
                "Antigravity (OpenCode)",
                Action::ToggleAntigravityX,
            ),
            child(
                "provider-antigravity-external-x",
                "External models",
                Action::ToggleAntigravityExternalX,
                "providers.antigravity.enabled && antigravity.x.enabled",
            ),
            pools_one_widget(
                "antigravity-pools-one-widget",
                Action::ToggleAntigravityPoolsOneWidget,
                "antigravity",
                "a",
                "x",
            ),
            ContextMenuItem::action(
                "provider-opencode",
                "OpenCode",
                Action::ToggleProvider {
                    provider: Provider::OpenCode,
                },
            ),
            ContextMenuItem::action(
                "provider-cursor",
                "Cursor",
                Action::ToggleProvider {
                    provider: Provider::Cursor,
                },
            ),
            ContextMenuItem::action(
                "provider-grok",
                "Grok",
                Action::ToggleProvider {
                    provider: Provider::Grok,
                },
            ),
            ContextMenuItem::action(
                "provider-copilot",
                "Copilot",
                Action::ToggleProvider {
                    provider: Provider::Copilot,
                },
            ),
            ContextMenuItem::action(
                "provider-qoder",
                "Qoder",
                Action::ToggleProvider {
                    provider: Provider::Qoder,
                },
            ),
        ],
    )
}

fn classic_settings_menu() -> ContextMenuItem {
    use ContextMenuAction as Action;

    let languages = std::iter::once(ContextMenuItem::action(
        "language-system",
        "System default",
        Action::SetLanguage {
            language: "system".into(),
        },
    ))
    .chain(
        crate::localization::LanguageId::ALL
            .into_iter()
            .map(|language| {
                ContextMenuItem::action(
                    &format!("language-{}", language.code().to_ascii_lowercase()),
                    language.native_name(),
                    Action::SetLanguage {
                        language: language.code().into(),
                    },
                )
            }),
    )
    .collect();
    ContextMenuItem::submenu(
        "settings",
        "Settings",
        vec![
            ContextMenuItem::action(
                "start-with-windows",
                "Start with Windows",
                Action::ToggleStartup,
            ),
            ContextMenuItem::action("lock-taskbar", "Lock in taskbar", Action::ToggleTaskbarLock),
            ContextMenuItem::action("show-pace", "Show pace", Action::ToggleShowPace),
            ContextMenuItem::submenu("language", "Language", languages),
            ContextMenuItem::separator("settings-separator"),
            ContextMenuItem::action(
                "check-updates",
                "Check for updates",
                Action::CheckForUpdates,
            ),
        ],
    )
}

pub fn dashboard_v2_context_menu() -> ContextMenuDocument {
    ContextMenuDocument {
        schema_version: CONTEXT_MENU_SCHEMA_VERSION,
        id: DASHBOARD_V2_CONTEXT_MENU_ID.into(),
        name: "Dashboard v2".into(),
        items: vec![
            ContextMenuItem::action(
                "open-dashboard",
                "Open Dashboard",
                ContextMenuAction::OpenDashboard,
            ),
            ContextMenuItem::separator("dashboard-separator"),
            ContextMenuItem::action("exit", "Exit", ContextMenuAction::Exit),
        ],
    }
}

pub fn context_menus_directory() -> PathBuf {
    crate::app_settings::app_data_directory().join("context-menus")
}

pub fn ensure_builtin_context_menus() -> Result<PathBuf, String> {
    let directory = context_menus_directory();
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut classic_path = None;
    for document in [classic_context_menu(), dashboard_v2_context_menu()] {
        let errors = document.validate();
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        let path = directory.join(format!("{}.json", document.id));
        let canonical = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
        if std::fs::read(&path).ok().as_deref() != Some(canonical.as_slice()) {
            crate::app_settings::write_json_atomic(&path, &document)?;
        }
        if document.id == CLASSIC_CONTEXT_MENU_ID {
            classic_path = Some(path);
        }
    }
    for legacy_id in [
        LEGACY_CLASSIC_CONTEXT_MENU_ID,
        LEGACY_DASHBOARD_V2_CONTEXT_MENU_ID,
    ] {
        let legacy_path = directory.join(format!("{legacy_id}.json"));
        match std::fs::remove_file(legacy_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    classic_path.ok_or_else(|| "The Classic context menu could not be created".into())
}

pub fn load_context_menu(path: &Path) -> Result<ContextMenuDocument, String> {
    let source = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document: ContextMenuDocument =
        serde_json::from_str(&source).map_err(|error| error.to_string())?;
    remove_legacy_context_menu_actions(&mut document.items);
    let errors = document.validate();
    if errors.is_empty() {
        Ok(document)
    } else {
        Err(errors.join("\n"))
    }
}

pub fn list_context_menus() -> Result<Vec<ContextMenuDescriptor>, String> {
    let built_in_path = ensure_builtin_context_menus()?;
    let mut menus = std::fs::read_dir(context_menus_directory())
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .filter_map(|path| {
            let menu = load_context_menu(&path).ok()?;
            Some(ContextMenuDescriptor {
                built_in: path == built_in_path || menu.is_builtin(),
                path,
                id: menu.id,
                name: menu.name,
            })
        })
        .collect::<Vec<_>>();
    menus.sort_by_key(|menu| (!menu.built_in, menu.name.to_ascii_lowercase()));
    Ok(menus)
}

pub fn resolve_context_menu(reference: Option<&str>) -> Result<ContextMenuDocument, String> {
    let reference = reference.unwrap_or(CLASSIC_CONTEXT_MENU_ID).trim();
    let reference = canonical_context_menu_reference(reference);
    let menus = list_context_menus()?;
    let mut matches = menus
        .iter()
        .filter(|menu| menu.id.eq_ignore_ascii_case(reference));
    if let Some(menu) = matches.next() {
        return load_context_menu(&menu.path);
    }
    let matches = menus
        .iter()
        .filter(|menu| menu.name.eq_ignore_ascii_case(reference))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [menu] => load_context_menu(&menu.path),
        [] => Err(format!("Context menu '{reference}' was not found")),
        _ => Err(format!(
            "Context menu name '{reference}' is ambiguous; use its id"
        )),
    }
}

/// Size and modification time of every file in the menus directory. A right
/// click compares this cheap listing instead of re-reading, re-parsing and
/// re-validating every menu file (F19). Any edit, addition or removal (also of
/// a built-in file, which `ensure_builtin_context_menus` would restore)
/// changes the fingerprint and forces a full resolve. Without a modification
/// time there is no fingerprint and every click reads the files as before.
/// Known limit: an edit that keeps both the size and the timestamp (a tool
/// that deliberately restores mtime) is not seen until another change.
type MenuDirectoryFingerprint = Vec<(PathBuf, u64, std::time::SystemTime)>;

fn menu_directory_fingerprint() -> Option<MenuDirectoryFingerprint> {
    let mut entries = std::fs::read_dir(context_menus_directory())
        .ok()?
        .map(|entry| {
            let path = entry.ok()?.path();
            // Follow symbolic links, as loading the menu does.
            let metadata = std::fs::metadata(&path).ok()?;
            Some((path, metadata.len(), metadata.modified().ok()?))
        })
        .collect::<Option<Vec<_>>>()?;
    entries.sort();
    Some(entries)
}

struct CachedMenu<F> {
    reference: String,
    fingerprint: F,
    document: ContextMenuDocument,
}

/// One resolved menu, valid while the reference and fingerprint match.
struct MenuCache<F> {
    entry: Option<CachedMenu<F>>,
}

impl<F: PartialEq> MenuCache<F> {
    const fn new() -> Self {
        Self { entry: None }
    }

    fn get(&self, reference: &str, fingerprint: &F) -> Option<ContextMenuDocument> {
        self.entry
            .as_ref()
            .filter(|entry| entry.reference == reference && &entry.fingerprint == fingerprint)
            .map(|entry| entry.document.clone())
    }

    fn store(&mut self, reference: &str, fingerprint: F, document: ContextMenuDocument) {
        self.entry = Some(CachedMenu {
            reference: reference.to_string(),
            fingerprint,
            document,
        });
    }
}

static MENU_CACHE: std::sync::Mutex<MenuCache<MenuDirectoryFingerprint>> =
    std::sync::Mutex::new(MenuCache::new());

/// `resolve_context_menu` with a cache for the right-click path. Errors are
/// never cached, so a broken menu is re-read (and logged) on every click as
/// before.
pub fn resolve_context_menu_cached(reference: Option<&str>) -> Result<ContextMenuDocument, String> {
    let key = reference.unwrap_or(CLASSIC_CONTEXT_MENU_ID).trim();
    let before = menu_directory_fingerprint();
    if let Some(fingerprint) = before.as_ref() {
        let cache = MENU_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(document) = cache.get(key, fingerprint) {
            return Ok(document);
        }
    }
    let document = resolve_context_menu(reference)?;
    // Cache only when the directory did not change while it was read. When
    // the resolve itself restored a built-in file, the next click caches.
    if let (Some(before), Some(after)) = (before, menu_directory_fingerprint()) {
        if before == after {
            MENU_CACHE.lock().unwrap_or_else(|e| e.into_inner()).store(
                key,
                after,
                document.clone(),
            );
        }
    }
    Ok(document)
}

fn canonical_context_menu_reference(reference: &str) -> &str {
    match reference {
        reference if reference.eq_ignore_ascii_case(LEGACY_CLASSIC_CONTEXT_MENU_ID) => {
            CLASSIC_CONTEXT_MENU_ID
        }
        reference if reference.eq_ignore_ascii_case(LEGACY_DASHBOARD_V2_CONTEXT_MENU_ID) => {
            DASHBOARD_V2_CONTEXT_MENU_ID
        }
        reference => reference,
    }
}

fn remove_legacy_context_menu_actions(items: &mut Vec<ContextMenuItem>) {
    for item in items.iter_mut() {
        if let ContextMenuItemKind::Submenu { items } = &mut item.kind {
            remove_legacy_context_menu_actions(items);
        }
    }
    items.retain(|item| match &item.kind {
        ContextMenuItemKind::Action {
            action: ContextMenuAction::LegacyResetPosition,
        } => false,
        ContextMenuItemKind::Submenu { items } => !items.is_empty(),
        _ => true,
    });
}

#[cfg(test)]
pub fn unique_document_id(name: &str) -> String {
    let base = safe_file_stem(name);
    let base = if base.is_empty() {
        "context-menu"
    } else {
        &base
    };
    let existing = list_context_menus().unwrap_or_default();
    if !existing
        .iter()
        .any(|menu| menu.id.eq_ignore_ascii_case(base))
    {
        return base.into();
    }
    (2..)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| {
            !existing
                .iter()
                .any(|menu| menu.id.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or_else(|| format!("{base}-copy"))
}

#[cfg(test)]
fn safe_file_stem(value: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !result.is_empty() {
            result.push('-');
            separator = true;
        }
    }
    result.trim_matches('-').to_string()
}

fn schema_version() -> u32 {
    CONTEXT_MENU_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_menu_is_valid_and_contains_the_v1_controls() {
        let menu = classic_context_menu();
        assert_eq!(menu.id, CLASSIC_CONTEXT_MENU_ID);
        assert_eq!(menu.name, "Classic v1");
        assert!(!serde_json::to_string(&menu)
            .unwrap()
            .contains("description"));
        assert!(menu.validate().is_empty());
        assert!(menu.items.iter().any(|item| item.id == "refresh"));
        assert!(menu.items.iter().any(|item| item.id == "update-frequency"));
        let settings = menu
            .items
            .iter()
            .find(|item| item.id == "settings")
            .unwrap();
        let ContextMenuItemKind::Submenu { items } = &settings.kind else {
            panic!("Settings must be a submenu");
        };
        assert!(items.iter().any(|item| {
            item.label == "Show pace"
                && matches!(
                    &item.kind,
                    ContextMenuItemKind::Action {
                        action: ContextMenuAction::ToggleShowPace
                    }
                )
        }));
        assert!(serde_json::to_string(&menu)
            .unwrap()
            .contains("provider-opencode"));
        assert!(serde_json::to_string(&menu)
            .unwrap()
            .contains("provider-cursor"));
        assert!(serde_json::to_string(&menu)
            .unwrap()
            .contains("provider-grok"));
        assert!(serde_json::to_string(&menu)
            .unwrap()
            .contains("provider-copilot"));
        assert!(menu.items.iter().any(|item| {
            item.id == "toggle-widget"
                && matches!(
                    &item.kind,
                    ContextMenuItemKind::Action {
                        action: ContextMenuAction::ToggleWidget
                    }
                )
        }));
        assert!(!serde_json::to_string(&menu)
            .unwrap()
            .contains("reset_position"));
        let mut legacy = menu.clone();
        legacy.id = LEGACY_CLASSIC_CONTEXT_MENU_ID.into();
        assert!(legacy.is_builtin());
    }

    fn classic_submenu<'a>(menu: &'a ContextMenuDocument, id: &str) -> &'a [ContextMenuItem] {
        let item = menu.items.iter().find(|item| item.id == id).unwrap();
        let ContextMenuItemKind::Submenu { items } = &item.kind else {
            panic!("expected submenu")
        };
        items
    }

    fn classic_providers_context() -> DataContext {
        let mut context = DataContext::from_usage(None, &theme_engine::Canvas::default());
        context.insert("providers.claude.enabled", 1.0);
        context.insert("providers.antigravity.enabled", 1.0);
        context
    }

    #[test]
    fn classic_fable_toggles_live_under_providers_not_settings() {
        let menu = classic_context_menu();
        assert!(!classic_submenu(&menu, "settings")
            .iter()
            .any(|item| item.id == "show-fable"));
        let providers = classic_submenu(&menu, "providers");
        assert!(matches!(
            providers[1].kind,
            ContextMenuItemKind::Action {
                action: ContextMenuAction::ToggleShowFable
            }
        ));
        assert!(matches!(
            providers[3].kind,
            ContextMenuItemKind::Action {
                action: ContextMenuAction::ToggleShowFableX
            }
        ));
        assert_eq!(
            serde_json::to_string(&ContextMenuAction::ToggleShowFableX).unwrap(),
            r#"{"type":"toggle_show_fable_x"}"#
        );
    }

    #[test]
    fn classic_antigravity_pools_are_first_level_provider_items() {
        let menu = classic_context_menu();
        let providers = classic_submenu(&menu, "providers");
        let context = classic_providers_context();
        // agy mirrors Claude: two first-level pool items, each with its own
        // External models child; no separate agy parent any more.
        for (id, label) in [
            ("provider-antigravity-a", "Antigravity (Main)"),
            ("provider-antigravity-x", "Antigravity (OpenCode)"),
        ] {
            let item = providers.iter().find(|item| item.id == id).unwrap();
            assert_eq!(item.label, label);
            assert!(item.should_render(&context));
        }
        assert!(providers
            .iter()
            .all(|item| item.id != "provider-antigravity-gemini" && !item.label.contains("agy (")));
    }

    #[test]
    fn classic_provider_children_follow_their_parents() {
        let menu = classic_context_menu();
        let providers = classic_submenu(&menu, "providers");
        let mut context = classic_providers_context();
        for (parent, child, binding, label) in [
            (
                "provider-claude-main",
                "show-fable",
                "claude.main.enabled",
                "Fable",
            ),
            (
                "provider-claude-x",
                "show-fable-x",
                "claude.x.enabled",
                "Fable",
            ),
            (
                "provider-antigravity-a",
                "provider-antigravity-external",
                "antigravity.a.enabled",
                "External models",
            ),
            (
                "provider-antigravity-x",
                "provider-antigravity-external-x",
                "antigravity.x.enabled",
                "External models",
            ),
        ] {
            let index = providers.iter().position(|item| item.id == parent).unwrap();
            let item = providers
                .iter()
                .skip(index + 1)
                .find(|item| item.id == child)
                .unwrap();
            assert!(
                providers[index + 1..]
                    .iter()
                    .take_while(|item| item.id != child)
                    .all(|item| item.label.starts_with(CHILD_INDENT)),
                "{child} must stay nested under {parent}"
            );
            assert_eq!(item.label, format!("{CHILD_INDENT}{label}"));
            context.insert(binding, 0.0);
            assert!(!item.should_render(&context));
            context.insert(binding, 1.0);
            assert!(item.should_render(&context));
        }
    }

    #[test]
    fn classic_pools_one_widget_needs_split_support_provider_and_both_pools() {
        let menu = classic_context_menu();
        let providers = classic_submenu(&menu, "providers");
        let mut context = classic_providers_context();
        // Every pool of the other providers stays enabled, as after the
        // parent/child checks.
        for binding in [
            "claude.main.enabled",
            "claude.x.enabled",
            "antigravity.a.enabled",
            "antigravity.x.enabled",
        ] {
            context.insert(binding, 1.0);
        }
        for (id, provider, main, x) in [
            (
                "claude-pools-one-widget",
                "providers.claude.enabled",
                "claude.main.enabled",
                "claude.x.enabled",
            ),
            (
                "codex-pools-one-widget",
                "providers.codex.enabled",
                "codex.main.enabled",
                "codex.x.enabled",
            ),
            (
                "antigravity-pools-one-widget",
                "providers.antigravity.enabled",
                "antigravity.a.enabled",
                "antigravity.x.enabled",
            ),
        ] {
            let item = providers.iter().find(|item| item.id == id).unwrap();
            for supported in [false, true] {
                context.insert("display.pool_split_supported", supported as u8 as f64);
                for p in [false, true] {
                    for a in [false, true] {
                        for b in [false, true] {
                            context.insert(provider, p as u8 as f64);
                            context.insert(main, a as u8 as f64);
                            context.insert(x, b as u8 as f64);
                            assert_eq!(
                                item.should_render(&context),
                                supported && p && a && b,
                                "{id}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dashboard_menu_is_builtin_and_separates_its_actions() {
        let menu = dashboard_v2_context_menu();
        assert_eq!(menu.id, DASHBOARD_V2_CONTEXT_MENU_ID);
        assert_eq!(menu.name, "Dashboard v2");
        assert!(menu.is_builtin());
        let mut legacy = menu.clone();
        legacy.id = LEGACY_DASHBOARD_V2_CONTEXT_MENU_ID.into();
        assert!(legacy.is_builtin());
        assert_eq!(
            canonical_context_menu_reference(LEGACY_DASHBOARD_V2_CONTEXT_MENU_ID),
            DASHBOARD_V2_CONTEXT_MENU_ID
        );
        assert!(menu.validate().is_empty());
        assert_eq!(
            menu.items,
            vec![
                ContextMenuItem::action(
                    "open-dashboard",
                    "Open Dashboard",
                    ContextMenuAction::OpenDashboard,
                ),
                ContextMenuItem::separator("dashboard-separator"),
                ContextMenuItem::action("exit", "Exit", ContextMenuAction::Exit),
            ]
        );
    }

    #[test]
    fn legacy_frequency_milliseconds_are_normalized_to_seconds() {
        let action: ContextMenuAction =
            serde_json::from_str(r#"{"type":"set_update_frequency","milliseconds":300000}"#)
                .unwrap();
        assert_eq!(
            action,
            ContextMenuAction::SetUpdateFrequency { seconds: 300 }
        );
        let serialized = serde_json::to_string(&action).unwrap();
        assert!(serialized.contains(r#""seconds":300"#));
        assert!(!serialized.contains("milliseconds"));
    }

    #[test]
    fn legacy_reset_position_actions_are_removed() {
        let mut items = vec![
            ContextMenuItem::action(
                "reset-position",
                "Reset position",
                ContextMenuAction::LegacyResetPosition,
            ),
            ContextMenuItem::submenu(
                "legacy-settings",
                "Settings",
                vec![ContextMenuItem::action(
                    "nested-reset-position",
                    "Reset position",
                    ContextMenuAction::LegacyResetPosition,
                )],
            ),
            ContextMenuItem::action("exit", "Exit", ContextMenuAction::Exit),
        ];
        remove_legacy_context_menu_actions(&mut items);
        assert_eq!(
            items,
            vec![ContextMenuItem::action(
                "exit",
                "Exit",
                ContextMenuAction::Exit
            )]
        );
    }

    #[test]
    fn urls_are_limited_to_user_facing_protocols() {
        assert!(supported_url("https://example.com"));
        assert!(supported_url("http://example.com"));
        assert!(!supported_url("mailto:test@example.com"));
        assert!(!supported_url("file:///C:/secret.txt"));
        assert!(!supported_url("powershell:whoami"));
    }

    #[test]
    fn duplicate_item_ids_are_rejected_across_submenus() {
        let mut menu = ContextMenuDocument::blank("Test");
        menu.items.push(ContextMenuItem::submenu(
            "submenu",
            "Submenu",
            vec![ContextMenuItem::action(
                "exit",
                "Duplicate",
                ContextMenuAction::Exit,
            )],
        ));
        assert!(menu
            .validate()
            .iter()
            .any(|error| error.contains("unique id")));
    }

    #[test]
    fn informational_text_accepts_usage_and_version_templates() {
        let mut menu = ContextMenuDocument::blank("Test");
        menu.items.insert(
            0,
            ContextMenuItem::text("usage", "v{app.version} - {claude.session:usage_line}"),
        );
        assert!(menu.validate().is_empty());
    }

    #[test]
    fn monthly_menu_labels_validate_without_live_usage() {
        let mut menu = classic_context_menu();
        for provider in ProviderId::ALL {
            let name = provider.descriptor().key;
            let mut item = ContextMenuItem::text(
                &format!("{name}-monthly"),
                &format!("{{{name}.monthly.label}}: {{{name}.monthly.remaining:0}}% left"),
            );
            item.render = Expression(format!(
                "providers.{name}.enabled && {name}.monthly.available"
            ));
            menu.items.push(item);
        }
        assert!(menu.validate().is_empty(), "{:?}", menu.validate());
    }

    #[test]
    fn legacy_items_default_to_visible_and_conditions_round_trip() {
        let mut item: ContextMenuItem =
            serde_json::from_str(r#"{"id":"usage","type":"text","label":"Usage"}"#).unwrap();
        let mut context = DataContext::from_usage(None, &theme_engine::Canvas::default());
        assert!(item.should_render(&context));
        assert_eq!(item.render.0, "1");
        item.render = Expression("providers.codex.enabled && codex.weekly.available".into());
        let encoded = serde_json::to_string(&item).unwrap();
        assert_eq!(
            serde_json::from_str::<ContextMenuItem>(&encoded).unwrap(),
            item
        );
        context.insert("providers.codex.enabled", 1.0);
        assert!(!item.should_render(&context));
        context.insert("codex.weekly.available", 1.0);
        assert!(item.should_render(&context));
        context.insert("providers.codex.enabled", 0.0);
        assert!(!item.should_render(&context));
    }

    #[test]
    fn menu_render_rejects_invalid_values_and_hides_falsy_rows() {
        let context = DataContext::from_usage(None, &theme_engine::Canvas::default());
        let mut menu = classic_context_menu();
        for source in ["missing.variable", "1 +", "sqrt(-1)", "\"text\""] {
            menu.items[0].render = Expression(source.into());
            assert!(!menu.items[0].should_render(&context));
            assert!(menu
                .validate()
                .iter()
                .any(|error| error.contains(".render")));
        }
        let item = &mut menu.items[0];
        for (render, visible) in [
            ("0", false),
            ("false", false),
            ("-1", true),
            ("1", true),
            ("true", true),
        ] {
            item.render = Expression(render.into());
            assert_eq!(item.should_render(&context), visible);
        }
    }

    #[test]
    fn menu_cache_hits_only_for_same_reference_and_fingerprint() {
        let mut cache = MenuCache::<u32>::new();
        assert!(cache.get("classic", &1).is_none());
        cache.store("classic", 1, classic_context_menu());
        assert_eq!(cache.get("classic", &1), Some(classic_context_menu()));
        assert!(
            cache.get("classic", &2).is_none(),
            "changed files must miss"
        );
        assert!(
            cache.get("other", &1).is_none(),
            "other reference must miss"
        );
        cache.store("classic", 2, dashboard_v2_context_menu());
        assert!(cache.get("classic", &1).is_none());
        assert_eq!(cache.get("classic", &2), Some(dashboard_v2_context_menu()));
    }
}
