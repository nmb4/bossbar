//! Pill layout and painting, independent of window lifecycle.
//!
//! [`BossBarApp`](crate::app::BossBarApp) owns the native window; this module
//! turns a [`BarStore`](crate::bars::BarStore) snapshot into pixels. Keeping
//! it pure makes the exact visuals testable through offscreen snapshots.

use std::sync::Arc;

use bossbar_proto::{BarKind, BarStatus, Request};
use eframe::egui::{
    self, epaint::Shadow, Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Vec2,
};

use crate::bars::BarStore;

// -- layout ------------------------------------------------------------------

/// Width of the expanded pill.
pub const PILL_WIDTH: f32 = 340.0;
/// Initial native window content size before any bar exists.
pub const INITIAL_PILL_SIZE: [f32; 2] = [PILL_WIDTH, COLLAPSED_HEIGHT];
const COLLAPSED_HEIGHT: f32 = 40.0;
const COLLAPSED_PAD_X: f32 = 15.0;
const PAD_X: f32 = 15.0;
const PAD_Y: f32 = 12.0;
const ROW_LABEL_H: f32 = 17.0;
const ROW_DETAIL_H: f32 = 15.0;
const ROW_INNER_GAP: f32 = 4.0;
const ROW_GAP: f32 = 12.0;
const HEADER_H: f32 = 16.0;
const HEADER_GAP: f32 = 9.0;
const BAR_H: f32 = 5.0;
/// Capsule radius cap; small pills use half their height instead.
const MAX_RADIUS: f32 = 26.0;
/// Transparent band around the pill used for the drop shadow. Windows is
/// trimmed to the pill itself with a shaped region instead.
pub const SHADOW_MARGIN: f32 = if cfg!(windows) { 0.0 } else { 10.0 };
const SPINNER_D: f32 = 18.0;
const SPINNER_STROKE: f32 = 2.2;
const CHEVRON_SIZE: f32 = 20.0;

/// Alcove-inspired dark surface.
mod skin {
    use eframe::egui::Color32;

    /// Fully opaque: a near-opaque surface still ghosts high-contrast content
    /// that sits behind the pill.
    pub const PILL: Color32 = Color32::from_rgb(10, 10, 13);
    pub const LINE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
    pub const TEXT: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf3);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9c, 0x9c, 0xa6);
    pub const TRACK: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
    pub const ACCENT: Color32 = Color32::from_rgb(0xed, 0xed, 0xf2);
    pub const OK: Color32 = Color32::from_rgb(0x6f, 0xcf, 0x97);
    pub const ERR: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
    pub const HOVER: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
}

fn tint(color: Color32, factor: f32) -> Color32 {
    color.gamma_multiply(factor.clamp(0.0, 1.0))
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("inter-semibold".into()))
}

fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("inter-medium".into()))
}

fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// Installs Inter as the proportional face, with Medium and SemiBold
/// available as named families.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        egui::FontData::from_static(include_bytes!("../fonts/Inter-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "inter-medium".into(),
        egui::FontData::from_static(include_bytes!("../fonts/Inter-Medium.ttf")).into(),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        egui::FontData::from_static(include_bytes!("../fonts/Inter-SemiBold.ttf")).into(),
    );

    let defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with_face = |face: &str| {
        let mut list = vec![face.to_owned()];
        list.extend(defaults.iter().cloned());
        list
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with_face("inter"));
    fonts.families.insert(
        FontFamily::Name("inter-medium".into()),
        with_face("inter-medium"),
    );
    fonts.families.insert(
        FontFamily::Name("inter-semibold".into()),
        with_face("inter-semibold"),
    );
    ctx.set_fonts(fonts);
}

// -- model -------------------------------------------------------------------

#[derive(Clone)]
pub struct RowModel {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub value_text: String,
    pub kind: BarKind,
    pub status: BarStatus,
    pub color: Color32,
    pub display: f32,
    pub alpha: f32,
}

