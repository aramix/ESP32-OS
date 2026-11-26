//! Bildausgabe auf beiden Kernen: Streifen rendern, vergrößern, drehen.
//!
//! Das Bild (640×360 oder 320×180, quer) wird in Streifen geteilt
//! ([`pudding_core::present::Layout`]); Kern 0 rendert die geraden, Kern 1
//! gleichzeitig die ungeraden. Jeder Kern hat eigene Puffer im **internen
//! RAM** (falls dort Platz ist):
//!
//! * Farbe und Tiefe für einen Streifen (je 10 KB) – der Tiefenpuffer
//!   verlässt den internen RAM nie.
//! * ein Block für die Nearest-Neighbor-Vergrößerung (40 KB); bei „weich“
//!   liegen darin zwei Farbstreifen, die der PPA direkt liest.
//!
//! Der interne RAM ist knapp und schnell: Was hier nicht liegt, bleibt dem
//! Spiel (die Simulation läuft im internen RAM etwa dreimal so schnell).
//!
//! Danach geht jeder Streifen sofort an den **PPA**, der ihn gedreht in den
//! Bildspeicher (PSRAM) schreibt, während der Kern schon den nächsten rendert:
//!
//! * **Scharf** (Standard): Die CPU verdoppelt die Pixel in einen Block, der
//!   PPA dreht ihn und vergrößert bei 320×180 noch einmal ×2 (bilinear auf
//!   verdoppelten Pixeln). Das sind 4-mal weniger PPA-Aufträge als ×4 auf der
//!   CPU; vorher wartete jeder Kern 5–8 ms pro Bild auf freie Puffer.
//! * Der PPA gibt im Hintergrund aus ([`Pipeline::render_async`]), während
//!   Kern 0 schon das nächste Bild simuliert; [`Pipeline::finish`] vor dem
//!   Anzeigen.
//! * **Weich**: Der PPA vergrößert (bilinear) und dreht den Streifen direkt.

use crate::board::{LCD_H, LCD_W};
use crate::display::FB_BYTES;
use crate::hw::{alloc_pixels, now_us, spawn_pinned, zeroed};
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use esp_idf_svc::sys::{self, EspError, esp};
use pudding_core::app::{App, FrameData, MAX_MESH_VERTICES};
use pudding_core::present::{CHUNK_PX, Layout, STRIP_PX, upscale_nearest};
use pudding_core::render::{Scratch, Target};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

/// Höchstzahl gleichzeitiger PPA-Aufträge eines Kerns: weich je eine
/// Streifengruppe in `chunk` und `chunk2` (siehe [`CoreCtx::slots`]),
/// scharf Block 0 und 1.
const SLOTS: usize = 2 * (CHUNK_PX / (GRUPPE * STRIP_PX));

/// PPA-Aufträge in Arbeit je Kern und Platz. Global, damit Kern 0 vor dem
/// Anzeigen auch auf die Aufträge von Kern 1 warten kann
/// ([`Pipeline::finish`]).
static BUSY: [[AtomicBool; SLOTS]; 2] = [const { [const { AtomicBool::new(false) }; SLOTS] }; 2];

/// Nächster freier Streifen im verteilten Modus ([`Pipeline::start`]): Wer
/// fertig ist, holt sich den nächsten.
static NEXT_STRIP: AtomicUsize = AtomicUsize::new(0);

/// Wo jeder Kern gerade steht (Fehlersuche, siehe `main.rs`): 0 frei,
/// 1 rendert, 2 wartet auf PPA-Slot, 3 vergrößert, 4 beauftragt PPA,
/// 5 wartet auf Auftrag; dazu fertige Streifen.
pub static STAGE: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
pub static STRIPS_DONE: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

/// Momentaufnahme für die Fehlersuche: Stufen, Streifenzähler, nächster
/// Streifen, belegte PPA-Slots.
pub fn debug_state() -> String {
    let busy: Vec<u8> = BUSY.iter().flatten().map(|b| b.load(Ordering::Relaxed) as u8).collect();
    format!(
        "Stufe {}/{}, Streifen {}/{}, nächster {}, PPA belegt {:?}",
        STAGE[0].load(Ordering::Relaxed),
        STAGE[1].load(Ordering::Relaxed),
        STRIPS_DONE[0].load(Ordering::Relaxed),
        STRIPS_DONE[1].load(Ordering::Relaxed),
        NEXT_STRIP.load(Ordering::Relaxed),
        busy
    )
}

