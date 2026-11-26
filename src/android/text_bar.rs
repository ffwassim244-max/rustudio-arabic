//! Portrait-only horizontally scrollable Script Editor helper bar sitting right
//! above the soft keyboard:
//! - Arrow buttons (`◀`, `▶`, `▲`, `▼`) to move the `|` caret inside the editor
//! - Spring-back center-snap slider (`◀──●──▶`) that slides the `|` caret left/right
//!   and snaps back to center on release
//! - `Select` toggle (holds Shift while dragging the slider or tapping arrows to select text)
//! - `Select All`, `Cut`, `Copy`, `Paste` (always live)

use egui::{Align, Color32, Event, Id, Key, Layout, Modifiers, Order, Pos2, Rect, RichText, Stroke, Vec2};

use super::keyboard::{Keyboard, Want};

/// The portrait helper bar's height, in points.
pub const HEIGHT: f32 = 44.0;

const ID: &str = "rustudio text bar";

#[derive(Default)]
pub struct TextBar {
    /// Events queued for the next frame's `raw_input_hook`.
    queued: Vec<Event>,
    /// The bar showed last frame.
    shown: bool,
    /// When true, arrow buttons and the spring-back slider hold Shift to select text.
    select_mode: bool,
    /// Current spring-back slider normalized offset in `[-1.0, 1.0]` (snaps to 0.0 on release).
    slider_norm: f32,
    /// Accumulated fractional cursor steps while sliding.
    slide_accum: f32,
    /// Last screen rect of the spring-back slider (registered in `direct_drag` so it slides immediately).
    pub slider_rect: Option<Rect>,
}

