//! Touch-friendly floating editor windows for Android (`NativeActivity`) and desktop.
//!
//! Because Android's single-window `NativeActivity` cannot spawn OS-level child
//! viewports, editor panels (Explorer, Properties, Output, Toolbox, Terrain
//! Editor, Asset Manager, File Sync, Avatar Settings, MicroProfiler, Scripts,
//! Plugins) are hosted inside `egui::Area` layers on `egui::Order::Middle`.
//!
//! Each floating window supports:
//! - Dragging via a touch-sized title bar (`34.0` pt touch / `28.0` pt desktop).
//! - Tap-to-collapse on the title bar (with `<= 10.0` pt touch slop and
//!   exclusion of title-bar action buttons), collapsing into a compact title
//!   strip (`150..=240` pt wide) at the exact same position while preserving
//!   expanded dimensions and panel identity.
//! - Resizing via a `22.0` pt bottom-right touch grip plus right/bottom edges,
//!   enforcing `min_size` and screen bounds.
//! - Automatic clamping to usable screen bounds so title bars can never escape
//!   off-screen across drags, resizes, orientation changes, or zoom changes.
//! - Z-order focus promotion on interaction and deterministic back-to-front
//!   rendering.
//! - Non-overlapping default placement so Output, Explorer, and Properties can
//!   coexist cleanly around the 3D viewport.

use std::collections::HashMap;

use egui::{
    Align2, CornerRadius, CursorIcon, FontId, Id, Order, Pos2, Rect, Sense, Stroke,
    StrokeKind, UiBuilder, Vec2,
};
use rustudio_engine::theme;
use serde::{Deserialize, Serialize};

use crate::app::Tab;
use crate::layout_store::SavedTab;

/// Title-bar height in touch / Android mode (logical points).
pub const TITLE_BAR_HEIGHT_TOUCH: f32 = 34.0;
/// Title-bar height in desktop / pointer mode (logical points).
pub const TITLE_BAR_HEIGHT_DESKTOP: f32 = 28.0;
/// Maximum pointer travel (in logical points) still treated as a title-bar tap
/// rather than a drag gesture.
pub const TITLE_TAP_SLOP: f32 = 10.0;
/// Minimum width of a collapsed title strip.
pub const COLLAPSED_MIN_WIDTH: f32 = 150.0;
/// Maximum width of a collapsed title strip.
pub const COLLAPSED_MAX_WIDTH: f32 = 240.0;
/// Default minimum expanded window dimensions (`[width, height]`).
pub const DEFAULT_MIN_SIZE: [f32; 2] = [220.0, 140.0];
/// Touch-friendly bottom-right corner resize hit target size.
pub const RESIZE_CORNER_HIT: f32 = 36.0;
/// Edge resize strip thickness in touch mode.
pub const RESIZE_EDGE_HIT_TOUCH: f32 = 18.0;
/// Edge resize strip thickness in desktop mode.
pub const RESIZE_EDGE_HIT_DESKTOP: f32 = 8.0;

const WINDOW_CORNER_RADIUS: u8 = 6;
const PLACEMENT_MARGIN: f32 = 10.0;
const PLACEMENT_GAP: f32 = 8.0;

fn default_min_size() -> [f32; 2] {
    DEFAULT_MIN_SIZE
}

/// Returns the title-bar height for the current input mode.
pub fn title_bar_height(touch_mode: bool) -> f32 {
    if touch_mode {
        TITLE_BAR_HEIGHT_TOUCH
    } else {
        TITLE_BAR_HEIGHT_DESKTOP
    }
}

/// Returns the compact strip width when a window of `expanded_width` is collapsed.
pub fn collapsed_width(expanded_width: f32) -> f32 {
    let w = if expanded_width.is_finite() {
        expanded_width
    } else {
        COLLAPSED_MAX_WIDTH
    };
    w.clamp(COLLAPSED_MIN_WIDTH, COLLAPSED_MAX_WIDTH)
}

/// Stable floating-window identifier for a [`Tab`].
///
/// Built-in panels use their canonical [`Tab::key`] (`"explorer"`, `"output"`,
/// `"properties"`, etc.). Script tabs include their instance `Ref` so multiple
/// scripts can float concurrently.
pub fn window_id_for_tab(tab: &Tab) -> String {
    match tab {
        Tab::Script(r) => format!("script:{r:?}"),
        other => other.key().to_string(),
    }
}

/// Distinguishes a title-bar tap (toggle collapse) from a drag or a click on
/// title-bar action buttons (Dock / Close).
pub fn is_title_tap(
    press_origin: Pos2,
    release_pos: Pos2,
    total_drag_distance: f32,
    button_rects: &[Rect],
) -> bool {
    if !press_origin.is_finite()
        || !release_pos.is_finite()
        || !total_drag_distance.is_finite()
    {
        return false;
    }
    if button_rects
        .iter()
        .any(|r| r.contains(press_origin) || r.contains(release_pos))
    {
        return false;
    }
    press_origin.distance(release_pos) <= TITLE_TAP_SLOP && total_drag_distance <= TITLE_TAP_SLOP
}

/// Runtime state of a single floating editor window.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatingWindowState {
    /// Stable window key (e.g. `"output"`, `"properties"`, `"script:..."`).
    pub id: String,
    /// Hosted editor tab.
    pub tab: Tab,
    /// Display title shown in the title bar.
    pub title: String,
    /// Top-left position in logical points.
    pub pos: [f32; 2],
    /// Expanded dimensions (`[width, height]`), preserved across collapse/expand.
    pub size: [f32; 2],
    /// Minimum expanded dimensions (`[width, height]`).
    pub min_size: [f32; 2],
    /// Whether the window is currently collapsed to a compact title strip.
    pub collapsed: bool,
    /// Whether the window is minimized to the bottom minimized bar.
    pub minimized: bool,
    /// Monotonically increasing focus order (higher renders on top).
    pub z_order: u64,
}

impl FloatingWindowState {
    /// Current visible dimensions, accounting for `collapsed` state.
    pub fn visible_size(&self, title_h: f32) -> Vec2 {
        if self.collapsed {
            Vec2::new(collapsed_width(self.size[0]), title_h)
        } else {
            Vec2::new(self.size[0], self.size[1])
        }
    }