/// Fehler im PPA-Treiber von ESP-IDF 5.5 (`ppa_core.c`, nicht blockierend):
/// Ein Auftrag wird eingereiht und versucht dann ohne Warten, die Einheit zu
/// belegen. Schließt genau dazwischen der Interrupt auf dem anderen Kern den
/// vorigen Auftrag ab (Warteschlange war für ihn noch leer), bleibt der neue
/// liegen, bis irgendein weiterer Auftrag kommt – am Ende eines Bildes kommt
/// keiner mehr, und alles wartet für immer. Hängt ein Auftrag länger als
/// [`STUCK_US`], schickt der wartende Kern einen 16×16-Mini-Auftrag hinterher;
/// dessen Interrupt startet den liegengebliebenen.
const STUCK_US: i64 = 3000;
/// Mini-Aufträge seit Start (Messung).
pub static KICKS: AtomicUsize = AtomicUsize::new(0);

/// Tasks, die gerade auf den PPA warten (je Kern höchstens einer): Der
/// Interrupt am Ende eines Auftrags weckt sie (O1). Vorher warteten sie mit
/// `yield_now`, das unter FreeRTOS nur gleich oder höher priorisierte Tasks
/// laufen lässt – die Programme (Priorität 3, z. B. der Browser beim
/// Scrollen) standen dabei auf beiden Kernen bis zu 20 ms je Bild still.
static WARTENDE: [AtomicPtr<c_void>; 2] = [const { AtomicPtr::new(core::ptr::null_mut()) }; 2];

/// Zeit in `ppa_do_scale_rotate_mirror` je Kern (µs, Messung).
pub static SUBMIT_US: [core::sync::atomic::AtomicU32; 2] = [const { core::sync::atomic::AtomicU32::new(0) }; 2];

/// Etwas, das einen Bildstreifen zeichnen kann (Spiel oder Testbild).
/// `Sync`, weil beide Kerne gleichzeitig daraus lesen.
pub trait StripSource: Sync {
    fn render_strip(&self, scratch: &mut Scratch, t: &mut Target);
}

impl StripSource for App {
    fn render_strip(&self, scratch: &mut Scratch, t: &mut Target) {
        self.render_band(scratch, t);
    }
}

impl StripSource for FrameData {
    fn render_strip(&self, scratch: &mut Scratch, t: &mut Target) {
        self.render_band(scratch, t);
    }
}

/// Ein Frame: Renderauflösung, Vergrößerungsart, Drehung, Ziel-Bildspeicher.
#[derive(Clone, Copy)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub smooth: bool,
    pub flip: bool,
    pub fb: *mut u16,
    /// Nur Streifen in diesen Zeilen ausgeben (von, bis); die übrigen stehen
    /// im Bildspeicher `fb` schon richtig (O1, siehe main.rs). None = alle.
    pub zeilen: Option<(usize, usize)>,
}

impl Frame {
    /// Muss der Streifen mit den Zeilen `y0..y1` ausgegeben werden?
    fn noetig(&self, y0: usize, y1: usize) -> bool {
        self.zeilen.is_none_or(|(a, b)| y0 < b && a < y1)
    }
}

/// Wo die Puffer eines Kerns gelandet sind (für die Startmeldung).
#[derive(Clone, Copy, Debug, Default)]
pub struct Placement {
    pub internal_kb: usize,
    pub psram_kb: usize,
}

/// Puffer und PPA-Zugang eines Kerns.
struct CoreCtx {
    core: usize,
    scratch: Scratch,
    /// Farbe des Streifens (scharf: die CPU vergrößert ihn sofort in den
    /// Block, der PPA liest ihn nie).
    color: &'static mut [u16],
    depth: &'static mut [u16],
    /// Vergrößerter Block für den PPA; weich: vier Farbstreifen, die der PPA
    /// direkt liest (mehr Plätze = seltener auf den PPA warten).
    chunk: &'static mut [u16],
    /// Zweiter Block (nur Kern 1, der etwa zwei Drittel der Streifen rendert):
    /// Er muss dann nicht warten, bis der PPA den vorigen abgeholt hat.
    chunk2: Option<&'static mut [u16]>,
    /// Abgegebene Blöcke (wechselt zwischen `chunk` und `chunk2`).
    chunks: usize,
    /// Quelle und Ziel der Mini-Aufträge (je 16×16, siehe [`STUCK_US`]).
    kick_buf: &'static mut [u16],
    /// PPA-Aufträge in Arbeit je Platz (weich: Farbstreifen, scharf: Block).
    busy: &'static [AtomicBool; SLOTS],
    ppa: sys::ppa_client_handle_t,
    placement: Placement,
}

