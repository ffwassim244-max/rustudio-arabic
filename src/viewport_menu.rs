//! Studio's viewport menu. Commands share the ribbon's availability and dispatch.
use crate::app::Editor;
use egui::{PointerButton, Response, Ui};

impl Editor {
    pub(crate) fn viewport_context_menu(&mut self, ui: &mut Ui, response: &Response) {
        if self.game.is_playing() { return; }
        if response.clicked_by(PointerButton::Secondary) {
            if let Some(p) = response.interact_pointer_pos() {
                let hit = self.pick(response.rect, p, ui.input(|i| i.modifiers.alt));
                match hit {
                    Some(r) if !self.game.doc.selection.contains(&r) => self.select(vec![r]),
                    None => self.select(Vec::new()),
                    _ => {}
                }
            }
        }
        let mut action = None;
        response.context_menu(|ui| {
            ui.set_min_width(254.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut item = |ui: &mut Ui, label: &str, shortcut: &str, command: &'static str| {
                let enabled = match command {
                    "parents" => self.game.doc.selection.iter().any(|r| self.game.doc.dom.get_by_ref(*r).is_some_and(|i| i.parent().is_some())),
                    "children" => self.game.doc.selection.iter().any(|r| self.game.doc.dom.get_by_ref(*r).is_some_and(|i| !i.children().is_empty())),
                    "focus" => self.selection_bounds().is_some(),
                    _ => self.action_state(command).0,
                };
                // Checkable items (Align Dragged Objects) show their tick.
                let checked = if command == "align_dragged" { self.action_state(command).1 } else { None };
                if crate::studio_actions::menu_item_at(ui, label, shortcut, enabled, checked, 182.0, 254.0).clicked() {
                    action = Some(command);
                    ui.close();
                }
            };
            item(ui,"قص","Ctrl+X","cut");
            item(ui,"نسخ","Ctrl+C","copy");
            ui.add_enabled_ui(false, |ui| { ui.menu_button("نسخ باسم", |_| {}); });
            item(ui,"لصق","Ctrl+V","paste");
            item(ui,"لصق داخل","Ctrl+Shift+V","paste_into");
            item(ui,"لصق في الموقع الأصلي","","paste_original");
            item(ui,"لصق داخل في الموقع الأصلي","","paste_into_original");
            item(ui,"تكرار","Ctrl+D","duplicate");
            item(ui,"حذف","Del","delete");
            ui.separator();
            item(ui,"تجميع كنموذج","Ctrl+G","group");
            item(ui,"تجميع كمجلد","Ctrl+Alt+G","group_folder");
            item(ui,"فك التجميع","Ctrl+U","ungroup");
            item(ui,"دمج","Ctrl+Shift+G","union");
            item(ui,"تقاطع","Ctrl+Shift+I","intersect");
            item(ui,"نفي","Ctrl+Shift+N","negate");
            item(ui,"فصل","Ctrl+Shift+U","separate");
            ui.add_enabled_ui(false, |ui| { ui.menu_button("توليد", |_| {}); });
            item(ui,"تحديد العنصر الأب","","parents");
            item(ui,"تحديد العناصر الأبناء","","children");
            item(ui,"تكبير إلى","F","focus");
            item(ui,"إظهار مؤشر الاتجاه","","orientation_indicator");
            item(ui,"محاذاة الكائنات المسحوبة","","align_dragged");
            ui.separator();
            item(ui,"تحديد الاتصالات","Alt+C","connections");
            item(ui,"تحديد الاتصال 0","Alt+1","connection_0");
            item(ui,"تحديد الاتصال 1","Alt+2","connection_1");
            item(ui,"تبديل المرفقات","","swap_attachments");
            ui.separator();
            item(ui,"إدراج جزء","","part");
            item(ui,"إدراج كائن…","Ctrl+I","insert_object");
            ui.separator();
            for label in ["تحويل إلى حزمة", "نشر في الحزمة", "التراجع عن التغييرات في الحزمة", "الحصول على أحدث حزمة", "عرض تغييرات السكريبت…", "تفاصيل الحزمة", "تحديث الكل", "مقارنة إصدارات الحزمة"] {
                item(ui,label,"","unsupported_package");
            }
            ui.separator();
            item(ui,"مساعدة","","documentation");
        });
        if let Some(action) = action {
            match action {
                "parents" | "children" => {
                    let mut refs = Vec::new();
                    for r in &self.game.doc.selection {
                        if let Some(i) = self.game.doc.dom.get_by_ref(*r) {
                            if action == "parents" {
                                if i.parent().is_some() && !refs.contains(&i.parent()) { refs.push(i.parent()); }
                            } else {
                                for r in i.children() { if !refs.contains(r) { refs.push(*r); } }
                            }
                        }
                    }
                    self.select(refs);
                }
                "focus" => self.focus_selection(),
                _ => self.ui_action(ui.ctx(), action),
            }
        }
    }
}
