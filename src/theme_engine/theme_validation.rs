use super::*;

pub(super) fn validate_scene_mouse_events(
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

pub(super) fn validate_scene_object(
    errors: &mut Vec<String>,
    context: &DataContext,
    object: &SceneObject,
) {
    // Gap resolves against the parent context; every other property can read
    // the resolved `this.gap`.
    let gap = evaluate(&object.gap.0, context)
        .ok()
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .max(0.0);
    let mut object_context = context.clone();
    object_context.insert("this.gap", gap);
    let mut check = ObjectValidator {
        errors,
        context: &object_context,
        name: &object.name,
    };
    for (property, expression) in [
        ("render", &object.render),
        ("visibility", &object.visibility),
        ("x", &object.x),
        ("y", &object.y),
        ("width", &object.width),
        ("height", &object.height),
        ("rotation", &object.rotation),
        ("corner_radius", &object.corner_radius),
    ] {
        check.expression(property, expression);
    }
    validate_expression(
        check.errors,
        context,
        &format!("{}.gap", object.name),
        &object.gap,
    );
    check.background(&object.background);
    if let Some(border) = &object.border {
        check.paint("border", &border.color);
        check.expression("border.width", &border.width);
    }
    check.content(&object.content);
}

/// Collects errors for one object's properties, labelled `<object>.<field>`.
struct ObjectValidator<'a> {
    errors: &'a mut Vec<String>,
    context: &'a DataContext,
    name: &'a str,
}

impl ObjectValidator<'_> {
    fn label(&self, field: &str) -> String {
        format!("{}.{field}", self.name)
    }

    fn expression(&mut self, field: &str, expression: &Expression) {
        let label = self.label(field);
        validate_expression(self.errors, self.context, &label, expression);
    }

    fn paint(&mut self, field: &str, paint: &Paint) {
        let label = self.label(field);
        validate_paint(self.errors, self.context, &label, paint);
    }

    fn background(&mut self, background: &LayerBackground) {
        match background {
            LayerBackground::None => {}
            LayerBackground::Colour { colour } => self.paint("background.colour", colour),
            LayerBackground::Gradient { start, end, angle } => {
                self.paint("background.gradient.start", start);
                self.paint("background.gradient.end", end);
                self.expression("background.gradient.angle", angle);
            }
            LayerBackground::Image { path, .. } => {
                if path.trim().is_empty() {
                    let error = format!("{}: choose an image", self.label("background.image"));
                    self.errors.push(error);
                }
            }
        }
    }

    fn content(&mut self, content: &SceneContent) {
        match content {
            SceneContent::None => {}
            SceneContent::Text {
                template,
                font_size,
                contrast,
                color,
                ..
            } => {
                self.expression("font_size", font_size);
                self.expression("contrast", contrast);
                self.paint("color", color);
                for error in validate_template(template, self.context) {
                    let error = format!("{}: {error}", self.label("template"));
                    self.errors.push(error);
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
                self.expression("value", value);
                self.expression("content.corner_radius", corner_radius);
                self.expression("segment_gap", segment_gap);
                if let Some(expression) = segments_expression {
                    self.expression("segments", expression);
                }
                self.paint("fill", fill);
                self.paint("track", track);
            }
        }
    }
}

pub(super) fn validate_expression(
    errors: &mut Vec<String>,
    context: &DataContext,
    label: &str,
    expression: &Expression,
) {
    if let Err(error) = evaluate(&expression.0, context) {
        errors.push(format!("{label}: {error}"));
    }
}

pub(super) fn validate_paint(
    errors: &mut Vec<String>,
    context: &DataContext,
    label: &str,
    paint: &Paint,
) {
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
