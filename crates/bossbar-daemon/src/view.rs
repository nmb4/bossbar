//! Pill layout and painting, independent of window lifecycle.
//!
//! [`BossBarApp`](crate::app::BossBarApp) owns the native window; this module
//! turns a [`BarStore`](crate::bars::BarStore) snapshot into pixels. Keeping
//! it pure makes the exact visuals testable through offscreen snapshots.

use std::sync::Arc;

use bossbar_proto::{BarKind, BarStatus, CollapseMode, Request};
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
const COLLAPSED_MIN_WIDTH: f32 = 150.0;
const COLLAPSED_MAX_WIDTH: f32 = 440.0;
/// Ring drawn next to each task in the collapsed normal layout.
const TASK_RING_D: f32 = 14.0;
/// Single shared ring in the collapsed compact layout.
const COMPACT_RING_D: f32 = 18.0;
const RING_GAP: f32 = 6.0;
const TASK_PAD_X: f32 = 10.0;
const DIVIDER_GAP: f32 = 9.0;
const DIVIDER_MIN_GAP: f32 = 3.0;
const PAD_X: f32 = 15.0;
const PAD_Y: f32 = 12.0;
const ROW_LABEL_H: f32 = 17.0;
const ROW_DETAIL_H: f32 = 15.0;
const ROW_INNER_GAP: f32 = 4.0;
const ROW_GAP: f32 = 12.0;
const BAR_H: f32 = 5.0;
/// Capsule radius cap; small pills use half their height instead.
const MAX_RADIUS: f32 = 26.0;
/// Transparent band around the pill used for the drop shadow. Windows keeps
/// the pill rect equal to the window rect (no shadow there); its antialiased
/// edge comes from the swapchain's per-pixel alpha instead.
pub const SHADOW_MARGIN: f32 = if cfg!(windows) { 0.0 } else { 10.0 };
const SPINNER_STROKE: f32 = 2.2;

/// Alcove-inspired dark surface.
mod skin {
    use eframe::egui::Color32;

    /// Fully opaque: a near-opaque surface still ghosts high-contrast content
    /// that sits behind the pill.
    pub const PILL: Color32 = Color32::from_rgb(10, 10, 13);
    pub const LINE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
    pub const LINE_HOVER: Color32 = Color32::from_rgba_premultiplied(34, 34, 34, 34);
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
    pub collapse_mode: CollapseMode,
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
            collapse_mode: store.collapse_mode,
            visible: store.visible,
            aggregate: store.aggregate(),
        }
    }

    /// True when the pill shows the collapsed layout instead of rows.
    pub fn uses_collapsed_layout(&self) -> bool {
        self.collapsed && !self.rows.is_empty()
    }

    /// Aggregate status color, used by the compact ring.
    pub fn status_color(&self) -> Color32 {
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
    if model.uses_collapsed_layout() {
        return collapsed_size(ctx, model);
    }
    if model.rows.is_empty() {
        return current;
    }

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
    Vec2::new(PILL_WIDTH, (PAD_Y * 2.0 + rows_height).round())
}

fn task_count_text(count: usize) -> String {
    if count == 1 {
        "1 task".to_owned()
    } else {
        format!("{count} tasks")
    }
}

fn collapsed_size(ctx: &egui::Context, model: &UiModel) -> Vec2 {
    match model.collapse_mode {
        CollapseMode::Compact => {
            let text = task_count_text(model.live_count);
            let text_width = measure(ctx, &text, semibold(12.5));
            let value_width = model
                .aggregate
                .map(|fraction| format!("{:.0}%", fraction * 100.0))
                .map(|value| measure(ctx, &value, medium(12.0)))
                .unwrap_or(0.0);
            let width = COLLAPSED_PAD_X * 2.0
                + COMPACT_RING_D
                + 9.0
                + text_width
                + if value_width > 0.0 {
                    8.0 + value_width
                } else {
                    0.0
                };
            Vec2::new(
                width.clamp(COLLAPSED_MIN_WIDTH, PILL_WIDTH).round(),
                COLLAPSED_HEIGHT,
            )
        }
        CollapseMode::Normal => {
            let plan = normal_plan(ctx, model);
            Vec2::new(
                plan.width.clamp(COLLAPSED_MIN_WIDTH, COLLAPSED_MAX_WIDTH),
                COLLAPSED_HEIGHT,
            )
        }
    }
}

/// Collapsed-normal layout: one ring per task plus an optional elided label.
struct NormalTask {
    /// Truncate width used when rendering the label.
    label_max: Option<f32>,
    /// Width the label actually occupies (`0.0` when hidden); the plan width
    /// is the exact sum of these plus rings, padding and dividers, so the
    /// pill never reserves space its content cannot fill.
    label_width: f32,
}

