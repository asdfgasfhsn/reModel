//! The reModel logo on black at startup, fading out over the game (or the
//! menu) as it starts. It never holds anything up: the game boots underneath.

use std::time::{Duration, Instant};

/// The logo, half the size of `assets/reModel-logo.png`.
const LOGO: &[u8] = include_bytes!("../../../assets/reModel-logo-1086.png");
/// How long the logo shows, then fades.
const HOLD: Duration = Duration::from_millis(1500);
const FADE: Duration = Duration::from_millis(500);
/// Largest share of the window's width and height the logo takes.
const FIT: (f32, f32) = (0.7, 0.5);

pub struct Splash {
    /// When it first drew; None before that.
    start: Option<Instant>,
    done: bool,
    texture: Option<egui::TextureHandle>,
}

impl Splash {
    pub fn new(enabled: bool) -> Self {
        Splash { start: None, done: !enabled, texture: None }
    }

    pub fn active(&self) -> bool {
        !self.done
    }

    /// End it now (a key, click or button).
    pub fn skip(&mut self) {
        self.done = true;
        self.texture = None;
    }

    /// Opacity now: 1 while holding, falling to 0 over the fade.
    fn alpha(&mut self) -> f32 {
        let t = self.start.get_or_insert_with(Instant::now).elapsed();
        if t < HOLD {
            1.0
        } else {
            1.0 - (t - HOLD).as_secs_f32() / FADE.as_secs_f32()
        }
    }

    /// Draw it over everything else egui draws this frame.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if self.done {
            return;
        }
        let alpha = self.alpha();
        if alpha <= 0.0 {
            return self.skip();
        }
        let texture = self.texture.get_or_insert_with(|| {
            let image = png::decode(LOGO).expect("embedded logo");
            let image = egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba);
            ctx.load_texture("splash", image, egui::TextureOptions::LINEAR)
        });
        let screen = ctx.content_rect();
        let size = texture.size_vec2();
        let scale = (screen.width() * FIT.0 / size.x).min(screen.height() * FIT.1 / size.y);
        let rect = egui::Rect::from_center_size(screen.center(), size * scale);
        let a = (alpha * 255.0).round() as u8;
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("splash")));
        painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(a));
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture.id(), rect, uv, egui::Color32::from_white_alpha(a));
        // Keep the fade moving even when nothing else redraws egui.
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_decodes() {
        let image = png::decode(LOGO).unwrap();
        assert_eq!((image.width, image.height), (1086, 362));
        let px = |x: usize, y: usize| &image.rgba[(y * image.width + x) * 4..][..4];
        // Transparent around the badge.
        assert_eq!(px(0, 0)[3], 0);
        assert_eq!(px(1085, 361)[3], 0);
        // The white of "Model" and the red underline, opaque.
        assert_eq!(px(600, 200), [254, 252, 253, 254]);
        assert_eq!(px(500, 260), [252, 4, 20, 253]);
    }

    /// Headless: the logo is centred, fits the window, keeps its shape.
    #[test]
    fn logo_fits_window() {
        let ctx = egui::Context::default();
        let mut splash = Splash::new(true);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| splash.ui(ui.ctx()));
        out.textures_delta.clear();
        let rects: Vec<egui::Rect> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => Some(m.calc_bounds()),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 1);
        let r = rects[0];
        assert!((r.center().x - 640.0).abs() < 1.0 && (r.center().y - 360.0).abs() < 1.0);
        assert!((r.width() - 1280.0 * FIT.0).abs() < 1.0);
        assert!((r.width() / r.height() - 1086.0 / 362.0).abs() < 0.01);
        assert!(splash.active());
        splash.skip();
        assert!(!splash.active());
    }
}