    /// Current visible screen rectangle, accounting for `collapsed` state.
    pub fn visible_rect(&self, title_h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.pos[0], self.pos[1]), self.visible_size(title_h))
    }

    /// Title-bar rectangle in screen coordinates.
    pub fn title_rect(&self, title_h: f32) -> Rect {
        let vis = self.visible_size(title_h);
        Rect::from_min_size(Pos2::new(self.pos[0], self.pos[1]), Vec2::new(vis.x, title_h))
    }

    /// Body rectangle in screen coordinates, or `None` when collapsed.
    pub fn body_rect(&self, title_h: f32) -> Option<Rect> {
        if self.collapsed {
            return None;
        }
        let min = Pos2::new(self.pos[0], self.pos[1] + title_h);
        let max = Pos2::new(self.pos[0] + self.size[0], self.pos[1] + self.size[1]);
        if max.x > min.x && max.y > min.y {
            Some(Rect::from_min_max(min, max))
        } else {
            None
        }
    }

    /// Clamps this window's size and position so its visible rect (expanded or
    /// collapsed) stays completely inside `bounds`.
    pub fn clamp_to_bounds(&mut self, bounds: Rect, title_h: f32) {
        if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        let title_h = if title_h.is_finite() && title_h > 0.0 {
            title_h
        } else {
            TITLE_BAR_HEIGHT_DESKTOP
        };

        // Sanitize min_size.
        let min_w = if self.min_size[0].is_finite() {
            self.min_size[0].max(120.0)
        } else {
            DEFAULT_MIN_SIZE[0]
        };
        let min_h = if self.min_size[1].is_finite() {
            self.min_size[1].max(title_h)
        } else {
            DEFAULT_MIN_SIZE[1].max(title_h)
        };
        self.min_size = [min_w, min_h];

        // Sanitize & clamp expanded size to fit within bounds.
        let eff_min_w = min_w.min(bounds.width());
        let eff_min_h = min_h.min(bounds.height());
        let raw_w = if self.size[0].is_finite() {
            self.size[0]
        } else {
            DEFAULT_MIN_SIZE[0]
        };
        let raw_h = if self.size[1].is_finite() {
            self.size[1]
        } else {
            DEFAULT_MIN_SIZE[1]
        };
        self.size[0] = raw_w.clamp(eff_min_w, bounds.width());
        self.size[1] = raw_h.clamp(eff_min_h, bounds.height());

        // Clamp position against the currently visible dimensions so the title
        // bar can never escape off-screen.
        let vis_w = if self.collapsed {
            collapsed_width(self.size[0]).min(bounds.width())
        } else {
            self.size[0]
        };
        let vis_h = if self.collapsed {
            title_h.min(bounds.height())
        } else {
            self.size[1]
        };

        let raw_x = if self.pos[0].is_finite() {
            self.pos[0]
        } else {
            bounds.min.x
        };
        let raw_y = if self.pos[1].is_finite() {
            self.pos[1]
        } else {
            bounds.min.y
        };
        let max_x = (bounds.max.x - vis_w).max(bounds.min.x);
        let max_y = (bounds.max.y - vis_h).max(bounds.min.y);
        self.pos[0] = raw_x.clamp(bounds.min.x, max_x);
        self.pos[1] = raw_y.clamp(bounds.min.y, max_y);
    }

    /// Moves the window by `delta` and clamps it inside `bounds`.
    pub fn drag_by(&mut self, delta: Vec2, bounds: Rect, title_h: f32) {
        if !delta.is_finite() {
            return;
        }
        self.pos[0] += delta.x;
        self.pos[1] += delta.y;
        self.clamp_to_bounds(bounds, title_h);
    }

    /// Resizes an expanded window by `delta`, enforcing `min_size` and `bounds`.
    /// No-op while collapsed.
    pub fn resize_by(&mut self, delta: Vec2, bounds: Rect, title_h: f32) {
        if self.collapsed
            || !delta.is_finite()
            || !bounds.is_finite()
            || bounds.width() <= 0.0
            || bounds.height() <= 0.0
        {
            return;
        }
        let max_w = (bounds.max.x - self.pos[0]).clamp(0.0, bounds.width());
        let max_h = (bounds.max.y - self.pos[1]).clamp(0.0, bounds.height());
        let min_w = self.min_size[0].min(max_w);
        let min_h = self.min_size[1].max(title_h).min(max_h);

        self.size[0] = (self.size[0] + delta.x).clamp(min_w, max_w);
        self.size[1] = (self.size[1] + delta.y).clamp(min_h, max_h);
        self.clamp_to_bounds(bounds, title_h);
    }
}

/// Serializable representation of a floating window for `layout.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedFloatingWindow {
    pub id: String,
    pub tab: SavedTab,
    pub title: String,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    #[serde(default = "default_min_size")]
    pub min_size: [f32; 2],
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default)]
    pub z_order: u64,
}

/// Action emitted by [`MobileWindowManager::show`] when the user docks back or
/// closes a floating window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileWindowAction {
    /// User tapped the Dock-back button to return the tab to the main dock.
    DockBack(Tab),
    /// User tapped the Close button (`×`).
    Closed(Tab),
}

/// Computed sub-rectangles of a window's title bar.
#[derive(Debug, Clone, Copy)]
pub struct TitleBarLayout {
    pub title_rect: Rect,
    pub chevron_rect: Rect,
    pub title_text_rect: Rect,
    pub drag_rect: Rect,
    pub min_rect: Rect,
    pub dock_rect: Rect,
    pub close_rect: Rect,
}

impl TitleBarLayout {
    /// Computes deterministic touch- or desktop-sized title-bar hit rectangles.
    pub fn compute(title_rect: Rect, touch_mode: bool) -> Self {
        let close_w = if touch_mode { 32.0 } else { 26.0 };
        let dock_w = if touch_mode { 46.0 } else { 40.0 };
        let min_w = if touch_mode { 32.0 } else { 26.0 };
        let pad_x = 4.0;
        let pad_y = 3.0;
        let gap = 3.0;

        let btn_top = title_rect.min.y + pad_y;
        let btn_bottom = (title_rect.max.y - pad_y).max(btn_top + 14.0);

        let close_max_x = (title_rect.max.x - pad_x).max(title_rect.min.x);
        let close_min_x = (close_max_x - close_w).max(title_rect.min.x);
        let close_rect =
            Rect::from_min_max(Pos2::new(close_min_x, btn_top), Pos2::new(close_max_x, btn_bottom));

        let dock_max_x = (close_rect.min.x - gap).max(title_rect.min.x);
        let dock_min_x = (dock_max_x - dock_w).max(title_rect.min.x);
        let dock_rect =
            Rect::from_min_max(Pos2::new(dock_min_x, btn_top), Pos2::new(dock_max_x, btn_bottom));

        let min_max_x = (dock_rect.min.x - gap).max(title_rect.min.x);
        let min_min_x = (min_max_x - min_w).max(title_rect.min.x);
        let min_rect =
            Rect::from_min_max(Pos2::new(min_min_x, btn_top), Pos2::new(min_max_x, btn_bottom));

        let chevron_min_x = title_rect.min.x + 6.0;
        let chevron_max_x = (chevron_min_x + 16.0).min(min_rect.min.x);
        let chevron_rect = Rect::from_min_max(
            Pos2::new(chevron_min_x, title_rect.min.y),
            Pos2::new(chevron_max_x.max(chevron_min_x), title_rect.max.y),
        );

        let drag_max_x = (min_rect.min.x - 2.0).max(title_rect.min.x);
        let drag_rect = Rect::from_min_max(
            title_rect.min,
            Pos2::new(drag_max_x, title_rect.max.y),
        );

        let text_min_x = (chevron_rect.max.x + 4.0).min(drag_max_x);
        let text_max_x = (min_rect.min.x - 6.0).max(text_min_x);
        let title_text_rect = Rect::from_min_max(
            Pos2::new(text_min_x, title_rect.min.y),
            Pos2::new(text_max_x, title_rect.max.y),
        );

        Self {
            title_rect,
            chevron_rect,
            title_text_rect,
            drag_rect,
            min_rect,
            dock_rect,
            close_rect,
        }
    }

    /// Action button rects excluded from title-bar tap-to-collapse.
    pub fn button_rects(&self) -> [Rect; 3] {
        [self.min_rect, self.dock_rect, self.close_rect]
    }
}

#[derive(Debug, Clone, Copy)]
struct TitlePressState {
    press_origin: Pos2,
    start_window_pos: [f32; 2],
    accumulated_dist: f32,
    moved_beyond_slop: bool,
}

