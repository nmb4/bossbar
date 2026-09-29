//! Bar model and store: the pure, testable heart of the daemon.
//!
//! Nothing here knows about egui. The store owns target values; the animated
//! `display`/`alpha` fields are advanced once per rendered frame via
//! [`BarStore::advance`].

use std::time::{Duration, Instant};

use bossbar_proto::{
    parse_color, Anchor, BarInit, BarKind, BarPatch, BarSnapshot, BarState, BarStatus,
    CollapseMode, MAX_TICKS,
};

/// How long a finished bar celebrates before sliding away.
pub const DEFAULT_FINISH_HOLD: Duration = Duration::from_millis(1600);
/// How long a failed bar stays visible before sliding away.
pub const DEFAULT_FAIL_HOLD: Duration = Duration::from_millis(4200);
/// Exit animation length.
pub const EXIT_ANIM: Duration = Duration::from_millis(220);
/// Text swap animation length: when a label or detail changes, the old text
/// slides up out of its line while the new one rises from below.
pub const TEXT_SWAP_ANIM: Duration = Duration::from_millis(260);
/// Upper bound of simultaneously live bars.
pub const MAX_BARS: usize = 12;
const MAX_LABEL_CHARS: usize = 80;
const MAX_DETAIL_CHARS: usize = 120;
const MAX_ID_CHARS: usize = 32;

pub struct Bar {
    pub id: String,
    pub label: String,
    pub kind: BarKind,
    pub percent: f64,
    pub tick: u32,
    pub status: BarStatus,
    pub detail: Option<String>,
    pub color: Option<[u8; 3]>,
    /// Smoothed fraction currently rendered.
    pub display: f32,
    /// Smoothed opacity, fades in on create and out on removal.
    pub alpha: f32,
    /// Progress of the label swap, from `0.0` (started) to `1.0` (settled).
    pub label_anim: f32,
    /// Label the swap is replacing; cleared once the swap settles.
    pub prev_label: Option<String>,
    /// Progress of the detail swap, from `0.0` (started) to `1.0` (settled).
    pub detail_anim: f32,
    /// Detail the swap is replacing; `None` while the detail is merely
    /// sliding in or once the swap settles.
    pub prev_detail: Option<String>,
    pub leaving_at: Option<Instant>,
    pub hold_until: Option<Instant>,
}

impl Bar {
    pub fn fraction(&self) -> f32 {
        match self.kind {
            BarKind::Percent => (self.percent / 100.0).clamp(0.0, 1.0) as f32,
            BarKind::Ticks { total } => {
                if total == 0 {
                    0.0
                } else {
                    (self.tick.min(total) as f32 / total as f32).clamp(0.0, 1.0)
                }
            }
            BarKind::Indeterminate => self.display.max(0.0),
        }
    }

    pub fn value_text(&self) -> String {
        self.snapshot().value_text()
    }

    pub fn snapshot(&self) -> BarSnapshot {
        BarSnapshot {
            id: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind,
            percent: self.percent,
            tick: self.tick,
            status: self.status,
            detail: self.detail.clone(),
            color: self
                .color
                .map(|rgb| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])),
        }
    }

    fn is_leaving(&self) -> bool {
        self.leaving_at.is_some()
    }
}

pub struct BarStore {
    bars: Vec<Bar>,
    next_id: u64,
    pub collapsed: bool,
    pub collapse_mode: CollapseMode,
    pub anchor: Anchor,
    pub visible: bool,
    /// Adds breathing room around the pill; `false` keeps it flush with the
    /// screen bounds.
    pub padding: bool,
}

impl Default for BarStore {
    fn default() -> Self {
        Self {
            bars: Vec::new(),
            next_id: 1,
            collapsed: false,
            collapse_mode: CollapseMode::default(),
            anchor: Anchor::default(),
            visible: true,
            padding: false,
        }
    }
}

impl BarStore {
    // -- queries ---------------------------------------------------------