unsafe extern "C" fn on_ppa_done(
    _client: sys::ppa_client_handle_t,
    _edata: *mut sys::ppa_event_data_t,
    user_data: *mut c_void,
) -> bool {
    // SAFETY: `user_data` zeigt auf ein `AtomicBool` aus `CoreCtx::busy`
    // (für immer gültig).
    if !user_data.is_null() {
        unsafe { (*(user_data as *const AtomicBool)).store(false, Ordering::Release) };
    }
    // Wartende wecken (beide: `finish` wartet auf Kern 0 auch auf die
    // Aufträge von Kern 1; ein überflüssiges Wecken prüft nur erneut).
    let mut geweckt: sys::BaseType_t = 0;
    for w in &WARTENDE {
        let t = w.load(Ordering::Acquire);
        if !t.is_null() {
            // SAFETY: `t` ist der Handle eines Tasks, der in `wait_flag`
            // wartet (eingetragen nur während des Wartens).
            unsafe { sys::vTaskGenericNotifyGiveFromISR(t.cast(), 0, &mut geweckt) };
        }
    }
    geweckt != 0
}

impl CoreCtx {
    /// Muss auf dem Kern laufen, der den Kontext benutzt (eigener PPA-Client
    /// pro Task, wie Espressif es empfiehlt).
    fn new(core: usize) -> Result<Self, EspError> {
        let mut placement = Placement::default();
        let mut take = |n: usize| {
            let (buf, internal) = alloc_pixels(n, true)
                .map(|b| (b, true))
                .or_else(|| alloc_pixels(n, false).map(|b| (b, false)))
                .expect("Speicher für Bildpuffer");
            if internal {
                placement.internal_kb += n * 2 / 1024;
            } else {
                placement.psram_kb += n * 2 / 1024;
            }
            buf
        };
        // Tiefe zuerst: sie wird am häufigsten gelesen und geschrieben.
        let depth = take(STRIP_PX);
        let color = take(STRIP_PX);
        const { assert!(CHUNK_PX >= 2 * STRIP_PX) };
        let chunk = take(CHUNK_PX);
        let chunk2 = (core == 1).then(|| take(CHUNK_PX));
        let kick_buf = take(2 * 256);

        let busy: &'static [AtomicBool; SLOTS] = &BUSY[core.min(1)];

        let mut cfg: sys::ppa_client_config_t = zeroed();
        cfg.oper_type = sys::ppa_operation_t_PPA_OPERATION_SRM;
        // Alle Plätze plus Mini-Auftrag (siehe `kick`).
        cfg.max_pending_trans_num = SLOTS as u32 + 1;
        cfg.data_burst_length = sys::ppa_data_burst_length_t_PPA_DATA_BURST_LENGTH_128;
        let mut ppa = core::ptr::null_mut();
        let cbs = sys::ppa_event_callbacks_t { on_trans_done: Some(on_ppa_done) };
        // SAFETY: gültige Konfiguration; der Client lebt so lange wie das Programm.
        unsafe {
            esp!(sys::ppa_register_client(&cfg, &mut ppa))?;
            esp!(sys::ppa_client_register_event_callbacks(ppa, &cbs))?;
        }
        Ok(Self {
            core,
            scratch: Scratch::with_capacity(MAX_MESH_VERTICES),
            color,
            depth,
            chunk,
            chunk2,
            chunks: 0,
            kick_buf,
            busy,
            ppa,
            placement,
        })
    }

    /// Plätze für Streifengruppen im weichen Modus (je [`GRUPPE`] Streifen).
    fn slots(&self) -> usize {
        (CHUNK_PX / (GRUPPE * STRIP_PX)) * (1 + self.chunk2.is_some() as usize)
    }