impl TextBar {
    /// In `App::raw_input_hook`: feed queued editing/navigation events to egui.
    pub fn before_frame(&mut self, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.queued);
    }

    fn push_key_tap(&mut self, key: Key, modifiers: Modifiers) {
        self.queued.push(Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        });
        self.queued.push(Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        });
    }

    /// At the end of `App::ui`: draws the horizontally scrollable helper bar
    /// **only in Portrait mode** (`screen.height() > screen.width()`) while the
    /// soft keyboard is up for an egui text editor.
    pub fn end_frame(&mut self, _ctx: &egui::Context, _keyboard: &Keyboard, _want: Option<Want>) {
        // Portrait Script Editor helper bar is now rendered directly inside
        // `script_editor.rs` as an `egui::Panel::bottom` so it is never clipped
        // by `safe_area_insets` or `content_rect` and never leaves a black gap.
        self.slider_rect = None;
        return;
        #[allow(unreachable_code)]
        let (ctx, keyboard, want) = (_ctx, _keyboard, _want);
        let screen = ctx.viewport_rect();
        let is_portrait = screen.height() > screen.width();
        let keyboard_h = keyboard.height(ctx);

        let Some(Want::Field(field_id)) = want.filter(|_| is_portrait && keyboard_h > 0.0) else {
            if self.shown {
                self.shown = false;
                self.select_mode = false;
                self.slider_norm = 0.0;
                self.slide_accum = 0.0;
            }
            return;
        };

        if !self.shown {
            self.shown = true;
            super::clipboard::prime();
        }

        let rect = Rect::from_min_max(
            Pos2::new(screen.min.x, (screen.max.y - keyboard_h - HEIGHT).max(screen.min.y)),
            Pos2::new(screen.max.x, (screen.max.y - keyboard_h).max(screen.min.y + HEIGHT)),
        );

        let selection = egui::TextEdit::load_state(ctx, field_id)
            .and_then(|s| s.cursor.char_range())
            .is_some_and(|r| !r.is_empty());

        let mut acted = false;
        let dt = ctx.input(|i| i.stable_dt).clamp(0.001, 0.1);
        let nav_mods = if self.select_mode { Modifiers::SHIFT } else { Modifiers::NONE };

        egui::Area::new(Id::new(ID))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                egui::Frame::new()
                    .fill(Color32::from_rgb(24, 26, 32))
                    .stroke(Stroke::new(1.0, rustudio_engine::theme::DIVIDER))
                    .inner_margin(egui::Margin::symmetric(6, 5))
                    .show(ui, |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt("portrait_text_bar_scroll")
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                            .show(ui, |ui| {
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let btn_h = 32.0;

                                    // 1. Directional arrows to move `|` inside the editor
                                    for (lbl, key) in [
                                        ("◀", Key::ArrowLeft),
                                        ("▶", Key::ArrowRight),
                                        ("▲", Key::ArrowUp),
                                        ("▼", Key::ArrowDown),
                                    ] {
                                        if ui.add(egui::Button::new(RichText::new(lbl).size(14.0)).min_size(Vec2::new(34.0, btn_h))).clicked() {
                                            self.push_key_tap(key, nav_mods);
                                            acted = true;
                                        }
                                    }

                                    // 2. Spring-back center-snap cursor slider (slides left/right, snaps back to middle!)
                                    let (track_rect, track_resp) = ui.allocate_exact_size(
                                        Vec2::new(136.0, btn_h),
                                        egui::Sense::click_and_drag(),
                                    );
                                    self.slider_rect = Some(track_rect.expand(6.0));

                                    if track_resp.dragged() {
                                        if let Some(pos) = track_resp.interact_pointer_pos() {
                                            let half = (track_rect.width() * 0.5 - 12.0).max(1.0);
                                            let norm = ((pos.x - track_rect.center().x) / half).clamp(-1.0, 1.0);
                                            self.slider_norm = norm;
                                            // Proportional continuous rate: gentle slide = precise char-by-char,
                                            // full deflection = fast smooth glide across the line.
                                            let abs_n = norm.abs();
                                            if abs_n > 0.14 {
                                                let speed = 5.0 + 26.0 * ((abs_n - 0.14) / 0.86).powf(1.5);
                                                self.slide_accum += norm.signum() * speed * dt;
                                                while self.slide_accum >= 1.0 {
                                                    self.slide_accum -= 1.0;
                                                    self.push_key_tap(Key::ArrowRight, nav_mods);
                                                    acted = true;
                                                }
                                                while self.slide_accum <= -1.0 {
                                                    self.slide_accum += 1.0;
                                                    self.push_key_tap(Key::ArrowLeft, nav_mods);
                                                    acted = true;
                                                }
                                            }
                                            ctx.request_repaint();
                                        }
                                    } else {
                                        // Snap smoothly back to center on release
                                        if self.slider_norm.abs() > 0.01 {
                                            self.slider_norm = 0.0;
                                            ctx.request_repaint();
                                        }
                                        self.slide_accum = 0.0;
                                    }

                                    // Paint the spring-back slider track, center tick, and thumb
                                    let p = ui.painter();
                                    let track_bg = if self.select_mode {
                                        Color32::from_rgb(36, 46, 68)
                                    } else {
                                        Color32::from_rgb(34, 37, 46)
                                    };
                                    p.rect_filled(track_rect, 15.0, track_bg);
                                    p.rect_stroke(
                                        track_rect,
                                        15.0,
                                        Stroke::new(1.0, if track_resp.dragged() { rustudio_engine::theme::ACCENT } else { rustudio_engine::theme::DIVIDER }),
                                        egui::StrokeKind::Inside,
                                    );
                                    // Center reference tick
                                    let cx = track_rect.center().x;
                                    let cy = track_rect.center().y;
                                    p.line_segment(
                                        [Pos2::new(cx, track_rect.min.y + 6.0), Pos2::new(cx, track_rect.max.y - 6.0)],
                                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(180, 190, 210, 70)),
                                    );
                                    // Left / right hint chevrons
                                    p.text(
                                        Pos2::new(track_rect.min.x + 10.0, cy),
                                        egui::Align2::CENTER_CENTER,
                                        "‹",
                                        egui::FontId::proportional(13.0),
                                        rustudio_engine::theme::TEXT_DIM,
                                    );
                                    p.text(
                                        Pos2::new(track_rect.max.x - 10.0, cy),
                                        egui::Align2::CENTER_CENTER,
                                        "›",
                                        egui::FontId::proportional(13.0),
                                        rustudio_engine::theme::TEXT_DIM,
                                    );
                                    // Movable thumb (snaps to center when idle)
                                    let max_travel = (track_rect.width() * 0.5 - 14.0).max(1.0);
                                    let thumb_x = cx + self.slider_norm * max_travel;
                                    let thumb_color = if self.select_mode {
                                        Color32::from_rgb(80, 165, 255)
                                    } else if track_resp.dragged() {
                                        rustudio_engine::theme::ACCENT
                                    } else {
                                        Color32::from_rgb(200, 210, 228)
                                    };
                                    p.circle_filled(Pos2::new(thumb_x, cy), 10.5, thumb_color);

                                    // 3. Select toggle (holds Shift while sliding or tapping arrows)
                                    let sel_btn = if self.select_mode {
                                        egui::Button::new(RichText::new("تحديد").size(12.5).strong().color(Color32::WHITE))
                                            .fill(rustudio_engine::theme::ACCENT)
                                            .min_size(Vec2::new(58.0, btn_h))
                                    } else {
                                        egui::Button::new(RichText::new("تحديد").size(12.5))
                                            .min_size(Vec2::new(58.0, btn_h))
                                    };
                                    if ui.add(sel_btn).clicked() {
                                        self.select_mode = !self.select_mode;
                                        acted = true;
                                    }

                                    // 4. Select All, Cut, Copy, Paste
                                    if ui.add(egui::Button::new(RichText::new("تحديد الكل").size(12.5)).min_size(Vec2::new(72.0, btn_h))).clicked() {
                                        self.push_key_tap(Key::A, Modifiers::COMMAND);
                                        self.select_mode = false;
                                        acted = true;
                                    }
                                    if ui.add_enabled(selection, egui::Button::new(RichText::new("قص").size(12.5)).min_size(Vec2::new(46.0, btn_h))).clicked() {
                                        self.queued.push(Event::Cut);
                                        self.select_mode = false;
                                        acted = true;
                                    }
                                    if ui.add_enabled(selection, egui::Button::new(RichText::new("نسخ").size(12.5)).min_size(Vec2::new(52.0, btn_h))).clicked() {
                                        self.queued.push(Event::Copy);
                                        self.select_mode = false;
                                        acted = true;
                                    }
                                    if ui.add(egui::Button::new(RichText::new("لصق").size(12.5)).min_size(Vec2::new(54.0, btn_h))).clicked() {
                                        let text = super::clipboard::text_now()
                                            .or_else(super::clipboard::text)
                                            .unwrap_or_default();
                                        if !text.is_empty() {
                                            self.queued.push(Event::Paste(text));
                                        }
                                        self.select_mode = false;
                                        acted = true;
                                    }
                                });
                            });
                    });
            });

        if acted || self.slider_norm != 0.0 {
            if !ctx.memory(|m| m.has_focus(field_id)) {
                ctx.memory_mut(|m| m.request_focus(field_id));
            }
            ctx.request_repaint();
        }
    }
}
