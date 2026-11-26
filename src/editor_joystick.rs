//! Virtual joystick and right-side touch camera look for Edit-mode navigation
//! on Android and touch screens.
//!
//! This module is strictly isolated to **Edit mode**: whenever Play mode is
//! active (`playing == true`) or [`JoystickConfig::enabled`] is `false`,
//! [`EditorJoystick`] and [`TouchLookController`] immediately reset their state
//! and yield all touch input so Roblox Play-mode controls are never disturbed.

use egui::{Color32, Painter, Pos2, Rect, Stroke, TouchId, TouchPhase, Vec2 as EguiVec2};
use glam::{Vec2, Vec3};
use rustudio_render::camera::Camera;
use serde::{Deserialize, Serialize};

/// Allowed range for [`JoystickConfig::size`] (outer diameter in points).
pub const SIZE_RANGE: std::ops::RangeInclusive<f32> = 72.0..=180.0;
/// Allowed range for [`JoystickConfig::opacity`].
pub const OPACITY_RANGE: std::ops::RangeInclusive<f32> = 0.15..=1.0;
/// Allowed range for [`JoystickConfig::dead_zone`] (normalized radius).
pub const DEAD_ZONE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.45;
/// Allowed range for [`JoystickConfig::move_sensitivity`].
pub const MOVE_SENSITIVITY_RANGE: std::ops::RangeInclusive<f32> = 0.25..=3.0;
/// Allowed range for [`JoystickConfig::move_speed`] (studs/sec multiplier).
pub const MOVE_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.25..=4.0;
/// Allowed range for [`JoystickConfig::look_sensitivity`].
pub const LOOK_SENSITIVITY_RANGE: std::ops::RangeInclusive<f32> = 0.25..=3.0;

pub const DEFAULT_SIZE: f32 = 112.0;
pub const DEFAULT_OPACITY: f32 = 0.55;
pub const DEFAULT_DEAD_ZONE: f32 = 0.12;
pub const DEFAULT_MOVE_SENSITIVITY: f32 = 1.0;
pub const DEFAULT_MOVE_SPEED: f32 = 1.0;
pub const DEFAULT_LOOK_SENSITIVITY: f32 = 1.0;
pub const DEFAULT_ANCHOR_OFFSET: [f32; 2] = [28.0, 28.0];

/// Exponential acceleration rate (1/sec) while the joystick is held.
pub const ACCEL_RATE: f32 = 14.0;
/// Scales desktop `base_fly_speed` (typically 60 studs/s) down to a comfortable,
/// controllable mobile touch-joystick cruising speed (~16.8 studs/s at 1.0x).
pub const MOBILE_FLY_SPEED_SCALE: f32 = 0.28;
/// Maximum single-frame touch-look delta (in points) before the jump guard
/// drops a spurious spike.
pub const MAX_LOOK_JUMP_PT: f32 = 160.0;
/// Minimum finger travel (in points) before a right-side touch is treated as
/// camera rotation rather than a tap/selection.
pub const LOOK_DRAG_SLOP_PT: f32 = 3.0;

/// Persistent user settings for the Edit-mode virtual joystick and touch look.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct JoystickConfig {
    /// Whether the virtual editor joystick is enabled (default `true` on
    /// Android, `false` on desktop).
    pub enabled: bool,
    /// Outer ring diameter in egui points (`72.0..=180.0`, default `112.0`).
    pub size: f32,
    /// Visual opacity (`0.15..=1.0`, default `0.55`).
    pub opacity: f32,
    /// Normalized radial dead zone (`0.0..=0.45`, default `0.12`).
    pub dead_zone: f32,
    /// Response-curve sensitivity (`0.25..=3.0`, default `1.0`).
    pub move_sensitivity: f32,
    /// Camera movement speed multiplier (`0.25..=4.0`, default `1.0`).
    pub move_speed: f32,
    /// Right-side touch camera rotation multiplier (`0.25..=3.0`, default `1.0`).
    pub look_sensitivity: f32,
    /// Offset from the bottom-left corner of the viewport rect in points
    /// (default `[28.0, 28.0]`), clamped to the viewport.
    pub anchor_offset: [f32; 2],
}

impl Default for JoystickConfig {
    fn default() -> Self {
        Self {
            enabled: cfg!(target_os = "android"),
            size: DEFAULT_SIZE,
            opacity: DEFAULT_OPACITY,
            dead_zone: DEFAULT_DEAD_ZONE,
            move_sensitivity: DEFAULT_MOVE_SENSITIVITY,
            move_speed: DEFAULT_MOVE_SPEED,
            look_sensitivity: DEFAULT_LOOK_SENSITIVITY,
            anchor_offset: DEFAULT_ANCHOR_OFFSET,
        }
    }
}

fn sanitize_clamped(value: f32, default: f32, range: std::ops::RangeInclusive<f32>) -> f32 {
    let v = if value.is_finite() { value } else { default };
    v.clamp(*range.start(), *range.end())
}