    /// Bars that are (still) part of the pill, newest last.
    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }

    /// Bars currently on screen (excludes ones already fading out).
    pub fn live(&self) -> impl Iterator<Item = &Bar> {
        self.bars.iter().filter(|bar| !bar.is_leaving())
    }

    pub fn live_count(&self) -> usize {
        self.live().count()
    }

    /// True when no bars exist at all (not even exit animations).
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    fn find(&self, id: &str) -> Result<usize, String> {
        self.bars
            .iter()
            .position(|bar| bar.id == id && !bar.is_leaving())
            .ok_or_else(|| format!("no bar with id '{id}'"))
    }

    /// Mean of the currently displayed fractions across determinate bars.
    /// This keeps the compact collapsed ring on the same interpolation curve
    /// as the expanded and normal-collapsed layouts.
    pub fn display_aggregate(&self) -> Option<f32> {
        let mut sum = 0.0;
        let mut count = 0;
        for bar in self.live().filter(|bar| bar.kind.is_determinate()) {
            sum += bar.display.clamp(0.0, 1.0);
            count += 1;
        }
        (count > 0).then(|| sum / count as f32)
    }

    pub fn snapshot(&self) -> BarState {
        BarState {
            bars: self
                .bars
                .iter()
                .filter(|bar| !bar.is_leaving())
                .map(Bar::snapshot)
                .collect(),
            collapsed: self.collapsed,
            collapse_mode: self.collapse_mode,
            anchor: self.anchor,
            visible: self.visible,
            padding: self.padding,
        }
    }

    // -- mutations -------------------------------------------------------

    pub fn create(&mut self, id: Option<String>, init: BarInit) -> Result<String, String> {
        if self.live_count() >= MAX_BARS {
            return Err(format!("already showing {MAX_BARS} bars; remove one first"));
        }
        let id = match id {
            Some(custom) => {
                let custom = sanitize_id(&custom)?;
                if self.bars.iter().any(|bar| bar.id == custom) {
                    return Err(format!("id '{custom}' is already in use"));
                }
                custom
            }
            None => loop {
                let candidate = format!("b{}", self.next_id);
                self.next_id += 1;
                if !self.bars.iter().any(|bar| bar.id == candidate) {
                    break candidate;
                }
            },
        };

        let kind = normalize_kind(init.kind);
        let total = match kind {
            BarKind::Ticks { total } => total,
            _ => 0,
        };
        let color = match init.color.as_deref() {
            Some(value) => {
                Some(parse_color(value).ok_or_else(|| format!("unknown color '{value}'"))?)
            }
            None => None,
        };

        self.bars.push(Bar {
            id: id.clone(),
            label: sanitize_text(&init.label, MAX_LABEL_CHARS, "Untitled"),
            kind,
            percent: init.percent.clamp(0.0, 100.0),
            tick: init.tick.min(total),
            status: BarStatus::Running,
            detail: init
                .detail
                .map(|detail| sanitize_text(&detail, MAX_DETAIL_CHARS, "")),
            color,
            display: 0.0,
            alpha: 0.0,
            label_anim: 1.0,
            prev_label: None,
            detail_anim: 1.0,
            prev_detail: None,
            leaving_at: None,
            hold_until: None,
        });
        Ok(id)
    }

    pub fn update(&mut self, id: &str, patch: BarPatch) -> Result<(), String> {
        let index = self.find(id)?;
        let bar = &mut self.bars[index];

        if let Some(label) = patch.label {
            let fallback = bar.label.clone();
            let next = sanitize_text(&label, MAX_LABEL_CHARS, &fallback);
            if next != bar.label {
                bar.prev_label = Some(std::mem::replace(&mut bar.label, next));
                bar.label_anim = 0.0;
            }
        }
        if let Some(color) = patch.color.as_deref() {
            bar.color = Some(parse_color(color).ok_or_else(|| format!("unknown color '{color}'"))?);
        }
        if let Some(detail) = patch.detail {
            let sanitized = sanitize_text(&detail, MAX_DETAIL_CHARS, "");
            set_detail(bar, (!sanitized.is_empty()).then_some(sanitized));
        }
        if let Some(kind) = patch.kind {
            bar.kind = normalize_kind(kind);
            if bar.kind == BarKind::Indeterminate {
                bar.display = 0.0;
            }
        }
        if let Some(percent) = patch.percent {
            bar.percent = percent.clamp(0.0, 100.0);
            if bar.kind == BarKind::Indeterminate {
                bar.kind = BarKind::Percent;
            }
        }
        if let Some(tick) = patch.tick {
            if let BarKind::Ticks { total } = bar.kind {
                bar.tick = tick.min(total);
            } else {
                return Err(format!("bar '{id}' is not a ticks bar"));
            }
        }
        if let Some(status) = patch.status {
            match status {
                BarStatus::Done => {
                    bar.status = BarStatus::Done;
                    bar.percent = 100.0;
                    if let BarKind::Ticks { total } = bar.kind {
                        bar.tick = total;
                    }
                    bar.hold_until = Some(Instant::now() + DEFAULT_FINISH_HOLD);
                }
                BarStatus::Failed => {
                    bar.status = BarStatus::Failed;
                    bar.hold_until = Some(Instant::now() + DEFAULT_FAIL_HOLD);
                }
                BarStatus::Running => {
                    bar.status = BarStatus::Running;
                    bar.hold_until = None;
                }
            }
        }
        Ok(())
    }

    pub fn tick(&mut self, id: &str, by: Option<i64>, to: Option<u32>) -> Result<(), String> {
        let index = self.find(id)?;
        let bar = &mut self.bars[index];
        let BarKind::Ticks { total } = bar.kind else {
            return Err(format!("bar '{id}' is not a ticks bar"));
        };
        let next = match to {
            Some(to) => to as i64,
            None => bar.tick as i64 + by.unwrap_or(1),
        };
        bar.tick = next.clamp(0, total as i64) as u32;
        Ok(())
    }

    pub fn finish(&mut self, id: &str, hold: Duration) -> Result<(), String> {
        let index = self.find(id)?;
        let bar = &mut self.bars[index];
        bar.status = BarStatus::Done;
        bar.percent = 100.0;
        if let BarKind::Ticks { total } = bar.kind {
            bar.tick = total;
        }
        if bar.kind == BarKind::Indeterminate {
            bar.kind = BarKind::Percent;
        }
        bar.hold_until = Some(Instant::now() + hold);
        Ok(())
    }

    pub fn fail(
        &mut self,
        id: &str,
        message: Option<String>,
        hold: Duration,
    ) -> Result<(), String> {
        let index = self.find(id)?;
        let bar = &mut self.bars[index];
        bar.status = BarStatus::Failed;
        if let Some(message) = message {
            let sanitized = sanitize_text(&message, MAX_DETAIL_CHARS, "");
            set_detail(bar, (!sanitized.is_empty()).then_some(sanitized));
        }
        bar.hold_until = Some(Instant::now() + hold);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let index = self.find(id)?;
        self.bars[index].leaving_at = Some(Instant::now());
        Ok(())
    }

    pub fn clear(&mut self) {
        let now = Instant::now();
        for bar in self.live_mut() {
            bar.leaving_at = Some(now);
        }
    }

    fn live_mut(&mut self) -> impl Iterator<Item = &mut Bar> {
        self.bars.iter_mut().filter(|bar| !bar.is_leaving())
    }

    pub fn set_collapsed(&mut self, value: Option<bool>) {
        self.collapsed = value.unwrap_or(!self.collapsed);
    }

    pub fn set_anchor(&mut self, anchor: Anchor) {
        self.anchor = anchor;
    }

    pub fn set_collapse_mode(&mut self, mode: CollapseMode) {
        self.collapse_mode = mode;
    }

    pub fn set_padding(&mut self, value: Option<bool>) {
        self.padding = value.unwrap_or(!self.padding);
    }

    pub fn set_visible(&mut self, value: Option<bool>) {
        self.visible = value.unwrap_or(!self.visible);
    }

    // -- animation -------------------------------------------------------

    /// Advances smoothing and hold timers, and reaps finished exit
    /// animations. Call once per frame.
    pub fn advance(&mut self, now: Instant, dt: f32) {
        let follow_alpha = 1.0 - (-13.0 * dt).exp();
        let follow_display = 1.0 - (-9.0 * dt).exp();
        for bar in &mut self.bars {
            if bar.leaving_at.is_none() && bar.hold_until.is_some_and(|until| now >= until) {
                bar.leaving_at = Some(now);
            }
            let alpha_target = if bar.is_leaving() { 0.0 } else { 1.0 };
            bar.alpha += (alpha_target - bar.alpha) * follow_alpha;
            let swap_step = dt / TEXT_SWAP_ANIM.as_secs_f32();
            if bar.label_anim < 1.0 {
                bar.label_anim = (bar.label_anim + swap_step).min(1.0);
                if bar.label_anim >= 1.0 {
                    bar.prev_label = None;
                }
            }
            if bar.detail_anim < 1.0 {
                bar.detail_anim = (bar.detail_anim + swap_step).min(1.0);
                if bar.detail_anim >= 1.0 {
                    bar.prev_detail = None;
                }
            }
            if bar.leaving_at.is_none() {
                let target = match bar.kind {
                    BarKind::Indeterminate => bar.display,
                    _ => bar.fraction(),
                };
                bar.display += (target - bar.display) * follow_display;
                if (target - bar.display).abs() < 0.001 {
                    bar.display = target;
                }
            }
        }
        self.bars.retain(|bar| {
            !bar.leaving_at
                .is_some_and(|started| now.saturating_duration_since(started) >= EXIT_ANIM)
        });
    }

    /// When the next frame is needed, or `None` if the pill has settled.
    pub fn next_repaint(&self, now: Instant) -> Option<Duration> {
        let mut next: Option<Duration> = None;
        let mut propose = |candidate: Duration| {
            next = Some(next.map_or(candidate, |current| current.min(candidate)));
        };
        for bar in &self.bars {
            if bar.alpha < 0.995 || bar.leaving_at.is_some() {
                propose(Duration::ZERO);
                continue;
            }
            let target = match bar.kind {
                BarKind::Indeterminate => bar.display,
                _ => bar.fraction(),
            };
            if (target - bar.display).abs() > 0.001 {
                propose(Duration::ZERO);
            }
            if bar.label_anim < 1.0 || bar.detail_anim < 1.0 {
                propose(Duration::ZERO);
            }
            if let Some(hold) = bar.hold_until {
                propose(hold.saturating_duration_since(now));
            }
        }
        next
    }
}

