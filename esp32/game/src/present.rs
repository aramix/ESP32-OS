//! Ausgabe auf das hochkant eingebaute Panel (720×1280): Aufteilung des
//! Bildes in Streifen, Lage jedes Blocks nach Vergrößern und Drehen durch den
//! PPA des ESP32-P4, Nearest-Neighbor-Vergrößerung und ein Rechenmodell des
//! PPA für Tests am PC.
//!
//! Ablauf pro Streifen (siehe Firmware, `pipeline.rs`):
//!
//! * Farbe und Tiefe eines Streifens liegen im **internen RAM** (je
//!   [`STRIP_PX`] Pixel); der Tiefenpuffer verlässt ihn nie.
//! * **Scharf (Nearest-Neighbor):** Die CPU verdoppelt die Pixel blockweise
//!   ([`upscale_nearest`], je [`CHUNK_PX`] Pixel, ebenfalls intern); der PPA
//!   dreht den Block und vergrößert bei 320×180 noch einmal ×2 (bilinear auf
//!   schon verdoppelten Pixeln – bleibt fast scharf, siehe [`Layout::cpu_scale`]).
//! * **Weich (bilinear):** Der PPA vergrößert und dreht den Streifen direkt.

/// Pixel eines Render-Streifens (Farbe bzw. Tiefe, je 2 Byte = 20 KB):
/// 16 Zeilen bei 640, 32 Zeilen bei 320 Pixeln Breite.
pub const STRIP_PX: usize = 640 * 8;
/// Pixel eines vergrößerten Blocks (40 KB): 1280×16 Panelpixel, also
/// 8 Quellzeilen bei Faktor 2 bzw. 4 bei Faktor 4.
pub const CHUNK_PX: usize = 1280 * 16;

/// Geometrie eines Frames: Renderauflösung (quer) → Panel (hochkant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Renderauflösung (quer).
    pub w: usize,
    pub h: usize,
    /// Scale factor in sixteenths, the PPA's resolution (32 = ×2 for 640×360
    /// on 720×1280, 20 = ×1.25 on 480×800).
    pub scale16: usize,
    /// Panel (hochkant), z. B. 480×800.
    pub panel_w: usize,
    pub panel_h: usize,
    /// 270° statt 90° gegen den Uhrzeigersinn drehen („Bild drehen“).
    pub flip: bool,
}

impl Layout {
    /// The largest picture with the same aspect ratio that fits the panel;
    /// the rest becomes black bars of equal width. Every block must come out
    /// as a whole number of panel lines, or there would be gaps between them.
    pub fn new(w: usize, h: usize, panel_w: usize, panel_h: usize, flip: bool) -> Self {
        let scale16 = (panel_h * 16 / w.max(1)).min(panel_w * 16 / h.max(1));
        let l = Self { w, h, scale16, panel_w, panel_h, flip };
        let ganz = |rows: usize| (rows * scale16).is_multiple_of(16);
        let letzter = |step: usize| if h % step == 0 { step } else { h % step };
        assert!(
            (w * scale16).is_multiple_of(16)
                && [l.strip_rows(), l.chunk_rows()].into_iter().all(|r| ganz(r) && ganz(letzter(r))),
            "factor {scale16}/16 does not split the strips evenly: {w}x{h}"
        );
        l
    }

    /// `n` source pixels in panel pixels.
    pub fn scaled(&self, n: usize) -> usize {
        n * self.scale16 / 16
    }

    /// Left/right bar with the panel in landscape (along `panel_h`).
    pub fn margin_long(&self) -> usize {
        (self.panel_h - self.scaled(self.w)) / 2
    }

    /// Top/bottom bar with the panel in landscape (along `panel_w`).
    pub fn margin_short(&self) -> usize {
        (self.panel_w - self.scaled(self.h)) / 2
    }

    /// Zeilen pro Render-Streifen.
    pub fn strip_rows(&self) -> usize {
        (STRIP_PX / self.w).clamp(1, self.h)
    }

    /// Anzahl der Streifen.
    pub fn strips(&self) -> usize {
        self.h.div_ceil(self.strip_rows())
    }

    /// Zeilen `[y0, y1)` des Streifens `k`.
    pub fn strip(&self, k: usize) -> (usize, usize) {
        let r = self.strip_rows();
        let y0 = (k * r).min(self.h);
        (y0, (y0 + r).min(self.h))
    }

