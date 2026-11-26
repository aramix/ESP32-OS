//! Eingaben: fünf Knöpfe (vorne, hinten, links, rechts, springen), der
//! BOOT-Knopf der Platine als Menü-/Pausetaste und der Touchscreen
//! (bis zu zwei Finger: Steuerknüppel und Springen gleichzeitig).

use crate::present::Layout;

/// Bitmasken der Knöpfe.
pub mod btn {
    /// Vorne (vom Pudding aus gesehen in Blickrichtung der Kamera).
    pub const UP: u8 = 1;
    /// Hinten.
    pub const DOWN: u8 = 2;
    pub const LEFT: u8 = 4;
    pub const RIGHT: u8 = 8;
    pub const JUMP: u8 = 16;
    /// BOOT-Knopf auf der Platine: Pause / zurück.
    pub const MENU: u8 = 32;
}

/// So viele gleichzeitige Berührungen wertet das Spiel aus.
pub const MAX_TOUCH: usize = 2;

/// Eingabezustand eines Frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    /// Gedrückte Knöpfe (Bitmaske aus [`btn`]).
    pub held: u8,
    /// Berührungen in logischen Koordinaten (320×180), die erste zuerst.
    pub touch: [Option<(u16, u16)>; MAX_TOUCH],
}

impl Input {
    /// Nur Knöpfe.
    pub fn buttons(held: u8) -> Self {
        Self { held, touch: [None; MAX_TOUCH] }
    }

    /// Ein Finger bei (`x`, `y`).
    pub fn touch_at(x: u16, y: u16) -> Self {
        Self { held: 0, touch: [Some((x, y)), None] }
    }

    pub fn down(&self, b: u8) -> bool {
        self.held & b != 0
    }

    /// Erste Berührung (für Menüs).
    pub fn first_touch(&self) -> Option<(u16, u16)> {
        self.touch.iter().flatten().next().copied()
    }

    pub fn touching(&self) -> bool {
        self.touch.iter().any(Option::is_some)
    }
}

/// Touch-Punkt eines hochkant eingebauten Panels (`panel_w`×`panel_h`, z. B.
/// 720×1280) → logische Spielkoordinaten (320×180, quer).
///
/// Das Bild wird um 90° gegen den Uhrzeigersinn auf das Panel gedreht
/// (quer (X, Y) → hochkant (Y, Breite−1−X), wie der PPA mit
/// `PPA_SRM_ROTATION_ANGLE_90`); `flip` = 270° (Option „Bild drehen“). Der
/// GT911 liefert Panel-Koordinaten (wie im Board-Support-Paket: ohne
/// Tauschen/Spiegeln, 720×1280).
pub fn panel_to_game(p: (u16, u16), panel_w: u16, panel_h: u16, flip: bool) -> (u16, u16) {
    use crate::gfx::{H, W};
    panel_to_raster(p, panel_w, panel_h, flip, (W as u16, H as u16))
}

/// Wie [`panel_to_game`], aber in ein beliebiges Raster (z. B. 640×360 für
/// die Spiele der Konsole).
///
/// If the raster does not fill the whole panel, it is centred like the picture
/// ([`crate::present::Layout`]); touches in the black bars count as the
/// nearest edge pixel.
pub fn panel_to_raster(p: (u16, u16), panel_w: u16, panel_h: u16, flip: bool, raster: (u16, u16)) -> (u16, u16) {
    let (w, h) = (raster.0.max(1) as i32, raster.1.max(1) as i32);
    let (lw, lh) = (panel_h as i32, panel_w as i32); // quer, z. B. 1280×720
    let mut x = lw - 1 - (p.1 as i32).clamp(0, lw - 1);
    let mut y = (p.0 as i32).clamp(0, lh - 1);
    if flip {
        x = lw - 1 - x;
        y = lh - 1 - y;
    }
    let l = Layout::new(w as usize, h as usize, panel_w as usize, panel_h as usize, flip);
    let (bw, bh) = (l.scaled(l.w) as i32, l.scaled(l.h) as i32);
    let x = (x - l.margin_long() as i32).clamp(0, bw - 1);
    let y = (y - l.margin_short() as i32).clamp(0, bh - 1);
    ((x * w / bw) as u16, (y * h / bh) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_corners_map_onto_landscape() {
        // Oberer linker Panelrand (hochkant) = obere rechte Ecke quer.
        assert_eq!(panel_to_game((0, 0), 720, 1280, false), (319, 0));
        assert_eq!(panel_to_game((719, 1279), 720, 1280, false), (0, 179));
        assert_eq!(panel_to_game((360, 640), 720, 1280, false), (159, 90));
        // Bild gedreht: Touch folgt.
        assert_eq!(panel_to_game((0, 0), 720, 1280, true), (0, 179));
        // Raster der Konsole (640×360).
        assert_eq!(panel_to_raster((0, 0), 720, 1280, false, (640, 360)), (639, 0));
        assert_eq!(panel_to_raster((719, 1279), 720, 1280, false, (640, 360)), (0, 359));
        assert_eq!(panel_to_raster((360, 640), 720, 1280, false, (640, 360)), (319, 180));
        // Guition 480×800: 15-line bars above and below (landscape).
        assert_eq!(panel_to_raster((15, 799), 480, 800, false, (640, 360)), (0, 0));
        assert_eq!(panel_to_raster((464, 0), 480, 800, false, (640, 360)), (639, 359));
        assert_eq!(panel_to_raster((0, 799), 480, 800, false, (640, 360)), (0, 0));
        assert_eq!(panel_to_raster((479, 0), 480, 800, false, (640, 360)), (639, 359));
        assert_eq!(panel_to_raster((240, 399), 480, 800, false, (640, 360)), (320, 180));
    }
}
