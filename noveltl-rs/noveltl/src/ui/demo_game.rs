//! Окно «игры» для демо (перенос `DemoGameWindow` из `noveltl/demo.py`): по щелчку — следующая реплика.

use eframe::egui::{
    self, epaint::Vertex, pos2, vec2, Color32, CornerRadius, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, ViewportBuilder, ViewportId,
};

use crate::demo::{DemoSource, DEMO_TITLE, SCRIPT};

const GAME_W: f32 = 1280.0;
const GAME_H: f32 = 720.0;

pub struct DemoGame {
    source: DemoSource,
    /// номер показанной реплики (-1 — ещё ничего)
    pub index: i32,
    /// HWND окна (0 — ещё не найден)
    pub hwnd: isize,
}

fn hex(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

fn vgradient(painter: &Painter, rect: Rect, stops: &[(f32, Color32)]) {
    let mut mesh = Mesh::default();
    for (t, color) in stops {
        let y = rect.top() + rect.height() * t;
        mesh.vertices.push(Vertex { pos: pos2(rect.left(), y), uv: egui::epaint::WHITE_UV, color: *color });
        mesh.vertices.push(Vertex { pos: pos2(rect.right(), y), uv: egui::epaint::WHITE_UV, color: *color });
    }
    for i in 0..stops.len().saturating_sub(1) as u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(Shape::mesh(mesh));
}

impl DemoGame {
    pub fn new(source: DemoSource) -> Self {
        DemoGame { source, index: -1, hwnd: 0 }
    }

    /// «Игра» показала следующую реплику: хуки шлют её контроллеру.
    pub fn advance(&mut self) {
        self.index += 1;
        self.source.emit_line(self.index as usize);
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let builder = ViewportBuilder::default()
            .with_title(DEMO_TITLE)
            .with_inner_size([GAME_W, GAME_H])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_position(match crate::winutil::test_area() {
                Some([x, y, _, _]) => [x + 40.0, y + 40.0],
                None => [80.0, 60.0],
            });
        ctx.show_viewport_immediate(ViewportId::from_hash_of("noveltl_demo_game"), builder, |ctx, _class| {
            let mut clicked = false;
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    clicked = true;
                }
                self.paint(&ui.painter_at(rect), rect);
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter)) {
                clicked = true;
            }
            if clicked {
                self.advance();
                ctx.request_repaint();
            }
        });
    }

    fn paint(&self, p: &Painter, rect: Rect) {
        let (w, h) = (rect.width(), rect.height());
        let at = |x: f32, y: f32| rect.min + vec2(x, y);
        vgradient(p, rect, &[(0.0, hex(0xf6c7a8)), (0.45, hex(0xe79aa5)), (1.0, hex(0x5b4a7a))]);
        // условные холмы и «тории»
        let hill = Color32::from_rgba_unmultiplied(60, 45, 85, 150);
        let ellipse = |x: f32, y: f32, ew: f32, eh: f32| {
            p.add(Shape::ellipse_filled(at(x + ew / 2.0, y + eh / 2.0), vec2(ew / 2.0, eh / 2.0), hill));
        };
        ellipse(-200.0, h * 0.55, w * 0.9, h * 0.8);
        ellipse(w * 0.45, h * 0.6, w * 0.9, h * 0.8);
        let red = hex(0xb5373f);
        let (gx, gy) = (w * 0.5 - 150.0, h * 0.22);
        p.rect_filled(Rect::from_min_size(at(gx - 30.0, gy), vec2(360.0, 22.0)), CornerRadius::same(6), red);
        p.rect_filled(Rect::from_min_size(at(gx, gy + 50.0), vec2(300.0, 14.0)), 0.0, red);
        p.rect_filled(Rect::from_min_size(at(gx + 30.0, gy + 20.0), vec2(22.0, 300.0)), 0.0, red);
        p.rect_filled(Rect::from_min_size(at(gx + 248.0, gy + 20.0), vec2(22.0, 300.0)), 0.0, red);

        // диалоговое окно игры
        let bx = Rect::from_min_size(at(60.0, h - 210.0), vec2(w - 120.0, 180.0));
        p.rect(
            bx,
            CornerRadius::same(14),
            Color32::from_rgba_unmultiplied(20, 14, 34, 215),
            Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 60)),
            StrokeKind::Inside,
        );
        if self.index < 0 {
            p.text(
                bx.center(),
                egui::Align2::CENTER_CENTER,
                "Это имитация игры. Щёлкайте здесь, чтобы листать реплики.",
                FontId::new(21.0, FontFamily::Proportional),
                Color32::WHITE,
            );
            return;
        }
        let line = &SCRIPT[self.index as usize % SCRIPT.len()];
        let jp = crate::ui::theme::jp_family();
        if !line.name.is_empty() {
            let g = p.layout_no_wrap(
                line.name.to_string(),
                FontId::new(23.0, crate::ui::theme::bold_family()),
                hex(0xffd27f),
            );
            p.galley(bx.min + vec2(28.0, 14.0), g, hex(0xffd27f));
        }
        let g = p.layout(line.text.to_string(), FontId::new(27.0, jp), Color32::WHITE, bx.width() - 56.0);
        let pos: Pos2 = bx.min + vec2(28.0, 58.0);
        p.galley(pos, g, Color32::WHITE);
    }
}