    /// Sharp upscaling by the CPU (nearest neighbour): always ×2. The PPA does
    /// the rest (bilinear; on doubled pixels it stays nearly sharp, even when
    /// it scales down again as for ×1.25).
    pub fn cpu_scale(&self) -> usize {
        2
    }

    /// The remaining factor (sixteenths) the PPA applies to each block.
    pub fn ppa_scale16(&self) -> usize {
        self.scale16 / self.cpu_scale()
    }

    /// Quellzeilen pro vergrößertem Block (Nearest-Neighbor um [`Layout::cpu_scale`]).
    pub fn chunk_rows(&self) -> usize {
        let c = self.cpu_scale();
        (CHUNK_PX / (self.w * c * c)).max(1)
    }

    /// Stelle (x, y) im Panel, an die der PPA den (vergrößerten, gedrehten)
    /// Block der Quellzeilen `[y0, y1)` schreibt. 90° gegen den
    /// Uhrzeigersinn: quer (X, Y) → hochkant (Y, Breite−1−X); 270°:
    /// (Höhe−1−Y, X).
    pub fn block_offset(&self, y0: usize, y1: usize) -> (usize, usize) {
        let (long, short) = (self.margin_long(), self.margin_short());
        if self.flip { (self.panel_w - short - self.scaled(y1), long) } else { (short + self.scaled(y0), long) }
    }
}

/// Nearest-Neighbor-Vergrößerung um den ganzzahligen Faktor `s`: `rows`
/// Zeilen zu je `w` Pixeln aus `src` → `dst` mit (w·s)×(rows·s) Pixeln.
#[cfg_attr(target_os = "espidf", unsafe(link_section = ".iram1.pudding"))]
#[inline(never)]
pub fn upscale_nearest(src: &[u16], w: usize, rows: usize, s: usize, dst: &mut [u16]) {
    let ow = w * s;
    assert!(src.len() >= w * rows && dst.len() >= ow * rows * s, "Puffer zu klein");
    for r in 0..rows {
        let srow = &src[r * w..(r + 1) * w];
        let o = r * s * ow;
        let drow = &mut dst[o..o + ow];
        match s {
            1 => drow.copy_from_slice(srow),
            2 => {
                // Beide Zielzeilen direkt, je Pixel ein 32-Bit-Wort (statt
                // erst eine Zeile zu schreiben und sie dann zu kopieren).
                let (r0, r1) = dst[o..o + 2 * ow].split_at_mut(ow);
                double_row(srow, r0, r1);
                continue;
            }
            4 => {
                for (d, &p) in drow.as_chunks_mut::<4>().0.iter_mut().zip(srow) {
                    *d = [p; 4];
                }
            }
            _ => {
                for (d, &p) in drow.chunks_exact_mut(s).zip(srow) {
                    d.fill(p);
                }
            }
        }
        for k in 1..s {
            dst.copy_within(o..o + ow, o + k * ow);
        }
    }
}

/// Verdoppelt `src` in die Zeilen `r0` und `r1` (je doppelt so breit).
#[inline]
fn double_row(src: &[u16], r0: &mut [u16], r1: &mut [u16]) {
    let n = src.len();
    assert!(r0.len() >= 2 * n && r1.len() >= 2 * n);
    // SAFETY: `align_to_mut` liefert nur korrekt ausgerichtete u32; ohne
    // Ausrichtung fällt es auf die einfache Schleife zurück.
    let (h0, w0, _) = unsafe { r0[..2 * n].align_to_mut::<u32>() };
    let (h1, w1, _) = unsafe { r1[..2 * n].align_to_mut::<u32>() };
    if h0.is_empty() && h1.is_empty() && w0.len() == n && w1.len() == n {
        let (sp, p0, p1) = (src.as_ptr(), w0.as_mut_ptr(), w1.as_mut_ptr());
        let mut i = 0;
        // SAFETY: i + 3 < n bzw. i < n; alle drei Puffer haben n Elemente.
        unsafe {
            while i + 4 <= n {
                let a = *sp.add(i) as u32;
                let b = *sp.add(i + 1) as u32;
                let c = *sp.add(i + 2) as u32;
                let d = *sp.add(i + 3) as u32;
                let (a, b, c, d) = (a | a << 16, b | b << 16, c | c << 16, d | d << 16);
                *p0.add(i) = a;
                *p0.add(i + 1) = b;
                *p0.add(i + 2) = c;
                *p0.add(i + 3) = d;
                *p1.add(i) = a;
                *p1.add(i + 1) = b;
                *p1.add(i + 2) = c;
                *p1.add(i + 3) = d;
                i += 4;
            }
            while i < n {
                let a = *sp.add(i) as u32;
                let a = a | a << 16;
                *p0.add(i) = a;
                *p1.add(i) = a;
                i += 1;
            }
        }
        return;
    }
    for ((d0, d1), &p) in r0.as_chunks_mut::<2>().0.iter_mut().zip(r1.as_chunks_mut::<2>().0.iter_mut()).zip(src) {
        *d0 = [p, p];
        *d1 = [p, p];
    }
}