#[derive(Clone)]
pub struct UiModel {
    pub rows: Vec<RowModel>,
    pub live_count: usize,
    pub collapsed: bool,
    pub visible: bool,
    pub aggregate: Option<f32>,
}

impl UiModel {
    pub fn collect(store: &BarStore) -> Self {
        let rows = store
            .bars()
            .iter()
            .map(|bar| RowModel {
                id: bar.id.clone(),
                label: bar.label.clone(),
                detail: bar.detail.clone(),
                value_text: bar.value_text(),
                kind: bar.kind,
                status: bar.status,
                color: match bar.status {
                    BarStatus::Running => bar
                        .color
                        .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                        .unwrap_or(skin::ACCENT),
                    BarStatus::Done => skin::OK,
                    BarStatus::Failed => skin::ERR,
                },
                display: bar.fraction(),
                alpha: bar.alpha,
            })
            .collect();
        Self {
            rows,
            live_count: store.live_count(),
            collapsed: store.collapsed,
            visible: store.visible,
            aggregate: store.aggregate(),
        }
    }

    /// True when the pill shows a single compact line with a progress ring.
    pub fn uses_spinner(&self) -> bool {
        self.collapsed && self.live_count >= 2
    }

    pub fn state_dot(&self) -> Color32 {
        if self.rows.iter().any(|row| row.status == BarStatus::Failed) {
            skin::ERR
        } else if self.live_count > 0
            && self
                .rows
                .iter()
                .filter(|row| row.alpha > 0.5)
                .all(|row| row.status == BarStatus::Done)
        {
            skin::OK
        } else {
            skin::ACCENT
        }
    }
}

/// Natural size of the pill, in points. `current` is returned when there is
/// nothing to show (so the window keeps its last size while fading out).
pub fn target_size(ctx: &egui::Context, model: &UiModel, current: Vec2) -> Vec2 {
    if model.uses_spinner() {
        return collapsed_size(ctx, model);
    }
    if model.rows.is_empty() {
        return current;
    }

    let header = if model.live_count >= 2 {
        HEADER_H + HEADER_GAP
    } else {
        0.0
    };
    let mut rows_height = 0.0;
    for (index, row) in model.rows.iter().enumerate() {
        if index > 0 {
            rows_height += ROW_GAP;
        }
        rows_height += ROW_LABEL_H + ROW_INNER_GAP + BAR_H;
        if row
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.is_empty())
        {
            rows_height += 3.0 + ROW_DETAIL_H;
        }
    }
    Vec2::new(PILL_WIDTH, (PAD_Y * 2.0 + header + rows_height).round())
}

fn collapsed_size(ctx: &egui::Context, model: &UiModel) -> Vec2 {
    let text = format!("{} tasks", model.live_count);
    let value = model
        .aggregate
        .map(|fraction| format!("{:.0}%", fraction * 100.0));
    let text_width = measure(ctx, &text, semibold(12.5));
    let value_width = value
        .as_deref()
        .map(|value| measure(ctx, value, medium(12.0)))
        .unwrap_or(0.0);
    let width = COLLAPSED_PAD_X * 2.0
        + SPINNER_D
        + 9.0
        + text_width
        + if value_width > 0.0 {
            8.0 + value_width
        } else {
            0.0
        }
        + 6.0
        + CHEVRON_SIZE;
    Vec2::new(width.clamp(150.0, PILL_WIDTH).round(), COLLAPSED_HEIGHT)
}

pub fn pill_radius(pill_size: Vec2) -> f32 {
    (pill_size.y / 2.0).min(MAX_RADIUS)
}

// -- painting ----------------------------------------------------------------