    fn wait(&self, slot: usize) {
        let st = STAGE[self.core.min(1)].swap(2, Ordering::Relaxed);
        self.wait_flag(&self.busy[slot]);
        STAGE[self.core.min(1)].store(st, Ordering::Relaxed);
    }

    /// Wartet, bis `flag` frei ist; schubst den PPA an, falls ein Auftrag
    /// liegen geblieben ist (siehe [`STUCK_US`]).
    ///
    /// Blockiert, bis der Interrupt des PPA weckt (höchstens 1 Tick = 1 ms,
    /// dann erneut prüfen und ggf. anschubsen) – die CPU ist währenddessen
    /// frei für Tasks jeder Priorität (siehe [`WARTENDE`]).
    fn wait_flag(&self, flag: &AtomicBool) {
        let mut since = now_us();
        let platz = &WARTENDE[self.core.min(1)];
        // SAFETY: Handle des laufenden Tasks.
        let ich = unsafe { sys::xTaskGetCurrentTaskHandle() };
        while flag.load(Ordering::Acquire) {
            if now_us() - since > STUCK_US {
                self.kick();
                since = now_us();
            }
            platz.store(ich.cast(), Ordering::Release);
            // fertig geworden, bevor der Task als wartend eingetragen war?
            if !flag.load(Ordering::Acquire) {
                break;
            }
            // SAFETY: wartet auf die eigene Benachrichtigung (Index 0).
            unsafe { sys::ulTaskGenericNotifyTake(0, 1, 1) };
        }
        platz.store(core::ptr::null_mut(), Ordering::Release);
    }

    /// 16×16-Mini-Auftrag ohne Merker: Sein Interrupt startet einen
    /// liegengebliebenen Auftrag aus der Warteschlange des Treibers.
    fn kick(&self) {
        KICKS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: beide Hälften von `kick_buf` sind 128-Byte-ausgerichtet
        // (256 Pixel = 512 Byte) und gehören nur diesem Kern; der Inhalt ist egal.
        unsafe {
            let mut op: sys::ppa_srm_oper_config_t = zeroed();
            op.in_.buffer = self.kick_buf.as_ptr().cast();
            op.in_.pic_w = 16;
            op.in_.pic_h = 16;
            op.in_.block_w = 16;
            op.in_.block_h = 16;
            op.in_.__bindgen_anon_1.srm_cm = sys::ppa_srm_color_mode_t_PPA_SRM_COLOR_MODE_RGB565;
            op.out.buffer = self.kick_buf[256..].as_ptr() as *mut c_void;
            op.out.buffer_size = 512;
            op.out.pic_w = 16;
            op.out.pic_h = 16;
            op.out.__bindgen_anon_1.srm_cm = sys::ppa_srm_color_mode_t_PPA_SRM_COLOR_MODE_RGB565;
            op.rotation_angle = sys::ppa_srm_rotation_angle_t_PPA_SRM_ROTATION_ANGLE_0;
            op.scale_x = 1.0;
            op.scale_y = 1.0;
            op.mode = sys::ppa_trans_mode_t_PPA_TRANS_MODE_NON_BLOCKING;
            op.user_data = core::ptr::null_mut();
            let _ = sys::ppa_do_scale_rotate_mirror(self.ppa, &op);
        }
    }