impl JoystickConfig {
    /// Returns a copy with all numeric fields sanitized and clamped to their
    /// supported ranges.
    pub fn clamped(&self) -> Self {
        let off_x = if self.anchor_offset[0].is_finite() {
            self.anchor_offset[0].max(0.0)
        } else {
            DEFAULT_ANCHOR_OFFSET[0]
        };
        let off_y = if self.anchor_offset[1].is_finite() {
            self.anchor_offset[1].max(0.0)
        } else {
            DEFAULT_ANCHOR_OFFSET[1]
        };
        Self {
            enabled: self.enabled,
            size: sanitize_clamped(self.size, DEFAULT_SIZE, SIZE_RANGE),
            opacity: sanitize_clamped(self.opacity, DEFAULT_OPACITY, OPACITY_RANGE),
            dead_zone: sanitize_clamped(self.dead_zone, DEFAULT_DEAD_ZONE, DEAD_ZONE_RANGE),
            move_sensitivity: sanitize_clamped(
                self.move_sensitivity,
                DEFAULT_MOVE_SENSITIVITY,
                MOVE_SENSITIVITY_RANGE,
            ),
            move_speed: sanitize_clamped(
                self.move_speed,
                DEFAULT_MOVE_SPEED,
                MOVE_SPEED_RANGE,
            ),
            look_sensitivity: sanitize_clamped(
                self.look_sensitivity,
                DEFAULT_LOOK_SENSITIVITY,
                LOOK_SENSITIVITY_RANGE,
            ),
            anchor_offset: [off_x, off_y],
        }
    }
}

/// Computes the screen-space bounding rectangle of the virtual joystick inside
/// `viewport_rect`, adapting to any screen size or orientation so the joystick
/// never clips outside the viewport.
///
/// Exposed at module level so `touch_input::Zones` and `viewport_ui` can
/// exclude the joystick region from object picking and camera rotation.
pub fn joystick_rect(viewport_rect: Rect, config: &JoystickConfig) -> Rect {
    EditorJoystick::base_rect(viewport_rect, config)
}

/// Virtual thumbstick state and physics for Edit-mode camera navigation.
#[derive(Clone, Debug, Default)]
pub struct EditorJoystick {
    active_touch: Option<TouchId>,
    pointer_active: bool,
    /// Raw normalized deflection in movement space:
    /// `+x` = strafe right, `+y` = move forward (up on screen), length `<= 1.0`.
    raw_deflection: Vec2,
    /// Smoothed velocity in `[-1.0, 1.0]` (length `<= 1.0`).
    velocity: Vec2,
    /// 0.0 = visible (touch), 1.0 = faded out (KBM).
    hidden_fade: f32,
}

impl EditorJoystick {
    pub fn new() -> Self {
        Self::default()
    }

    /// Immediately zeroes velocity and raw deflection and releases any active
    /// touch or pointer drag. Guarantees zero drift or stuck movement on lift,
    /// cancel, disable, or entering Play mode.
    pub fn reset(&mut self) {
        self.active_touch = None;
        self.pointer_active = false;
        self.raw_deflection = Vec2::ZERO;
        self.velocity = Vec2::ZERO;
    }

    /// Returns `true` when the joystick should be active in the editor.
    ///
    /// Returns `false` whenever `playing == true` or `!config.enabled`, and
    /// immediately calls [`Self::reset`] so Play-mode controls are never
    /// affected.
    pub fn is_active_in_editor(&mut self, playing: bool, config: &JoystickConfig) -> bool {
        if playing || !config.enabled {
            self.reset();
            return false;
        }
        true
    }

    /// Whether a touch or pointer drag is currently holding the joystick.
    pub fn is_active(&self) -> bool {
        self.active_touch.is_some() || self.pointer_active
    }

    /// Active touch ID currently driving the joystick, if any.
    pub fn active_touch(&self) -> Option<TouchId> {
        self.active_touch
    }

    /// Raw normalized deflection (`+x` right, `+y` forward), length `<= 1.0`.
    pub fn raw_deflection(&self) -> Vec2 {
        self.raw_deflection
    }

    /// Current smoothed movement velocity (`+x` right, `+y` forward), length `<= 1.0`.
    pub fn velocity(&self) -> Vec2 {
        self.velocity
    }

    /// Computes the joystick base rectangle anchored to the bottom-left of
    /// `viewport_rect` and clamped so it never clips outside `viewport_rect`
    /// regardless of screen size, aspect ratio, or orientation.
    pub fn base_rect(viewport_rect: Rect, config: &JoystickConfig) -> Rect {
        if !viewport_rect.is_positive() {
            return Rect::NOTHING;
        }
        let cfg = config.clamped();
        let w = viewport_rect.width();
        let h = viewport_rect.height();
        let margin = 8.0_f32.min(w * 0.1).min(h * 0.1);
        let avail_w = (w - 2.0 * margin).max(0.0);
        let avail_h = (h - 2.0 * margin).max(0.0);
        let max_diameter = avail_w.min(avail_h);
        if max_diameter <= 0.0 {
            return Rect::NOTHING;
        }
        let diameter = cfg.size.min(max_diameter);

        let min_x_lo = viewport_rect.min.x + margin;
        let min_x_hi = (viewport_rect.max.x - margin - diameter).max(min_x_lo);
        let min_y_lo = viewport_rect.min.y + margin;
        let min_y_hi = (viewport_rect.max.y - margin - diameter).max(min_y_lo);

        let min_x = (viewport_rect.min.x + cfg.anchor_offset[0]).clamp(min_x_lo, min_x_hi);
        let min_y =
            (viewport_rect.max.y - cfg.anchor_offset[1] - diameter).clamp(min_y_lo, min_y_hi);

        Rect::from_min_size(Pos2::new(min_x, min_y), EguiVec2::splat(diameter))
    }

