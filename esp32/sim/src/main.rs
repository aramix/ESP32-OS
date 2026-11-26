//! PC-Simulation der ESP32-P4-Fassung – kopflos, ohne Hardware.
//!
//! Benutzt exakt denselben Kern wie die Firmware und ersetzt nur Display,
//! Lautsprecher, Knöpfe, Touch und Flash. Wie auf dem Gerät wird jedes Bild
//! in zwei Streifen auf zwei Threads gerendert.
//!
//! ```text
//! pudding-sim gif   DATEI.gif [SEK]   Spielszene (640×360, wie auf dem Display)
//! pudding-sim shots ORDNER            PNG-Bilder aller Bildschirme
//! pudding-sim testbild ORDNER         Testbild des Hardwaretests (quer + Panel)
//! pudding-sim wav   ORDNER            Musik und Soundeffekte als WAV
//! pudding-sim mem                     Speicherbedarf (Heap-Spitze) je Level
//! pudding-sim bench                   Renderzeit am PC (nur zum Vergleich)
//! pudding-sim kapitel ORDNER [BREITE] Ganze Kampagne wie auf dem Handy (20:9) als PNG
//! pudding-sim level N [ORDNER] [0|1|2] Kapitel N per Autopilot (Zeit, Platscher; Bilder alle 6 s)
//! ```

use pudding_core::app::{App, MAX_MESH_VERTICES, Screen};
use pudding_core::audio::{AudioCmd, SAMPLE_RATE, Sfx, SongId, Synth};
use pudding_core::gfx::{MAX_H, MAX_W, to_rgb888};
use pudding_core::input::{Input, btn};
use pudding_core::render::{Scratch, Target};
use pudding_core::save::SaveData;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Zählt den belegten Heap (aktuell und Spitze) – für die Speicherbilanz.
struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = CUR.fetch_add(l.size(), Ordering::Relaxed) + l.size();
        PEAK.fetch_max(n, Ordering::Relaxed);
        // SAFETY: gleiche Layout-Anforderungen wie der Aufrufer.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        CUR.fetch_sub(l.size(), Ordering::Relaxed);
        // SAFETY: `p` stammt aus `alloc` mit demselben Layout.
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Vorschaugröße (die volle Renderauflösung; das Display zeigt sie 2× bzw. 4×).
const OUT_W: usize = MAX_W;
const OUT_H: usize = MAX_H;

/// Bild wie auf dem Gerät, als 640×360 (320×180 wird verdoppelt).
fn display_image(color: &[u16], w: usize, h: usize) -> Vec<u32> {
    let s = OUT_W / w;
    let mut out = vec![0u32; OUT_W * OUT_H];
    for y in 0..h.min(OUT_H / s) {
        for x in 0..w {
            let c = to_rgb888(color[y * w + x]);
            for dy in 0..s {
                let o = (y * s + dy) * OUT_W + x * s;
                out[o..o + s].fill(c);
            }
        }
    }
    out
}

/// Simuliertes Gerät: App + Bildpuffer + zwei Arbeitsspeicher + Synth.
struct Device {
    app: App,
    color: Vec<u16>,
    depth: Vec<u16>,
    scratch: [Scratch; 2],
    synth: Synth,
    audio: Vec<i16>,
}

impl Device {
    fn new(show_fps: bool, hires: bool) -> Self {
        let mut save = SaveData::default();
        save.settings.show_fps = show_fps;
        save.settings.hires = hires;
        let mut app = App::new(Some(save));
        // Wie auf der Konsole ohne Knöpfe: Touch-Steuerung immer sichtbar.
        app.touch_always = true;
        Self {
            app,
            color: vec![0; MAX_W * MAX_H],
            depth: vec![0; MAX_W * MAX_H],
            scratch: [Scratch::with_capacity(MAX_MESH_VERTICES), Scratch::with_capacity(MAX_MESH_VERTICES)],
            synth: Synth::new(),
            audio: Vec::new(),
        }
    }

    fn size(&self) -> (usize, usize) {
        self.app.render_size()
    }

    /// Ein Frame mit Eingabe; Ton wird mitgerendert.
    fn frame_input(&mut self, input: Input, dt: f32) {
        self.app.update(input, dt);
        let synth = &mut self.synth;
        self.app.drain_audio(|c| synth.command(c));
        let n = (dt * SAMPLE_RATE as f32) as usize * 2;
        let start = self.audio.len();
        self.audio.resize(start + n, 0);
        self.synth.render(&mut self.audio[start..]);
        let _ = self.app.take_save();
        self.render();
    }

    fn frame(&mut self, held: u8, dt: f32) {
        self.frame_input(Input::buttons(held), dt);
    }

    /// Wie auf dem Gerät: zwei Streifen auf zwei Threads (= zwei Kerne).
    fn render(&mut self) {
        let (w, h) = self.size();
        self.app.begin_frame(w, h);
        let (mut top, mut bottom) = Target::new(&mut self.color, &mut self.depth, w, h).split(h / 2);
        let app = &self.app;
        let [s0, s1] = &mut self.scratch;
        std::thread::scope(|sc| {
            sc.spawn(|| app.render_band(s1, &mut bottom));
            app.render_band(s0, &mut top);
        });
    }

    fn tris(&self) -> u32 {
        self.scratch.iter().map(|s| s.stats.tris_drawn).sum()
    }

    fn image(&self) -> Vec<u32> {
        let (w, h) = self.size();
        display_image(&self.color[..w * h], w, h)
    }
}

