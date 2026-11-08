//! The games' art built into the player: posters (`assets/posters`, 2:3
//! WebP, one per set) for the launcher's cards, decoded with `image-webp`
//! off the main thread (`Posters`), and the WebP decoding the bezels use
//! too.

use std::sync::mpsc;

include!(concat!(env!("OUT_DIR"), "/posters.rs"));

/// A WebP image, decoded.
pub fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
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

/// `game`'s poster: its own, else its parent set's (a clone is the same
/// game), else the placeholder.
pub fn poster(game: &romset::Game) -> Option<&'static [u8]> {
    std::iter::once(game.id)
        .chain(game.parent)
        .chain(["placeholder"])
        .find_map(|s| POSTERS.iter().find(|p| p.0 == s))
        .map(|p| p.1)
}

/// Art decoded on a thread of their own, arriving as it's done, for the
/// launcher to upload as textures.
pub struct Loader {
    rx: mpsc::Receiver<(String, Kind, egui::ColorImage)>,
    bezels: mpsc::Sender<&'static romset::Game>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Poster,
    Bezel,
}

impl Loader {
    /// Decode the posters of `games` (in that order: the first show
    /// first); then the bezels asked for (`bezel`), each a full-HD image,
    /// so only those shown are decoded.
    pub fn start(games: Vec<&'static romset::Game>) -> Loader {
        let (tx, rx) = mpsc::channel();
        let (bezels, requests) = mpsc::channel::<&'static romset::Game>();
        let _ = std::thread::Builder::new().name("art".into()).spawn(move || {
            for g in &games {
                if let Some(image) = poster(g).and_then(decode) {
                    if tx.send((g.id.to_string(), Kind::Poster, image)).is_err() {
                        return;
                    }
                }
            }
            while let Ok(g) = requests.recv() {
                if let (Some(set), Some(image)) = (crate::bezel::art_set(g), crate::bezel::image(g)) {
                    if tx.send((set.to_string(), Kind::Bezel, image)).is_err() {
                        return;
                    }
                }
            }
        });
        Loader { rx, bezels }
    }

    /// Decode `game`'s bezel (it arrives through `take`).
    pub fn bezel(&self, game: &'static romset::Game) {
        let _ = self.bezels.send(game);
    }

    /// What's been decoded since the last call.
    pub fn take(&self) -> Vec<(String, Kind, egui::ColorImage)> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_poster_decodes_portrait() {
        assert!(super::POSTERS.iter().any(|p| p.0 == "placeholder"));
        for (set, bytes) in super::POSTERS {
            let img = super::decode(bytes).unwrap_or_else(|| panic!("{set}: doesn't decode"));
            assert_eq!(img.size[0] * 3, img.size[1] * 2, "{set}: 2:3");
        }
        // Every game has one, a clone its parent's.
        for g in romset::games::ALL {
            assert!(super::poster(g).is_some(), "{}", g.id);
        }
    }
}