    /// PPA-Auftrag: Block `bw`×`bh` aus `src` um `scale` vergrößern, drehen,
    /// bei (`ox`, `oy`) in den Bildspeicher `fb` schreiben. Kehrt sofort
    /// zurück; `busy[slot]` wird frei, wenn der PPA fertig ist.
    #[allow(clippy::too_many_arguments)]
    fn submit(
        &self,
        slot: usize,
        src: &[u16],
        bw: usize,
        bh: usize,
        scale: f32,
        flip: bool,
        fb: *mut u16,
        ox: usize,
        oy: usize,
    ) {
        // SAFETY: `src` ist 128-Byte-ausgerichtet (alloc_pixels); der Block
        // wird erst wieder beschrieben, wenn `busy[slot]` frei ist; der
        // Bildspeicher wird gerade nicht angezeigt (Display::acquire).
        // Den CPU-Cache des Quellblocks schreibt der Treiber selbst zurück
        // (ppa_srm.c: esp_cache_msync C2M über den Block).
        let t0 = now_us();
        unsafe {
            let mut op: sys::ppa_srm_oper_config_t = zeroed();
            op.in_.buffer = src.as_ptr().cast();
            op.in_.pic_w = bw as u32;
            op.in_.pic_h = bh as u32;
            op.in_.block_w = bw as u32;
            op.in_.block_h = bh as u32;
            op.in_.__bindgen_anon_1.srm_cm = sys::ppa_srm_color_mode_t_PPA_SRM_COLOR_MODE_RGB565;
            op.out.buffer = fb.cast();
            op.out.buffer_size = FB_BYTES as u32;
            op.out.pic_w = LCD_W as u32;
            op.out.pic_h = LCD_H as u32;
            op.out.block_offset_x = ox as u32;
            op.out.block_offset_y = oy as u32;
            op.out.__bindgen_anon_1.srm_cm = sys::ppa_srm_color_mode_t_PPA_SRM_COLOR_MODE_RGB565;
            // Quer → hochkant: 90° gegen den Uhrzeigersinn (gedreht: 270°).
            op.rotation_angle = if flip {
                sys::ppa_srm_rotation_angle_t_PPA_SRM_ROTATION_ANGLE_270
            } else {
                sys::ppa_srm_rotation_angle_t_PPA_SRM_ROTATION_ANGLE_90
            };
            op.scale_x = scale;
            op.scale_y = scale;
            op.mode = sys::ppa_trans_mode_t_PPA_TRANS_MODE_NON_BLOCKING;
            op.user_data = (&self.busy[slot] as *const AtomicBool).cast_mut().cast();
            self.busy[slot].store(true, Ordering::Release);
            if let Err(e) = esp!(sys::ppa_do_scale_rotate_mirror(self.ppa, &op)) {
                self.busy[slot].store(false, Ordering::Release);
                log::error!("PPA: {e}");
            }
        }
        SUBMIT_US[self.core.min(1)].fetch_add((now_us() - t0) as u32, Ordering::Relaxed);
    }

    /// Alle Streifen dieses Kerns rendern und an den PPA geben; kehrt zurück,
    /// sobald alles beauftragt ist (fertig wird der PPA im Hintergrund, siehe
    /// [`Pipeline::finish`]). Misst dabei, wofür die Zeit draufgeht.
    fn run<S: StripSource + ?Sized>(&mut self, src: &S, f: &Frame, cores: usize) -> CoreStats {
        let n = einheiten(f);
        self.run_strips(src, f, (self.core..n).step_by(cores))
    }

    /// Wie [`CoreCtx::run`], aber die Einheiten werden geholt, solange es
    /// welche gibt ([`NEXT_STRIP`]) – der früher fertige Kern übernimmt mehr.
    fn run_claim<S: StripSource + ?Sized>(&mut self, src: &S, f: &Frame) -> CoreStats {
        let n = einheiten(f);
        let claim = core::iter::from_fn(|| {
            let k = NEXT_STRIP.fetch_add(1, Ordering::Relaxed);
            (k < n).then_some(k)
        });
        self.run_strips(src, f, claim)
    }