/// Ein Drehbuch: (Sekunden, gehaltene Knöpfe, Autopilot an?).
fn script() -> Vec<(f32, u8, Option<bool>)> {
    let tap = 0.1;
    vec![
        (2.6, 0, None),
        (tap, btn::JUMP, None), // Titel → Hauptmenü
        (1.2, 0, None),
        (tap, btn::DOWN, None),
        (0.3, 0, None),
        (tap, btn::DOWN, None), // auf OPTIONEN
        (0.4, 0, None),
        (tap, btn::JUMP, None),
        (1.6, 0, None),
        (tap, btn::MENU, None), // zurück
        (0.6, 0, None),
        (tap, btn::UP, None),
        (0.3, 0, None),
        (tap, btn::UP, None), // NEUES SPIEL
        (0.4, 0, None),
        (tap, btn::JUMP, None),
        (0.1, 0, Some(true)), // Autopilot übernimmt die Knöpfe
        (20.0, 0, None),
        (tap, btn::MENU, None), // Pause
        (1.6, 0, None),
        (tap, btn::JUMP, None), // weiter
        (8.0, 0, None),
    ]
}

/// Spielt das Drehbuch ab und ruft `each` für jedes Bild (bei `fps`).
fn run_script(fps: f32, secs: f32, mut each: impl FnMut(usize, &Device)) {
    let mut dev = Device::new(false, true);
    let dt = 1.0 / fps;
    let mut t = 0.0;
    let mut i = 0;
    'outer: for (dur, held, ap) in script() {
        if let Some(on) = ap {
            dev.app.set_autopilot(on);
        }
        let mut left = dur;
        while left > 1e-4 {
            dev.frame(held, dt);
            each(i, &dev);
            i += 1;
            left -= dt;
            t += dt;
            if t >= secs {
                break 'outer;
            }
        }
    }
}

fn gif(path: &Path, secs: f32) -> std::io::Result<()> {
    use spatz::image::{GifEncoder, GifPalette, RGB555_BINS, rgb555};
    const FPS: f32 = 20.0;
    let mut hist = vec![0u32; RGB555_BINS];
    run_script(FPS, secs, |i, d| {
        if i % 3 == 0 {
            for p in d.image() {
                hist[rgb555(p)] += 1;
            }
        }
    });
    let pal = GifPalette::from_histogram(&hist);
    let mut enc = GifEncoder::new(OUT_W, OUT_H, &pal);
    let mut frames = 0;
    run_script(FPS, secs, |_, d| {
        enc.frame(&d.image(), (100.0 / FPS) as u16);
        frames += 1;
    });
    let bytes = enc.finish();
    std::fs::write(path, &bytes)?;
    println!("{}: {} Bilder ({OUT_W}x{OUT_H}), {:.1} MB", path.display(), frames, bytes.len() as f64 / 1e6);
    Ok(())
}

/// Ein Bild eines Konsolen-Spiels (über den Rahmen, also mit FPS-Anzeige
/// und Home-Taste) in 8-Zeilen-Streifen wie auf dem Board zeichnen und als
/// PNG speichern.
fn konsole_png(path: &Path, rahmen: &mut konsole::Rahmen, fps: f32) -> std::io::Result<()> {
    use pudding_core::app::{FrameData, Perf};
    let (w, h) = rahmen.render_size();
    let mut fd = FrameData::new();
    rahmen.bild(&mut fd, &Perf { fps, ..Perf::default() });
    let (mut color, mut depth) = (vec![0u16; w * h], vec![0u16; w * h]);
    let mut s = Scratch::default();
    let mut rest = Target::new(&mut color, &mut depth, w, h);
    for y in (8..h).step_by(8).chain([h]) {
        let (mut band, tail) = rest.split(y);
        fd.render_band(&mut s, &mut band);
        rest = tail;
    }
    let rgb: Vec<u32> = color.iter().map(|&c| to_rgb888(c)).collect();
    std::fs::write(path, spatz::image::encode_png(w, h, &rgb))?;
    println!("{}", path.display());
    Ok(())
}

/// Bilder des Konsolen-Menüs: jede Karte in der Mitte, mitten im Wischen,
/// beim Starten.
fn konsole_bilder(dir: &Path) -> std::io::Result<()> {
    use konsole::launcher::Launcher;
    use konsole::{Rahmen, SpielId};
    std::fs::create_dir_all(dir)?;
    let dt = 1.0 / 55.0;
    let mut r = Rahmen::new(Box::new(Launcher::new(SpielId::Pudding)));
    let warte = |r: &mut Rahmen, n: usize, input: Input| {
        for _ in 0..n {
            r.update(&input, dt);
        }
    };
    warte(&mut r, 40, Input::default());
    konsole_png(&dir.join("1_pudding.png"), &mut r, 55.0)?;
    // Halb gewischt (Finger noch drauf)
    for i in 0..5 {
        r.update(&Input::touch_at(500 - i * 25, 180), dt);
    }
    konsole_png(&dir.join("2_wischen.png"), &mut r, 55.0)?;
    for i in 5..12 {
        r.update(&Input::touch_at(500 - i * 25, 180), dt);
    }
    warte(&mut r, 80, Input::default());
    konsole_png(&dir.join("3_doom.png"), &mut r, 54.6)?;
    // Rechte Nachbarkarte antippen
    warte(&mut r, 2, Input::touch_at(570, 185));
    warte(&mut r, 80, Input::default());
    konsole_png(&dir.join("4_wuerfelwelt.png"), &mut r, 55.0)?;
    // Mitte antippen: Startanimation
    warte(&mut r, 2, Input::touch_at(320, 185));
    warte(&mut r, 9, Input::default());
    konsole_png(&dir.join("5_start.png"), &mut r, 55.0)?;
    // Pudding-Panik in der Konsole: Hauptmenü mit „ZUM DESKTOP“.
    let mut p = Rahmen::new(Box::new(konsole::pudding::neu(None)));
    warte(&mut p, 30, Input::default());
    warte(&mut p, 1, Input::buttons(pudding_core::input::btn::JUMP));
    warte(&mut p, 60, Input::default());
    konsole_png(&dir.join("6_pudding_menue.png"), &mut p, 55.0)?;
    // Im Spiel (Kapitel 1–3, Autopilot), wie auf dem Board in 640×360.
    for k in 0..3 {
        let mut app = konsole::pudding::neu(None);
        app.touch_always = true;
        app.start_chapter(k, pudding_core::campaign::Difficulty::Easy);
        app.set_autopilot(true);
        let mut p = Rahmen::new(Box::new(app));
        warte(&mut p, 55 * 6, Input::default());
        konsole_png(&dir.join(format!("7_pudding_kapitel{}.png", k + 1)), &mut p, 55.0)?;
    }
    Ok(())
}

