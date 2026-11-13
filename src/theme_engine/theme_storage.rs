use super::*;

pub fn themes_directory() -> PathBuf {
    crate::app_settings::app_data_directory().join("themes")
}

pub fn save_theme(theme: &ThemeDocument) -> Result<PathBuf, String> {
    let mut theme = theme.clone();
    theme.prepare_runtime();
    if theme.is_builtin() {
        return Err(format!(
            "{} is read-only; duplicate it to make changes",
            theme.name
        ));
    }
    let errors = theme.validate();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let directory = themes_directory();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{}.json", safe_file_stem(&theme.id)));
    crate::app_settings::write_json_atomic(&path, &theme)?;
    Ok(path)
}

pub fn load_theme(path: &Path) -> Result<ThemeDocument, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut theme: ThemeDocument = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    theme.prepare_runtime();
    let errors = theme.validate();
    if errors.is_empty() {
        Ok(theme)
    } else {
        Err(errors.join("\n"))
    }
}

pub fn ensure_starter_theme() -> Result<PathBuf, String> {
    let directory = themes_directory();
    for (expected_id, source) in BUILTIN_THEME_SOURCES {
        let mut theme: ThemeDocument = serde_json::from_str(source)
            .map_err(|error| format!("Built-in theme '{expected_id}' is invalid JSON: {error}"))?;
        if theme.id != *expected_id {
            return Err(format!(
                "Built-in theme id '{}' does not match '{expected_id}'",
                theme.id
            ));
        }
        theme.prepare_runtime();
        let errors = theme.validate();
        if !errors.is_empty() {
            return Err(format!(
                "Built-in theme '{}' is invalid:\n{}",
                theme.name,
                errors.join("\n")
            ));
        }
        let path = directory.join(format!("{expected_id}.json"));
        let canonical = serde_json::to_vec_pretty(&theme).map_err(|error| error.to_string())?;
        let current = std::fs::read(&path).ok();
        if current.as_deref() != Some(canonical.as_slice()) {
            crate::app_settings::write_json_atomic(&path, &theme)?;
        }
    }
    for removed_id in REMOVED_BUILTIN_THEME_IDS {
        let path = directory.join(format!("{removed_id}.json"));
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(directory.join(format!("{COMPACT_FLUENT_QUAD_THEME_ID}.json")))
}

/// Ignore retired managed selections without affecting themes in other folders.
pub fn is_removed_managed_theme(path: &Path) -> bool {
    // Windows paths are case-insensitive and may use `.`/`..`, junctions or
    // either separator. Resolve existing folders first, then compare text.
    let same_folder = |left: &Path, right: &Path| {
        let normalise = |path: &Path| {
            let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            resolved
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
        };
        normalise(left) == normalise(right)
    };
    path.parent()
        .is_some_and(|parent| same_folder(parent, &themes_directory()))
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| {
                [
                    "classic-usage-widget",
                    "classic-vertical",
                    "theme-minecraft",
                    "minecraft-codex",
                ]
                .iter()
                .any(|id| stem.eq_ignore_ascii_case(id))
            })
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

/// Load a saved selection, falling back to the bundled theme for retired managed paths.
pub fn load_selected_theme(path: &Path) -> Option<ThemeDocument> {
    if is_removed_managed_theme(path) {
        None
    } else {
        load_theme(path).ok()
    }
}