    /// Current screen-space center of the movable knob inside `viewport_rect`.
    pub fn knob_center(&self, viewport_rect: Rect, config: &JoystickConfig) -> Pos2 {
        let base = Self::base_rect(viewport_rect, config);
        let center = base.center();
        let max_travel = base.width() * 0.5 * 0.72;
        Pos2::new(
            center.x + self.raw_deflection.x * max_travel,
            center.y - self.raw_deflection.y * max_travel,
        )
    }

    /// Computes the dead-zone-rescaled, sensitivity-curved target velocity from
    /// a raw normalized deflection vector without any axis snapping.
    pub fn target_velocity(raw_deflection: Vec2, config: &JoystickConfig) -> Vec2 {
        if !raw_deflection.is_finite() {
            return Vec2::ZERO;
        }
        let cfg = config.clamped();
        let len = raw_deflection.length();
        if len <= cfg.dead_zone || len <= 1e-6 {
            return Vec2::ZERO;
        }
        let clamped_len = len.min(1.0);
        let rescaled = ((clamped_len - cfg.dead_zone) / (1.0 - cfg.dead_zone)).clamp(0.0, 1.0);
        let curved = rescaled.powf(1.0 / cfg.move_sensitivity).clamp(0.0, 1.0);
        let dir = raw_deflection / len;
        dir * curved
    }

    fn update_deflection_from_pos(&mut self, pos: Pos2, base: Rect) {
        let radius = (base.width() * 0.5).max(1.0);
        let center = base.center();
        let dx = (pos.x - center.x) / radius;
        // Screen Y grows downward; upward drag maps to +forward (+y).
        let dy = -(pos.y - center.y) / radius;
        let v = Vec2::new(dx, dy);
        if !v.is_finite() {
            self.raw_deflection = Vec2::ZERO;
            return;
        }
        let len = v.length();
        self.raw_deflection = if len > 1.0 { v / len } else { v };
    }

    /// Processes a raw touch event. Returns `true` if the joystick consumed
    /// this touch (so the viewport should not pass it to camera look or picking).
    pub fn handle_touch(
        &mut self,
        id: TouchId,
        phase: TouchPhase,
        pos: Pos2,
        viewport_rect: Rect,
        playing: bool,
        config: &JoystickConfig,
    ) -> bool {
        if !self.is_active_in_editor(playing, config) {
            return false;
        }
        let base = Self::base_rect(viewport_rect, config);
        if !base.is_positive() {
            self.reset();
            return false;
        }
        match phase {
            TouchPhase::Start => {
                if self.active_touch.is_none() && base.contains(pos) {
                    self.active_touch = Some(id);
                    self.pointer_active = false;
                    self.update_deflection_from_pos(pos, base);
                    return true;
                }
                false
            }
            TouchPhase::Move => {
                if self.active_touch == Some(id) {
                    self.update_deflection_from_pos(pos, base);
                    return true;
                }
                false
            }
            TouchPhase::End | TouchPhase::Cancel => {
                if self.active_touch == Some(id) {
                    self.reset();
                    return true;
                }
                false
            }
        }
    }

    /// Processes a primary-pointer drag for desktop testing or stylus/mouse
    /// interaction. Returns `true` while the joystick owns the pointer drag.
    pub fn handle_pointer_drag(
        &mut self,
        pos: Option<Pos2>,
        primary_down: bool,
        viewport_rect: Rect,
        playing: bool,
        config: &JoystickConfig,
    ) -> bool {
        if !self.is_active_in_editor(playing, config) {
            return false;
        }
        if self.active_touch.is_some() {
            return false;
        }
        if !primary_down {
            if self.pointer_active {
                self.reset();
            }
            return false;
        }
        let base = Self::base_rect(viewport_rect, config);
        if !base.is_positive() {
            self.reset();
            return false;
        }
        let Some(p) = pos else {
            if self.pointer_active {
                self.reset();
            }
            return false;
        };
        if self.pointer_active {
            self.update_deflection_from_pos(p, base);
            return true;
        }
        if base.contains(p) {
            self.pointer_active = true;
            self.update_deflection_from_pos(p, base);
            return true;
        }
        false
    }