/// Doom-Bilder: Titel, Menü, im Spiel mit Steuerung (braucht
/// doom/wad/DOOM1.WAD).
fn doom_bilder(dir: &Path) -> std::io::Result<()> {
    use doom::Doom;
    use konsole::Rahmen;
    std::fs::create_dir_all(dir)?;
    let wad = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../doom/wad/DOOM1.WAD"))?;
    let ordner = std::env::temp_dir().join("doom_konsole_sim/");
    std::fs::create_dir_all(&ordner)?;
    Doom::virtuelle_zeit();
    let mut r = Rahmen::new(Box::new(Doom::starten(Box::leak(wad.into_boxed_slice()), ordner.to_str().unwrap())));
    let dt = 1.0 / 55.0;
    let mut warte = |r: &mut Rahmen, n: usize, input: Input| {
        for _ in 0..n {
            r.update(&input, dt);
        }
    };
    let zwei = |a: (u16, u16), b: (u16, u16)| Input { held: 0, touch: [Some(a), Some(b)] };
    warte(&mut r, 110, Input::default());
    konsole_png(&dir.join("1_titel.png"), &mut r, 55.0)?;
    warte(&mut r, 1, Input::touch_at(320, 180));
    warte(&mut r, 30, Input::default());
    konsole_png(&dir.join("2_menue.png"), &mut r, 55.0)?;
    for _ in 0..2 {
        warte(&mut r, 1, Input::touch_at(320, 128));
        warte(&mut r, 25, Input::default());
    }
    // Schwierigkeit „Hurt me plenty“ (dritter Eintrag).
    warte(&mut r, 1, Input::touch_at(320, 128 + 2 * 29));
    warte(&mut r, 120, Input::default());
    konsole_png(&dir.join("3_spiel.png"), &mut r, 55.0)?;
    // Laufen und dabei drehen, dann Feuer halten.
    warte(&mut r, 1, zwei((150, 280), (450, 200)));
    for i in 0..40u16 {
        warte(&mut r, 1, zwei((150, 230), (450 + i, 200)));
    }
    warte(&mut r, 1, Input::touch_at(150, 230));
    warte(&mut r, 1, zwei((150, 230), (600, 318)));
    warte(&mut r, 20, zwei((150, 230), (600, 318)));
    konsole_png(&dir.join("4_laufen_feuern.png"), &mut r, 55.0)?;
    warte(&mut r, 5, Input::default());
    // Menü im Spiel.
    warte(&mut r, 1, Input::touch_at(600, 34));
    warte(&mut r, 30, Input::default());
    konsole_png(&dir.join("5_menue_im_spiel.png"), &mut r, 55.0)?;
    Ok(())
}

/// Quake-Bilder: Vorführung, Menü, im Spiel (braucht quake/pak/pak0.pak).
fn quake_bilder(dir: &Path) -> std::io::Result<()> {
    use konsole::Rahmen;
    use quake::{Pak, Quake};
    std::fs::create_dir_all(dir)?;
    let pfad = Path::new(env!("CARGO_MANIFEST_DIR")).join("../quake/pak/pak0.pak");
    let pak = std::fs::read(&pfad)?;
    let ordner = std::env::temp_dir().join("quake_konsole_sim");
    std::fs::create_dir_all(ordner.join("id1"))?;
    Quake::virtuelle_zeit();
    let q = Quake::starten(Pak::Speicher(Box::leak(pak.into_boxed_slice())), pfad.to_str().unwrap(), ordner.to_str().unwrap());
    let mut r = Rahmen::new(Box::new(q));
    let dt = 1.0 / 30.0;
    let warte = |r: &mut Rahmen, n: usize, input: Input| {
        for _ in 0..n {
            r.update(&input, dt);
        }
    };
    let zwei = |a: (u16, u16), b: (u16, u16)| Input { held: 0, touch: [Some(a), Some(b)] };
    warte(&mut r, 150, Input::default());
    konsole_png(&dir.join("1_vorfuehrung.png"), &mut r, 30.0)?;
    warte(&mut r, 1, Input::touch_at(320, 180));
    warte(&mut r, 20, Input::default());
    konsole_png(&dir.join("2_menue.png"), &mut r, 30.0)?;
    // Single Player → New Game (OK zweimal).
    for _ in 0..2 {
        warte(&mut r, 1, Input::touch_at(600, 304));
        warte(&mut r, 15, Input::default());
    }
    warte(&mut r, 90, Input::default());
    // Laufen, umsehen, feuern.
    warte(&mut r, 1, zwei((150, 250), (420, 200)));
    for i in 0..30u16 {
        warte(&mut r, 1, zwei((150, 205), (420 + i * 2, 200)));
    }
    warte(&mut r, 1, Input::touch_at(150, 205));
    warte(&mut r, 8, zwei((150, 205), (600, 310)));
    konsole_png(&dir.join("3_spiel.png"), &mut r, 30.0)?;
    Ok(())
}

