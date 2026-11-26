//! Der ganze Ausgabeweg der Firmware am PC nachgestellt: Streifen im kleinen
//! (internen) Puffer rendern, abwechselnd auf zwei „Kernen“, blockweise
//! vergrößern, per PPA-Modell gedreht ins hochkant liegende Panel schreiben –
//! das Ergebnis muss pixelgenau dem ganzen Bild entsprechen, vergrößert und
//! gedreht.

use pudding_core::app::{App, MAX_MESH_VERTICES};
use pudding_core::input::{Input, btn};
use pudding_core::present::{CHUNK_PX, Layout, STRIP_PX, ppa_model, upscale_nearest};
use pudding_core::render::{Scratch, Target};
use pudding_core::save::SaveData;

const PANEL_W: usize = 720;
const PANEL_H: usize = 1280;

fn app(hires: bool, flip: bool) -> App {
    let mut save = SaveData::default();
    save.settings.hires = hires;
    save.settings.flip = flip;
    let mut app = App::new(Some(save));
    // Ins Spiel, damit HUD und 3D-Szene im Bild sind.
    for held in [btn::JUMP, 0, btn::JUMP, 0] {
        app.update(Input::buttons(held), 1.0 / 30.0);
    }
    app.set_autopilot(true);
    for _ in 0..90 {
        app.update(Input::default(), 1.0 / 30.0);
    }
    app
}

/// Referenz: ganzes Bild rendern, dann in einem Stück vergrößern und drehen.
fn reference(app: &mut App) -> Vec<u16> {
    let (w, h) = app.render_size();
    let (mut c, mut d) = (vec![0u16; w * h], vec![0u16; w * h]);
    app.render_full(&mut Target::new(&mut c, &mut d, w, h));
    let l = Layout::new(w, h, PANEL_W, PANEL_H, app.settings().flip);
    let mut panel = vec![0u16; PANEL_W * PANEL_H];
    let (ox, oy) = l.block_offset(0, h);
    ppa_model(&c, w, h, l.scale16, l.flip, &mut panel, PANEL_W, ox, oy);
    panel
}

/// Wie die Firmware: Streifen k auf Kern k % 2, Farbe/Tiefe in kleinen
/// Puffern; `nearest`: CPU vergrößert blockweise, PPA dreht nur.
fn pipeline(app: &mut App, nearest: bool) -> Vec<u16> {
    let (w, h) = app.render_size();
    app.begin_frame(w, h);
    let l = Layout::new(w, h, PANEL_W, PANEL_H, app.settings().flip);
    let mut panel = vec![0u16; PANEL_W * PANEL_H];
    /// Arbeitsspeicher, Farbe, Tiefe, vergrößerter Block – je „Kern“.
    type Core = (Scratch, Vec<u16>, Vec<u16>, Vec<u16>);
    let mut cores: Vec<Core> = (0..2)
        .map(|_| (Scratch::with_capacity(MAX_MESH_VERTICES), vec![0; STRIP_PX], vec![0; STRIP_PX], vec![0; CHUNK_PX]))
        .collect();
    for k in 0..l.strips() {
        let (scratch, color, depth, chunk) = &mut cores[k % 2];
        let (y0, y1) = l.strip(k);
        let n = (y1 - y0) * w;
        let mut t = Target { color: &mut color[..n], depth: &mut depth[..n], w, h, row0: y0 };
        app.render_band(scratch, &mut t);
        if nearest {
            let mut r0 = y0;
            while r0 < y1 {
                let r1 = (r0 + l.chunk_rows()).min(y1);
                let c = l.cpu_scale();
                upscale_nearest(&color[(r0 - y0) * w..], w, r1 - r0, c, chunk);
                let (ox, oy) = l.block_offset(r0, r1);
                ppa_model(chunk, w * c, (r1 - r0) * c, l.ppa_scale16(), l.flip, &mut panel, PANEL_W, ox, oy);
                r0 = r1;
            }
        } else {
            let (ox, oy) = l.block_offset(y0, y1);
            ppa_model(&color[..n], w, y1 - y0, l.scale16, l.flip, &mut panel, PANEL_W, ox, oy);
        }
    }
    panel
}

#[test]
fn strip_pipeline_equals_full_frame_scaled_and_rotated() {
    for hires in [true, false] {
        for flip in [false, true] {
            for nearest in [true, false] {
                let mut a = app(hires, flip);
                let expected = reference(&mut a);
                let got = pipeline(&mut a, nearest);
                let diff = expected.iter().zip(&got).filter(|(x, y)| x != y).count();
                assert_eq!(diff, 0, "hires={hires} flip={flip} nearest={nearest}: {diff} Pixel anders");
            }
        }
    }
}