/// Draws the pill at `pill_size` inside the current UI, offset by the shadow
/// margin. `actions` receives requests produced by clicks.
pub fn render_pill(
    ui: &mut egui::Ui,
    model: &UiModel,
    pill_size: Vec2,
    appear: f32,
    spinner_phase: f32,
    actions: &mut dyn FnMut(Request),
) {
    let margin = SHADOW_MARGIN;
    let rect = Rect::from_min_size(Pos2::new(margin, margin), pill_size);
    let radius = pill_radius(pill_size);
    let mut consumed = false;
    let pill_response = ui.interact(rect, ui.id().with("pill-body"), Sense::click());
    let painter = ui.painter().clone();

    if margin > 0.0 {
        let shadow = Shadow {
            offset: [0, 5],
            blur: 22,
            spread: 0,
            color: Color32::from_black_alpha((70.0 * appear) as u8),
        };
        painter.add(Shape::from(shadow.as_shape(rect, radius)));
    }
    painter.rect(
        rect,
        radius,
        tint(skin::PILL, appear),
        Stroke::new(1.0_f32, tint(skin::LINE, appear)),
        StrokeKind::Inside,
    );

    if model.uses_spinner() {
        render_collapsed(
            ui,
            &painter,
            rect,
            model,
            appear,
            spinner_phase,
            actions,
            &mut consumed,
        );
    } else {
        render_expanded(
            ui,
            &painter,
            rect,
            model,
            appear,
            spinner_phase,
            actions,
            &mut consumed,
        );
    }

    if pill_response.clicked() && !consumed && model.live_count >= 2 {
        actions(Request::Collapse { value: None });
    }
}

#[allow(clippy::too_many_arguments)]
fn render_expanded(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    model: &UiModel,
    appear: f32,
    spinner_phase: f32,
    actions: &mut dyn FnMut(Request),
    consumed: &mut bool,
) {
    let content = rect.shrink2(Vec2::new(PAD_X, PAD_Y));
    let mut y = content.top();

    if model.live_count >= 2 {
        let header = Rect::from_min_size(
            Pos2::new(content.left(), y),
            Vec2::new(content.width(), HEADER_H),
        );
        render_header(ui, painter, header, model, appear, actions, consumed);
        y += HEADER_H + HEADER_GAP;
    }

    for (index, row) in model.rows.iter().enumerate() {
        if index > 0 {
            y += ROW_GAP;
        }
        let has_detail = row
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.is_empty());
        let row_height =
            ROW_LABEL_H + ROW_INNER_GAP + BAR_H + if has_detail { 3.0 + ROW_DETAIL_H } else { 0.0 };
        let row_rect = Rect::from_min_size(
            Pos2::new(content.left(), y),
            Vec2::new(content.width(), row_height),
        );
        render_row(
            ui,
            painter,
            row_rect,
            row,
            appear,
            spinner_phase,
            actions,
            consumed,
        );
        y += row_height;
    }
}