/// Würfelwelt-Bilder: Tag, Umsehen, Bauen, Abend, Nacht.
fn wuerfelwelt_bilder(dir: &Path) -> std::io::Result<()> {
    use konsole::Rahmen;
    use wuerfelwelt::Wuerfelwelt;
    std::fs::create_dir_all(dir)?;
    let dt = 1.0 / 55.0;
    let t0 = std::time::Instant::now();
    let mut w = Wuerfelwelt::neu(None);
    println!("Welt erzeugt und vernetzt in {:?}", t0.elapsed());
    w.setze_zeit(0.36);
    let mut r = Rahmen::new(Box::new(w));
    let warte = |r: &mut Rahmen, n: usize, input: Input| {
        for _ in 0..n {
            r.update(&input, dt);
        }
    };
    let zwei = |a: (u16, u16), b: (u16, u16)| Input { held: 0, touch: [Some(a), Some(b)] };
    warte(&mut r, 60, Input::default());
    konsole_png(&dir.join("1_tag.png"), &mut r, 55.0)?;
    // Laufen und dabei nach links drehen und etwas nach oben sehen.
    warte(&mut r, 1, zwei((120, 240), (420, 200)));
    for i in 0..60u16 {
        warte(&mut r, 1, zwei((120, 195), (420 - i * 3, 200 - i / 3)));
    }
    warte(&mut r, 1, Input::default());
    konsole_png(&dir.join("2_umsehen.png"), &mut r, 55.0)?;
    // Bauen: Ziegel wählen, nach unten sehen, mehrmals setzen.
    warte(&mut r, 1, zwei((120, 240), (420, 120)));
    for i in 0..20u16 {
        warte(&mut r, 1, zwei((120, 240), (420, 120 + i * 4)));
    }
    warte(&mut r, 1, Input::default());
    let (x, y, ww, hh) = wuerfelwelt::steuerung::slot_rechteck(3);
    warte(&mut r, 1, Input::touch_at((x + ww / 2) as u16, (y + hh / 2) as u16));
    warte(&mut r, 1, Input::default());
    for _ in 0..4 {
        warte(&mut r, 1, Input::touch_at(590, 214));
        warte(&mut r, 3, Input::default());
    }
    warte(&mut r, 20, Input::touch_at(520, 262));
    konsole_png(&dir.join("3_bauen.png"), &mut r, 55.0)?;
    Ok(())
}

/// Würfelwelt zu einer Tageszeit (Blick über die Insel).
fn wuerfelwelt_zeit(dir: &Path, name: &str, zeit: f32) -> std::io::Result<()> {
    use konsole::Rahmen;
    use wuerfelwelt::Wuerfelwelt;
    let mut w = Wuerfelwelt::neu(None);
    w.setze_zeit(zeit);
    let mut r = Rahmen::new(Box::new(w));
    for _ in 0..40 {
        r.update(&Input::default(), 1.0 / 55.0);
    }
    konsole_png(&dir.join(name), &mut r, 55.0)
}

fn png(path: &Path, dev: &Device) -> std::io::Result<()> {
    std::fs::write(path, spatz::image::encode_png(OUT_W, OUT_H, &dev.image()))
}

fn shots(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut dev = Device::new(true, true);
    let dt = 1.0 / 25.0;
    let wait = |dev: &mut Device, s: f32, held: u8| {
        for _ in 0..(s / dt) as usize {
            dev.frame(held, dt);
        }
    };
    let shot = |dev: &mut Device, name: &str| -> std::io::Result<PathBuf> {
        let p = dir.join(format!("{name}.png"));
        png(&p, dev)?;
        Ok(p)
    };
    let mut out = vec![];
    wait(&mut dev, 3.0, 0);
    dev.app.perf.fps = 0.0;
    out.push(shot(&mut dev, "01_titel")?);
    wait(&mut dev, 0.1, btn::JUMP);
    wait(&mut dev, 1.5, 0);
    out.push(shot(&mut dev, "02_hauptmenue")?);
    for b in [btn::DOWN, btn::DOWN, btn::JUMP] {
        wait(&mut dev, 0.1, b);
        wait(&mut dev, 0.3, 0);
    }
    wait(&mut dev, 1.0, 0);
    out.push(shot(&mut dev, "03_optionen")?);
    // Untermenü TOUCH-STEUERUNG (9. Punkt), dann zurück ins Hauptmenü.
    for _ in 0..8 {
        wait(&mut dev, 0.1, btn::DOWN);
        wait(&mut dev, 0.1, 0);
    }
    wait(&mut dev, 0.1, btn::JUMP);
    wait(&mut dev, 0.6, 0);
    out.push(shot(&mut dev, "03b_touch_steuerung")?);
    wait(&mut dev, 0.1, btn::MENU);
    wait(&mut dev, 0.3, 0);
    wait(&mut dev, 0.1, btn::MENU);
    wait(&mut dev, 0.3, 0);
    for b in [btn::UP, btn::UP, btn::JUMP] {
        wait(&mut dev, 0.1, b);
        wait(&mut dev, 0.3, 0);
    }
    dev.app.set_autopilot(true);
    // FPS-Anzeige mit den hier am PC gemessenen Zeiten.
    let t0 = Instant::now();
    wait(&mut dev, 7.6, 0);
    let ms = t0.elapsed().as_secs_f32() * 1000.0 / (7.6 / dt);
    dev.app.perf.fps = 1000.0 / ms.max(0.001);
    dev.app.perf.render_ms = ms;
    dev.app.perf.frame_ms = ms;
    wait(&mut dev, 0.04, 0);
    out.push(shot(&mut dev, "04_spiel")?);
    wait(&mut dev, 6.5, 0);
    // Zwei Finger: Steuerknüppel links, Springen rechts (der Autopilot
    // steuert weiter, die Anzeige erscheint trotzdem).
    let fingers = |a: (u16, u16), b: (u16, u16)| Input { held: 0, touch: [Some(a), Some(b)] };
    dev.frame_input(fingers((52, 128), (272, 140)), dt);
    for _ in 0..3 {
        dev.frame_input(fingers((66, 112), (272, 140)), dt);
    }
    out.push(shot(&mut dev, "05_touch")?);
    dev.frame_input(Input::default(), dt);
    wait(&mut dev, 0.1, btn::MENU);
    wait(&mut dev, 0.5, 0);
    out.push(shot(&mut dev, "06_pause")?);
    wait(&mut dev, 0.1, btn::JUMP);
    for _ in 0..40 * 25 {
        dev.frame(0, dt);
        if dev.app.screen == Screen::Results {
            break;
        }
    }
    wait(&mut dev, 1.0, 0);
    out.push(shot(&mut dev, "07_ergebnis")?);
    // Schnelle Grafik (320×180) zum Vergleich.
    dev.app.save.settings.hires = false;
    wait(&mut dev, 0.1, btn::JUMP);
    wait(&mut dev, 2.0, 0);
    out.push(shot(&mut dev, "08_spiel_320x180")?);
    for p in out {
        println!("{}", p.display());
    }
    Ok(())
}