    /// Advances smooth velocity physics for frame delta `dt` (seconds) and
    /// returns `(strafe_x, 0.0, forward_z)` in camera-local axes, ready for
    /// `Camera::fly` or direct translation along `cam.right()` and `cam.forward()`.
    ///
    /// When released or disabled, velocity immediately drops to zero with no
    /// drift.
    pub fn update_and_move(&mut self, dt: f32, config: &JoystickConfig) -> Vec3 {
        if !config.enabled {
            self.reset();
            return Vec3::ZERO;
        }
        if !self.is_active() {
            self.velocity = Vec2::ZERO;
            return Vec3::ZERO;
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.1)
        } else {
            0.0
        };
        let target = Self::target_velocity(self.raw_deflection, config);
        if dt > 0.0 {
            let alpha = 1.0 - (-ACCEL_RATE * dt).exp();
            self.velocity = self.velocity.lerp(target, alpha);
            if target == Vec2::ZERO && self.velocity.length_squared() < 1e-6 {
                self.velocity = Vec2::ZERO;
            }
        }
        Vec3::new(self.velocity.x, 0.0, self.velocity.y)
    }

    /// Updates joystick physics and directly translates `cam` along
    /// `cam.right()` and `cam.forward()` scaled by `base_fly_speed * config.move_speed`,
    /// preserving proportional analog speed. Returns `true` if the camera moved.
    pub fn apply_to_camera(
        &mut self,
        cam: &mut Camera,
        dt: f32,
        base_fly_speed: f32,
        playing: bool,
        config: &JoystickConfig,
    ) -> bool {
        if !self.is_active_in_editor(playing, config) {
            return false;
        }
        let mv = self.update_and_move(dt, config);
        if mv == Vec3::ZERO {
            return false;
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.1)
        } else {
            0.0
        };
        let cfg = config.clamped();
        // Smooth non-linear response curve: gentle thumb nudges move slowly for
        // precision placement while full deflection glides smoothly.
        let mag = mv.length().clamp(0.0, 1.0);
        let curved = if mag > 1e-4 { mv * mag.powf(0.5) } else { Vec3::ZERO };
        let speed = base_fly_speed.max(0.0) * MOBILE_FLY_SPEED_SCALE * cfg.move_speed;
        let delta = (cam.right() * curved.x + cam.forward() * curved.z) * speed * dt;
        cam.pos += delta;
        cam.focus += delta;
        true
    }

    /// Renders the compact, translucent virtual joystick (outer ring, dead-zone
    /// guide, cardinal tick marks, and soft glowing knob) inside `viewport_rect`.
    /// Draws nothing in Play mode or when disabled.
    /// Advances smooth fade-in (touch) / fade-out (KBM); returns true while animating.
    pub fn update_fade(&mut self, visible: bool, dt: f32) -> bool {
        let target = if visible { 0.0 } else { 1.0 };
        let diff = target - self.hidden_fade;
        if diff.abs() < 0.01 {
            self.hidden_fade = target;
            return false;
        }
        let step = (dt.clamp(0.0, 0.1) * 5.0).min(1.0);
        self.hidden_fade = (self.hidden_fade + diff * step).clamp(0.0, 1.0);
        true
    }

    pub fn draw(
        &self,
        painter: &Painter,
        viewport_rect: Rect,
        playing: bool,
        config: &JoystickConfig,
    ) {
        let vis = 1.0 - self.hidden_fade;
        if playing || !config.enabled || vis <= 0.01 {
            return;
        }
        let base = Self::base_rect(viewport_rect, config);
        if !base.is_positive() {
            return;
        }
        let cfg = config.clamped();
        let active = self.is_active();
        let alpha = (if active {
            (cfg.opacity + 0.22).min(1.0)
        } else {
            cfg.opacity
        }) * vis;
        let a = |factor: f32| -> u8 { (alpha * factor * 255.0).round().clamp(0.0, 255.0) as u8 };

        let center = base.center();
        let radius = base.width() * 0.5;

        // Translucent dark-tinted base disc and crisp outer ring.
        let base_fill = Color32::from_rgba_unmultiplied(18, 22, 30, a(0.45));
        let ring_color = Color32::from_rgba_unmultiplied(235, 242, 255, a(0.75));
        painter.circle(center, radius, base_fill, Stroke::new(1.5, ring_color));

        // Subtle inner dead-zone guide ring.
        let dead_radius = (radius * cfg.dead_zone).max(2.0);
        if cfg.dead_zone > 0.01 {
            painter.circle_stroke(
                center,
                dead_radius,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(220, 230, 245, a(0.28))),
            );
        }

        // Four cardinal directional tick marks (N, S, E, W).
        let tick_outer = radius - 4.0;
        let tick_inner = (radius - 11.0).max(dead_radius + 2.0);
        let tick_stroke = Stroke::new(1.5, Color32::from_rgba_unmultiplied(230, 238, 255, a(0.55)));
        for dir in [
            EguiVec2::new(0.0, -1.0),
            EguiVec2::new(0.0, 1.0),
            EguiVec2::new(1.0, 0.0),
            EguiVec2::new(-1.0, 0.0),
        ] {
            painter.line_segment(
                [center + dir * tick_inner, center + dir * tick_outer],
                tick_stroke,
            );
        }

        // Soft glowing knob.
        let knob_pos = self.knob_center(viewport_rect, config);
        let knob_radius = radius * 0.34;
        let glow_color = if active {
            Color32::from_rgba_unmultiplied(120, 190, 255, a(0.35))
        } else {
            Color32::from_rgba_unmultiplied(200, 225, 255, a(0.18))
        };
        painter.circle_filled(knob_pos, knob_radius * 1.38, glow_color);

        let knob_fill = if active {
            Color32::from_rgba_unmultiplied(185, 222, 255, a(0.88))
        } else {
            Color32::from_rgba_unmultiplied(230, 238, 250, a(0.68))
        };
        let knob_stroke = Stroke::new(
            1.5,
            Color32::from_rgba_unmultiplied(255, 255, 255, a(0.92)),
        );
        painter.circle(knob_pos, knob_radius, knob_fill, knob_stroke);
    }
}

/// Dedicated single-finger touch controller for Edit-mode camera rotation on
/// the right side / open viewport outside the virtual joystick.
///
/// Guarantees:
/// - Zero jump on touch start (`last_pos` is recorded without emitting delta).
/// - Single-frame spike guard (`> MAX_LOOK_JUMP_PT`) prevents sudden jumps.
/// - Clean reset on release, cancel, entering Play mode, or 2+ finger pinch/pan.
/// - Exposes [`Self::is_rotating`] and [`Self::consumed_tap`] so the viewport
///   suppresses accidental object picking and marquee selection.
#[derive(Clone, Debug, Default)]
pub struct TouchLookController {
    active_touch: Option<TouchId>,
    start_pos: Pos2,
    last_pos: Pos2,
    rotating: bool,
    consumed_tap: bool,
}