fn normalize_kind(kind: BarKind) -> BarKind {
    match kind {
        BarKind::Ticks { total } => BarKind::Ticks {
            total: total.clamp(1, MAX_TICKS),
        },
        other => other,
    }
}

/// Replaces a bar's detail text, starting the swap animation when there is an
/// old text to slide out. A detail that appears slides in on its own; one that
/// is removed takes its line with it immediately.
fn set_detail(bar: &mut Bar, next: Option<String>) {
    if bar.detail == next {
        return;
    }
    if next.is_none() {
        bar.detail = None;
        bar.prev_detail = None;
        bar.detail_anim = 1.0;
        return;
    }
    bar.prev_detail = bar.detail.take();
    bar.detail = next;
    bar.detail_anim = 0.0;
}

fn sanitize_text(value: &str, max_chars: usize, fallback: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return fallback.to_owned();
    }
    trimmed.chars().take(max_chars).collect()
}

fn sanitize_id(value: &str) -> Result<String, String> {
    let cleaned: String = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .take(MAX_ID_CHARS)
        .collect();
    if cleaned.is_empty() {
        Err("bar ids must contain letters or digits".to_owned())
    } else {
        Ok(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bossbar_proto::BarInit;

    fn percent_bar(label: &str) -> BarInit {
        BarInit::new(label)
    }

    #[test]
    fn create_assigns_stable_ids_and_clamps_values() {
        let mut store = BarStore::default();
        let first = store.create(None, percent_bar("build")).unwrap();
        let second = store.create(None, percent_bar("test")).unwrap();
        assert_eq!(first, "b1");
        assert_eq!(second, "b2");

        let id = store
            .create(
                Some("custom! id".into()),
                BarInit {
                    label: "  spaced  ".into(),
                    kind: BarKind::Ticks { total: 5000 },
                    percent: 0.0,
                    tick: 9999,
                    detail: None,
                    color: Some("blue".into()),
                },
            )
            .unwrap();
        assert_eq!(id, "customid");
        let bar = store.bars().iter().find(|bar| bar.id == id).unwrap();
        assert_eq!(bar.label, "spaced");
        assert_eq!(bar.kind, BarKind::Ticks { total: MAX_TICKS });
        assert_eq!(bar.tick, MAX_TICKS);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut store = BarStore::default();
        store
            .create(Some("build".into()), percent_bar("one"))
            .unwrap();
        let error = store
            .create(Some("build".into()), percent_bar("two"))
            .unwrap_err();
        assert!(error.contains("already in use"));
    }

    #[test]
    fn tick_clamps_and_rejects_percent_bars() {
        let mut store = BarStore::default();
        let id = store
            .create(
                None,
                BarInit {
                    label: "compile".into(),
                    kind: BarKind::Ticks { total: 8 },
                    percent: 0.0,
                    tick: 0,
                    detail: None,
                    color: None,
                },
            )
            .unwrap();

        store.tick(&id, Some(3), None).unwrap();
        assert_eq!(store.bars()[0].tick, 3);
        store.tick(&id, Some(-10), None).unwrap();
        assert_eq!(store.bars()[0].tick, 0);
        store.tick(&id, Some(100), None).unwrap();
        assert_eq!(store.bars()[0].tick, 8);
        store.tick(&id, None, Some(5)).unwrap();
        assert_eq!(store.bars()[0].tick, 5);

        let percent = store.create(None, percent_bar("build")).unwrap();
        assert!(store.tick(&percent, Some(1), None).is_err());
    }

    #[test]
    fn finish_fills_and_hold_expires_into_exit() {
        let mut store = BarStore::default();
        let id = store
            .create(
                None,
                BarInit {
                    kind: BarKind::Ticks { total: 10 },
                    ..percent_bar("compile")
                },
            )
            .unwrap();
        store.finish(&id, Duration::from_millis(50)).unwrap();
        assert_eq!(store.bars()[0].status, BarStatus::Done);
        assert_eq!(store.bars()[0].tick, 10);

        let now = Instant::now();
        store.advance(now + Duration::from_millis(60), 0.05);
        assert!(store.bars()[0].leaving_at.is_some(), "hold should expire");

        store.advance(
            now + Duration::from_millis(60) + EXIT_ANIM + Duration::from_millis(10),
            0.05,
        );
        assert!(store.is_empty(), "exit animation should reap the bar");
    }

    #[test]
    fn update_status_transitions() {
        let mut store = BarStore::default();
        let id = store.create(None, percent_bar("build")).unwrap();
        store
            .update(
                &id,
                BarPatch {
                    percent: Some(42.0),
                    status: Some(BarStatus::Running),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.bars()[0].percent, 42.0);

        store
            .update(
                &id,
                BarPatch {
                    status: Some(BarStatus::Failed),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.bars()[0].status, BarStatus::Failed);
        assert!(store.bars()[0].hold_until.is_some());
    }

    #[test]
    fn label_change_swaps_the_text_through_an_animation() {
        let mut store = BarStore::default();
        let id = store.create(None, percent_bar("first")).unwrap();
        assert_eq!(store.bars()[0].label_anim, 1.0);

        store
            .update(
                &id,
                BarPatch {
                    label: Some("second".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let bar = &store.bars()[0];
        assert_eq!(bar.label, "second");
        assert_eq!(bar.prev_label.as_deref(), Some("first"));
        assert_eq!(bar.label_anim, 0.0);

        let now = Instant::now();
        assert_eq!(
            store.next_repaint(now),
            Some(Duration::ZERO),
            "a swap in flight needs frames"
        );
        store.advance(now, TEXT_SWAP_ANIM.as_secs_f32() / 2.0);
        assert!(store.bars()[0].label_anim > 0.0 && store.bars()[0].label_anim < 1.0);
        store.advance(now, TEXT_SWAP_ANIM.as_secs_f32() / 2.0 + 0.01);
        assert_eq!(store.bars()[0].label_anim, 1.0);
        assert!(store.bars()[0].prev_label.is_none());

        // Updating to the same (sanitized) label must not restart the swap.
        store
            .update(
                &id,
                BarPatch {
                    label: Some("  second  ".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.bars()[0].label_anim, 1.0);
        assert!(store.bars()[0].prev_label.is_none());
    }

    #[test]
    fn detail_change_swaps_the_text_through_an_animation() {
        let mut store = BarStore::default();
        let id = store.create(None, percent_bar("build")).unwrap();
        assert_eq!(store.bars()[0].detail_anim, 1.0);

        // The first detail slides in without an outgoing text.
        store
            .update(
                &id,
                BarPatch {
                    detail: Some("linking".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let bar = &store.bars()[0];
        assert_eq!(bar.detail.as_deref(), Some("linking"));
        assert!(bar.prev_detail.is_none());
        assert_eq!(bar.detail_anim, 0.0);

        let now = Instant::now();
        assert_eq!(
            store.next_repaint(now),
            Some(Duration::ZERO),
            "a swap in flight needs frames"
        );
        store.advance(now, TEXT_SWAP_ANIM.as_secs_f32() + 0.01);
        assert_eq!(store.bars()[0].detail_anim, 1.0);

        // Changing it swaps the old text out.
        store
            .update(
                &id,
                BarPatch {
                    detail: Some("compiling".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let bar = &store.bars()[0];
        assert_eq!(bar.detail.as_deref(), Some("compiling"));
        assert_eq!(bar.prev_detail.as_deref(), Some("linking"));
        assert_eq!(bar.detail_anim, 0.0);

        // Removing the detail ends the swap without one.
        store
            .update(
                &id,
                BarPatch {
                    detail: Some("  ".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let bar = &store.bars()[0];
        assert_eq!(bar.detail, None);
        assert!(bar.prev_detail.is_none());
        assert_eq!(bar.detail_anim, 1.0);
    }

    #[test]
    fn display_aggregate_averages_determinate_bars_only() {
        let mut store = BarStore::default();
        assert_eq!(store.display_aggregate(), None);

        let a = store.create(None, percent_bar("a")).unwrap();
        let b = store
            .create(
                None,
                BarInit {
                    kind: BarKind::Ticks { total: 4 },
                    ..percent_bar("b")
                },
            )
            .unwrap();
        store
            .create(
                None,
                BarInit {
                    kind: BarKind::Indeterminate,
                    ..percent_bar("c")
                },
            )
            .unwrap();

        store
            .update(
                &a,
                BarPatch {
                    percent: Some(50.0),
                    ..Default::default()
                },
            )
            .unwrap();
        store.tick(&b, None, Some(1)).unwrap();
        assert_eq!(store.display_aggregate(), Some(0.0));

        store.advance(Instant::now(), 0.016);
        let displayed = store.display_aggregate().unwrap();
        assert!(displayed > 0.0 && displayed < 0.375);

        for _ in 0..200 {
            store.advance(Instant::now(), 0.016);
        }
        assert_eq!(store.display_aggregate(), Some(0.375));
    }

    #[test]
    fn clear_snapshot_and_live_count() {
        let mut store = BarStore::default();
        store.create(None, percent_bar("one")).unwrap();
        store.create(None, percent_bar("two")).unwrap();
        assert_eq!(store.live_count(), 2);
        assert_eq!(store.snapshot().bars.len(), 2);

        store.clear();
        assert_eq!(store.live_count(), 0);
        assert!(
            store.snapshot().bars.is_empty(),
            "leaving bars leave the snapshot"
        );
        assert!(
            !store.is_empty(),
            "leaving bars animate before being reaped"
        );
    }

    #[test]
    fn collapse_and_visible_toggle() {
        let mut store = BarStore::default();
        assert!(!store.collapsed);
        store.set_collapsed(None);
        assert!(store.collapsed);
        store.set_collapsed(Some(false));
        assert!(!store.collapsed);
        assert!(store.visible);
        store.set_visible(None);
        assert!(!store.visible);
    }

    #[test]
    fn collapse_mode_defaults_to_normal_and_updates() {
        let mut store = BarStore::default();
        assert_eq!(store.snapshot().collapse_mode, CollapseMode::Normal);
        store.set_collapse_mode(CollapseMode::Compact);
        assert_eq!(store.snapshot().collapse_mode, CollapseMode::Compact);
    }

    #[test]
    fn padding_starts_off_and_toggles() {
        let mut store = BarStore::default();
        assert!(!store.snapshot().padding, "no padding is the default");
        store.set_padding(None);
        assert!(store.padding);
        store.set_padding(Some(false));
        assert!(!store.padding);
    }

    #[test]
    fn unknown_colors_are_rejected() {
        let mut store = BarStore::default();
        let error = store
            .create(
                None,
                BarInit {
                    color: Some("chartreuse".into()),
                    ..percent_bar("build")
                },
            )
            .unwrap_err();
        assert!(error.contains("chartreuse"));
    }

    #[test]
    fn next_repaint_is_idle_when_settled() {
        let mut store = BarStore::default();
        let id = store.create(None, percent_bar("build")).unwrap();
        store
            .update(
                &id,
                BarPatch {
                    percent: Some(50.0),
                    ..Default::default()
                },
            )
            .unwrap();
        let now = Instant::now();
        assert_eq!(
            store.next_repaint(now),
            Some(Duration::ZERO),
            "active interpolation should request the next presented frame"
        );
        // Settle alpha and display.
        for _ in 0..200 {
            store.advance(now, 0.016);
        }
        assert!(store.next_repaint(now).is_none());
    }
}