/// Computes a clean, non-overlapping default `(pos, size)` for a floating panel
/// inside `bounds` given any `existing` floating windows.
///
/// Layout conventions:
/// - `explorer`: top-right column.
/// - `properties`, `file_sync`: bottom-right column (below Explorer).
/// - `output`, `avatar_settings`, `asset_manager`, `microprofiler`: bottom-left /
///   bottom-center, leaving the right column and upper 3D viewport clear.
/// - `toolbox`, `terrain_editor`: top-left column, staying above Output.
/// - `script` / plugin / other panels: centered-left with cascading offset.
pub fn default_placement_for(
    tab_key: &str,
    bounds: Rect,
    existing: &[FloatingWindowState],
) -> (Pos2, Vec2) {
    if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return (Pos2::ZERO, Vec2::new(DEFAULT_MIN_SIZE[0], DEFAULT_MIN_SIZE[1]));
    }

    let w_avail = (bounds.width() - 2.0 * PLACEMENT_MARGIN).max(DEFAULT_MIN_SIZE[0].min(bounds.width()));
    let h_avail = (bounds.height() - 2.0 * PLACEMENT_MARGIN).max(DEFAULT_MIN_SIZE[1].min(bounds.height()));

    let right_col_w = (bounds.width() * 0.28).clamp(240.0, 340.0).min(w_avail);
    let left_col_w = (bounds.width() * 0.26).clamp(240.0, 320.0).min(w_avail);
    let bottom_h = (bounds.height() * 0.28).clamp(140.0, 240.0).min(h_avail);
    let right_top_h = ((h_avail - PLACEMENT_GAP) * 0.48)
        .clamp(150.0, 360.0)
        .min(h_avail);
    let right_bot_h = (h_avail - right_top_h - PLACEMENT_GAP)
        .clamp(150.0, 360.0)
        .min(h_avail);

    let (pref_pos, pref_size) = match tab_key {
        "explorer" => {
            let size = Vec2::new(right_col_w, right_top_h);
            let pos = Pos2::new(bounds.max.x - PLACEMENT_MARGIN - size.x, bounds.min.y + PLACEMENT_MARGIN);
            (pos, size)
        }
        "properties" | "file_sync" => {
            // Stack cleanly below an existing Explorer window if present, otherwise
            // anchor to the bottom-right so Explorer can still open above it.
            let explorer_bottom = existing
                .iter()
                .filter(|w| !w.minimized)
                .find(|w| w.id == "explorer" || w.tab.key() == "explorer")
                .map(|w| w.pos[1] + w.size[1]);
            let (y, h) = if let Some(exp_bot) = explorer_bottom {
                let top = exp_bot + PLACEMENT_GAP;
                let rem = (bounds.max.y - PLACEMENT_MARGIN - top).max(DEFAULT_MIN_SIZE[1].min(h_avail));
                let h = rem.clamp(140.0, 380.0).min(h_avail);
                (top.min(bounds.max.y - PLACEMENT_MARGIN - h), h)
            } else {
                let h = right_bot_h;
                (bounds.max.y - PLACEMENT_MARGIN - h, h)
            };
            let size = Vec2::new(right_col_w, h);
            let pos = Pos2::new(bounds.max.x - PLACEMENT_MARGIN - size.x, y);
            (pos, size)
        }
        "output" | "avatar_settings" | "asset_manager" | "microprofiler" => {
            // Stay to the left of the right-side column so Output + Properties + Explorer
            // never overlap on standard landscape or tablet viewports.
            let max_left_span = (bounds.width() - right_col_w - 2.0 * PLACEMENT_MARGIN - PLACEMENT_GAP)
                .max(DEFAULT_MIN_SIZE[0].min(w_avail));
            let w = (bounds.width() * 0.48)
                .clamp(260.0, 560.0)
                .min(max_left_span)
                .min(w_avail);
            let h = bottom_h;
            let pos = Pos2::new(
                bounds.min.x + PLACEMENT_MARGIN,
                bounds.max.y - PLACEMENT_MARGIN - h,
            );
            (pos, Vec2::new(w, h))
        }
        "toolbox" | "terrain_editor" => {
            // Top-left, leaving vertical room for Output at bottom-left.
            let max_top_h = (h_avail - bottom_h - PLACEMENT_GAP).max(DEFAULT_MIN_SIZE[1].min(h_avail));
            let h = (h_avail * 0.56).clamp(180.0, 380.0).min(max_top_h).min(h_avail);
            let size = Vec2::new(left_col_w, h);
            let pos = Pos2::new(bounds.min.x + PLACEMENT_MARGIN, bounds.min.y + PLACEMENT_MARGIN);
            (pos, size)
        }
        _ => {
            // Scripts, plugins, and other panels: comfortably sized near center-left.
            let w = (bounds.width() * 0.46).clamp(300.0, 620.0).min(w_avail);
            let h = (bounds.height() * 0.54).clamp(220.0, 460.0).min(h_avail);
            let x = bounds.min.x + ((bounds.width() - w) * 0.35).max(PLACEMENT_MARGIN);
            let y = bounds.min.y + ((bounds.height() - h) * 0.25).max(PLACEMENT_MARGIN);
            (Pos2::new(x, y), Vec2::new(w, h))
        }
    };

    let size = Vec2::new(
        pref_size.x.clamp(DEFAULT_MIN_SIZE[0].min(bounds.width()), bounds.width()),
        pref_size.y.clamp(DEFAULT_MIN_SIZE[1].min(bounds.height()), bounds.height()),
    );
    let clamp_pos = |p: Pos2| -> Pos2 {
        Pos2::new(
            p.x.clamp(bounds.min.x, (bounds.max.x - size.x).max(bounds.min.x)),
            p.y.clamp(bounds.min.y, (bounds.max.y - size.y).max(bounds.min.y)),
        )
    };

    let primary = clamp_pos(pref_pos);
    let existing_rects: Vec<Rect> = existing
        .iter()
        .filter(|w| !w.minimized)
        .map(|w| {
            let vis = if w.collapsed {
                Vec2::new(collapsed_width(w.size[0]), TITLE_BAR_HEIGHT_TOUCH)
            } else {
                Vec2::new(w.size[0], w.size[1])
            };
            Rect::from_min_size(Pos2::new(w.pos[0], w.pos[1]), vis)
        })
        .collect();

    if overlap_area(Rect::from_min_size(primary, size), &existing_rects) <= 1.0 {
        return (primary, size);
    }

    // Try candidate non-overlapping / cascaded positions and pick the least overlapping.
    let step = 28.0;
    let mut candidates = vec![primary];
    for k in 1..=6 {
        let offset = step * k as f32;
        candidates.push(clamp_pos(primary + Vec2::new(offset, offset)));
        candidates.push(clamp_pos(primary + Vec2::new(offset, 0.0)));
        candidates.push(clamp_pos(primary + Vec2::new(-offset, 0.0)));
        candidates.push(clamp_pos(primary + Vec2::new(0.0, -offset)));
    }

    let mut best_pos = primary;
    let mut best_overlap = overlap_area(Rect::from_min_size(primary, size), &existing_rects);
    for cand in candidates {
        let ov = overlap_area(Rect::from_min_size(cand, size), &existing_rects);
        if ov + 0.5 < best_overlap {
            best_overlap = ov;
            best_pos = cand;
            if best_overlap <= 1.0 {
                break;
            }
        }
    }

    (best_pos, size)
}

fn overlap_area(candidate: Rect, others: &[Rect]) -> f32 {
    others
        .iter()
        .map(|other| {
            let inter = candidate.intersect(*other);
            if inter.is_positive() {
                inter.width() * inter.height()
            } else {
                0.0
            }
        })
        .sum()
}

/// Manages all touch-friendly floating editor windows inside the main viewport.
#[derive(Debug, Clone)]
pub struct MobileWindowManager {
    windows: Vec<FloatingWindowState>,
    next_z: u64,
    touch_mode: bool,
    title_press: HashMap<String, TitlePressState>,
}