impl TouchLookController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Immediately clears all look tracking and suppression state.
    pub fn reset(&mut self) {
        self.active_touch = None;
        self.start_pos = Pos2::ZERO;
        self.last_pos = Pos2::ZERO;
        self.rotating = false;
        self.consumed_tap = false;
    }

    /// Whether a camera-rotation drag is actively in progress.
    pub fn is_rotating(&self) -> bool {
        self.rotating
    }

    /// Whether a touch finger is currently tracked by the look controller
    /// (including before crossing the drag slop threshold).
    pub fn is_tracking(&self) -> bool {
        self.active_touch.is_some()
    }

    /// Active touch ID currently driving camera look, if any.
    pub fn active_touch(&self) -> Option<TouchId> {
        self.active_touch
    }

    /// Returns `true` during a rotation drag and immediately after its release
    /// so the viewport suppresses object picking and marquee selection.
    pub fn consumed_tap(&self) -> bool {
        self.consumed_tap
    }

    /// Reads and clears [`Self::consumed_tap`] after the viewport has used it
    /// to suppress a release-frame click/pick.
    pub fn take_consumed_tap(&mut self) -> bool {
        std::mem::take(&mut self.consumed_tap)
    }

    /// Resets look tracking when a multi-touch gesture (2+ fingers) begins.
    pub fn on_multi_touch(&mut self, num_touches: usize) {
        if num_touches >= 2 {
            self.reset();
        }
    }

    /// Processes a touch event for Edit-mode camera rotation.
    ///
    /// Returns the sensitivity-scaled rotation delta `(dx, dy)` in points to
    /// pass to [`Camera::rotate`] (or [`Self::apply_to_camera`]). Returns
    /// `Vec2::ZERO` on `TouchPhase::Start`, inside the joystick zone, in Play
    /// mode, during 2+ finger gestures, or when a frame delta exceeds
    /// [`MAX_LOOK_JUMP_PT`].
    pub fn handle_touch(
        &mut self,
        id: TouchId,
        phase: TouchPhase,
        pos: Pos2,
        viewport_rect: Rect,
        playing: bool,
        active_touch_count: usize,
        config: &JoystickConfig,
    ) -> Vec2 {
        if playing || active_touch_count >= 2 {
            self.reset();
            return Vec2::ZERO;
        }
        let joy_rect = if config.enabled {
            joystick_rect(viewport_rect, config)
        } else {
            Rect::NOTHING
        };

        match phase {
            TouchPhase::Start => {
                if self.active_touch.is_some() && self.active_touch != Some(id) {
                    // Second finger started on the viewport -> yield to pinch/pan.
                    self.reset();
                    return Vec2::ZERO;
                }
                if !viewport_rect.contains(pos) || joy_rect.contains(pos) {
                    return Vec2::ZERO;
                }
                self.active_touch = Some(id);
                self.start_pos = pos;
                self.last_pos = pos;
                self.rotating = false;
                self.consumed_tap = false;
                Vec2::ZERO
            }
            TouchPhase::Move => {
                if self.active_touch != Some(id) {
                    return Vec2::ZERO;
                }
                let raw = Vec2::new(pos.x - self.last_pos.x, pos.y - self.last_pos.y);
                self.last_pos = pos;
                if !raw.is_finite() || raw.length() > MAX_LOOK_JUMP_PT {
                    return Vec2::ZERO;
                }
                if !self.rotating {
                    if pos.distance(self.start_pos) < LOOK_DRAG_SLOP_PT {
                        return Vec2::ZERO;
                    }
                    self.rotating = true;
                    self.consumed_tap = true;
                }
                let sens = config.clamped().look_sensitivity;
                raw * sens
            }
            TouchPhase::End => {
                if self.active_touch == Some(id) {
                    let was_rotating = self.rotating;
                    self.active_touch = None;
                    self.rotating = false;
                    self.consumed_tap = was_rotating;
                }
                Vec2::ZERO
            }
            TouchPhase::Cancel => {
                if self.active_touch == Some(id) {
                    self.reset();
                }
                Vec2::ZERO
            }
        }
    }

    /// Applies a non-zero look delta to `cam` via [`Camera::rotate`].
    /// Returns `true` if the camera rotated.
    pub fn apply_to_camera(&self, cam: &mut Camera, delta: Vec2) -> bool {
        if delta == Vec2::ZERO || !delta.is_finite() {
            return false;
        }
        cam.rotate(delta.x, delta.y);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> JoystickConfig {
        JoystickConfig {
            enabled: true,
            ..JoystickConfig::default()
        }
    }

    fn landscape_viewport() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 0.0), EguiVec2::new(960.0, 540.0))
    }

    fn portrait_viewport() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 48.0), EguiVec2::new(390.0, 760.0))
    }

    #[test]
    fn config_defaults_and_clamping() {
        let def = JoystickConfig::default();
        assert_eq!(def.enabled, cfg!(target_os = "android"));
        assert_eq!(def.size, 112.0);
        assert_eq!(def.opacity, 0.55);
        assert_eq!(def.dead_zone, 0.12);
        assert_eq!(def.move_sensitivity, 1.0);
        assert_eq!(def.move_speed, 1.0);
        assert_eq!(def.look_sensitivity, 1.0);
        assert_eq!(def.anchor_offset, [28.0, 28.0]);

        let extreme = JoystickConfig {
            enabled: true,
            size: 500.0,
            opacity: -1.0,
            dead_zone: 0.99,
            move_sensitivity: f32::NAN,
            move_speed: 99.0,
            look_sensitivity: 0.01,
            anchor_offset: [-10.0, f32::INFINITY],
        }
        .clamped();
        assert_eq!(extreme.size, *SIZE_RANGE.end());
        assert_eq!(extreme.opacity, *OPACITY_RANGE.start());
        assert_eq!(extreme.dead_zone, *DEAD_ZONE_RANGE.end());
        assert_eq!(extreme.move_sensitivity, DEFAULT_MOVE_SENSITIVITY);
        assert_eq!(extreme.move_speed, *MOVE_SPEED_RANGE.end());
        assert_eq!(extreme.look_sensitivity, *LOOK_SENSITIVITY_RANGE.start());
        assert_eq!(extreme.anchor_offset, [0.0, DEFAULT_ANCHOR_OFFSET[1]]);
    }

    #[test]
    fn config_serde_roundtrip() {
        let cfg = JoystickConfig {
            enabled: true,
            size: 136.0,
            opacity: 0.72,
            dead_zone: 0.18,
            move_sensitivity: 1.5,
            move_speed: 2.25,
            look_sensitivity: 1.35,
            anchor_offset: [40.0, 32.0],
        };
        let json = serde_json::to_string(&cfg).expect("serialize JoystickConfig");
        let back: JoystickConfig = serde_json::from_str(&json).expect("deserialize JoystickConfig");
        assert_eq!(cfg, back);
    }

    #[test]
    fn dead_zone_suppresses_small_deflections_and_rescales_smoothly() {
        let cfg = JoystickConfig {
            enabled: true,
            dead_zone: 0.2,
            move_sensitivity: 1.0,
            ..JoystickConfig::default()
        };

        // Inside dead zone -> exactly zero.
        assert_eq!(EditorJoystick::target_velocity(Vec2::new(0.0, 0.0), &cfg), Vec2::ZERO);
        assert_eq!(EditorJoystick::target_velocity(Vec2::new(0.1, 0.1), &cfg), Vec2::ZERO);
        assert_eq!(EditorJoystick::target_velocity(Vec2::new(0.2, 0.0), &cfg), Vec2::ZERO);

        // Just past dead zone -> tiny non-zero value (continuous from 0, no step jump).
        let just_past = EditorJoystick::target_velocity(Vec2::new(0.208, 0.0), &cfg);
        assert!(just_past.x > 0.0 && just_past.x < 0.02, "expected smooth start, got {:?}", just_past);

        // Halfway between dead_zone (0.2) and 1.0 is 0.6 -> rescaled 0.5.
        let mid = EditorJoystick::target_velocity(Vec2::new(0.6, 0.0), &cfg);
        assert!((mid.x - 0.5).abs() < 1e-5, "expected 0.5, got {:?}", mid);
        assert!(mid.y.abs() < 1e-6);

        // Full deflection -> 1.0.
        let full = EditorJoystick::target_velocity(Vec2::new(0.0, 1.0), &cfg);
        assert!((full.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cardinal_and_diagonal_vectors_are_normalized_and_smooth() {
        let cfg = enabled_config();
        let vp = landscape_viewport();
        let base = EditorJoystick::base_rect(vp, &cfg);
        let center = base.center();
        let r = base.width() * 0.5;

        let cases = [
            // (screen offset, expected unit direction (strafe_x, forward_z))
            (EguiVec2::new(0.0, -r), Vec3::new(0.0, 0.0, 1.0)),  // forward (up)
            (EguiVec2::new(0.0, r), Vec3::new(0.0, 0.0, -1.0)),  // back (down)
            (EguiVec2::new(-r, 0.0), Vec3::new(-1.0, 0.0, 0.0)), // left
            (EguiVec2::new(r, 0.0), Vec3::new(1.0, 0.0, 0.0)),   // right
            (
                EguiVec2::new(r, -r),
                Vec3::new(std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2),
            ), // forward-right diagonal (clamped to unit circle, no axis snap)
            (
                EguiVec2::new(-r * 0.8, r * 0.6),
                Vec3::new(-0.8, 0.0, -0.6),
            ), // arbitrary 360° angle preserved
        ];

        for (i, (offset, expected)) in cases.iter().enumerate() {
            let mut stick = EditorJoystick::new();
            let id = TouchId(10 + i as u64);
            assert!(stick.handle_touch(id, TouchPhase::Start, center, vp, false, &cfg));
            assert!(stick.handle_touch(id, TouchPhase::Move, center + *offset, vp, false, &cfg));

            let mut mv = Vec3::ZERO;
            for _ in 0..80 {
                mv = stick.update_and_move(1.0 / 60.0, &cfg);
            }
            assert_eq!(mv.y, 0.0);
            assert!(
                (mv - *expected).length() < 1e-3,
                "case {i}: expected {expected:?}, got {mv:?}"
            );
            assert!(mv.length() <= 1.0 + 1e-5);
        }
    }

    #[test]
    fn smooth_acceleration_and_immediate_stop_on_release_and_cancel() {
        let cfg = enabled_config();
        let vp = landscape_viewport();
        let base = EditorJoystick::base_rect(vp, &cfg);
        let center = base.center();
        let r = base.width() * 0.5;

        // 1. Release via TouchPhase::End
        let mut stick = EditorJoystick::new();
        let id = TouchId(1);
        assert!(stick.handle_touch(id, TouchPhase::Start, center, vp, false, &cfg));
        assert!(stick.handle_touch(id, TouchPhase::Move, center + EguiVec2::new(0.0, -r), vp, false, &cfg));

        let v1 = stick.update_and_move(1.0 / 60.0, &cfg);
        let v2 = stick.update_and_move(1.0 / 60.0, &cfg);
        let v3 = stick.update_and_move(1.0 / 60.0, &cfg);
        assert!(v1.z > 0.0 && v1.z < 0.5, "should ramp smoothly, got {}", v1.z);
        assert!(v2.z > v1.z && v3.z > v2.z, "velocity should monotonically accelerate");

        // Lift finger -> immediate stop, zero velocity, zero drift.
        assert!(stick.handle_touch(id, TouchPhase::End, center + EguiVec2::new(0.0, -r), vp, false, &cfg));
        assert!(!stick.is_active());
        assert_eq!(stick.velocity(), Vec2::ZERO);
        assert_eq!(stick.update_and_move(1.0 / 60.0, &cfg), Vec3::ZERO);

        // 2. Interruption via TouchPhase::Cancel
        let id2 = TouchId(2);
        assert!(stick.handle_touch(id2, TouchPhase::Start, center, vp, false, &cfg));
        assert!(stick.handle_touch(id2, TouchPhase::Move, center + EguiVec2::new(r, 0.0), vp, false, &cfg));
        let moving = stick.update_and_move(0.05, &cfg);
        assert!(moving.x > 0.2);

        assert!(stick.handle_touch(id2, TouchPhase::Cancel, center + EguiVec2::new(r, 0.0), vp, false, &cfg));
        assert!(!stick.is_active());
        assert_eq!(stick.velocity(), Vec2::ZERO);
        assert_eq!(stick.raw_deflection(), Vec2::ZERO);
        assert_eq!(stick.update_and_move(1.0 / 60.0, &cfg), Vec3::ZERO);
    }

    #[test]
    fn play_mode_immediately_resets_joystick_and_look() {
        let cfg = enabled_config();
        let vp = landscape_viewport();
        let base = EditorJoystick::base_rect(vp, &cfg);
        let center = base.center();
        let r = base.width() * 0.5;

        let mut stick = EditorJoystick::new();
        let mut look = TouchLookController::new();

        // Engage joystick and right-side look in Edit mode.
        assert!(stick.handle_touch(TouchId(1), TouchPhase::Start, center, vp, false, &cfg));
        assert!(stick.handle_touch(TouchId(1), TouchPhase::Move, center + EguiVec2::new(0.0, -r), vp, false, &cfg));
        assert!(stick.update_and_move(0.05, &cfg).z > 0.2);

        let right_start = Pos2::new(700.0, 260.0);
        assert_eq!(look.handle_touch(TouchId(2), TouchPhase::Start, right_start, vp, false, 1, &cfg), Vec2::ZERO);
        let d = look.handle_touch(TouchId(2), TouchPhase::Move, right_start + EguiVec2::new(24.0, -8.0), vp, false, 1, &cfg);
        assert!(d.length() > 0.0);
        assert!(look.is_rotating());

        // Enter Play mode -> both immediately reset and stop consuming input.
        assert!(!stick.is_active_in_editor(true, &cfg));
        assert!(!stick.is_active());
        assert_eq!(stick.velocity(), Vec2::ZERO);
        assert!(!stick.handle_touch(TouchId(3), TouchPhase::Start, center, vp, true, &cfg));

        let play_delta = look.handle_touch(
            TouchId(2),
            TouchPhase::Move,
            right_start + EguiVec2::new(40.0, -8.0),
            vp,
            true,
            1,
            &cfg,
        );
        assert_eq!(play_delta, Vec2::ZERO);
        assert!(!look.is_rotating());
        assert!(!look.is_tracking());
        assert!(!look.consumed_tap());
    }

    #[test]
    fn touch_look_no_initial_jump_sensitivity_scaling_selection_suppression_and_cancel() {
        let mut cfg = enabled_config();
        cfg.look_sensitivity = 1.75;
        let vp = landscape_viewport();
        let joy = joystick_rect(vp, &cfg);
        let mut look = TouchLookController::new();

        // Touch inside joystick zone is ignored by TouchLookController.
        assert_eq!(
            look.handle_touch(TouchId(99), TouchPhase::Start, joy.center(), vp, false, 1, &cfg),
            Vec2::ZERO
        );
        assert!(!look.is_tracking());

        // Start on right side -> zero initial delta, not rotating yet.
        let p0 = Pos2::new(640.0, 280.0);
        assert_eq!(
            look.handle_touch(TouchId(5), TouchPhase::Start, p0, vp, false, 1, &cfg),
            Vec2::ZERO
        );
        assert!(look.is_tracking());
        assert!(!look.is_rotating());
        assert!(!look.consumed_tap());

        // Move past drag slop -> scales by look_sensitivity and suppresses selection.
        let p1 = p0 + EguiVec2::new(20.0, -10.0);
        let delta = look.handle_touch(TouchId(5), TouchPhase::Move, p1, vp, false, 1, &cfg);
        assert!((delta.x - 35.0).abs() < 1e-5);
        assert!((delta.y - -17.5).abs() < 1e-5);
        assert!(look.is_rotating());
        assert!(look.consumed_tap());

        // Single-frame spike > MAX_LOOK_JUMP_PT is ignored (no camera jump).
        let p_spike = p1 + EguiVec2::new(300.0, 0.0);
        assert_eq!(
            look.handle_touch(TouchId(5), TouchPhase::Move, p_spike, vp, false, 1, &cfg),
            Vec2::ZERO
        );

        // End drag -> stops rotating, keeps consumed_tap true for release-frame pick suppression.
        assert_eq!(
            look.handle_touch(TouchId(5), TouchPhase::End, p_spike, vp, false, 1, &cfg),
            Vec2::ZERO
        );
        assert!(!look.is_rotating());
        assert!(!look.is_tracking());
        assert!(look.consumed_tap());
        assert!(look.take_consumed_tap());
        assert!(!look.consumed_tap());

        // Cancel mid-drag resets everything cleanly.
        assert_eq!(
            look.handle_touch(TouchId(6), TouchPhase::Start, p0, vp, false, 1, &cfg),
            Vec2::ZERO
        );
        let _ = look.handle_touch(TouchId(6), TouchPhase::Move, p1, vp, false, 1, &cfg);
        assert!(look.is_rotating());
        assert_eq!(
            look.handle_touch(TouchId(6), TouchPhase::Cancel, p1, vp, false, 1, &cfg),
            Vec2::ZERO
        );
        assert!(!look.is_rotating());
        assert!(!look.is_tracking());
        assert!(!look.consumed_tap());

        // Multi-touch (2+ fingers) resets active look cleanly.
        assert_eq!(
            look.handle_touch(TouchId(7), TouchPhase::Start, p0, vp, false, 1, &cfg),
            Vec2::ZERO
        );
        let _ = look.handle_touch(TouchId(7), TouchPhase::Move, p1, vp, false, 1, &cfg);
        assert!(look.is_rotating());
        assert_eq!(
            look.handle_touch(TouchId(7), TouchPhase::Move, p1 + EguiVec2::new(5.0, 0.0), vp, false, 2, &cfg),
            Vec2::ZERO
        );
        assert!(!look.is_rotating());
        assert!(!look.is_tracking());
    }

    #[test]
    fn base_rect_adapts_to_portrait_landscape_and_small_viewports() {
        let cfg = enabled_config();
        let viewports = [
            landscape_viewport(),
            portrait_viewport(),
            // Very small / split-screen viewport
            Rect::from_min_size(Pos2::new(12.0, 20.0), EguiVec2::new(90.0, 80.0)),
            // Tiny viewport smaller than default joystick size
            Rect::from_min_size(Pos2::new(0.0, 0.0), EguiVec2::new(48.0, 48.0)),
        ];

        for vp in viewports {
            let rect = EditorJoystick::base_rect(vp, &cfg);
            assert!(rect.is_positive(), "rect should be positive for {vp:?}");
            assert!(rect.min.x >= vp.min.x - 1e-4, "left clipped: {rect:?} vs {vp:?}");
            assert!(rect.min.y >= vp.min.y - 1e-4, "top clipped: {rect:?} vs {vp:?}");
            assert!(rect.max.x <= vp.max.x + 1e-4, "right clipped: {rect:?} vs {vp:?}");
            assert!(rect.max.y <= vp.max.y + 1e-4, "bottom clipped: {rect:?} vs {vp:?}");
            assert!((rect.width() - rect.height()).abs() < 1e-4, "joystick must remain circular/square");
        }

        // Huge custom anchor_offset still clamps inside viewport.
        let far_cfg = JoystickConfig {
            enabled: true,
            anchor_offset: [5000.0, 5000.0],
            ..JoystickConfig::default()
        };
        let vp = portrait_viewport();
        let clamped_rect = joystick_rect(vp, &far_cfg);
        assert!(clamped_rect.min.x >= vp.min.x && clamped_rect.max.x <= vp.max.x);
        assert!(clamped_rect.min.y >= vp.min.y && clamped_rect.max.y <= vp.max.y);
    }

    #[test]
    fn apply_to_camera_moves_and_rotates_camera() {
        let cfg = enabled_config();
        let vp = landscape_viewport();
        let base = EditorJoystick::base_rect(vp, &cfg);
        let center = base.center();
        let r = base.width() * 0.5;

        let mut cam = Camera::default();
        let start_pos = cam.pos;
        let start_yaw = cam.yaw;

        let mut stick = EditorJoystick::new();
        assert!(stick.handle_touch(TouchId(1), TouchPhase::Start, center, vp, false, &cfg));
        assert!(stick.handle_touch(TouchId(1), TouchPhase::Move, center + EguiVec2::new(0.0, -r), vp, false, &cfg));
        assert!(stick.apply_to_camera(&mut cam, 0.05, 40.0, false, &cfg));
        let moved = cam.pos - start_pos;
        assert!(moved.dot(cam.forward()) > 0.0, "camera should move forward");

        let look = TouchLookController::new();
        assert!(look.apply_to_camera(&mut cam, Vec2::new(25.0, 0.0)));
        assert!(cam.yaw < start_yaw, "positive dx should turn camera right (decreasing yaw)");
    }
}
