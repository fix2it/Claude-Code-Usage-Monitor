use super::*;

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
    split_action_statements(source)?
        .iter()
        .map(|statement| parse_action_statement(statement))
        .collect()
}

fn parse_action_statement(statement: &str) -> Result<MouseAction, String> {
    let open = statement
        .find('(')
        .ok_or_else(|| format!("Expected an action call: {statement}"))?;
    if !statement.ends_with(')') {
        return Err(format!("Expected ')' at the end of: {statement}"));
    }
    let name = statement[..open].trim().to_ascii_lowercase();
    let args = split_action_arguments(&statement[open + 1..statement.len() - 1])?;
    match name.as_str() {
        "show_dashboard" | "toggle_dashboard" => parse_dashboard_action(&name, &args),
        "open_url" => parse_open_url_action(&args),
        "show_context_menu" => parse_context_menu_action(&args),
        "set" | "increase" | "decrease" => parse_value_action(&name, &args),
        "toggle" | "reset" => parse_property_action(&name, &args),
        _ => Err(format!("Unknown action: {name}")),
    }
}

fn parse_dashboard_action(name: &str, args: &[String]) -> Result<MouseAction, String> {
    if !args.is_empty() {
        return Err(format!("{name}() does not take arguments"));
    }
    Ok(if name == "show_dashboard" {
        MouseAction::ShowDashboard
    } else {
        MouseAction::ToggleDashboard
    })
}

fn parse_open_url_action(args: &[String]) -> Result<MouseAction, String> {
    let [url] = args else {
        return Err("open_url expects one quoted URL".into());
    };
    let url = parse_quoted_action_string(url, "URL")?;
    if !crate::context_menu::supported_url(&url) {
        return Err("open_url URL must start with http:// or https://".into());
    }
    Ok(MouseAction::OpenUrl { url })
}

fn parse_context_menu_action(args: &[String]) -> Result<MouseAction, String> {
    let menu = match args {
        [] => None,
        [menu] => Some(parse_quoted_action_string(menu, "context menu id or name")?),
        _ => return Err("show_context_menu expects zero or one quoted menu id or name".into()),
    };
    Ok(MouseAction::ShowContextMenu { menu })
}