/// Rechenmodell des PPA (nur für Tests und die PC-Vorschau): Block `bw`×`bh`
/// aus `src` um `s16`/16 skalieren (Nearest-Neighbor), um 90° (bzw. 270° bei
/// `rot270`) gegen den Uhrzeigersinn drehen und bei (`ox`, `oy`) in das Bild
/// `dst` (Breite `dw`) schreiben.
#[allow(clippy::too_many_arguments)]
pub fn ppa_model(
    src: &[u16],
    bw: usize,
    bh: usize,
    s16: usize,
    rot270: bool,
    dst: &mut [u16],
    dw: usize,
    ox: usize,
    oy: usize,
) {
    let (sw, sh) = (bw * s16 / 16, bh * s16 / 16);
    for y in 0..sh {
        for x in 0..sw {
            let p = src[(y * 16 / s16) * bw + x * 16 / s16];
            let (px, py) = if rot270 { (sh - 1 - y, x) } else { (y, sw - 1 - x) };
            dst[(oy + py) * dw + ox + px] = p;
        }
    }
}

/// Gleichmäßiger Spieltakt bei VSync.
///
/// Statt der gemessenen CPU-Zeit (die je nach Renderdauer um einige ms
/// schwankt) rückt die Spielzeit pro angezeigtem Bild um genau die Zahl der
/// Bildwechsel des Panels seit dem vorigen Bild × Bilddauer vor. Bei stabiler
/// Bildrate ist das jedes Mal exakt eine Bilddauer – die Bewegung wirkt
/// gleichmäßig. Fällt ein Bild aus (Rendern länger als eine Bilddauer), rückt
/// das nächste um zwei Bilddauern vor, die Spielgeschwindigkeit bleibt also
/// richtig. Die Bilddauer wird laufend aus Zähler und Uhr nachgemessen.
#[derive(Clone, Debug)]
pub struct FramePacer {
    period: f32,
    last_shown: Option<u32>,
    /// Beginn der Messung: (Zähler, µs).
    start: Option<(u32, i64)>,
}

impl FramePacer {
    /// `nominal_hz`: rechnerische Bildwiederholrate des Panels.
    pub fn new(nominal_hz: f32) -> Self {
        Self { period: 1.0 / nominal_hz, last_shown: None, start: None }
    }

    /// Gemessene (bzw. anfangs rechnerische) Bilddauer in Sekunden.
    pub fn period(&self) -> f32 {
        self.period
    }

    /// Nach dem Anzeigen eines Bildes: `shown_at` = Bildwechsel-Zähler, ab dem
    /// es sichtbar ist; `now_us` = Uhrzeit. Liefert die Spielzeit für das
    /// nächste Bild.
    pub fn frame(&mut self, shown_at: u32, now_us: i64) -> f32 {
        match self.start {
            None => self.start = Some((shown_at, now_us)),
            Some((c0, t0)) => {
                let n = shown_at.wrapping_sub(c0);
                // Erst nach ~2 s messen: dann ist der Fehler (< 1 Bild) klein.
                if n >= 100 {
                    self.period = ((now_us - t0) as f32 * 1e-6 / n as f32).clamp(1.0 / 240.0, 1.0 / 20.0);
                }
            }
        }
        let frames = match self.last_shown {
            Some(prev) => shown_at.wrapping_sub(prev).clamp(1, 6),
            None => 1,
        };
        self.last_shown = Some(shown_at);
        frames as f32 * self.period
    }
}