fn render_header(
    ui: &egui::Ui,
    painter: &egui::Painter,
    header: Rect,
    model: &UiModel,
    appear: f32,
    actions: &mut dyn FnMut(Request),
    consumed: &mut bool,
) {
    let center_y = header.center().y;
    painter.circle_filled(
        Pos2::new(header.left() + 3.0, center_y),
        3.0,
        tint(model.state_dot(), appear),
    );
    painter.text(
        Pos2::new(header.left() + 13.0, center_y),
        Align2::LEFT_CENTER,
        format!("{} TASKS", model.live_count),
        medium(9.5),
        tint(skin::TEXT_DIM, appear),
    );

    let chevron = Rect::from_center_size(
        Pos2::new(header.right() - CHEVRON_SIZE / 2.0, center_y),
        Vec2::splat(CHEVRON_SIZE),
    );
    let response = ui.interact(chevron, ui.id().with("header-chevron"), Sense::click());
    if response.hovered() {
        painter.rect_filled(chevron, CornerRadius::same(7), tint(skin::HOVER, appear));
    }
    paint_chevron(
        painter,
        chevron.center(),
        4.5,
        true,
        Stroke::new(
            1.5_f32,
            tint(
                if response.hovered() {
                    skin::TEXT
                } else {
                    skin::TEXT_DIM
                },
                appear,
            ),
        ),
    );
    if response.clicked() {
        *consumed = true;
        actions(Request::Collapse { value: None });
    }

    if let Some(fraction) = model.aggregate {
        painter.text(
            Pos2::new(chevron.left() - 4.0, center_y),
            Align2::RIGHT_CENTER,
            format!("{:.0}%", fraction * 100.0),
            medium(11.0),
            tint(skin::TEXT_DIM, appear),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    ui: &egui::Ui,
    painter: &egui::Painter,
    row_rect: Rect,
    row: &RowModel,
    appear: f32,
    spinner_phase: f32,
    actions: &mut dyn FnMut(Request),
    consumed: &mut bool,
) {
    let row_alpha = appear * row.alpha;
    let ctx = ui.ctx();
    let label_center_y = row_rect.top() + ROW_LABEL_H / 2.0;
    let hovered = row_alpha > 0.5 && ui.rect_contains_pointer(row_rect);

    painter.circle_filled(
        Pos2::new(row_rect.left() + 3.0, label_center_y),
        3.0,
        tint(row.color, row_alpha),
    );

    let close_width = if hovered { 20.0 } else { 0.0 };
    let value_width = if row.value_text.is_empty() {
        0.0
    } else {
        measure(ctx, &row.value_text, medium(12.0))
    };
    let label_left = row_rect.left() + 13.0;
    let label_max_width =
        (row_rect.right() - label_left - value_width - 8.0 - close_width).max(24.0);
    let galley = elide(
        ctx,
        &row.label,
        semibold(12.5),
        tint(skin::TEXT, row_alpha),
        label_max_width,
    );
    painter.galley(
        Pos2::new(label_left, label_center_y - galley.size().y / 2.0),
        galley,
        tint(skin::TEXT, row_alpha),
    );

    if !row.value_text.is_empty() {
        painter.text(
            Pos2::new(row_rect.right() - close_width, label_center_y),
            Align2::RIGHT_CENTER,
            &row.value_text,
            medium(12.0),
            tint(skin::TEXT_DIM, row_alpha),
        );
    }

    if hovered {
        let close = Rect::from_center_size(
            Pos2::new(row_rect.right() - 9.0, label_center_y),
            Vec2::splat(18.0),
        );
        let response = ui.interact(
            close,
            ui.id().with(("row-close", row.id.as_str())),
            Sense::click(),
        );
        if response.hovered() {
            painter.rect_filled(close, CornerRadius::same(6), tint(skin::HOVER, appear));
        }
        paint_close(
            painter,
            close.center(),
            3.4,
            Stroke::new(
                1.4_f32,
                tint(
                    if response.hovered() {
                        skin::TEXT
                    } else {
                        skin::TEXT_DIM
                    },
                    appear,
                ),
            ),
        );
        if response.clicked() {
            *consumed = true;
            actions(Request::Remove { id: row.id.clone() });
        }
    }

    let bar_top = row_rect.top() + ROW_LABEL_H + ROW_INNER_GAP;
    let bar_rect = Rect::from_min_size(
        Pos2::new(row_rect.left(), bar_top),
        Vec2::new(row_rect.width(), BAR_H),
    );
    let radius = CornerRadius::same((BAR_H / 2.0).round() as u8);
    let track = tint(skin::TRACK, row_alpha);
    let fill = tint(row.color, row_alpha);
    match row.kind {
        BarKind::Percent => {
            painter.rect_filled(bar_rect, radius, track);
            let width = bar_rect.width() * row.display.clamp(0.0, 1.0);
            if width > 0.5 {
                let filled = Rect::from_min_size(bar_rect.min, Vec2::new(width.max(BAR_H), BAR_H));
                painter.rect_filled(filled, radius, fill);
            }
        }
        BarKind::Ticks { total } => {
            paint_ticks(painter, bar_rect, radius, total, row.display, track, fill);
        }
        BarKind::Indeterminate => {
            painter.rect_filled(bar_rect, radius, track);
            paint_sheen(painter, bar_rect, radius, spinner_phase, fill);
        }
    }

    if let Some(detail) = row.detail.as_deref().filter(|detail| !detail.is_empty()) {
        let galley = elide(
            ctx,
            detail,
            regular(10.5),
            tint(skin::TEXT_DIM, row_alpha),
            row_rect.width(),
        );
        painter.galley(
            Pos2::new(row_rect.left(), bar_top + BAR_H + 3.0),
            galley,
            tint(skin::TEXT_DIM, row_alpha),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_collapsed(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    model: &UiModel,
    appear: f32,
    spinner_phase: f32,
    actions: &mut dyn FnMut(Request),
    consumed: &mut bool,
) {
    let center_y = rect.center().y;
    let spinner_center = Pos2::new(rect.left() + COLLAPSED_PAD_X + SPINNER_D / 2.0, center_y);
    paint_ring(
        painter,
        spinner_center,
        SPINNER_D / 2.0 - SPINNER_STROKE / 2.0,
        SPINNER_STROKE,
        model.aggregate,
        spinner_phase,
        tint(skin::ACCENT, appear),
        tint(skin::TRACK, appear),
    );

    painter.text(
        Pos2::new(spinner_center.x + SPINNER_D / 2.0 + 9.0, center_y),
        Align2::LEFT_CENTER,
        format!("{} tasks", model.live_count),
        semibold(12.5),
        tint(skin::TEXT, appear),
    );

    let chevron = Rect::from_center_size(
        Pos2::new(
            rect.right() - COLLAPSED_PAD_X - CHEVRON_SIZE / 2.0,
            center_y,
        ),
        Vec2::splat(CHEVRON_SIZE),
    );
    let response = ui.interact(chevron, ui.id().with("collapsed-chevron"), Sense::click());
    if response.hovered() {
        painter.rect_filled(chevron, CornerRadius::same(7), tint(skin::HOVER, appear));
    }
    paint_chevron(
        painter,
        chevron.center(),
        4.5,
        false,
        Stroke::new(
            1.5_f32,
            tint(
                if response.hovered() {
                    skin::TEXT
                } else {
                    skin::TEXT_DIM
                },
                appear,
            ),
        ),
    );
    if response.clicked() {
        *consumed = true;
        actions(Request::Collapse { value: None });
    }

    if let Some(fraction) = model.aggregate {
        painter.text(
            Pos2::new(chevron.left() - 4.0, center_y),
            Align2::RIGHT_CENTER,
            format!("{:.0}%", fraction * 100.0),
            medium(12.0),
            tint(skin::TEXT_DIM, appear),
        );
    }
}

fn measure(ctx: &egui::Context, text: &str, font: FontId) -> f32 {
    ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), font, skin::TEXT)
            .size()
            .x
    })
}

fn elide(
    ctx: &egui::Context,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(1.0));
    ctx.fonts_mut(|fonts| fonts.layout_job(job))
}