/// Testbild des Hardwaretests: so gerendert (quer) und so, wie es nach
/// Vergrößern und Drehen im hochkant liegenden Panel steht.
fn testbild(dir: &Path) -> std::io::Result<()> {
    use pudding_core::present::{Layout, ppa_model};
    use pudding_core::testbild::{TestInfo, draw};
    use std::fmt::Write as _;
    std::fs::create_dir_all(dir)?;
    let mut info = TestInfo { frame: 40, ..TestInfo::default() };
    let _ = info.lines[0].write_str("55 FPS  FRAME 18.2MS  RENDER 6.1MS");
    let _ = info.lines[1].write_str("DISPLAY 55.0 HZ  WARTEN 9.8MS  DT 18.2MS");
    let _ = info.lines[2].write_str("CHIP V3.1 2 KERNE 400 MHZ PSRAM 32 MB");
    let _ = info.lines[3].write_str("(BEISPIELWERTE AUS DER PC-VORSCHAU)");
    info.touches[0] = Some(((150, 120), (480, 680)));
    info.buttons = btn::JUMP;
    let (w, h) = (MAX_W, MAX_H);
    let (mut c, mut d) = (vec![0u16; w * h], vec![0u16; w * h]);
    draw(&info, &mut Target::new(&mut c, &mut d, w, h));
    let rgb: Vec<u32> = c.iter().map(|&p| to_rgb888(p)).collect();
    let p1 = dir.join("testbild_640x360.png");
    std::fs::write(&p1, spatz::image::encode_png(w, h, &rgb))?;
    let l = Layout::new(w, h, 720, 1280, false);
    let mut panel = vec![0u16; 720 * 1280];
    let (ox, oy) = l.block_offset(0, h);
    ppa_model(&c, w, h, l.scale16, false, &mut panel, 720, ox, oy);
    let rgb: Vec<u32> = panel.iter().map(|&p| to_rgb888(p)).collect();
    let p2 = dir.join("testbild_panel_720x1280.png");
    std::fs::write(&p2, spatz::image::encode_png(720, 1280, &rgb))?;
    println!("{}\n{}", p1.display(), p2.display());
    Ok(())
}

fn write_wav(path: &Path, samples: &[i16]) -> std::io::Result<()> {
    let data = samples.len() * 2;
    let mut b = Vec::with_capacity(44 + data);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // Stereo
    b.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    b.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data as u32).to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b)
}

fn wav(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let render = |cmds: &[(f32, AudioCmd)], secs: f32| {
        let mut s = Synth::new();
        let mut out = vec![0i16; (secs * SAMPLE_RATE as f32) as usize * 2];
        let mut pos = 0;
        let mut cmds = cmds.to_vec();
        cmds.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (t, c) in cmds {
            let at = ((t * SAMPLE_RATE as f32) as usize * 2).min(out.len());
            s.render(&mut out[pos..at]);
            pos = at;
            s.command(c);
        }
        s.render(&mut out[pos..]);
        out
    };
    let files = [
        ("musik_level.wav", render(&[(0.0, AudioCmd::Song(Some(SongId::Level)))], 62.0)),
        ("musik_menue.wav", render(&[(0.0, AudioCmd::Song(Some(SongId::Menu)))], 42.0)),
        (
            "soundeffekte.wav",
            render(
                &Sfx::ALL
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (0.3 + i as f32 * 1.1, AudioCmd::Sfx(*s)))
                    .collect::<Vec<_>>(),
                24.0,
            ),
        ),
    ];
    for (name, samples) in files {
        let p = dir.join(name);
        write_wav(&p, &samples)?;
        let peak = samples.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        println!(
            "{} ({:.0} s, Spitze {:.0} %)",
            p.display(),
            samples.len() as f32 / 2.0 / SAMPLE_RATE as f32,
            peak as f32 / 327.67
        );
    }
    let order: Vec<String> = Sfx::ALL.iter().map(|s| format!("{s:?}")).collect();
    println!("Reihenfolge der Effekte (je 1,1 s): {}", order.join(", "));
    Ok(())
}

