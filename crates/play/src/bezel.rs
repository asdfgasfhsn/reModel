//! The game's bezel: original artwork (`assets/bezels`, one 16:9 WebP per
//! set, its middle 4:3 plain) drawn beside the 4:3 picture when the window
//! is wider than it. It's only drawn: emulation, the HUD and the guns (the
//! area beside the picture is off the screen) don't know it's there.

include!(concat!(env!("OUT_DIR"), "/bezels.rs"));

/// The bezel art for `game` (its own, else its parent set's), decoded.
pub fn image(game: &romset::Game) -> Option<egui::ColorImage> {
    let bytes = std::iter::once(game.id).chain(game.parent).find_map(|s| BEZELS.iter().find(|b| b.0 == s))?.1;
    let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let (w, h) = d.dimensions();
    let mut buf = vec![0; d.output_buffer_size()?];
    d.read_image(&mut buf).ok()?;
    let size = [w as usize, h as usize];
    Some(if d.has_alpha() {
        egui::ColorImage::from_rgba_unmultiplied(size, &buf)
    } else {
        egui::ColorImage::from_rgb(size, &buf)
    })
}

/// Where the bezel goes, in window pixels: its rectangle (x, y, width,
/// height), the art's height matching the picture's and centred on it.
/// None when it doesn't show: a widened view (`pad`, game area added at
/// the sides), or a picture as wide as the window (a window 4:3 or
/// taller). `pic` is the picture's top left corner and size; `art` the
/// image's size.
pub fn layout(window_w: u32, pic: (u32, u32, u32, u32), pad: f32, art: [usize; 2]) -> Option<[f32; 4]> {
    let (x, y, w, h) = pic;
    if pad >= 1.0 || w + 1 >= window_w || art[1] == 0 {
        return None;
    }
    let bw = h as f32 * art[0] as f32 / art[1] as f32;
    Some([x as f32 + w as f32 / 2.0 - bw / 2.0, y as f32, bw, h as f32])
}

/// Paint the bezel at `rect` (window pixels, from `layout`) beside the
/// picture `pic` (left and right edges, pixels), behind everything else
/// egui draws; `scale` is pixels per point.
pub fn paint(
    ctx: &egui::Context,
    tex: &egui::TextureHandle,
    rect: [f32; 4],
    pic: (f32, f32),
    window_w: f32,
    scale: f32,
) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let [bx, by, bw, bh] = rect;
    // The parts left and right of the picture, cut to the window.
    for (x0, x1) in [(bx.max(0.0), pic.0), (pic.1, (bx + bw).min(window_w))] {
        if x1 <= x0 {
            continue;
        }
        let r = egui::Rect::from_min_max(egui::pos2(x0 / scale, by / scale), egui::pos2(x1 / scale, (by + bh) / scale));
        let uv = egui::Rect::from_min_max(egui::pos2((x0 - bx) / bw, 0.0), egui::pos2((x1 - bx) / bw, 1.0));
        painter.image(tex.id(), r, uv, egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::layout;

    #[test]
    fn beside_a_4_3_picture_only() {
        // 1920x1080 window, 1440x1080 picture in the middle: 16:9 art fills it.
        assert_eq!(layout(1920, (240, 0, 1440, 1080), 0.0, [1920, 1080]), Some([0.0, 0.0, 1920.0, 1080.0]));
        // Wider than the art: centred, the rest black.
        assert_eq!(layout(2560, (560, 0, 1440, 1080), 0.0, [1920, 1080]), Some([320.0, 0.0, 1920.0, 1080.0]));
        // A widened view, or a window 4:3 or taller: none.
        assert_eq!(layout(1920, (0, 0, 1920, 1080), 80.0, [1920, 1080]), None);
        assert_eq!(layout(1024, (0, 0, 1024, 768), 0.0, [1920, 1080]), None);
        assert_eq!(layout(1000, (0, 50, 1000, 750), 0.0, [1920, 1080]), None);
    }

    #[test]
    fn every_bezel_decodes() {
        for (set, _) in super::BEZELS {
            let game = romset::games::find(set).unwrap_or_else(|e| panic!("bezel for unknown set {set}: {e}"));
            let img = super::image(game).expect("decodes");
            assert!(img.size[0] > img.size[1], "{set}: landscape");
        }
    }
}