impl Default for MobileWindowManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MobileWindowManager {
    /// Creates an empty manager. `touch_mode` defaults to `true` on Android and
    /// `false` on desktop platforms.
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_z: 1,
            touch_mode: cfg!(target_os = "android"),
            title_press: HashMap::new(),
        }
    }

    /// Builder-style override for touch vs desktop chrome metrics.
    pub fn with_touch_mode(mut self, touch_mode: bool) -> Self {
        self.touch_mode = touch_mode;
        self
    }

    /// Updates touch vs desktop chrome metrics at runtime.
    pub fn set_touch_mode(&mut self, touch_mode: bool) {
        self.touch_mode = touch_mode;
    }

    /// Whether touch-sized chrome metrics are active.
    pub fn touch_mode(&self) -> bool {
        self.touch_mode
    }

    /// Current title-bar height in logical points.
    pub fn current_title_bar_height(&self) -> f32 {
        title_bar_height(self.touch_mode)
    }

    /// Read-only slice of all open floating windows.
    pub fn windows(&self) -> &[FloatingWindowState] {
        &self.windows
    }

    /// Mutable slice of all open floating windows.
    pub fn windows_mut(&mut self) -> &mut [FloatingWindowState] {
        &mut self.windows
    }

    /// Number of open floating windows.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no floating windows are open.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Looks up a floating window by `id` (or fallback `tab.key()`).
    pub fn get(&self, key: &str) -> Option<&FloatingWindowState> {
        self.find_index(key).map(|i| &self.windows[i])
    }

    /// Returns `true` if a floating window matching `key` is open.
    pub fn contains(&self, key: &str) -> bool {
        self.find_index(key).is_some()
    }

    /// Returns `true` if a floating window hosting `tab` is open.
    pub fn contains_tab(&self, tab: &Tab) -> bool {
        let id = window_id_for_tab(tab);
        self.windows.iter().any(|w| w.id == id || w.tab == *tab)
    }

    /// Returns the `id` of the topmost (highest `z_order`) floating window.
    pub fn focused_id(&self) -> Option<&str> {
        self.windows
            .iter()
            .filter(|w| !w.minimized)
            .max_by_key(|w| w.z_order)
            .map(|w| w.id.as_str())
    }

    /// Promotes the window matching `key` to the front of the z-order stack.
    pub fn focus(&mut self, key: &str) -> bool {
        if let Some(idx) = self.find_index(key) {
            let z = self.alloc_z();
            self.windows[idx].z_order = z;
            true
        } else {
            false
        }
    }

    /// Opens `tab` as a floating window, or focuses (and expands) it if already open.
    pub fn open_or_focus(&mut self, tab: Tab, title: String, bounds: Rect) {
        let id = window_id_for_tab(&tab);
        let title_h = self.current_title_bar_height();

        if let Some(idx) = self.windows.iter().position(|w| w.id == id || w.tab == tab) {
            if !title.is_empty() {
                self.windows[idx].title = title;
            }
            self.windows[idx].minimized = false;
            self.windows[idx].collapsed = false;
            self.windows[idx].z_order = self.alloc_z();
            self.windows[idx].clamp_to_bounds(bounds, title_h);
            return;
        }

        let (pos, size) = default_placement_for(tab.key(), bounds, &self.windows);
        let z = self.alloc_z();
        let mut win = FloatingWindowState {
            id,
            tab,
            title,
            pos: [pos.x, pos.y],
            size: [size.x, size.y],
            min_size: DEFAULT_MIN_SIZE,
            collapsed: false,
            minimized: false,
            z_order: z,
        };
        win.clamp_to_bounds(bounds, title_h);
        self.windows.push(win);
    }

    /// Closes the floating window matching `key` (`id` or `tab.key()`), returning
    /// its hosted [`Tab`] if found.
    pub fn close(&mut self, key: &str) -> Option<Tab> {
        let idx = self.find_index(key)?;
        let removed = self.windows.remove(idx);
        self.title_press.remove(&removed.id);
        Some(removed.tab)
    }

    /// Toggles `collapsed` on the window matching `key`, preserving `pos`,
    /// expanded `size`, and `tab` identity.
    pub fn toggle_collapse(&mut self, key: &str) {
        if let Some(idx) = self.find_index(key) {
            self.windows[idx].collapsed = !self.windows[idx].collapsed;
            self.windows[idx].z_order = self.alloc_z();
        }
    }

    /// Clamps every floating window so it stays inside `bounds`.
    pub fn clamp_all(&mut self, bounds: Rect) {
        let title_h = self.current_title_bar_height();
        for w in &mut self.windows {
            w.clamp_to_bounds(bounds, title_h);
        }
    }

    /// Returns the screen rectangles currently occupied by all floating windows
    /// (collapsed title strips or full expanded frames), in ascending `z_order`.
    ///
    /// Touch input handlers and the 3D viewport can query this to avoid passing
    /// touch drags or taps through floating windows.
    pub fn hit_rects(&self) -> Vec<Rect> {
        let title_h = self.current_title_bar_height();
        let mut order: Vec<usize> = (0..self.windows.len())
            .filter(|&i| !self.windows[i].minimized)
            .collect();
        order.sort_by_key(|&i| (self.windows[i].z_order, i));
        order
            .into_iter()
            .map(|i| self.windows[i].visible_rect(title_h))
            .collect()
    }

    /// Returns title-bar and resize-grip rects for immediate touch dragging.
    pub fn drag_handle_rects(&self) -> Vec<Rect> {
        let title_h = self.current_title_bar_height();
        let edge_in = if self.touch_mode { 10.0 } else { 6.0 };
        let edge_out = if self.touch_mode { 14.0 } else { 8.0 };
        let mut out = Vec::new();
        for w in self.windows.iter().filter(|w| !w.minimized) {
            let r = w.visible_rect(title_h);
            let title_r = Rect::from_min_max(r.min, Pos2::new(r.max.x, (r.min.y + title_h).min(r.max.y)));
            let lay = TitleBarLayout::compute(title_r, self.touch_mode);
            out.push(lay.drag_rect);
            if !w.collapsed {
                let corner = Rect::from_min_max(
                    Pos2::new(r.max.x - edge_in, r.max.y - edge_in),
                    Pos2::new(r.max.x + edge_out, r.max.y + edge_out),
                );
                out.push(corner);
            }
        }
        out
    }

    /// Returns `true` if `pos` falls inside any visible (non-minimized) floating window.
    pub fn point_in_any_window(&self, pos: Pos2) -> bool {
        let title_h = self.current_title_bar_height();
        self.windows.iter().filter(|w| !w.minimized).any(|w| w.visible_rect(title_h).contains(pos))
    }

    /// Restores a minimized floating window and brings it to front.
    pub fn restore(&mut self, id: &str) -> bool {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.minimized = false;
            w.collapsed = false;
            let z = self.next_z;
            self.next_z = self.next_z.saturating_add(1);
            w.z_order = z;
            return true;
        }
        false
    }

    /// Lists currently minimized floating windows as `(id, title, tab)`.
    pub fn minimized_windows(&self) -> Vec<(String, String, Tab)> {
        self.windows
            .iter()
            .filter(|w| w.minimized)
            .map(|w| (w.id.clone(), w.title.clone(), w.tab))
            .collect()
    }

    /// Serializes all persistable floating windows for `layout.json`.
    ///
    /// Ephemeral tabs not represented in [`SavedTab`] (scripts and plugin widgets)
    /// are skipped, matching `layout_store::saved_dock`.
    pub fn to_saved(&self) -> Vec<SavedFloatingWindow> {
        self.windows
            .iter()
            .filter_map(|w| {
                let saved_tab = SavedTab::from_tab(&w.tab)?;
                Some(SavedFloatingWindow {
                    id: w.id.clone(),
                    tab: saved_tab,
                    title: w.title.clone(),
                    pos: w.pos,
                    size: w.size,
                    min_size: w.min_size,
                    collapsed: w.collapsed,
                    minimized: w.minimized,
                    z_order: w.z_order,
                })
            })
            .collect()
    }

    /// Restores a [`MobileWindowManager`] from saved `layout.json` entries.
    pub fn from_saved(saved: &[SavedFloatingWindow]) -> Self {
        let mut mgr = Self::new();
        let mut max_z = 0u64;
        for entry in saved {
            if entry.id.is_empty() || mgr.windows.iter().any(|w| w.id == entry.id) {
                continue;
            }
            let tab = entry.tab.tab();
            let min_w = if entry.min_size[0].is_finite() {
                entry.min_size[0].max(120.0)
            } else {
                DEFAULT_MIN_SIZE[0]
            };
            let min_h = if entry.min_size[1].is_finite() {
                entry.min_size[1].max(TITLE_BAR_HEIGHT_DESKTOP)
            } else {
                DEFAULT_MIN_SIZE[1]
            };
            let size_w = if entry.size[0].is_finite() {
                entry.size[0].max(min_w)
            } else {
                DEFAULT_MIN_SIZE[0]
            };
            let size_h = if entry.size[1].is_finite() {
                entry.size[1].max(min_h)
            } else {
                DEFAULT_MIN_SIZE[1]
            };
            let pos_x = if entry.pos[0].is_finite() { entry.pos[0] } else { PLACEMENT_MARGIN };
            let pos_y = if entry.pos[1].is_finite() { entry.pos[1] } else { PLACEMENT_MARGIN };

            max_z = max_z.max(entry.z_order);
            mgr.windows.push(FloatingWindowState {
                id: entry.id.clone(),
                tab,
                title: entry.title.clone(),
                pos: [pos_x, pos_y],
                size: [size_w, size_h],
                min_size: [min_w, min_h],
                collapsed: entry.collapsed,
                minimized: entry.minimized,
                z_order: entry.z_order,
            });
        }
        mgr.next_z = max_z.saturating_add(1).max(1);
        mgr
    }

    /// Renders all floating windows on `egui::Order::Middle` inside `bounds`,
    /// handling dragging, tap-to-collapse, resizing, focus promotion, Dock-back,
    /// and Close.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        bounds: Rect,
        modal_open: bool,
        mut render_body: impl FnMut(&mut egui::Ui, &mut Tab),
    ) -> Vec<MobileWindowAction> {
        self.clamp_all(bounds);
        if self.windows.is_empty() {
            return Vec::new();
        }

        let title_h = self.current_title_bar_height();
        let edge_hit = if self.touch_mode {
            RESIZE_EDGE_HIT_TOUCH
        } else {
            RESIZE_EDGE_HIT_DESKTOP
        };
        let top_z = self.windows.iter().filter(|w| !w.minimized).map(|w| w.z_order).max().unwrap_or(0);

        let mut order: Vec<usize> = (0..self.windows.len()).collect();
        order.sort_by_key(|&i| (self.windows[i].z_order, i));

        let primary_pressed = ctx.input(|i| i.pointer.primary_pressed());
        let pointer_pos = ctx.input(|i| i.pointer.interact_pos());

        let mut actions = Vec::new();
        let mut to_remove: Vec<String> = Vec::new();
        let mut to_minimize: Vec<String> = Vec::new();
        let mut to_focus: Option<String> = None;

        for idx in order {
            if self.windows[idx].minimized {
                continue;
            }
            let win_id = self.windows[idx].id.clone();
            let is_focused = self.windows[idx].z_order == top_z;
            let collapsed = self.windows[idx].collapsed;
            let vis_size = self.windows[idx].visible_size(title_h);
            let pos = Pos2::new(self.windows[idx].pos[0], self.windows[idx].pos[1]);
            let title_text = self.windows[idx].title.clone();
            let tab_copy = self.windows[idx].tab;

            egui::Area::new(Id::new(("mobile_floating_window", &win_id)))
                .order(Order::Middle)
                .fixed_pos(pos)
                .movable(false)
                .constrain(false)
                .interactable(!modal_open)
                .show(ctx, |ui| {
                    // Only enforce Middle z_order when no modal dialog is open above us.
                    if !modal_open {
                        ui.ctx().move_to_top(ui.layer_id());
                    }

                    let (win_rect, _bg_resp) = ui.allocate_exact_size(vis_size, Sense::click_and_drag());
                    let painter = ui.painter().clone();

                    // Promote focus on pointer press anywhere inside the window.
                    // Because we iterate back-to-front, the topmost window under the
                    // pointer naturally wins `to_focus`.
                    if primary_pressed
                        && pointer_pos.is_some_and(|p| win_rect.contains(p))
                    {
                        to_focus = Some(win_id.clone());
                    }

                    // Window container background & border.
                    let rounding = CornerRadius::same(WINDOW_CORNER_RADIUS);
                    let border_color = if is_focused {
                        theme::ACCENT
                    } else {
                        theme::DIVIDER
                    };
                    let border_width = if is_focused { 1.5 } else { 1.0 };

                    painter.rect_filled(win_rect, rounding, theme::PANEL);

                    // Title bar background.
                    let title_rect = Rect::from_min_size(win_rect.min, Vec2::new(win_rect.width(), title_h));
                    let title_rounding = if collapsed {
                        rounding
                    } else {
                        CornerRadius {
                            nw: WINDOW_CORNER_RADIUS,
                            ne: WINDOW_CORNER_RADIUS,
                            sw: 0,
                            se: 0,
                        }
                    };
                    let title_bg = if is_focused {
                        theme::PANEL_DARK
                    } else {
                        theme::MENU_BAR
                    };
                    painter.rect_filled(title_rect, title_rounding, title_bg);

                    if !collapsed {
                        painter.line_segment(
                            [
                                Pos2::new(title_rect.min.x, title_rect.max.y),
                                Pos2::new(title_rect.max.x, title_rect.max.y),
                            ],
                            Stroke::new(1.0, theme::DIVIDER),
                        );
                    }

                    let layout = TitleBarLayout::compute(title_rect, self.touch_mode);

                    // Collapse/expand chevron indicator.
                    let chevron = if collapsed { "▸" } else { "▾" };
                    let chevron_color = if is_focused {
                        theme::TEXT
                    } else {
                        theme::TEXT_DIM
                    };
                    painter.text(
                        layout.chevron_rect.center(),
                        Align2::CENTER_CENTER,
                        chevron,
                        FontId::proportional(if self.touch_mode { 14.0 } else { 12.0 }),
                        chevron_color,
                    );

                    // Clipped window title text.
                    let title_clip = painter.with_clip_rect(layout.title_text_rect);
                    title_clip.text(
                        Pos2::new(layout.title_text_rect.min.x, layout.title_text_rect.center().y),
                        Align2::LEFT_CENTER,
                        &title_text,
                        FontId::proportional(if self.touch_mode { 13.5 } else { 12.5 }),
                        if is_focused { theme::TEXT } else { theme::TEXT_DIM },
                    );

                    // Title-bar action button: Minimize (−).
                    let min_resp = ui
                        .interact(layout.min_rect, ui.id().with("min_btn"), Sense::click())
                        .on_hover_text("Minimize panel");
                    let min_fill = if min_resp.is_pointer_button_down_on() {
                        theme::ACTIVE_TAB
                    } else if min_resp.hovered() {
                        theme::HOVER_BG
                    } else {
                        theme::PANEL_ALT
                    };
                    painter.rect_filled(layout.min_rect, CornerRadius::same(4), min_fill);
                    painter.rect_stroke(
                        layout.min_rect,
                        CornerRadius::same(4),
                        Stroke::new(1.0, theme::DIVIDER),
                        StrokeKind::Inside,
                    );
                    painter.text(
                        layout.min_rect.center(),
                        Align2::CENTER_CENTER,
                        "−",
                        FontId::proportional(if self.touch_mode { 15.0 } else { 13.5 }),
                        theme::TEXT,
                    );
                    if min_resp.clicked() {
                        to_minimize.push(win_id.clone());
                    }

                    // Title-bar action button: Dock back.
                    let dock_resp = ui
                        .interact(layout.dock_rect, ui.id().with("dock_btn"), Sense::click())
                        .on_hover_text("Dock back into main layout");
                    let dock_fill = if dock_resp.is_pointer_button_down_on() {
                        theme::ACTIVE_TAB
                    } else if dock_resp.hovered() {
                        theme::HOVER_BG
                    } else {
                        theme::PANEL_ALT
                    };
                    painter.rect_filled(layout.dock_rect, CornerRadius::same(4), dock_fill);
                    painter.rect_stroke(
                        layout.dock_rect,
                        CornerRadius::same(4),
                        Stroke::new(1.0, theme::DIVIDER),
                        StrokeKind::Inside,
                    );
                    painter.text(
                        layout.dock_rect.center(),
                        Align2::CENTER_CENTER,
                        "Dock",
                        FontId::proportional(if self.touch_mode { 11.5 } else { 10.5 }),
                        theme::TEXT,
                    );
                    if dock_resp.clicked() {
                        actions.push(MobileWindowAction::DockBack(tab_copy));
                        to_remove.push(win_id.clone());
                    }

                    // Title-bar action button: Close (×).
                    let close_resp = ui
                        .interact(layout.close_rect, ui.id().with("close_btn"), Sense::click())
                        .on_hover_text("Close panel");
                    let close_fill = if close_resp.is_pointer_button_down_on() {
                        theme::ERROR.gamma_multiply(0.55)
                    } else if close_resp.hovered() {
                        theme::ERROR.gamma_multiply(0.35)
                    } else {
                        theme::PANEL_ALT
                    };
                    painter.rect_filled(layout.close_rect, CornerRadius::same(4), close_fill);
                    painter.rect_stroke(
                        layout.close_rect,
                        CornerRadius::same(4),
                        Stroke::new(1.0, theme::DIVIDER),
                        StrokeKind::Inside,
                    );
                    painter.text(
                        layout.close_rect.center(),
                        Align2::CENTER_CENTER,
                        "×",
                        FontId::proportional(if self.touch_mode { 15.0 } else { 13.5 }),
                        theme::TEXT,
                    );
                    if close_resp.clicked() {
                        actions.push(MobileWindowAction::Closed(tab_copy));
                        to_remove.push(win_id.clone());
                    }

                    // Title-bar drag + tap-to-collapse (strictly excludes Dock/Close buttons).
                    let title_resp = ui.interact(
                        layout.drag_rect,
                        ui.id().with("title_drag"),
                        Sense::click_and_drag(),
                    );
                    if title_resp.dragged() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    } else if title_resp.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    }

                    let mut toggled_this_frame = false;
                    let button_rects = layout.button_rects();

                    if title_resp.drag_started()
                        || (title_resp.is_pointer_button_down_on()
                            && !self.title_press.contains_key(&win_id))
                    {
                        to_focus = Some(win_id.clone());
                        if let Some(origin) = title_resp.interact_pointer_pos().or(pointer_pos) {
                            self.title_press.insert(
                                win_id.clone(),
                                TitlePressState {
                                    press_origin: origin,
                                    start_window_pos: self.windows[idx].pos,
                                    accumulated_dist: 0.0,
                                    moved_beyond_slop: false,
                                },
                            );
                        }
                    }

                    if title_resp.dragged() {
                        to_focus = Some(win_id.clone());
                        let delta = title_resp.drag_delta();
                        if let Some(state) = self.title_press.get_mut(&win_id) {
                            state.accumulated_dist += delta.length();
                            let current = title_resp.interact_pointer_pos().or(pointer_pos);
                            let displacement = current
                                .map(|c| c.distance(state.press_origin))
                                .unwrap_or(state.accumulated_dist);
                            if !state.moved_beyond_slop
                                && (state.accumulated_dist > TITLE_TAP_SLOP
                                    || displacement > TITLE_TAP_SLOP)
                            {
                                state.moved_beyond_slop = true;
                                if let Some(cur) = current {
                                    let total = cur - state.press_origin;
                                    self.windows[idx].pos = [
                                        state.start_window_pos[0] + total.x,
                                        state.start_window_pos[1] + total.y,
                                    ];
                                    self.windows[idx].clamp_to_bounds(bounds, title_h);
                                } else {
                                    self.windows[idx].drag_by(delta, bounds, title_h);
                                }
                            } else if state.moved_beyond_slop {
                                self.windows[idx].drag_by(delta, bounds, title_h);
                            }
                        } else {
                            self.windows[idx].drag_by(delta, bounds, title_h);
                        }
                    }

                    if title_resp.drag_stopped() {
                        if let Some(state) = self.title_press.remove(&win_id) {
                            let release = title_resp
                                .interact_pointer_pos()
                                .or(pointer_pos)
                                .unwrap_or(state.press_origin);
                            if !state.moved_beyond_slop
                                && is_title_tap(
                                    state.press_origin,
                                    release,
                                    state.accumulated_dist,
                                    &button_rects,
                                )
                            {
                                self.windows[idx].collapsed = !self.windows[idx].collapsed;
                                self.windows[idx].clamp_to_bounds(bounds, title_h);
                                to_focus = Some(win_id.clone());
                                toggled_this_frame = true;
                            }
                        }
                    }

                    if !toggled_this_frame && title_resp.clicked() {
                        self.title_press.remove(&win_id);
                        let click_pos = title_resp
                            .interact_pointer_pos()
                            .or(pointer_pos)
                            .unwrap_or(layout.drag_rect.center());
                        if is_title_tap(click_pos, click_pos, 0.0, &button_rects) {
                            self.windows[idx].collapsed = !self.windows[idx].collapsed;
                            self.windows[idx].clamp_to_bounds(bounds, title_h);
                            to_focus = Some(win_id.clone());
                        }
                    }

                    // Expanded body content & non-occluding resize grips.
                    if !self.windows[idx].collapsed {
                        let edge_in = if self.touch_mode { 10.0 } else { 6.0 };
                        let edge_out = (edge_hit - edge_in).max(6.0);
                        let corner_out = if self.touch_mode { 14.0 } else { 8.0 };
                        let body_rect = Rect::from_min_max(
                            Pos2::new(win_rect.min.x + 2.0, title_rect.max.y + 1.0),
                            Pos2::new(
                                (win_rect.max.x - edge_in).max(win_rect.min.x + 2.0),
                                (win_rect.max.y - edge_in).max(title_rect.max.y + 1.0),
                            ),
                        );
                        if body_rect.is_positive() {
                            let mut child_ui =
                                ui.new_child(UiBuilder::new().max_rect(body_rect));
                            child_ui.set_clip_rect(body_rect);
                            render_body(&mut child_ui, &mut self.windows[idx].tab);
                        }

                        // Bottom-right corner resize grip straddling the outer border (disjoint from body_rect).
                        let corner_rect = Rect::from_min_max(
                            Pos2::new(body_rect.max.x, body_rect.max.y),
                            Pos2::new(win_rect.max.x + corner_out, win_rect.max.y + corner_out),
                        );
                        let corner_resp = ui.interact(
                            corner_rect,
                            ui.id().with("resize_se"),
                            Sense::drag(),
                        );
                        if corner_resp.hovered() || corner_resp.dragged() {
                            ui.ctx().set_cursor_icon(CursorIcon::ResizeSouthEast);
                        }
                        if corner_resp.drag_started() || corner_resp.dragged() {
                            to_focus = Some(win_id.clone());
                            self.windows[idx].resize_by(corner_resp.drag_delta(), bounds, title_h);
                        }

                        // Right edge resize strip outside body_rect.
                        let right_rect = Rect::from_min_max(
                            Pos2::new(body_rect.max.x, title_rect.max.y),
                            Pos2::new(win_rect.max.x + edge_out, corner_rect.min.y.max(title_rect.max.y)),
                        );
                        if right_rect.is_positive() {
                            let right_resp = ui.interact(
                                right_rect,
                                ui.id().with("resize_e"),
                                Sense::drag(),
                            );
                            if right_resp.hovered() || right_resp.dragged() {
                                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                            }
                            if right_resp.drag_started() || right_resp.dragged() {
                                to_focus = Some(win_id.clone());
                                self.windows[idx].resize_by(
                                    Vec2::new(right_resp.drag_delta().x, 0.0),
                                    bounds,
                                    title_h,
                                );
                            }
                        }

                        // Bottom edge resize strip outside body_rect.
                        let bottom_rect = Rect::from_min_max(
                            Pos2::new(win_rect.min.x, body_rect.max.y),
                            Pos2::new(corner_rect.min.x.max(win_rect.min.x), win_rect.max.y + edge_out),
                        );
                        if bottom_rect.is_positive() {
                            let bottom_resp = ui.interact(
                                bottom_rect,
                                ui.id().with("resize_s"),
                                Sense::drag(),
                            );
                            if bottom_resp.hovered() || bottom_resp.dragged() {
                                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
                            }
                            if bottom_resp.drag_started() || bottom_resp.dragged() {
                                to_focus = Some(win_id.clone());
                                self.windows[idx].resize_by(
                                    Vec2::new(0.0, bottom_resp.drag_delta().y),
                                    bounds,
                                    title_h,
                                );
                            }
                        }

                        // Draw subtle diagonal resize grip indicator in bottom-right corner.
                        let grip_color = if corner_resp.hovered() || corner_resp.dragged() {
                            theme::ACCENT
                        } else {
                            theme::TEXT_DIM
                        };
                        let grip_stroke = Stroke::new(1.25, grip_color);
                        for offset in [5.0_f32, 9.5, 14.0] {
                            painter.line_segment(
                                [
                                    Pos2::new(win_rect.max.x - offset, win_rect.max.y - 3.0),
                                    Pos2::new(win_rect.max.x - 3.0, win_rect.max.y - offset),
                                ],
                                grip_stroke,
                            );
                        }
                    }

                    // Outer window border drawn last so it frames title bar and body cleanly.
                    painter.rect_stroke(
                        win_rect,
                        rounding,
                        Stroke::new(border_width, border_color),
                        StrokeKind::Inside,
                    );
                });
        }

        if let Some(focus_id) = to_focus {
            self.focus(&focus_id);
        }

        for id in to_minimize {
            if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                w.minimized = true;
            }
        }
        for id in to_remove {
            self.close(&id);
        }

        actions
    }

    fn alloc_z(&mut self) -> u64 {
        let z = self.next_z;
        self.next_z = self.next_z.saturating_add(1);
        z
    }

    fn find_index(&self, key: &str) -> Option<usize> {
        self.windows
            .iter()
            .position(|w| w.id == key)
            .or_else(|| self.windows.iter().position(|w| w.tab.key() == key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tablet_bounds() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(1024.0, 600.0))
    }

    #[test]
    fn drag_and_clamp_cannot_escape_screen_bounds() {
        let bounds = tablet_bounds();
        let title_h = TITLE_BAR_HEIGHT_TOUCH;
        let mut mgr = MobileWindowManager::new().with_touch_mode(true);
        mgr.open_or_focus(Tab::Output, "المخرجات".into(), bounds);

        let win = &mut mgr.windows_mut()[0];
        // Drag far past top-left.
        win.drag_by(Vec2::new(-5000.0, -5000.0), bounds, title_h);
        assert_eq!(win.pos[0], bounds.min.x);
        assert_eq!(win.pos[1], bounds.min.y);

        // Drag far past bottom-right.
        win.drag_by(Vec2::new(9000.0, 9000.0), bounds, title_h);
        let vis = win.visible_rect(title_h);
        assert!(vis.min.x >= bounds.min.x);
        assert!(vis.min.y >= bounds.min.y);
        assert!((vis.max.x - bounds.max.x).abs() < 1e-3);
        assert!((vis.max.y - bounds.max.y).abs() < 1e-3);

        // Simulate orientation change / smaller viewport: clamp_all keeps title bar reachable.
        let portrait = Rect::from_min_size(Pos2::new(0.0, 32.0), Vec2::new(390.0, 720.0));
        mgr.clamp_all(portrait);
        let clamped_rect = mgr.windows()[0].visible_rect(title_h);
        assert!(portrait.contains_rect(clamped_rect));
    }

    #[test]
    fn resize_enforces_min_size_and_screen_max_bounds() {
        let bounds = tablet_bounds();
        let title_h = TITLE_BAR_HEIGHT_TOUCH;
        let mut win = FloatingWindowState {
            id: "properties".into(),
            tab: Tab::Properties,
            title: "الخصائص".into(),
            pos: [200.0, 100.0],
            size: [320.0, 260.0],
            min_size: DEFAULT_MIN_SIZE,
            collapsed: false,
            minimized: false,
            z_order: 1,
        };

        // Shrink far below minimum size.
        win.resize_by(Vec2::new(-1000.0, -1000.0), bounds, title_h);
        assert_eq!(win.size[0], DEFAULT_MIN_SIZE[0]);
        assert_eq!(win.size[1], DEFAULT_MIN_SIZE[1]);

        // Grow far beyond screen max bounds from pos [200, 100].
        win.resize_by(Vec2::new(5000.0, 5000.0), bounds, title_h);
        assert_eq!(win.size[0], bounds.max.x - win.pos[0]);
        assert_eq!(win.size[1], bounds.max.y - win.pos[1]);
        assert!(bounds.contains_rect(win.visible_rect(title_h)));

        // Resizing while collapsed must not alter expanded dimensions.
        let expanded = win.size;
        win.collapsed = true;
        win.resize_by(Vec2::new(-80.0, -80.0), bounds, title_h);
        assert_eq!(win.size, expanded);
    }

    #[test]
    fn click_to_collapse_preserves_pos_size_tab_and_restores_exact_dimensions() {
        let bounds = tablet_bounds();
        let title_h = TITLE_BAR_HEIGHT_TOUCH;
        let mut mgr = MobileWindowManager::new().with_touch_mode(true);
        mgr.open_or_focus(Tab::Properties, "الخصائص".into(), bounds);

        let orig = mgr.get("properties").unwrap().clone();
        assert!(!orig.collapsed);
        assert!(orig.body_rect(title_h).is_some());

        // Collapse.
        mgr.toggle_collapse("properties");
        let collapsed = mgr.get("properties").unwrap().clone();
        assert!(collapsed.collapsed);
        assert_eq!(collapsed.id, orig.id);
        assert_eq!(collapsed.tab, Tab::Properties);
        assert_eq!(collapsed.pos, orig.pos);
        assert_eq!(collapsed.size, orig.size);
        assert!(collapsed.body_rect(title_h).is_none());

        let strip = collapsed.visible_rect(title_h);
        assert_eq!(strip.min, Pos2::new(orig.pos[0], orig.pos[1]));
        assert_eq!(strip.height(), title_h);
        assert!(strip.width() >= COLLAPSED_MIN_WIDTH && strip.width() <= COLLAPSED_MAX_WIDTH);

        // Expand again: exact dimensions, position, and identity restored.
        mgr.toggle_collapse("properties");
        let restored = mgr.get("properties").unwrap().clone();
        assert!(!restored.collapsed);
        assert_eq!(restored.id, orig.id);
        assert_eq!(restored.tab, orig.tab);
        assert_eq!(restored.pos, orig.pos);
        assert_eq!(restored.size, orig.size);
        assert_eq!(
            restored.visible_rect(title_h),
            Rect::from_min_size(Pos2::new(orig.pos[0], orig.pos[1]), Vec2::new(orig.size[0], orig.size[1]))
        );
    }

    #[test]
    fn title_tap_vs_drag_threshold_and_button_exclusion() {
        let title_rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(280.0, TITLE_BAR_HEIGHT_TOUCH));
        let layout = TitleBarLayout::compute(title_rect, true);
        let buttons = layout.button_rects();

        // Small finger jitter (<= 10 pt) on title text counts as a tap.
        let start = Pos2::new(140.0, 96.0);
        let jitter_end = Pos2::new(146.0, 102.0); // distance ~8.49 pt
        assert!(is_title_tap(start, jitter_end, 8.5, &buttons));

        // Exact boundary (10.0 pt) still counts as a tap.
        let boundary_end = Pos2::new(150.0, 96.0);
        assert!(is_title_tap(start, boundary_end, 10.0, &buttons));

        // Movement > 10 pt is a drag, not a tap.
        let drag_end = Pos2::new(151.0, 96.0);
        assert!(!is_title_tap(start, drag_end, 11.0, &buttons));

        // Out-and-back drag that returns to start but traveled > 10 pt is NOT a tap.
        assert!(!is_title_tap(start, start, 18.0, &buttons));

        // Taps on Dock-back or Close buttons must NEVER trigger collapse.
        assert!(!is_title_tap(layout.dock_rect.center(), layout.dock_rect.center(), 0.0, &buttons));
        assert!(!is_title_tap(layout.close_rect.center(), layout.close_rect.center(), 0.0, &buttons));
        // Drag starting on title bar and releasing on Close button is excluded.
        assert!(!is_title_tap(start, layout.close_rect.center(), 2.0, &buttons));
    }

    #[test]
    fn z_order_focus_promotion() {
        let bounds = tablet_bounds();
        let mut mgr = MobileWindowManager::new();
        mgr.open_or_focus(Tab::Explorer, "المستكشف".into(), bounds);
        mgr.open_or_focus(Tab::Properties, "الخصائص".into(), bounds);
        mgr.open_or_focus(Tab::Output, "المخرجات".into(), bounds);

        let z_exp = mgr.get("explorer").unwrap().z_order;
        let z_prop = mgr.get("properties").unwrap().z_order;
        let z_out = mgr.get("output").unwrap().z_order;
        assert!(z_exp < z_prop && z_prop < z_out);
        assert_eq!(mgr.focused_id(), Some("output"));

        // Focusing Explorer promotes it above Output.
        assert!(mgr.focus("explorer"));
        assert_eq!(mgr.focused_id(), Some("explorer"));
        assert!(mgr.get("explorer").unwrap().z_order > mgr.get("output").unwrap().z_order);

        // open_or_focus on an existing window also promotes it without duplicating.
        mgr.open_or_focus(Tab::Properties, "الخصائص".into(), bounds);
        assert_eq!(mgr.len(), 3);
        assert_eq!(mgr.focused_id(), Some("properties"));
    }

    #[test]
    fn coexistence_placement_of_output_properties_explorer_is_non_overlapping() {
        let bounds = tablet_bounds();
        let title_h = TITLE_BAR_HEIGHT_TOUCH;
        let mut mgr = MobileWindowManager::new().with_touch_mode(true);

        mgr.open_or_focus(Tab::Explorer, "المستكشف".into(), bounds);
        mgr.open_or_focus(Tab::Properties, "الخصائص".into(), bounds);
        mgr.open_or_focus(Tab::Output, "المخرجات".into(), bounds);

        let exp = mgr.get("explorer").unwrap().visible_rect(title_h);
        let prop = mgr.get("properties").unwrap().visible_rect(title_h);
        let out = mgr.get("output").unwrap().visible_rect(title_h);

        // Every window must be fully inside bounds.
        assert!(bounds.contains_rect(exp));
        assert!(bounds.contains_rect(prop));
        assert!(bounds.contains_rect(out));

        // Zero pairwise overlap between Explorer, Properties, and Output.
        assert!(!exp.intersects(prop), "Explorer {exp:?} overlapped Properties {prop:?}");
        assert!(!exp.intersects(out), "Explorer {exp:?} overlapped Output {out:?}");
        assert!(!prop.intersects(out), "Properties {prop:?} overlapped Output {out:?}");

        // Explorer sits above Properties on the right; Output sits at bottom-left.
        assert!(exp.max.y <= prop.min.y);
        assert!(out.max.x <= prop.min.x);

        // hit_rects returns all 3 windows in ascending z_order.
        assert_eq!(mgr.hit_rects().len(), 3);
        assert!(mgr.point_in_any_window(out.center()));
    }

    #[test]
    fn json_persistence_round_trip() {
        let bounds = tablet_bounds();
        let mut mgr = MobileWindowManager::new().with_touch_mode(true);
        mgr.open_or_focus(Tab::Explorer, "المستكشف".into(), bounds);
        mgr.open_or_focus(Tab::Output, "المخرجات".into(), bounds);
        mgr.toggle_collapse("output");

        let saved = mgr.to_saved();
        assert_eq!(saved.len(), 2);

        let json = serde_json::to_string_pretty(&saved).expect("serialize SavedFloatingWindow");
        let decoded: Vec<SavedFloatingWindow> =
            serde_json::from_str(&json).expect("deserialize SavedFloatingWindow");
        assert_eq!(saved, decoded);

        let restored = MobileWindowManager::from_saved(&decoded);
        assert_eq!(restored.len(), 2);
        let out = restored.get("output").expect("restored output");
        assert!(out.collapsed);
        assert_eq!(out.tab, Tab::Output);
        assert_eq!(out.size, mgr.get("output").unwrap().size);
        assert_eq!(out.pos, mgr.get("output").unwrap().pos);
        assert_eq!(out.z_order, mgr.get("output").unwrap().z_order);

        let exp = restored.get("explorer").expect("restored explorer");
        assert!(!exp.collapsed);
        assert_eq!(exp.tab, Tab::Explorer);
    }

    #[test]
    fn close_removes_window_and_returns_tab() {
        let bounds = tablet_bounds();
        let mut mgr = MobileWindowManager::new();
        mgr.open_or_focus(Tab::Toolbox, "صندوق الأدوات".into(), bounds);
        assert!(mgr.contains("toolbox"));

        let closed = mgr.close("toolbox");
        assert_eq!(closed, Some(Tab::Toolbox));
        assert!(!mgr.contains("toolbox"));
        assert!(mgr.is_empty());
        assert_eq!(mgr.close("toolbox"), None);
    }

    #[test]
    fn show_renders_expanded_and_collapsed_frames_with_egui() {
        let ctx = egui::Context::default();
        let bounds = tablet_bounds();
        let mut mgr = MobileWindowManager::new().with_touch_mode(true);
        mgr.open_or_focus(Tab::Explorer, "المستكشف".into(), bounds);
        mgr.open_or_focus(Tab::Output, "المخرجات".into(), bounds);
        mgr.toggle_collapse("output");

        let mut rendered_tabs = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(bounds),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let actions = mgr.show(ui.ctx(), bounds, |body_ui, tab| {
                rendered_tabs.push(*tab);
                body_ui.label("panel body");
            });
            assert!(actions.is_empty());
        });
        output.textures_delta.clear();

        // Only the expanded Explorer body should render; collapsed Output hides its body.
        assert_eq!(rendered_tabs, vec![Tab::Explorer]);
    }
}