/// Bildzeiten über einen Messzeitraum (für Vorher/Nachher-Messungen am
/// Board): Arbeitszeit je Bild als Mittel, 99. Perzentil und Maximum, dazu
/// verpasste Bildwechsel. Feste Eimer zu 0,25 ms (bis 128 ms), keine Zuteilung.
#[derive(Clone, Debug)]
pub struct Bildbilanz {
    eimer: [u32; Self::EIMER],
    bilder: u32,
    summe_us: u64,
    max_us: u32,
    verpasst: u32,
}

impl Default for Bildbilanz {
    fn default() -> Self {
        Self { eimer: [0; Self::EIMER], bilder: 0, summe_us: 0, max_us: 0, verpasst: 0 }
    }
}

impl Bildbilanz {
    const EIMER: usize = 512;
    const EIMER_US: u32 = 250;

    /// Ein Bild: Arbeitszeit in µs und dabei verpasste Bildwechsel.
    pub fn bild(&mut self, arbeit_us: u32, verpasst: u32) {
        let i = ((arbeit_us / Self::EIMER_US) as usize).min(Self::EIMER - 1);
        self.eimer[i] += 1;
        self.bilder += 1;
        self.summe_us += arbeit_us as u64;
        self.max_us = self.max_us.max(arbeit_us);
        self.verpasst += verpasst;
    }

    pub fn bilder(&self) -> u32 {
        self.bilder
    }

    pub fn verpasst(&self) -> u32 {
        self.verpasst
    }

    pub fn mittel_ms(&self) -> f32 {
        if self.bilder == 0 { 0.0 } else { self.summe_us as f32 / self.bilder as f32 / 1000.0 }
    }

    pub fn max_ms(&self) -> f32 {
        self.max_us as f32 / 1000.0
    }