fn paint_ticks(
    painter: &egui::Painter,
    rect: Rect,
    radius: CornerRadius,
    total: u32,
    fraction: f32,
    track: Color32,
    fill: Color32,
) {
    let total = total.max(1);
    let gap = match total {
        0..=20 => 3.0,
        21..=40 => 2.0,
        _ => 1.0,
    };
    let segment_width = ((rect.width() - gap * (total - 1) as f32) / total as f32).max(1.0);
    let filled_ticks = fraction.clamp(0.0, 1.0) * total as f32;
    for index in 0..total {
        let left = rect.left() + index as f32 * (segment_width + gap);
        let segment = Rect::from_min_size(
            Pos2::new(left, rect.top()),
            Vec2::new(segment_width, rect.height()),
        );
        painter.rect_filled(segment, radius, track);
        let fill_amount = (filled_ticks - index as f32).clamp(0.0, 1.0);
        if fill_amount > 0.05 {
            let filled = Rect::from_min_size(
                segment.min,
                Vec2::new(segment.width() * fill_amount, segment.height()),
            );
            painter.rect_filled(filled, radius, fill);
        }
    }
}

fn paint_sheen(
    painter: &egui::Painter,
    rect: Rect,
    radius: CornerRadius,
    phase: f32,
    color: Color32,
) {
    let sheen_width = rect.width() * 0.3;
    let travel = rect.width() + sheen_width;
    let left = rect.left() - sheen_width + phase * travel;
    let sheen = Rect::from_min_size(
        Pos2::new(left, rect.top()),
        Vec2::new(sheen_width, rect.height()),
    );
    painter
        .with_clip_rect(rect)
        .rect_filled(sheen, radius, color);
}