    /// Rendert die Einheiten `order` (weich: Gruppen aus [`GRUPPE`]
    /// Streifen je PPA-Auftrag, scharf: einzelne Streifen) und gibt sie aus.
    fn run_strips<S: StripSource + ?Sized>(
        &mut self,
        src: &S,
        f: &Frame,
        order: impl Iterator<Item = usize>,
    ) -> CoreStats {
        let start = now_us();
        let mut st = CoreStats::default();
        let l = Layout::new(f.w, f.h, LCD_W, LCD_H, f.flip);
        let (mut strips, mut gruppen) = (0usize, 0usize);
        // Weich: Plätze reihum (der älteste ist am ehesten schon abgeholt).
        let platz = GRUPPE * STRIP_PX;
        let (per, slots) = (CHUNK_PX / platz, self.slots());
        for u in order {
            if f.smooth {
                // Weich: Der PPA liest die Farbstreifen selbst – die Streifen
                // einer Gruppe liegen lückenlos hintereinander in einem Platz.
                let (k0, k1) = (u * GRUPPE, ((u + 1) * GRUPPE).min(l.strips()));
                let (y0, y1) = (l.strip(k0).0, l.strip(k1 - 1).1);
                if !f.noetig(y0, y1) {
                    continue;
                }
                let ci = gruppen % slots;
                gruppen += 1;
                let t0 = now_us();
                self.wait(ci);
                st.output_us += (now_us() - t0) as u32;
                for k in k0..k1 {
                    let (sy0, sy1) = l.strip(k);
                    let (o, n) = ((sy0 - y0) * f.w, (sy1 - sy0) * f.w);
                    let t1 = now_us();
                    STAGE[self.core.min(1)].store(1, Ordering::Relaxed);
                    {
                        let (blk, j) = match self.chunk2.as_deref_mut() {
                            Some(b) if ci >= per => (b, ci - per),
                            _ => (&mut *self.chunk, ci),
                        };
                        let color = &mut blk[j * platz + o..j * platz + o + n];
                        let mut t = Target { color, depth: &mut self.depth[..n], w: f.w, h: f.h, row0: sy0 };
                        src.render_strip(&mut self.scratch, &mut t);
                    }
                    STRIPS_DONE[self.core.min(1)].fetch_add(1, Ordering::Relaxed);
                    STAGE[self.core.min(1)].store(3, Ordering::Relaxed);
                    st.render_us += (now_us() - t1) as u32;
                    strips += 1;
                }
                let t2 = now_us();
                let (ox, oy) = l.block_offset(y0, y1);
                let (blk, j): (&[u16], usize) = match self.chunk2.as_deref() {
                    Some(b) if ci >= per => (b, ci - per),
                    _ => (self.chunk, ci),
                };
                let src = &blk[j * platz..j * platz + (y1 - y0) * f.w];
                self.submit(ci, src, f.w, y1 - y0, l.scale16 as f32 / 16.0, f.flip, f.fb, ox, oy);
                st.output_us += (now_us() - t2) as u32;
                continue;
            }
            // Scharf: Streifen einzeln; die CPU vergrößert, der PPA dreht.
            let (y0, y1) = l.strip(u);
            if !f.noetig(y0, y1) {
                continue;
            }
            let n = (y1 - y0) * f.w;
            strips += 1;
            let t1 = now_us();
            STAGE[self.core.min(1)].store(1, Ordering::Relaxed);
            {
                let mut t = Target { color: &mut self.color[..n], depth: &mut self.depth[..n], w: f.w, h: f.h, row0: y0 };
                src.render_strip(&mut self.scratch, &mut t);
            }
            STRIPS_DONE[self.core.min(1)].fetch_add(1, Ordering::Relaxed);
            STAGE[self.core.min(1)].store(3, Ordering::Relaxed);
            st.render_us += (now_us() - t1) as u32;
            let mut r0 = y0;
            while r0 < y1 {
                let r1 = (r0 + l.chunk_rows()).min(y1);
                // Block abwechselnd (Kern 1) bzw. immer derselbe (Kern 0).
                let second = self.chunk2.is_some() && self.chunks % 2 == 1;
                self.chunks += 1;
                let slot = second as usize;
                let t3 = now_us();
                self.wait(slot);
                let t4 = now_us();
                // CPU ×2 (bzw. ×4 → siehe Layout::cpu_scale), PPA den Rest.
                let c = l.cpu_scale();
                let (bw, bh) = (f.w * c, (r1 - r0) * c);
                let dst: &mut [u16] = match (&mut self.chunk2, second) {
                    (Some(b), true) => b,
                    _ => self.chunk,
                };
                upscale_nearest(&self.color[(r0 - y0) * f.w..], f.w, r1 - r0, c, dst);
                let t5 = now_us();
                let (ox, oy) = l.block_offset(r0, r1);
                let src: &[u16] = match (&self.chunk2, second) {
                    (Some(b), true) => b,
                    _ => self.chunk,
                };
                self.submit(slot, &src[..bw * bh], bw, bh, l.ppa_scale16() as f32 / 16.0, f.flip, f.fb, ox, oy);
                st.scale_us += (t5 - t4) as u32;
                st.output_us += ((t4 - t3) + (now_us() - t5)) as u32;
                r0 = r1;
            }
        }
        st.total_us = (now_us() - start) as u32;
        st.strips = strips as u32;
        STAGE[self.core.min(1)].store(0, Ordering::Relaxed);
        st
    }
}