struct NormalPlan {
    tasks: Vec<NormalTask>,
    /// Padding after each task's content.
    pad: f32,
    /// Space on each side of the divider between tasks.
    divider_gap: f32,
    task_ring: f32,
    width: f32,
}

fn normal_plan(ctx: &egui::Context, model: &UiModel) -> NormalPlan {
    let tasks = model.rows.len().max(1);
    let ring = TASK_RING_D;
    let budget = COLLAPSED_MAX_WIDTH - COLLAPSED_PAD_X * 2.0;
    let label_widths: Vec<f32> = model
        .rows
        .iter()
        .map(|row| measure(ctx, &row.label, medium(12.0)))
        .collect();

    let mut plan = NormalPlan {
        tasks: Vec::with_capacity(tasks),
        pad: TASK_PAD_X,
        divider_gap: DIVIDER_GAP,
        task_ring: ring,
        width: 0.0,
    };

    let dividers = (tasks.saturating_sub(1)) as f32 * (plan.divider_gap * 2.0 + 1.0);
    let fixed = tasks as f32 * (ring + RING_GAP + plan.pad * 2.0);
    let labels_total: f32 = label_widths.iter().sum();
    if fixed + dividers + labels_total <= budget {
        // Everything fits with full labels.
        plan.tasks = label_widths
            .iter()
            .map(|width| NormalTask {
                label_max: Some(*width),
                label_width: *width,
            })
            .collect();
        plan.width = COLLAPSED_PAD_X * 2.0
            + fixed
            + dividers
            + plan.tasks.iter().map(|task| task.label_width).sum::<f32>();
        return plan;
    }

    // Find the largest shared label cap that fits the budget: labels get as
    // much room as the pill can offer, and the pill ends up exactly as wide
    // as its content.
    let elided_total = |cap: f32| -> f32 {
        model
            .rows
            .iter()
            .map(|row| measure_elided(ctx, &row.label, medium(12.0), cap))
            .sum()
    };
    let mut low = 22.0_f32;
    let mut high = label_widths.iter().cloned().fold(0.0_f32, f32::max);
    let fits = |cap: f32| fixed + dividers + elided_total(cap) <= budget;
    if fits(low) {
        for _ in 0..10 {
            let middle = (low + high) / 2.0;
            if fits(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        let cap = low;
        let widths: Vec<f32> = model
            .rows
            .iter()
            .map(|row| measure_elided(ctx, &row.label, medium(12.0), cap))
            .collect();
        plan.tasks = widths
            .iter()
            .map(|width| NormalTask {
                label_max: Some(cap),
                label_width: *width,
            })
            .collect();
        plan.width = COLLAPSED_PAD_X * 2.0
            + fixed
            + dividers
            + plan.tasks.iter().map(|task| task.label_width).sum::<f32>();
        return plan;
    }

    // Rings only, shrinking padding and divider gaps until it fits.
    plan.tasks = (0..tasks)
        .map(|_| NormalTask {
            label_max: None,
            label_width: 0.0,
        })
        .collect();
    loop {
        let fixed = tasks as f32 * (ring + plan.pad * 2.0);
        let dividers = (tasks.saturating_sub(1)) as f32 * (plan.divider_gap * 2.0 + 1.0);
        let total = COLLAPSED_PAD_X * 2.0 + fixed + dividers;
        if total <= COLLAPSED_MAX_WIDTH {
            plan.width = total;
            break;
        }
        if plan.divider_gap > DIVIDER_MIN_GAP {
            plan.divider_gap -= 1.0;
        } else if plan.pad > 3.0 {
            plan.pad -= 1.0;
        } else {
            plan.width = COLLAPSED_MAX_WIDTH;
            break;
        }
    }
    plan
}

pub fn pill_radius(pill_size: Vec2) -> f32 {
    (pill_size.y / 2.0).min(MAX_RADIUS)
}

// -- painting ----------------------------------------------------------------

/// Draws the pill at `pill_size` inside the current UI, offset by the shadow
/// margin (`content_trim` lifts it when the OS refused to place the window
/// all the way at the screen top). `actions` receives requests from clicks.
pub fn render_pill(
    ui: &mut egui::Ui,
    model: &UiModel,
    pill_size: Vec2,
    content_trim: f32,
    appear: f32,
    spinner_phase: f32,
    actions: &mut dyn FnMut(Request),
) {
    let margin = SHADOW_MARGIN;
    let rect = Rect::from_min_size(
        Pos2::new(margin, margin - content_trim.clamp(0.0, margin)),
        pill_size,
    );
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
    let border = if pill_response.hovered() && !model.rows.is_empty() {
        skin::LINE_HOVER
    } else {
        skin::LINE
    };
    painter.rect(
        rect,
        radius,
        tint(skin::PILL, appear),
        Stroke::new(1.0_f32, tint(border, appear)),
        StrokeKind::Inside,
    );

    if model.uses_collapsed_layout() {
        match model.collapse_mode {
            CollapseMode::Compact => {
                render_collapsed_compact(ui, &painter, rect, model, appear, spinner_phase);
            }
            CollapseMode::Normal => {
                render_collapsed_normal(ui, &painter, rect, model, appear, spinner_phase);
            }
        }
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

    // The whole pill toggles the collapsed layout; bar controls take priority.
    if pill_response.clicked() && !consumed && !model.rows.is_empty() {
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

    let close_width = if hovered { 20.0 } else { 0.0 };
    let value_width = if row.value_text.is_empty() {
        0.0
    } else {
        measure(ctx, &row.value_text, medium(12.0))
    };
    let label_max_width = (row_rect.width() - value_width - 8.0 - close_width).max(24.0);
    let galley = elide(
        ctx,
        &row.label,
        semibold(12.5),
        tint(skin::TEXT, row_alpha),
        label_max_width,
    );
    painter.galley(
        Pos2::new(row_rect.left(), label_center_y - galley.size().y / 2.0),
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

fn render_collapsed_compact(
    _ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    model: &UiModel,
    appear: f32,
    spinner_phase: f32,
) {
    let center_y = rect.center().y;
    let text = task_count_text(model.live_count);
    let text_width = measure(_ui.ctx(), &text, semibold(12.5));
    let value = model
        .aggregate
        .map(|fraction| format!("{:.0}%", fraction * 100.0));
    let value_width = value
        .as_deref()
        .map(|value| measure(_ui.ctx(), value, medium(12.0)))
        .unwrap_or(0.0);
    let total = COMPACT_RING_D
        + 9.0
        + text_width
        + if value_width > 0.0 {
            8.0 + value_width
        } else {
            0.0
        };
    let start_x = rect.center().x - total / 2.0;

    paint_ring(
        painter,
        Pos2::new(start_x + COMPACT_RING_D / 2.0, center_y),
        COMPACT_RING_D / 2.0 - SPINNER_STROKE / 2.0,
        SPINNER_STROKE,
        model.aggregate,
        spinner_phase,
        tint(model.status_color(), appear),
        tint(skin::TRACK, appear),
    );
    painter.text(
        Pos2::new(start_x + COMPACT_RING_D + 9.0, center_y),
        Align2::LEFT_CENTER,
        text,
        semibold(12.5),
        tint(skin::TEXT, appear),
    );
    if let Some(value) = value {
        painter.text(
            Pos2::new(start_x + total, center_y),
            Align2::RIGHT_CENTER,
            value,
            medium(12.0),
            tint(skin::TEXT_DIM, appear),
        );
    }
}

fn render_collapsed_normal(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    model: &UiModel,
    appear: f32,
    spinner_phase: f32,
) {
    let ctx = ui.ctx();
    let plan = normal_plan(ctx, model);
    let center_y = rect.center().y;

    // Measure the actual content first so the row can be centered.
    let galleys: Vec<Option<Arc<egui::Galley>>> = model
        .rows
        .iter()
        .zip(&plan.tasks)
        .map(|(row, task)| {
            task.label_max.map(|max_width| {
                elide(
                    ctx,
                    &row.label,
                    medium(12.0),
                    tint(skin::TEXT, appear * row.alpha.max(0.2)),
                    max_width,
                )
            })
        })
        .collect();

    let mut total = 0.0;
    for (index, row) in model.rows.iter().enumerate() {
        if index > 0 {
            total += plan.divider_gap * 2.0 + 1.0;
        }
        total += plan.task_ring + plan.pad * 2.0;
        total += galleys[index]
            .as_ref()
            .map(|galley| RING_GAP + galley.size().x)
            .unwrap_or(0.0);
        let _ = row;
    }

    let mut x = rect.center().x - total / 2.0;
    for (index, row) in model.rows.iter().enumerate() {
        if index > 0 {
            let divider_x = x + plan.divider_gap;
            painter.line_segment(
                [
                    Pos2::new(divider_x, center_y - 8.0),
                    Pos2::new(divider_x, center_y + 8.0),
                ],
                Stroke::new(1.0_f32, tint(skin::LINE_HOVER, appear)),
            );
            x = divider_x + 1.0 + plan.divider_gap;
        }
        let row_alpha = appear * row.alpha;
        let fraction = (!matches!(row.kind, BarKind::Indeterminate)).then_some(row.display);
        paint_ring(
            painter,
            Pos2::new(x + plan.pad + plan.task_ring / 2.0, center_y),
            plan.task_ring / 2.0 - SPINNER_STROKE / 2.0,
            SPINNER_STROKE,
            fraction,
            spinner_phase,
            tint(row.color, row_alpha),
            tint(skin::TRACK, row_alpha),
        );
        x += plan.pad + plan.task_ring;
        if let Some(galley) = &galleys[index] {
            x += RING_GAP;
            painter.galley(
                Pos2::new(x, center_y - galley.size().y / 2.0),
                galley.clone(),
                tint(skin::TEXT, row_alpha),
            );
            x += galley.size().x;
        }
        x += plan.pad;
    }
}

fn measure_elided(ctx: &egui::Context, text: &str, font: FontId, max_width: f32) -> f32 {
    ctx.fonts_mut(|fonts| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, skin::TEXT);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(1.0));
        fonts.layout_job(job).size().x
    })
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
                            render_pill(ui, &model, pill_size, 0.0, 1.0, 0.35, &mut |_request| {});
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

    fn three_tasks() -> BarStore {
        let mut store = BarStore::default();
        for (index, (label, color)) in [("Build", "blue"), ("Test", "cyan"), ("Deploy", "green")]
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
        store
    }

    #[test]
    fn collapsed_normal_shows_a_ring_per_task() {
        let mut store = three_tasks();
        store.set_collapsed(Some(true));
        assert_eq!(store.collapse_mode, CollapseMode::Normal);
        snapshot("collapsed-normal", &settled(&mut store));
    }

    #[test]
    fn collapsed_compact_shows_one_ring() {
        let mut store = three_tasks();
        store.set_collapsed(Some(true));
        store.set_collapse_mode(CollapseMode::Compact);
        snapshot("collapsed-compact", &settled(&mut store));
    }

    #[test]
    fn collapsed_normal_pill_hugs_its_content() {
        // Three long labels: the elided layout must not reserve unused width.
        let mut store = BarStore::default();
        for _ in 0..3 {
            let id = store
                .create(None, BarInit::new("Building wire-app"))
                .unwrap();
            store
                .update(
                    &id,
                    BarPatch {
                        percent: Some(40.0),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        store.set_collapsed(Some(true));
        let model = UiModel::collect(&settled(&mut store));

        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), setup_fonts);
        let mut plan = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            plan = Some(normal_plan(ctx, &model));
        });
        let plan = plan.unwrap();

        let content = COLLAPSED_PAD_X * 2.0
            + plan
                .tasks
                .iter()
                .map(|task| {
                    plan.pad * 2.0
                        + plan.task_ring
                        + if task.label_max.is_some() {
                            RING_GAP + task.label_width
                        } else {
                            0.0
                        }
                })
                .sum::<f32>()
            + (plan.tasks.len().saturating_sub(1)) as f32 * (plan.divider_gap * 2.0 + 1.0);
        assert!(
            (plan.width - content).abs() < 0.5,
            "pill width {} should hug content {}",
            plan.width,
            content
        );
        assert!(
            plan.width <= COLLAPSED_MAX_WIDTH,
            "plan must respect the max width"
        );
    }

    #[test]
    fn collapsed_three_long_labels() {
        let mut store = BarStore::default();
        for _ in 0..3 {
            let id = store
                .create(None, BarInit::new("Building wire-app"))
                .unwrap();
            store
                .update(
                    &id,
                    BarPatch {
                        percent: Some(40.0),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        store.set_collapsed(Some(true));
        snapshot("collapsed-three-long", &settled(&mut store));
    }

    #[test]
    fn collapsed_single_bar() {
        let mut store = BarStore::default();
        let id = store
            .create(None, BarInit::new("Building wire-app"))
            .unwrap();
        store
            .update(
                &id,
                BarPatch {
                    percent: Some(62.0),
                    ..Default::default()
                },
            )
            .unwrap();
        store.set_collapsed(Some(true));
        snapshot("collapsed-single", &settled(&mut store));
    }

    #[test]
    fn collapsed_normal_elides_many_labels() {
        let mut store = BarStore::default();
        for index in 0..8 {
            let id = store
                .create(None, BarInit::new(format!("Long task name number {index}")))
                .unwrap();
            store
                .update(
                    &id,
                    BarPatch {
                        percent: Some(10.0 * index as f64),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        store.set_collapsed(Some(true));
        snapshot("collapsed-many", &settled(&mut store));
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
