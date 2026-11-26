//! View > GPU Driver: displays the active native graphics adapter, backend
//! (Vulkan or GLES), driver metadata, hardware limits, and quick links to
//! Studio Settings and the Performance Overlay.

use egui::RichText;

use crate::app::Editor;

impl Editor {
    /// The window, while View > GPU Driver has it open.
    pub fn graphics_settings_ui(&mut self, ctx: &egui::Context) {
        if !self.ui.graphics_settings {
            return;
        }
        let mut open = true;
        let screen = ctx.content_rect();
        egui::Window::new("GPU & Display Info")
            .id(egui::Id::new("graphics_settings_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .default_pos(screen.center() - egui::vec2(200.0, (screen.height() * 0.45).min(320.0)))
            .vscroll(true)
            .show(ctx, |ui| self.graphics_settings_body(ui));
        self.ui.graphics_settings = open;
    }

    fn graphics_settings_body(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.max(28.0);
        match self.game.render_state.as_ref() {
            Some(rs) => {
                let i = rs.adapter.get_info();
                let caps = rustudio_render::gpu_caps::GpuCaps::of_adapter(&rs.adapter, &rs.device);
                ui.label(RichText::new(&i.name).strong());
                ui.small(
                    format!("{:?}, {:?}; driver {} {}", i.backend, i.device_type, i.driver, i.driver_info)
                        .trim_end()
                        .to_string(),
                );
                ui.small(format!("Surface format: {:?}", rs.target_format));
                ui.small(format!(
                    "Limits: {} sampled textures/stage, max 2D {} px, MSAA {}, compute {}",
                    caps.limits.max_sampled_textures_per_shader_stage,
                    caps.limits.max_texture_dimension_2d,
                    if caps.sampled_msaa() { "supported" } else { "off (GLES)" },
                    if caps.compute() { "supported" } else { "off" },
                ));
                let scale_pct = (self.game.last_render_scale * 100.0).round() as u32;
                let [w, h] = self.game.last_render_target;
                if w > 0 && h > 0 {
                    ui.small(format!("3D render scale: {scale_pct}% ({w}x{h} px)"));
                }
            }
            None => {
                ui.label(RichText::new("No GPU renderer active").strong());
            }
        }
        ui.separator();
        ui.small("Using the device's native graphics driver (Vulkan with GLES 3 fallback).");
        ui.add_space(4.0);
        let mut overlay = rustudio_core::rustudio_settings::perf_overlay_enabled();
        if ui.checkbox(&mut overlay, "Show Performance Overlay").changed() {
            rustudio_core::rustudio_settings::set_perf_overlay_enabled(overlay);
        }
        let mut float_win = rustudio_core::rustudio_settings::mobile_floating_windows();
        if ui.checkbox(&mut float_win, "Floating Mobile Windows (drag / resize / tap title to collapse)").changed() {
            rustudio_core::rustudio_settings::set_mobile_floating_windows(float_win);
        }
        ui.separator();
        ui.label(RichText::new("Editor UI Size").strong());
        let screen = ui.ctx().content_rect();
        let native_ppp = ui.ctx().native_pixels_per_point().unwrap_or(1.0).max(0.5);
        let longer_px = screen.width().max(screen.height()) * ui.ctx().pixels_per_point();
        let min_comfortable = (longer_px / (native_ppp * 1100.0)).clamp(0.75, 1.60) * 0.72;
        ui.horizontal(|ui| {
            let mut pct = (self.ui.zoom * 100.0).round().clamp(50.0, 200.0);
            let resp = ui.add(
                egui::Slider::new(&mut pct, 50.0..=200.0)
                    .step_by(5.0)
                    .suffix("%")
                    .text("Custom"),
            );
            if resp.changed() {
                let z = (pct / 100.0).clamp(0.50, 2.00);
                self.ui.zoom = z;
                self.auto_zoom = false;
                rustudio_core::rustudio_settings::set_ui_scale(Some(z));
            }
            if ui.button("Auto").clicked() {
                rustudio_core::rustudio_settings::set_ui_scale(None);
                self.auto_zoom = true;
            }
        });
        if self.ui.zoom < min_comfortable {
            ui.label(RichText::new("⚠ Current UI size may be too small for your screen").small().color(egui::Color32::from_rgb(245, 190, 70)));
        }
        ui.separator();
        ui.label(RichText::new("Editor Virtual Joystick & Touch Look").strong());
        let mut joy_on = rustudio_core::rustudio_settings::joystick_enabled();
        if ui.checkbox(&mut joy_on, "Enable Left-Side Editor Virtual Joystick").changed() {
            rustudio_core::rustudio_settings::set_joystick_enabled(joy_on);
            self.joystick_config.enabled = joy_on;
        }
        ui.add_enabled_ui(joy_on, |ui| {
            use crate::editor_joystick as ej;
            ui.add(egui::Slider::new(&mut self.joystick_config.size, ej::SIZE_RANGE).text("Joystick size (pt)"));
            ui.add(egui::Slider::new(&mut self.joystick_config.opacity, ej::OPACITY_RANGE).text("Opacity"));
            ui.add(egui::Slider::new(&mut self.joystick_config.dead_zone, ej::DEAD_ZONE_RANGE).text("Dead zone"));
            ui.add(egui::Slider::new(&mut self.joystick_config.move_sensitivity, ej::MOVE_SENSITIVITY_RANGE).text("Move sensitivity"));
            ui.add(egui::Slider::new(&mut self.joystick_config.move_speed, ej::MOVE_SPEED_RANGE).text("Move speed"));
            ui.add(egui::Slider::new(&mut self.joystick_config.look_sensitivity, ej::LOOK_SENSITIVITY_RANGE).text("Right-side look sensitivity"));
            ui.horizontal(|ui| {
                ui.label("Position offset:");
                ui.add(egui::DragValue::new(&mut self.joystick_config.anchor_offset[0]).range(0.0..=320.0).prefix("X "));
                ui.add(egui::DragValue::new(&mut self.joystick_config.anchor_offset[1]).range(0.0..=320.0).prefix("Y "));
            });
            self.joystick_config = self.joystick_config.clamped();
        });
        ui.add_space(4.0);
        if ui.button("Open Studio Settings (Graphics & Display)...").clicked() {
            self.ui.studio_settings.open_at("rustudio", false);
        }
        ui.small(
            RichText::new(
                "Configure quality presets (Automatic, Low, Medium, High, Custom), 3D render resolution scale, shadows, AA, and virtual joystick in File > Studio Settings > rustudio.",
            )
            .weak(),
        );
    }
}