/// Weich: so viele Streifen gehen als ein PPA-Auftrag hinaus. 16 statt 8
/// Quellzeilen: halb so viele Aufträge (Treiberaufwand), doppelt so lange
/// Schreibstücke des gedrehten Bildes ins PSRAM, halb so viele Nähte der
/// bilinearen Skalierung (sie interpoliert nur innerhalb eines Auftrags).
const GRUPPE: usize = 2;

/// Arbeitseinheiten eines Bildes: weich Gruppen, scharf Streifen.
fn einheiten(f: &Frame) -> usize {
    let n = Layout::new(f.w, f.h, LCD_W, LCD_H, f.flip).strips();
    if f.smooth { n.div_ceil(GRUPPE) } else { n }
}

/// Zeiten eines Kerns für einen Frame (µs).
#[derive(Clone, Copy, Debug, Default)]
pub struct CoreStats {
    /// Streifen rendern (3D + HUD bzw. Testbild).
    pub render_us: u32,
    /// Scharfes Vergrößern durch die CPU (0 bei „weich“).
    pub scale_us: u32,
    /// PPA beauftragen (inkl. Cache-Abgleich) und auf ihn warten.
    pub output_us: u32,
    /// Alles zusammen.
    pub total_us: u32,
    /// Gerenderte Streifen.
    pub strips: u32,
}

/// Zeiten eines ganzen Frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub cores: [CoreStats; 2],
    /// Wanduhrzeit vom Start, bis beide Kerne alles beauftragt haben.
    pub wall_us: u32,
    /// Warten auf die letzten PPA-Aufträge in [`Pipeline::finish`].
    pub finish_us: u32,
}

impl FrameStats {
    /// Längste Renderzeit der beiden Kerne in ms.
    pub fn render_ms(&self) -> f32 {
        self.cores.iter().map(|c| c.render_us).max().unwrap_or(0) as f32 / 1000.0
    }

    /// Längste Vergrößerungszeit der beiden Kerne in ms.
    pub fn scale_ms(&self) -> f32 {
        self.cores.iter().map(|c| c.scale_us).max().unwrap_or(0) as f32 / 1000.0
    }
}

/// Auftrag an Kern 1 (Zeiger nur gültig, solange `Pipeline::render` wartet).
struct Job {
    src: *const (),
    run: fn(*const (), &mut CoreCtx, &Frame) -> CoreStats,
    frame: Frame,
}

// SAFETY: siehe `Pipeline::render` – Quelle und Bildspeicher bleiben gültig,
// bis Kern 1 „fertig“ meldet.
unsafe impl Send for Job {}

fn run_erased<S: StripSource>(src: *const (), ctx: &mut CoreCtx, f: &Frame) -> CoreStats {
    // SAFETY: `src` stammt aus `&S` in `Pipeline::render`, das wartet.
    let src = unsafe { &*(src as *const S) };
    ctx.run(src, f, 2)
}

fn run_claim_erased<S: StripSource>(src: *const (), ctx: &mut CoreCtx, f: &Frame) -> CoreStats {
    // SAFETY: `src` stammt aus `&S` in `Pipeline::start`; der Aufrufer lässt
    // es bis nach `Pipeline::join` unverändert.
    let src = unsafe { &*(src as *const S) };
    ctx.run_claim(src, f)
}

/// Ein mit [`Pipeline::start`] begonnener Frame.
struct Active {
    job: Job,
    start: i64,
}

/// Beide Kerne als Renderer mit PPA-Ausgabe.
pub struct Pipeline {
    local: CoreCtx,
    jobs: SyncSender<Job>,
    done: Receiver<(Placement, CoreStats)>,
    /// Pufferlage beider Kerne (Kern 1 meldet sich nach dem ersten Frame).
    pub placement: [Placement; 2],
    active: Option<Active>,
}