    /// Perzentil `p` (0..=1) in ms: Obergrenze des Eimers, in dem es liegt.
    pub fn perzentil_ms(&self, p: f32) -> f32 {
        if self.bilder == 0 {
            return 0.0;
        }
        // Aufrunden ohne `std` (no_std).
        let z = self.bilder as f32 * p;
        let ziel = ((z as u32) + (z > (z as u32) as f32) as u32).max(1);
        let mut n = 0;
        for (i, &c) in self.eimer.iter().enumerate() {
            n += c;
            if n >= ziel {
                return ((i as u32 + 1) * Self::EIMER_US).min(self.max_us.max(1)) as f32 / 1000.0;
            }
        }
        self.max_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{vec, vec::Vec};

    #[test]
    fn pacer_steps_in_whole_refreshes() {
        let mut p = FramePacer::new(55.0);
        let t = |n: u32| (n as f64 * 1e6 / 54.8) as i64; // echtes Panel: 54,8 Hz
        let mut shown = 10;
        let mut dts = vec![];
        for i in 0..400 {
            // Jedes 50. Bild dauert zwei Bildwechsel (Ruckler beim Rendern).
            shown += if i % 50 == 49 { 2 } else { 1 };
            dts.push(p.frame(shown, t(shown) + 300 * (i % 7) as i64));
        }
        assert!((p.period() - 1.0 / 54.8).abs() < 2e-5, "nachgemessen: {}", p.period());
        let one = p.period();
        assert!((dts[398] - one).abs() < 2e-5);
        assert!((dts[349] - 2.0 * one).abs() < 4e-5, "ausgefallenes Bild zählt doppelt");
    }

    #[test]
    fn strips_cover_the_frame_and_blocks_tile_the_panel() {
        for (w, h) in [(640, 360), (320, 180)] {
            for flip in [false, true] {
                let l = Layout::new(w, h, 720, 1280, flip);
                assert_eq!(l.scale16, 16 * 1280 / w);
                assert_eq!((l.margin_long(), l.margin_short()), (0, 0));
                let mut next = 0;
                let mut cols = vec![false; 720];
                for k in 0..l.strips() {
                    let (y0, y1) = l.strip(k);
                    assert_eq!(y0, next);
                    assert!(y1 > y0 && (y1 - y0) * w <= STRIP_PX);
                    next = y1;
                    let (ox, oy) = l.block_offset(y0, y1);
                    assert_eq!(oy, 0);
                    for c in &mut cols[ox..ox + l.scaled(y1 - y0)] {
                        assert!(!*c, "Spalte doppelt");
                        *c = true;
                    }
                }
                assert_eq!(next, h);
                assert!(cols.iter().all(|&c| c), "Panel lückenlos");
                assert!(w * l.cpu_scale() * l.chunk_rows() * l.cpu_scale() <= CHUNK_PX);
                assert_eq!(l.cpu_scale() * l.ppa_scale16(), l.scale16);
            }
        }
    }

    /// Guition JC4880P443C: 640×360 ×1.25 = 800×450 on 480×800, with 15-line
    /// bars; the blocks cover the 450 columns without gaps.
    #[test]
    fn fractional_scale_is_centred_without_gaps() {
        for (w, h) in [(640, 360), (320, 180)] {
            for flip in [false, true] {
                let l = Layout::new(w, h, 480, 800, flip);
                assert_eq!((l.margin_long(), l.margin_short()), (0, 15));
                let mut cols = vec![0u8; 480];
                for k in 0..l.strips() {
                    let (s0, s1) = l.strip(k);
                    let mut r0 = s0;
                    while r0 < s1 {
                        let r1 = (r0 + l.chunk_rows()).min(s1);
                        let (ox, oy) = l.block_offset(r0, r1);
                        assert_eq!(oy, 0);
                        // how the PPA computes the block height
                        let out = (r1 - r0) * l.cpu_scale() * l.ppa_scale16() / 16;
                        cols[ox..ox + out].iter_mut().for_each(|c| *c += 1);
                        r0 = r1;
                    }
                }
                assert!(cols.iter().enumerate().all(|(x, &n)| n == u8::from((15..465).contains(&x))));
            }
        }
    }

    /// Der schnelle ×2-Weg (32-Bit-Wörter, beide Zeilen) stimmt mit der
    /// einfachen Verdopplung überein – ausgerichtet und versetzt.
    #[test]
    fn fast_double_matches_reference() {
        for w in [1usize, 3, 4, 7, 640] {
            let rows = 3;
            let src: Vec<u16> = (0..w * rows).map(|i| (i * 7919 + 13) as u16).collect();
            for off in [0usize, 1] {
                let mut d = vec![0u16; off + w * rows * 4];
                upscale_nearest(&src, w, rows, 2, &mut d[off..]);
                for y in 0..rows * 2 {
                    for x in 0..w * 2 {
                        assert_eq!(d[off + y * w * 2 + x], src[(y / 2) * w + x / 2], "w={w} off={off} ({x},{y})");
                    }
                }
            }
        }
    }

    #[test]
    fn nearest_upscale_and_model() {
        let src = [1u16, 2, 3, 4, 5, 6]; // 3×2
        let mut d = [0u16; 6 * 4];
        upscale_nearest(&src, 3, 2, 2, &mut d);
        assert_eq!(&d[..6], &[1, 1, 2, 2, 3, 3]);
        assert_eq!(&d[6..12], &[1, 1, 2, 2, 3, 3]);
        assert_eq!(&d[18..], &[4, 4, 5, 5, 6, 6]);
        // 90° gegen den Uhrzeigersinn: oberste Zeile → linke Spalte, von unten.
        let mut out = [0u16; 2 * 3];
        ppa_model(&src, 3, 2, 16, false, &mut out, 2, 0, 0);
        assert_eq!(out, [3, 6, 2, 5, 1, 4]);
        ppa_model(&src, 3, 2, 16, true, &mut out, 2, 0, 0);
        assert_eq!(out, [4, 1, 5, 2, 6, 3]);
    }

    #[test]
    fn bildbilanz_perzentile() {
        let mut b = Bildbilanz::default();
        for k in 0..99 {
            b.bild(10_000 + k, 0);
        }
        b.bild(40_000, 2);
        assert_eq!(b.bilder(), 100);
        assert_eq!(b.verpasst(), 2);
        assert!((b.mittel_ms() - 10.35).abs() < 0.01, "{}", b.mittel_ms());
        assert!((b.perzentil_ms(0.99) - 10.25).abs() < 1e-3, "{}", b.perzentil_ms(0.99));
        assert!((b.perzentil_ms(1.0) - 40.0).abs() < 1e-3);
        assert!((b.max_ms() - 40.0).abs() < 1e-3);
    }
}