#[allow(clippy::too_many_arguments)]
fn paint_ring(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    stroke_width: f32,
    fraction: Option<f32>,
    phase: f32,
    color: Color32,
    track: Color32,
) {
    use std::f32::consts::{FRAC_PI_2, TAU};

    painter.circle_stroke(center, radius, Stroke::new(stroke_width, track));
    let (start, sweep) = match fraction {
        Some(fraction) => (-FRAC_PI_2, fraction.clamp(0.0, 1.0) * TAU),
        None => (phase * TAU, TAU * 0.3),
    };
    if sweep < 0.08 {
        return;
    }
    let steps = ((sweep / (TAU / 72.0)).ceil() as usize).clamp(2, 72);
    let points: Vec<Pos2> = (0..=steps)
        .map(|index| {
            let angle = start + sweep * (index as f32 / steps as f32);
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(stroke_width, color),
    ));
    painter.circle_filled(points[0], stroke_width / 2.0, color);
    painter.circle_filled(*points.last().unwrap(), stroke_width / 2.0, color);
}

fn paint_chevron(painter: &egui::Painter, center: Pos2, half: f32, up: bool, stroke: Stroke) {
    let direction = if up { -1.0 } else { 1.0 };
    let left = center + Vec2::new(-half, -direction * half * 0.55);
    let middle = center + Vec2::new(0.0, direction * half * 0.55);
    let right = center + Vec2::new(half, -direction * half * 0.55);
    painter.line_segment([left, middle], stroke);
    painter.line_segment([middle, right], stroke);
}