impl Pipeline {
    pub fn new() -> Result<Self, EspError> {
        let local = CoreCtx::new(0)?;
        let (jobs, job_rx) = sync_channel::<Job>(1);
        let (done_tx, done) = sync_channel::<(Placement, CoreStats)>(1);
        spawn_pinned(c"render1", 1, 5, 16 * 1024, move || {
            let mut ctx = match CoreCtx::new(1) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Kern 1: PPA nicht verfügbar ({e}) – rendere nur auf Kern 0");
                    return;
                }
            };
            loop {
                STAGE[1].store(5, Ordering::Relaxed);
                let Ok(job) = job_rx.recv() else { return };
                let stats = (job.run)(job.src, &mut ctx, &job.frame);
                if done_tx.send((ctx.placement, stats)).is_err() {
                    return;
                }
            }
        });
        let placement = [local.placement, Placement::default()];
        Ok(Self { local, jobs, done, placement, active: None })
    }

    /// Rendert den Frame auf beiden Kernen und gibt ihn über den PPA in den
    /// Bildspeicher `frame.fb` aus. Kehrt zurück, wenn alles dort angekommen
    /// ist.
    pub fn render<S: StripSource>(&mut self, src: &S, frame: Frame) -> FrameStats {
        let mut st = self.render_async(src, frame);
        st.finish_us = self.finish();
        st.wall_us += st.finish_us;
        st
    }

    /// Wie [`Pipeline::render`], kehrt aber zurück, sobald beide Kerne fertig
    /// gerendert und alles beauftragt haben: Die Quelle (das Spiel) darf danach
    /// schon weiterrechnen, während der PPA die letzten Streifen ausgibt. Vor
    /// dem Anzeigen [`Pipeline::finish`] aufrufen.
    pub fn render_async<S: StripSource>(&mut self, src: &S, frame: Frame) -> FrameStats {
        let start = now_us();
        let job = Job { src: (src as *const S).cast(), run: run_erased::<S>, frame };
        if self.jobs.send(job).is_ok() {
            let c0 = self.local.run(src, &frame, 2);
            // Erst nach dieser Antwort dürfen Quelle und Bildspeicher wieder
            // verändert werden.
            if let Ok((p, c1)) = self.done.recv() {
                self.placement[1] = p;
                return FrameStats { cores: [c0, c1], wall_us: (now_us() - start) as u32, finish_us: 0 };
            }
        }
        // Kern 1 nicht verfügbar: alle Streifen hier.
        let c0 = self.local.run(src, &frame, 1);
        FrameStats { cores: [c0, CoreStats::default()], wall_us: (now_us() - start) as u32, finish_us: 0 }
    }

    /// Verteilter Modus: Kern 1 beginnt sofort mit den Streifen von `src`,
    /// Kern 0 kehrt zurück (und simuliert schon das nächste Bild); mit
    /// [`Pipeline::join`] hilft er mit, bis alle Streifen vergeben sind.
    ///
    /// # Safety
    /// `src` und `frame.fb` müssen bis nach [`Pipeline::join`] gültig und
    /// unverändert bleiben.
    pub unsafe fn start<S: StripSource>(&mut self, src: &S, frame: Frame) {
        debug_assert!(self.active.is_none(), "start ohne join");
        NEXT_STRIP.store(0, Ordering::Relaxed);
        let job = Job { src: (src as *const S).cast(), run: run_claim_erased::<S>, frame };
        let copy = Job { src: job.src, run: job.run, frame };
        if self.jobs.send(job).is_err() {
            log::error!("Kern 1 nicht erreichbar");
        }
        self.active = Some(Active { job: copy, start: now_us() });
    }

    /// Kern 0 rendert die übrigen Streifen des mit [`Pipeline::start`]
    /// begonnenen Frames und wartet auf Kern 1 (nicht auf den PPA – dafür
    /// [`Pipeline::finish`]).
    pub fn join(&mut self) -> FrameStats {
        let Some(a) = self.active.take() else {
            return FrameStats::default();
        };
        let c0 = (a.job.run)(a.job.src, &mut self.local, &a.job.frame);
        let c1 = match self.done.recv() {
            Ok((p, c1)) => {
                self.placement[1] = p;
                c1
            }
            Err(_) => CoreStats::default(),
        };
        FrameStats { cores: [c0, c1], wall_us: (now_us() - a.start) as u32, finish_us: 0 }
    }

    /// Wartet, bis der PPA alle Aufträge beider Kerne in den Bildspeicher
    /// geschrieben hat; liefert die Wartezeit in µs.
    pub fn finish(&self) -> u32 {
        let start = now_us();
        for flag in BUSY.iter().flatten() {
            self.local.wait_flag(flag);
        }
        (now_us() - start) as u32
    }
}