fn mem() {
    use pudding_core::level::{self, LEVELS};
    use pudding_core::play::PuddingGame;
    let base = CUR.load(Ordering::Relaxed);
    let s = Scratch::with_capacity(MAX_MESH_VERTICES);
    println!("Renderer-Arbeitsspeicher je Kern: {:.1} KB", (CUR.load(Ordering::Relaxed) - base) as f64 / 1024.0);
    drop(s);
    for (n, (name, text)) in LEVELS.iter().enumerate() {
        let before = CUR.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let def = level::parse(text).unwrap();
        let g = PuddingGame::new(&def, false);
        let tris: usize = g.store.total_tris();
        println!(
            "Level {} {name}: belegt {:.0} KB (Spitze beim Laden {:.0} KB), {} Meshes, {} Dreiecke",
            n + 1,
            (CUR.load(Ordering::Relaxed) - before) as f64 / 1024.0,
            (PEAK.load(Ordering::Relaxed) - before) as f64 / 1024.0,
            g.store.len(),
            tris
        );
        drop(g);
    }
    // Ganze App ohne die Puffer der Simulation: Menü, Level 1.
    let (mut color, mut depth) = (vec![0u16; MAX_W * MAX_H], vec![0u16; MAX_W * MAX_H]);
    let before = CUR.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let mut app = App::new(None);
    app.save.record_win(0, 60_000, 1, 0);
    let mut max_v = 0;
    let mut max_items = 0;
    let mut step = |app: &mut App, held: u8, n: usize| {
        for _ in 0..n {
            app.update(Input::buttons(held), 1.0 / 25.0);
            app.drain_audio(|_| {});
            let _ = app.take_save();
            let (w, h) = app.render_size();
            app.render_full(&mut Target::new(&mut color, &mut depth, w, h));
            max_v = max_v.max(app.debug_sizes().0);
            max_items = max_items.max(app.debug_sizes().1);
        }
    };
    step(&mut app, 0, 50);
    step(&mut app, btn::JUMP, 1);
    step(&mut app, 0, 5);
    step(&mut app, btn::JUMP, 1); // Schwierigkeit
    step(&mut app, 0, 5);
    step(&mut app, btn::JUMP, 1); // Story-Karte
    step(&mut app, 0, 30);
    step(&mut app, btn::JUMP, 1); // Kapitel 1
    app.set_autopilot(true);
    step(&mut app, 0, 400);
    println!(
        "Ganze App (Menü + Level 1 gespielt): belegt {:.0} KB, Spitze {:.0} KB",
        (CUR.load(Ordering::Relaxed) - before) as f64 / 1024.0,
        (PEAK.load(Ordering::Relaxed) - before) as f64 / 1024.0
    );
    println!("Größtes Mesh: {max_v} Vertices, Zeichenaufrufe pro Frame höchstens {max_items}");
    println!("Synth (ohne Heap): {:.1} KB", std::mem::size_of::<Synth>() as f64 / 1024.0);
    let fb = |w: usize, h: usize| (w * h * 2) as f64 / 1024.0;
    println!(
        "Bildpuffer: 640x360 Farbe {:.0} KB (x2) + Tiefe {:.0} KB, 320x180 Farbe {:.0} KB (x2) + Tiefe {:.0} KB, \
         Display 720x1280 {:.0} KB (x3)",
        fb(640, 360),
        fb(640, 360),
        fb(320, 180),
        fb(320, 180),
        fb(720, 1280)
    );
}

/// Ganze Kampagne wie auf einem Handy (logische Breite `lw`, Faktor 4):
/// Titel, Menüs, Schwierigkeit, jede Story-Karte, jedes Kapitel (Autopilot)
/// und jedes Ergebnis als PNG.
fn kapitel(dir: &Path, lw: usize) -> std::io::Result<()> {
    use pudding_core::gfx::{H, set_logical_w};
    std::fs::create_dir_all(dir)?;
    set_logical_w(lw);
    let s = 4;
    let mut app = App::new(None);
    app.phone = true;
    app.touch_always = true;
    app.native_scale = s;
    app.save.settings.show_fps = false;
    let (w, h) = app.render_size();
    assert_eq!(h, H * s);
    let (mut color, mut depth) = (vec![0u16; w * h], vec![0u16; w * h]);
    let dt = 1.0 / 30.0;
    let mut n = 0;
    let mut shot = |app: &mut App, name: &str| -> std::io::Result<()> {
        app.render_full(&mut Target::new(&mut color, &mut depth, w, h));
        let rgb: Vec<u32> = color[..w * h].iter().map(|&c| to_rgb888(c)).collect();
        n += 1;
        let p = dir.join(format!("{n:02}_{name}.png"));
        std::fs::write(&p, spatz::image::encode_png(w, h, &rgb))?;
        println!("{}", p.display());
        Ok(())
    };
    let wait = |app: &mut App, secs: f32, held: u8| {
        for _ in 0..(secs / dt) as usize {
            app.update(Input::buttons(held), dt);
            app.drain_audio(|_| {});
            let _ = app.take_save();
        }
    };
    let press = |app: &mut App, b: u8| {
        app.update(Input::buttons(b), dt);
        app.update(Input::buttons(0), dt);
    };
    wait(&mut app, 2.0, 0);
    shot(&mut app, "titel")?;
    press(&mut app, btn::JUMP);
    wait(&mut app, 0.5, 0);
    shot(&mut app, "hauptmenue")?;
    press(&mut app, btn::JUMP);
    press(&mut app, btn::DOWN);
    wait(&mut app, 0.3, 0);
    shot(&mut app, "schwierigkeit")?;
    press(&mut app, btn::UP);
    press(&mut app, btn::JUMP);
    for chapter in 0..pudding_core::save::LEVEL_COUNT {
        wait(&mut app, 3.0, 0);
        shot(&mut app, &format!("story_{}", chapter + 1))?;
        press(&mut app, btn::JUMP);
        app.set_autopilot(true);
        wait(&mut app, 1.5, 0);
        shot(&mut app, &format!("kapitel{}_start", chapter + 1))?;
        let mut t: f32 = 1.5;
        let mut k = 0;
        while app.screen != Screen::Results && t < 120.0 {
            wait(&mut app, 0.5, 0);
            t += 0.5;
            if (t as usize).is_multiple_of(7) && t.fract() == 0.0 && k < 4 {
                k += 1;
                shot(&mut app, &format!("kapitel{}_{k}", chapter + 1))?;
            }
        }
        wait(&mut app, 1.0, 0);
        shot(&mut app, &format!("kapitel{}_ergebnis", chapter + 1))?;
        press(&mut app, btn::JUMP);
    }
    wait(&mut app, 3.0, 0);
    shot(&mut app, "abspann")?;
    Ok(())
}