/// `set`, `increase` and `decrease`: a target property plus a value expression.
fn parse_value_action(name: &str, args: &[String]) -> Result<MouseAction, String> {
    let (target, property, value) = match args {
        [target_property, value] => {
            let (target, property) = parse_target_property(target_property)?;
            (target, property, value.as_str())
        }
        [target, property, value] => (
            parse_action_target(target)?,
            parse_action_property(property)?,
            value.as_str(),
        ),
        _ => {
            return Err(format!(
                "{name} expects {name}(target.property, value) or {name}(target, property, value)"
            ))
        }
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
    Ok(match name {
        "set" => MouseAction::Set {
            target,
            property,
            value,
        },
        "increase" => MouseAction::Increase {
            target,
            property,
            amount: value,
        },
        _ => MouseAction::Decrease {
            target,
            property,
            amount: value,
        },
    })
}

/// `toggle` and `reset`: a target property without a value.
fn parse_property_action(name: &str, args: &[String]) -> Result<MouseAction, String> {
    let (target, property) = match args {
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
    if name == "reset" {
        return Ok(MouseAction::Reset { target, property });
    }
    if property != MouseActionProperty::Render {
        return Err("toggle currently supports the render property only".into());
    }
    Ok(MouseAction::Toggle { target, property })
}

pub(super) fn split_action_statements(source: &str) -> Result<Vec<String>, String> {
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

pub(super) fn split_action_arguments(source: &str) -> Result<Vec<String>, String> {
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

pub(super) fn parse_target_property(
    source: &str,
) -> Result<(MouseActionTarget, MouseActionProperty), String> {
    let (target, property) = source
        .rsplit_once('.')
        .ok_or_else(|| format!("Expected target.property, got: {source}"))?;
    Ok((
        parse_action_target(target)?,
        parse_action_property(property)?,
    ))
}

pub(super) fn parse_action_target(source: &str) -> Result<MouseActionTarget, String> {
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

pub(super) fn parse_quoted_action_string(source: &str, label: &str) -> Result<String, String> {
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

pub(super) fn parse_action_property(source: &str) -> Result<MouseActionProperty, String> {
    MouseActionProperty::parse(source).ok_or_else(|| {
        format!(
            "Unknown layer property '{}'; use render, visibility, x, y, width, height, or rotation",
            source.trim()
        )
    })
}

pub(super) fn action_target_id<'a>(target: &'a MouseActionTarget, self_id: &'a str) -> &'a str {
    match target {
        MouseActionTarget::SelfObject => self_id,
        MouseActionTarget::Object(id) => id,
    }
}

pub(super) fn surface_object_by_id<'a>(
    surface: &'a SceneObject,
    id: &str,
) -> Option<&'a SceneObject> {
    std::iter::once(surface)
        .chain(surface.children.iter())
        .find(|object| object.id.eq_ignore_ascii_case(id))
}

pub(super) fn surface_object_by_id_mut<'a>(
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

pub(super) fn mouse_action_target<'a>(
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

pub(super) fn mouse_property_expression(
    object: &SceneObject,
    property: MouseActionProperty,
) -> &Expression {
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

pub(super) fn set_mouse_property_expression(
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

pub(super) fn mouse_action_object_context(
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

pub(super) struct MouseActionExecutionContext<'a> {
    theme: &'a ThemeDocument,
    surface_index: usize,
    self_id: &'a str,
    data: Option<&'a AppUsageData>,
    runtime: ThemeRuntime,
}

impl MouseActionExecutionContext<'_> {
    /// Host effects are returned; property actions update `overrides` instead.
    fn run(
        &self,
        action: MouseAction,
        cached: Option<&DisplayMetrics>,
        overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
    ) -> Result<Option<MouseActionEffect>, String> {
        let effect = match action {
            MouseAction::ShowDashboard => MouseActionEffect::ShowDashboard,
            MouseAction::ToggleDashboard => MouseActionEffect::ToggleDashboard,
            MouseAction::OpenUrl { url } => MouseActionEffect::OpenUrl(url),
            MouseAction::ShowContextMenu { menu } => MouseActionEffect::ShowContextMenu(menu),
            MouseAction::Set {
                target,
                property,
                value,
            } => {
                return self
                    .set_property(&target, property, value, overrides)
                    .map(|()| None)
            }
            MouseAction::Reset { target, property } => {
                return self
                    .reset_property(&target, property, overrides)
                    .map(|()| None)
            }
            MouseAction::Increase {
                target,
                property,
                amount,
            } => {
                return self
                    .adjust_property(&target, property, &amount, 1.0, cached, overrides)
                    .map(|()| None)
            }
            MouseAction::Decrease {
                target,
                property,
                amount,
            } => {
                return self
                    .adjust_property(&target, property, &amount, -1.0, cached, overrides)
                    .map(|()| None)
            }
            MouseAction::Toggle { target, property } => {
                return self
                    .toggle_property(&target, property, cached, overrides)
                    .map(|()| None)
            }
        };
        Ok(Some(effect))
    }

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
        reject_root_placement(&effective.surfaces[target_surface_index], object, property)?;
        let object_id = object.id.clone();
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

    fn set_property(
        &self,
        target: &MouseActionTarget,
        property: MouseActionProperty,
        value: Expression,
        overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
    ) -> Result<(), String> {
        let (target_surface_index, object) =
            mouse_action_target(self.theme, self.surface_index, self.self_id, target)?;
        reject_root_placement(&self.theme.surfaces[target_surface_index], object, property)?;
        overrides.insert(
            MouseActionOverrideKey {
                surface_index: target_surface_index,
                object_id: object.id.clone(),
                property,
            },
            value,
        );
        Ok(())
    }

    fn reset_property(
        &self,
        target: &MouseActionTarget,
        property: MouseActionProperty,
        overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
    ) -> Result<(), String> {
        let (target_surface_index, object) =
            mouse_action_target(self.theme, self.surface_index, self.self_id, target)?;
        overrides.remove(&MouseActionOverrideKey {
            surface_index: target_surface_index,
            object_id: object.id.clone(),
            property,
        });
        Ok(())
    }

    /// Flip a property between 0 and 1, evaluated against the surface (not
    /// the object) context; an unevaluable value counts as 0.
    fn toggle_property(
        &self,
        target: &MouseActionTarget,
        property: MouseActionProperty,
        cached: Option<&DisplayMetrics>,
        overrides: &mut HashMap<MouseActionOverrideKey, Expression>,
    ) -> Result<(), String> {
        let effective = apply_mouse_action_overrides(self.theme, overrides);
        let (target_surface_index, object) =
            mouse_action_target(&effective, self.surface_index, self.self_id, target)?;
        let surface = &effective.surfaces[target_surface_index];
        let object_id = object.id.clone();
        let metrics = cached.map_or_else(
            || display_metrics(&effective, self.data, self.runtime),
            |metrics| metrics_for_phase(metrics.clone(), self.runtime),
        );
        let (width, height) = resolve_surface_content_size_with_metrics(
            &effective,
            target_surface_index,
            self.data,
            self.runtime,
            &metrics,
        );
        let canvas = Canvas {
            width,
            width_expression: Some(surface.width.clone()),
            height,
            height_expression: Some(surface.height.clone()),
            background: surface.background.canvas_paint(),
        };
        let mut context = DataContext::from_usage_with_runtime(self.data, &canvas, self.runtime);
        insert_display_metrics(&mut context, &metrics);
        let current =
            evaluate(&mouse_property_expression(object, property).0, &context).unwrap_or(0.0);
        overrides.insert(
            MouseActionOverrideKey {
                surface_index: target_surface_index,
                object_id,
                property,
            },
            Expression(if current == 0.0 { "1" } else { "0" }.into()),
        );
        Ok(())
    }
}

/// Root layers are placed on screen by the host, so X/Y cannot be scripted.
fn reject_root_placement(
    surface: &SceneObject,
    object: &SceneObject,
    property: MouseActionProperty,
) -> Result<(), String> {
    if object.id == surface.id
        && matches!(property, MouseActionProperty::X | MouseActionProperty::Y)
    {
        return Err(format!(
            "{}.{} cannot be changed because root layers use screen placement",
            object.name,
            property.name()
        ));
    }
    Ok(())
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
    let execution = MouseActionExecutionContext {
        theme,
        surface_index,
        self_id,
        data,
        runtime,
    };
    let mut effects = Vec::new();
    for action in parse_mouse_actions(source)? {
        match execution.run(action, cached, overrides)? {
            Some(effect) => effects.push(effect),
            // An override changed, so cached metrics no longer match the theme.
            None => cached = None,
        }
    }
    Ok(effects)
}