fn paint_close(painter: &egui::Painter, center: Pos2, radius: f32, stroke: Stroke) {
    painter.line_segment(
        [
            center + Vec2::new(-radius, -radius),
            center + Vec2::new(radius, radius),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + Vec2::new(-radius, radius),
            center + Vec2::new(radius, -radius),
        ],
        stroke,
    );
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bossbar_proto::{BarInit, BarPatch};

    use super::*;

    /// Builds a store, settles its animations, and renders it through the
    /// real layout code on a dark backdrop.
    fn snapshot(name: &str, store: &BarStore) {
        let model = UiModel::collect(store);

        // Fonts are installed on one pass and available on the next; mirror
        // what the daemon does at app creation time.
        let probe = egui::Context::default();
        let _ = probe.run(egui::RawInput::default(), setup_fonts);
        let mut pill_size = Vec2::new(PILL_WIDTH, COLLAPSED_HEIGHT);
        {
            let model_ref = &model;
            let size_out = &mut pill_size;
            let _ = probe.run(egui::RawInput::default(), |ctx| {
                *size_out = target_size(ctx, model_ref, *size_out);
            });
        }

        let mut harness = egui_kittest::Harness::builder()
            .with_size(pill_size + Vec2::splat(SHADOW_MARGIN * 2.0))
            .wgpu()
            .build_state(
                move |ctx, state: &mut (bool, UiModel, Vec2)| {
                    if !state.0 {
                        setup_fonts(ctx);
                        state.0 = true;
                        ctx.request_repaint();
                        return;
                    }
                    let model = state.1.clone();
                    let pill_size = state.2;
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            // A desktop-like backdrop so the shadow and the
                            // transparent margin are visible in the PNG.
                            ui.painter().rect_filled(
                                ui.max_rect(),
                                0.0,
                                egui::Color32::from_rgb(0x24, 0x26, 0x2b),
                            );
                            render_pill(ui, &model, pill_size, 1.0, 0.35, &mut |_request| {});
                        });
                },
                (false, model, pill_size),
            );
        harness.run();
        harness.snapshot(name);
    }

    fn settled(store: &mut BarStore) -> BarStore {
        // Animate with a fixed clock so nothing expires or drifts.
        let now = Instant::now();
        for _ in 0..240 {
            store.advance(now, 1.0 / 60.0);
        }
        std::mem::take(store)
    }

    #[test]
    fn expanded_single_percent_bar() {
        let mut store = BarStore::default();
        let id = store
            .create(None, BarInit::new("Building wire-app"))
            .unwrap();
        store
            .update(
                &id,
                BarPatch {
                    percent: Some(47.0),
                    ..Default::default()
                },
            )
            .unwrap();
        snapshot("expanded-single", &settled(&mut store));
    }

    #[test]
    fn expanded_multiple_bars() {
        let mut store = BarStore::default();
        let build = store
            .create(
                Some("build".into()),
                BarInit {
                    detail: Some("cargo build --release · 214 crates".into()),
                    color: Some("blue".into()),
                    ..BarInit::new("Building wire-app")
                },
            )
            .unwrap();
        store
            .update(
                &build,
                BarPatch {
                    percent: Some(38.0),
                    ..Default::default()
                },
            )
            .unwrap();

        let compile = store
            .create(
                Some("compile".into()),
                BarInit {
                    kind: BarKind::Ticks { total: 20 },
                    ..BarInit::new("Compiling bossbar")
                },
            )
            .unwrap();
        store.tick(&compile, None, Some(7)).unwrap();

        store
            .create(
                Some("fetch".into()),
                BarInit {
                    kind: BarKind::Indeterminate,
                    color: Some("violet".into()),
                    ..BarInit::new("Waiting for device")
                },
            )
            .unwrap();
        snapshot("expanded-multi", &settled(&mut store));
    }

    #[test]
    fn collapsed_single_line() {
        let mut store = BarStore::default();
        for (index, (label, color)) in [("Build", "blue"), ("Test", "cyan"), ("Lint", "green")]
            .into_iter()
            .enumerate()
        {
            let id = store
                .create(
                    None,
                    BarInit {
                        color: Some(color.into()),
                        ..BarInit::new(label)
                    },
                )
                .unwrap();
            store
                .update(
                    &id,
                    BarPatch {
                        percent: Some(20.0 + index as f64 * 30.0),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        store.set_collapsed(Some(true));
        snapshot("collapsed", &settled(&mut store));
    }

    #[test]
    fn long_labels_elide() {
        let mut store = BarStore::default();
        let id = store
            .create(
                None,
                BarInit {
                    detail: Some(
                        "downloading https://example.com/really/long/path/to/an/artifact-\
                         with-a-huge-name.tar.zst from cache node 12"
                            .into(),
                    ),
                    ..BarInit::new("Rebuilding the entire dependency graph for the release channel")
                },
            )
            .unwrap();
        store
            .update(
                &id,
                BarPatch {
                    percent: Some(12.0),
                    ..Default::default()
                },
            )
            .unwrap();
        snapshot("long-labels", &settled(&mut store));
    }

    #[test]
    fn done_and_failed_states() {
        let mut store = BarStore::default();
        let done = store.create(None, BarInit::new("Deploy web")).unwrap();
        store.finish(&done, Duration::from_secs(120)).unwrap();

        let failed = store.create(None, BarInit::new("Deploy api")).unwrap();
        store
            .fail(
                &failed,
                Some("connection refused after 3 retries".into()),
                Duration::from_secs(120),
            )
            .unwrap();
        snapshot("statuses", &settled(&mut store));
    }
}