/// Kapitel `n` (1-basiert) per Autopilot: Zeit, Platscher, Kirschen; bleibt
/// er stecken, die Position. Mit `dir` alle 6 s ein Bild (640×360).
fn level(n: usize, dir: Option<&Path>, difficulty: u8) -> std::io::Result<()> {
    use pudding_core::campaign::Difficulty;
    use pudding_core::save::LEVEL_COUNT;
    if n == 0 || n > LEVEL_COUNT {
        eprintln!("Kapitel 1 bis {LEVEL_COUNT}");
        std::process::exit(2);
    }
    if let Some(d) = dir {
        std::fs::create_dir_all(d)?;
    }
    let mut app = App::new(None);
    app.save.settings.show_fps = false;
    app.start_chapter(n - 1, Difficulty::from_u8(difficulty));
    app.set_autopilot(true);
    let (w, h) = (MAX_W, MAX_H);
    let (mut color, mut depth) = (vec![0u16; w * h], vec![0u16; w * h]);
    let dt = 1.0 / 30.0;
    let mut t = 0.0f32;
    let mut next_shot = 1.0f32;
    let mut k = 0;
    let mut max_melt = 0.0f32;
    while app.screen != Screen::Results && t < 300.0 {
        app.update(Input::default(), dt);
        app.drain_audio(|_| {});
        let _ = app.take_save();
        t += dt;
        max_melt = max_melt.max(app.game().player.melt);
        if let Some(d) = dir
            && t >= next_shot
        {
            next_shot += 6.0;
            k += 1;
            app.render_full(&mut Target::new(&mut color, &mut depth, w, h));
            let rgb: Vec<u32> = color.iter().map(|&c| to_rgb888(c)).collect();
            let p = d.join(format!("kapitel{n}_{k:02}_{:03}s.png", t as u32));
            std::fs::write(&p, spatz::image::encode_png(w, h, &rgb))?;
        }
    }
    let g = app.game();
    match g.finish_time {
        Some(ft) => println!(
            "Kapitel {n}: geschafft in {ft:.1} s, {} Platscher, {}/{} Kirschen, Hitze/Frost max {:.0} %",
            g.player.deaths,
            g.player.cherries,
            g.total_cherries,
            max_melt * 100.0
        ),
        None => {
            let p = g.player.body.pos;
            println!(
                "Kapitel {n}: NICHT geschafft nach {t:.0} s, {} Platscher, steckt bei ({:.1}, {:.1}, {:.1})",
                g.player.deaths, p.x, p.y, p.z
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Grobes Profil eines Simulationsschritts (PC, relativ): Welt, Spielfigur,
/// Weichkörper, Kamera/Partikel und das Vorbereiten des Bildes.
fn profil(n: usize) {
    use pudding_core::level::{self, LEVELS};
    use pudding_core::play::PuddingGame;
    use pudding_core::player::Controls;
    let def = level::parse(LEVELS[n.clamp(1, LEVELS.len()) - 1].1).unwrap();
    let mut g = PuddingGame::new(&def, true);
    let dt = 1.0 / 120.0;
    let mut t = [0.0f64; 6];
    let steps = 120 * 40;
    for i in 0..steps {
        let c = g.controls(Controls::default(), dt);
        let a = Instant::now();
        g.world.update(dt);
        let b = Instant::now();
        let _ = g.player.update(&c, g.rig.yaw, &g.world, dt);
        let d = Instant::now();
        g.jelly.step(&g.player, dt);
        let e = Instant::now();
        g.rig.update(&g.player, 0.0, dt, &g.world.collision);
        g.particles.update(dt);
        let f = Instant::now();
        g.step(&c, dt);
        let h = Instant::now();
        if i % 2 == 0 {
            let _ = g.prepare(0.5);
        }
        let k = Instant::now();
        for (j, (x, y)) in [(a, b), (b, d), (d, e), (e, f), (f, h), (h, k)].iter().enumerate() {
            t[j] += (*y - *x).as_secs_f64();
        }
    }
    let us = |x: f64, n: usize| x * 1e6 / n as f64;
    println!("Kapitel {n}, je Schritt (µs, PC): Welt {:.2}, Spielfigur {:.2}, Weichkörper {:.2}, Kamera+Partikel {:.2}; ganzer step {:.2}; Bild vorbereiten {:.2}",
        us(t[0], steps), us(t[1], steps), us(t[2], steps), us(t[3], steps), us(t[4], steps), us(t[5], steps / 2));
}

/// Renderer-Profil am PC (relativ): Sortieren/Vorbereiten, Dreiecke
/// aufbereiten (build), Streifen zeichnen – bei 640×360, Kapitel `n`.
fn rprofil(n: usize) {
    use pudding_core::campaign::Difficulty;
    use pudding_core::gfx::{H, W};
    let mut app = App::new(None);
    app.start_chapter(n.max(1) - 1, Difficulty::Easy);
    app.set_autopilot(true);
    let (w, h) = (W * 2, H * 2);
    let (mut c, mut d) = (vec![0u16; w * h], vec![0u16; w * h]);
    let mut s = Scratch::with_capacity(MAX_MESH_VERTICES);
    let (mut t_begin, mut t_draw, mut frames, mut prims) = (0.0f64, 0.0f64, 0, 0usize);
    for i in 0..1500 {
        app.update(Input::default(), 1.0 / 30.0);
        app.drain_audio(|_| {});
        let _ = app.take_save();
        if i < 60 {
            continue;
        }
        let a = Instant::now();
        app.begin_frame(w, h);
        let b = Instant::now();
        let mut rest = Target::new(&mut c, &mut d, w, h);
        for y in (16..h).step_by(16).chain([h]) {
            let (mut band, tail) = rest.split(y);
            app.render_band(&mut s, &mut band);
            rest = tail;
        }
        let e = Instant::now();
        t_begin += (b - a).as_secs_f64();
        t_draw += (e - b).as_secs_f64();
        frames += 1;
        prims += app.debug_prims();
    }
    println!(
        "Kapitel {n} 640x360 (PC, je Bild): Vorbereiten+Aufbereiten {:.3} ms, Streifen zeichnen {:.3} ms, {} Dreiecke aufbereitet",
        t_begin * 1e3 / frames as f64,
        t_draw * 1e3 / frames as f64,
        prims / frames
    );
}

fn bench() {
    for hires in [false, true] {
        let mut dev = Device::new(true, hires);
        let dt = 1.0 / 30.0;
        for b in [btn::JUMP, 0, btn::JUMP, 0] {
            dev.frame(b, dt);
        }
        dev.app.set_autopilot(true);
        let mut times = vec![];
        let mut tris = 0u64;
        for _ in 0..800 {
            let t0 = Instant::now();
            dev.frame(0, dt);
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            tris += dev.tris() as u64;
        }
        times.sort_by(|a, b| a.total_cmp(b));
        let (w, h) = dev.size();
        println!(
            "PC: Frame (Simulation + Render {w}x{h} auf 2 Threads + Ton) Median {:.3} ms, 99 % {:.3} ms, \
             im Schnitt {} Dreiecke",
            times[times.len() / 2],
            times[times.len() * 99 / 100],
            tris / times.len() as u64
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let r = match arg(0) {
        Some("gif") => {
            gif(Path::new(arg(1).unwrap_or("pudding_esp32.gif")), arg(2).and_then(|s| s.parse().ok()).unwrap_or(30.0))
        }
        Some("shots") => shots(Path::new(arg(1).unwrap_or("esp32_bilder"))),
        Some("testbild") => testbild(Path::new(arg(1).unwrap_or("esp32_testbild"))),
        Some("wav") => wav(Path::new(arg(1).unwrap_or("esp32_audio"))),
        Some("wuerfelwelt") => {
            let d = Path::new(arg(1).unwrap_or("wuerfelwelt_bilder")).to_path_buf();
            wuerfelwelt_bilder(&d)
                .and_then(|_| wuerfelwelt_zeit(&d, "4_abend.png", 0.745))
                .and_then(|_| wuerfelwelt_zeit(&d, "5_nacht.png", 0.02))
        }
        Some("quake") => quake_bilder(Path::new(arg(1).unwrap_or("quake_bilder"))),
        Some("doom") => doom_bilder(Path::new(arg(1).unwrap_or("doom_bilder"))),
        Some("konsole") => konsole_bilder(Path::new(arg(1).unwrap_or("konsole_bilder"))),
        Some("kapitel") => kapitel(
            Path::new(arg(1).unwrap_or("kapitel_bilder")),
            arg(2).and_then(|s| s.parse().ok()).unwrap_or(400),
        ),
        Some("level") => level(
            arg(1).and_then(|s| s.parse().ok()).unwrap_or(1),
            arg(2).filter(|s| *s != "-").map(Path::new),
            arg(3).and_then(|s| s.parse().ok()).unwrap_or(0),
        ),
        Some("profil") => {
            profil(arg(1).and_then(|s| s.parse().ok()).unwrap_or(1));
            Ok(())
        }
        Some("rprofil") => {
            rprofil(arg(1).and_then(|s| s.parse().ok()).unwrap_or(1));
            Ok(())
        }
        Some("mem") => {
            mem();
            Ok(())
        }
        Some("bench") => {
            bench();
            Ok(())
        }
        _ => {
            eprintln!(
                "Aufruf: pudding-sim gif DATEI [SEK] | shots ORDNER | testbild ORDNER | wav ORDNER | mem | bench"
            );
            Ok(())
        }
    };
    if let Err(e) = r {
        eprintln!("Fehler: {e}");
        std::process::exit(1);
    }
}
