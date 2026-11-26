//! Übersetzt NetSurf (Upstream 3.11, vorbereitet von
//! `werkzeuge/quellen_holen.py` unter `C:\ns\3.11`, anders mit
//! `NETSURF_QUELLEN`) mit QuickJS-ng und der ESP32-OS-Anbindung.
//!
//! Die Quellgruppen (je eine Bibliothek mit eigenen Pfaden und Defines)
//! stehen einmal in [`gruppen`]: Am PC übersetzt dieses Skript sie mit
//! clang-cl; für das Board schreibt es dieselben Gruppen als CMake-Liste
//! `<Quellen>/erzeugt/netsurf_quellen.cmake`, die die ESP-IDF-Komponente
//! `firmware/komponenten/netsurf` einliest – beide Bauten übersetzen so
//! genau dieselben Dateien. Quelllisten der Bibliotheken liest es wie das
//! Upstream-Buildsystem aus den `DIR_SOURCES` der Makefiles unter `src/`.

use std::path::{Path, PathBuf};

fn quellen() -> PathBuf {
    println!("cargo:rerun-if-env-changed=NETSURF_QUELLEN");
    let p = PathBuf::from(std::env::var("NETSURF_QUELLEN").unwrap_or_else(|_| r"C:\ns\3.11".into()));
    assert!(p.join(".fertig").exists(), "NetSurf-Quellen fehlen: python esp32/netsurf/werkzeuge/quellen_holen.py");
    p
}

/// `DIR_SOURCES` aller Makefiles unter `bib/src` (wie `Makefile.subdir`).
fn dir_sources(bib: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut stapel = vec![bib.join("src")];
    while let Some(d) = stapel.pop() {
        let mut eintraege: Vec<_> = std::fs::read_dir(&d).expect("Ordner").flatten().map(|e| e.path()).collect();
        eintraege.sort();
        for e in &eintraege {
            if e.is_dir() {
                stapel.push(e.clone());
            }
        }
        let mk = d.join("Makefile");
        if let Ok(t) = std::fs::read_to_string(&mk) {
            let t = t.replace("\\\r\n", " ").replace("\\\n", " ");
            for z in t.lines() {
                let z = z.trim_start();
                if let Some(rest) = z.strip_prefix("DIR_SOURCES") {
                    let rest = rest.trim_start().trim_start_matches(':').trim_start_matches('+').trim_start_matches('=');
                    v.extend(rest.split_whitespace().filter(|w| w.ends_with(".c")).map(|w| d.join(w)));
                }
            }
        }
    }
    v.sort();
    v
}

/// Wie die Quellen einer Gruppe den Programm-Modus bekommen (32 Bit).
#[derive(Clone, Copy, PartialEq)]
enum Modus {
    /// compat/gemeinsam/ns_programm.h: Speicher aus dem Heap des Programms
    Voll,
    /// nur compat/gemeinsam/ns_abschnitt.h (QuickJS: Speicher über
    /// JS_NewRuntime2, die Makros vertrügen sich nicht mit windows.h)
    Abschnitt,
    /// ohne (c/programm_daten.c: liegt ausserhalb des Abschnitts)
    Ohne,
}

/// Eine Bibliothek: Dateien, Pfade, Defines.
#[derive(Clone)]
struct Gruppe {
    name: &'static str,
    dateien: Vec<PathBuf>,
    /// nur in den PC-Tests (Proben)
    proben: Vec<PathBuf>,
    pfade: Vec<PathBuf>,
    defines: Vec<(String, Option<String>)>,
    /// compat/esp32os_compat.h voranstellen (nur MSVC)
    msvc_compat: bool,
    modus: Modus,
}

impl Gruppe {
    fn neu(name: &'static str, modus: Modus) -> Self {
        Gruppe { name, dateien: Vec::new(), proben: Vec::new(), pfade: Vec::new(), defines: Vec::new(), msvc_compat: true, modus }
    }
    fn d(&mut self, n: &str, w: Option<&str>) -> &mut Self {
        self.defines.push((n.into(), w.map(Into::into)));
        self
    }
    fn p(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.pfade.push(p.as_ref().to_path_buf());
        self
    }
    fn f(&mut self, f: impl AsRef<Path>) -> &mut Self {
        self.dateien.push(f.as_ref().to_path_buf());
        self
    }
    /// Defines wie die Upstream-Makefiles (Release, clang/gcc)
    fn standard(&mut self) -> &mut Self {
        self.d("_DEFAULT_SOURCE", None).d("NDEBUG", None);
        self.d("_ALIGNED", Some("__attribute__((aligned))")).d("STMTEXPR", Some("1"));
        self.d("_USE_MATH_DEFINES", None)
    }
}

/// Alle Gruppen. `board`: Einstellungen fürs Board (Ressourcenpfad, keine
/// Proben), sonst für die PC-Tests. `aus`: Ordner für erzeugte Köpfe
/// (dom/bindings/hubbub, vorspann.h).
fn gruppen(q: &Path, aus: &Path, board: bool) -> Vec<Gruppe> {
    let bib = |n: &str| q.join(n);
    let hier = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let c = |f: &str| hier.join("c").join(f);
    let inc = aus.join("include");
    let qjs = q.parent().expect("NetSurf-Quellen in einem Ordner").join("quickjs-0.18.0");
    let zlib = q.parent().expect("NetSurf-Quellen in einem Ordner").join("zlib-1.4");
    let mut v = Vec::new();

    // libwapcaplet, libparserutils, libhubbub, libdom (+ Hubbub-Anbindung)
    let mut g = Gruppe::neu("netsurf_dom", Modus::Voll);
    g.p(&inc);
    for n in ["libwapcaplet", "libparserutils", "libhubbub", "libcss", "libdom"] {
        g.p(bib(n).join("include")).p(bib(n).join("src"));
    }
    g.p(bib("libdom")).standard().d("WITHOUT_ICONV_FILTER", None).d("WITH_HUBBUB_BINDING", None);
    for n in ["libwapcaplet", "libparserutils", "libhubbub", "libdom"] {
        for f in dir_sources(&bib(n)) {
            // B7: als Kopie (dom_textarea_kopie)
            if f.ends_with("html/html_text_area_element.c") || f.ends_with("html/html_document.c") {
                continue;
            }
            g.f(f);
        }
    }
    g.f(bib("libdom").join("bindings/hubbub/parser.c"));
    g.f(dom_textarea_kopie(q, &aus.join("dom")));
    g.f(dom_dokument_kopie(q, &aus.join("dom")));
    g.proben.push(c("probe.c"));
    v.push(g);

    // libcss eigens: gleichnamige Dateien wie libhubbub (charset/detect.c …)
    let mut g = Gruppe::neu("netsurf_css", Modus::Voll);
    for n in ["libwapcaplet", "libparserutils", "libcss"] {
        g.p(bib(n).join("include"));
    }
    g.p(bib("libcss").join("src")).standard();
    let mut autogen: Vec<PathBuf> = std::fs::read_dir(bib("libcss").join("src/parse/properties"))
        .expect("libcss")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("autogenerated_") && n.ends_with(".c")))
        .collect();
    autogen.sort();
    for f in dir_sources(&bib("libcss")).into_iter().chain(autogen) {
        // Q3: language.c als geprüfte Kopie (meldet verworfene Deklarationen)
        if f.ends_with("src/parse/language.c") {
            continue;
        }
        g.f(f);
    }
    g.f(css_sprache_kopie(&bib("libcss"), &aus.join("css"))).f(c("css_luecken.c"));
    v.push(g);

    // libnsutils, libnsbmp, libnsgif (je eigene Bibliothek)
    for (n, name) in [("libnsutils", "netsurf_nsutils"), ("libnsbmp", "netsurf_nsbmp"), ("libnsgif", "netsurf_nsgif")] {
        let mut g = Gruppe::neu(name, Modus::Voll);
        g.p(bib(n).join("include")).p(bib(n).join("src")).standard();
        for f in dir_sources(&bib(n)) {
            g.f(f);
        }
        v.push(g);
    }

    // libnsfb: Kern und Plotter; Flächen surface.c + ram.c (die übrigen
    // brauchen SDL, X, VNC, Wayland); „esp32os“ kommt mit dem Frontend
    let mut g = Gruppe::neu("netsurf_nsfb", Modus::Voll);
    let nsfb = bib("libnsfb");
    g.p(nsfb.join("include")).p(nsfb.join("src")).standard();
    for f in dir_sources(&nsfb) {
        g.f(f);
    }
    g.f(nsfb.join("src/surface/surface.c")).f(nsfb.join("src/surface/ram.c"));
    g.proben.push(c("probe_fb.c"));
    v.push(g);

    // QuickJS-ng 0.18.0 als Amalgam
    let mut g = Gruppe::neu("netsurf_quickjs", Modus::Abschnitt);
    g.msvc_compat = false;
    // B-Review: ohne NDEBUG bräche assert(gc_obj_list leer) in
    // JS_FreeRuntime das ganze Programm ab, wenn eine JS-Hülle verloren ging
    // (Laufzeit je Dokument seit B10) – lieber ein kleines Leck
    g.p(&qjs).d("NDEBUG", None).f(qjs.join("quickjs-amalgam.c"));
    g.proben.push(c("probe_js.c"));
    v.push(g);

    // zlib 1.4 (gz*: utils/hashtable.c liest die Messages über gzopen)
    let mut g = Gruppe::neu("netsurf_zlib", Modus::Voll);
    g.msvc_compat = false;
    g.p(&zlib);
    for f in "adler32 crc32 deflate infback inffast inflate inftrees trees zutil compress uncompr gzclose gzlib gzread gzwrite".split_whitespace() {
        g.f(zlib.join(format!("{f}.c")));
    }
    v.push(g);

    let mut g = Gruppe::neu("netsurf_utf8proc", Modus::Voll);
    g.p(bib("libutf8proc").join("include/libutf8proc")).standard().d("UTF8PROC_STATIC", None);
    g.f(bib("libutf8proc").join("src/utf8proc.c"));
    v.push(g);

    // NetSurf-Kern: S_COMMON + S_IMAGE + S_BROWSER der Upstream-Makefiles,
    // kein curl, JavaScript über QuickJS (c/js_quickjs.c), Bilder BMP/ICO/GIF,
    // JPEG/PNG über den Dekoder des Systems (c/bild.c, B8)
    let ns = bib("netsurf");
    let mut g = Gruppe::neu("netsurf_kern", Modus::Voll);
    g.p(&inc).p(&ns).p(ns.join("include")).p(ns.join("content/handlers"));
    for n in ["libwapcaplet", "libparserutils", "libhubbub", "libcss", "libdom", "libnsutils", "libnsbmp", "libnsgif"] {
        g.p(bib(n).join("include"));
    }
    g.p(bib("libutf8proc").join("include")).p(&zlib).p(ns.join("frontends")).p(&qjs).p(aus);
    g.standard().d("UTF8PROC_STATIC", None);
    for d in ["WITH_BMP", "WITH_GIF", "WITH_JPEG", "WITH_PNG", "WITH_UTF8PROC", "nsframebuffer"] {
        g.d(d, None);
    }
    g.d("NETSURF_HOMEPAGE", Some("\"resource:welcome.html\"")).d("NETSURF_LOG_LEVEL", Some("WARNING"));
    g.d("NETSURF_BUILTIN_LOG_FILTER", Some("\"(level:WARNING || cat:jserrors)\""));
    g.d("NETSURF_BUILTIN_VERBOSE_FILTER", Some("\"(level:VERBOSE || cat:jserrors)\""));
    for (dir, dateien) in KERN {
        for f in dateien.split_whitespace() {
            // Q3: script.c als geprüfte Kopie (ES-Module überspringen)
            if *dir == "content/handlers/html" && f == "script.c" {
                g.f(script_kopie(q, aus));
                continue;
            }
            // Q3: CSS-Text vorbereiten (c/css_vorbereitung.c)
            if *dir == "content/handlers/css" && f == "css.c" {
                g.f(css_inhalt_kopie(q, aus));
                continue;
            }
            if *dir == "content/handlers/css" && f == "select.c" {
                g.f(css_select_kopie(q, aus));
                continue;
            }
            if *dir == "desktop" && f == "browser_window.c" {
                g.f(browser_window_kopie(q, aus));
                continue;
            }
            g.f(ns.join(dir).join(f));
        }
    }
    // NS_JS_WIE_BOARD=1 (PC): JS-Grenzen wie am Board, um Fehler dort am PC
    // nachzustellen
    println!("cargo:rerun-if-env-changed=NS_JS_WIE_BOARD");
    if board || std::env::var("NS_JS_WIE_BOARD").is_ok_and(|v| v == "1") {
        // Board: JS-Laufzeit im 10-MB-Heap des Programms, Stapel 384 KB.
        // Vorher feste 2 MB: ein JS-lastiger erster Versuch (Wikipedia/
        // MediaWiki) durfte den Heap nicht so füllen, dass die Seite danach
        // ohne JS nicht mehr passt (gemessen 22.11.: feste 4 MB → Heap voll
        // beim Neuladen). Q3 (25.11.): JS bis 6 MB, aber nie in die letzten
        // 3 MB des Heaps (NS_JS_RESERVE, c/js_quickjs.c) – eine kleine Seite
        // mit grossem Skript (OpenStreetMap: 1,5 MB Quelltext bei 2,7 MB
        // Heap) bekommt mehr, eine volle Seite nicht mehr als vorher.
        g.d("NS_JS_SPEICHER", Some("(6u*1024u*1024u)"))
            .d("NS_JS_RESERVE", Some("(3u*1024u*1024u)"))
            .d("NS_JS_STAPEL", Some("(192u*1024u)"));
        // B8: Bitmaps liegen im Heap des Browsers – je Bild höchstens 1 MB
        // (256 K Bildpunkte), grössere werden verkleinert
        g.d("NS_BILD_MAX_BITMAP", Some("(256u*1024u)"));
    }
    g.f(c("iconv.c")).f(c("css_vorbereitung.c")).f(c("js_quickjs.c")).f(c("neu_layout.c")).f(c("eingabe.c")).f(c("ohne_js.c")).f(c("lesezeichen.c")).f(c("touch.c")).f(c("bild.c")).f(c("ablage.c"));
    // html.c als geprüfte Kopie: Seiten auf der Liste „ohne JavaScript“
    // (c/ohne_js.c) laden ohne Skripte
    g.f(html_kopie(q, aus));
    // llcache.c als geprüfte Kopie: Grösse des Zwischenspeichers nach jedem
    // Aufräumen messbar (B3)
    g.f(llcache_kopie(q, aus));
    // urldb.c als Kopie (B7, Cookie-Fix in urldb.c, siehe urldb_kopie)
    g.f(urldb_kopie(q, aus));
    // box_textarea.c und interaction.c als Kopien, c/formular.c (B7:
    // Formular-Ereignisse input/submit, abgebrochener Klick)
    g.f(box_textarea_kopie(q, aus)).f(interaction_kopie(q, aus)).f(form_kopie(q, aus)).f(c("formular.c"));
    g.proben.push(c("probe_kern.c"));
    // B2: content/fetch.c mit WITH_CURL (ruft fetch_curl_register, das
    // c/abruf.c bereitstellt) – nur diese Datei, utils/time.c bliebe sonst
    // bei curl_getdate; gleiche Einstellungen wie der Kern
    let mut a = Gruppe::neu("netsurf_abruf", Modus::Voll);
    a.pfade = g.pfade.clone();
    a.defines = g.defines.clone();
    a.d("WITH_CURL", None);
    a.f(ns.join("content/fetch.c")).f(c("abruf.c"));
    v.push(g);
    v.push(a);
    // Transport: am Board esp_http_client (ausserhalb des Neustart-Abschnitts,
    // netsurf.lf), am PC http über Winsock zum Prüfserver (ohne
    // ns_programm.h: eigener Speicher der C-Laufzeit)
    if board {
        let mut t = Gruppe::neu("netsurf_transport", Modus::Voll);
        t.msvc_compat = false;
        t.f(c("transport_esp.c"));
        v.push(t);
    } else {
        let mut t = Gruppe::neu("netsurf_transport", Modus::Ohne);
        t.msvc_compat = false;
        t.f(c("transport_pc.c"));
        v.push(t);
    }

    // Framebuffer-Frontend (S_FRONTEND, fbtk, interne Schrift; Bilder und
    // Schrift erzeugt das Holskript). `main` heisst netsurf_fb_main.
    let fe = ns.join("frontends/framebuffer");
    let erzeugt = q.join("erzeugt");
    let mut g = Gruppe::neu("netsurf_fb", Modus::Voll);
    g.p(&inc).p(&ns).p(ns.join("include")).p(ns.join("content/handlers")).p(ns.join("frontends"));
    for n in ["libwapcaplet", "libparserutils", "libhubbub", "libcss", "libdom", "libnsutils", "libnsfb"] {
        g.p(bib(n).join("include"));
    }
    g.p(&erzeugt).p(bib("libnsfb").join("src")).standard();
    g.d("nsframebuffer", None).d("small", None).d("main", Some("netsurf_fb_main"));
    g.d("NETSURF_LOG_LEVEL", Some("WARNING")).d("NETSURF_HOMEPAGE", Some("\"resource:welcome.html\""));
    // Ressourcen: am Board aus dem Firmware-Abbild über das schreibgeschützte
    // Dateisystem /nsres (firmware/komponenten/esp32os/eingebettet.c), in
    // den PC-Tests über Umgebungsvariablen (je Eintrag expandiert, ':' trennt)
    if board {
        g.d("NETSURF_FB_RESPATH", Some("\"/nsres/\""));
        // B3: Zwischenspeicher (llcache) 2 MB statt 12 MB – NetSurfs
        // Untergrenze; davon 25 % für Bilder. Der Browser hat 10 MB Heap.
        g.d("NS_SPEICHER_CACHE", Some("(2*1024*1024)"));
    } else {
        g.d("NETSURF_FB_RESPATH", Some("\"${NETSURF_RES3}:${NETSURF_RES1}:${NETSURF_RES2}\""));
    }
    g.d("NETSURF_FB_FONTPATH", Some("\"/nsres/\""));
    for (n, w) in [
        ("SANS_SERIF", "DejaVuSans.ttf"),
        ("SANS_SERIF_BOLD", "DejaVuSans-Bold.ttf"),
        ("SANS_SERIF_ITALIC", "DejaVuSans-Oblique.ttf"),
        ("SANS_SERIF_ITALIC_BOLD", "DejaVuSans-BoldOblique.ttf"),
        ("SERIF", "DejaVuSerif.ttf"),
        ("SERIF_BOLD", "DejaVuSerif-Bold.ttf"),
        ("MONOSPACE", "DejaVuSansMono.ttf"),
        ("MONOSPACE_BOLD", "DejaVuSansMono-Bold.ttf"),
        ("CURSIVE", "Comic_Sans_MS.ttf"),
        ("FANTASY", "Impact.ttf"),
    ] {
        let w = format!("\"{w}\"");
        g.d(&format!("NETSURF_FB_FONT_{n}"), Some(&w));
    }
    // gui.c als geprüfte Kopie (Suche aus der Adressleiste, Suchanbieter)
    g.f(gui_kopie(q, aus));
    for f in "framebuffer.c bitmap.c fetch.c findfile.c corewindow.c local_history.c clipboard.c font_internal.c".split_whitespace() {
        g.f(fe.join(f));
    }
    // schedule.c als geprüfte Kopie (BR2: calloc ohne Prüfung)
    g.f(schedule_kopie(q, aus));
    // fbtk.c als geprüfte Kopie (B6: Schreibmarke → Bildschirmtastatur);
    // sie bindet widget.h aus dem fbtk-Ordner ein
    g.p(fe.join("fbtk"));
    g.f(fbtk_kopie(q, aus));
    // B8: Tabs (braucht die Frontend-Kopfdateien)
    g.f(c("tabs.c")).f(c("download.c"));
    // event.c und text.c als geprüfte Kopien (B8: Zeichen der
    // Bildschirmtastatur als Unicode, Textfeld in UTF-8)
    g.f(event_kopie(q, aus)).f(text_kopie(q, aus));
    for f in "fill.c bitmap.c user.c window.c scroll.c osk.c".split_whitespace() {
        g.f(fe.join("fbtk").join(f));
    }
    g.f(c("getopt.c"));
    let mut erz: Vec<PathBuf> =
        std::fs::read_dir(&erzeugt).expect("erzeugt").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "c")).collect();
    erz.sort();
    for f in erz {
        g.f(f);
    }
    // Fläche „esp32os“ (Kern-Fenster) und Einstieg des Programms: nur 32 Bit
    if board || ziel_32() {
        g.f(c("flaeche_esp32os.c"));
    }
    g.proben.push(c("probe_lauf.c"));
    v.push(g);
    v
}

fn ziel_32() -> bool {
    std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").as_deref() == Ok("32")
}

fn main() {
    println!("cargo:rerun-if-changed=c");
    println!("cargo:rerun-if-changed=compat");
    println!("cargo:rerun-if-changed=js");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("espidf") {
        // Am Board baut ESP-IDF (firmware/komponenten/netsurf) nach der
        // CMake-Liste, die der PC-Bau schreibt; hier nur die Ressourcen
        ressourcen_liste(&quellen());
        return;
    }
    let q = quellen();
    let aus = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // Erzeugte Köpfe: Hubbub-Anbindung von libdom (upstream nach dem
    // Installieren unter <dom/bindings/hubbub/…>) und der JS-Vorspann
    let hb = aus.join("include/dom/bindings/hubbub");
    std::fs::create_dir_all(&hb).expect("Ordner");
    for h in ["parser.h", "errors.h"] {
        std::fs::copy(q.join("libdom/bindings/hubbub").join(h), hb.join(h)).expect("Kopie");
    }
    vorspann_erzeugen(&aus.join("vorspann.h"));

    // Für das Board: dieselben Gruppen als CMake-Liste (mit eigenen
    // erzeugten Köpfen neben der Liste)
    board_liste(&q);
    // The PC tests only run on Windows. Elsewhere the board build just needs
    // the list: `NETSURF_LIST_ONLY=1 cargo build -p netsurf`
    println!("cargo:rerun-if-env-changed=NETSURF_LIST_ONLY");
    if std::env::var_os("NETSURF_LIST_ONLY").is_some() {
        return;
    }

    // 32 Bit (i686-PC-Tests): NetSurf als ESP32-OS-Programm
    let programm = ziel_32();
    for g in gruppen(&q, &aus, false) {
        let mut b = cc::Build::new();
        b.compiler("C:/Program Files/LLVM/bin/clang-cl.exe");
        b.flag("-Wno-everything").flag("/O2");
        // POSIX-Kleinigkeiten für die MSVC-Laufzeit (nur NetSurf-Quellen),
        // dann gemeinsame Köpfe (auch am Board)
        if g.msvc_compat {
            b.include("compat");
        }
        b.include("compat/gemeinsam");
        for p in &g.pfade {
            b.include(p);
        }
        for (n, w) in &g.defines {
            b.define(n, w.as_deref());
        }
        if g.msvc_compat {
            b.flag("/FIesp32os_compat.h");
        }
        if programm {
            match g.modus {
                Modus::Voll => {
                    b.flag("/FIns_programm.h");
                }
                Modus::Abschnitt => {
                    b.flag("/FIns_abschnitt.h");
                }
                Modus::Ohne => {}
            }
        }
        for f in g.dateien.iter().chain(&g.proben) {
            b.file(f);
        }
        b.compile(g.name);
    }
    if programm {
        // ohne ns_programm.h: die Sicherung liegt ausserhalb des Abschnitts
        let mut d = cc::Build::new();
        d.compiler("C:/Program Files/LLVM/bin/clang-cl.exe");
        d.flag("-Wno-everything").flag("/O2").file("c/programm_daten.c");
        d.compile("netsurf_programm");
    }
    // Transport der PC-Tests (c/transport_pc.c)
    println!("cargo:rustc-link-lib=ws2_32");
}

/// Ressourcen fürs Board (CSS, Messages de/en, Willkommensseiten, Symbole)
/// als `OUT_DIR/ressourcen.rs`: `DATEIEN`, eine Tabelle im Format von
/// firmware/komponenten/esp32os/eingebettet.c (Name, Zeiger, Länge) mit den
/// eingebetteten Inhalten; das Programm meldet sie als /nsres an.
fn ressourcen_liste(q: &Path) {
    let res = q.join("netsurf/frontends/framebuffer/res");
    let mut liste: Vec<(String, PathBuf)> = Vec::new();
    for f in ["adblock.css", "credits.html", "default.css", "favicon.png", "internal.css", "licence.html", "netsurf.png", "quirks.css"] {
        liste.push((f.into(), res.join(f)));
    }
    for s in ["de", "en"] {
        liste.push((format!("{s}/Messages"), q.join("erzeugt/res").join(s).join("Messages")));
    }
    // eigene Ressourcen (netsurf/res): Startseite mit Suchfeld (ersetzt die
    // welcome.html des Frontends – resource: kennt nur feste Namen),
    // Suchanbieter
    let eigen = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("res");
    println!("cargo:rerun-if-changed={}", eigen.display());
    for f in ["SearchEngines", "welcome.html", "de/welcome.html", "en/welcome.html"] {
        liste.push((f.into(), eigen.join(f)));
    }
    let icons = q.join("netsurf/resources/icons");
    let mut png: Vec<PathBuf> =
        std::fs::read_dir(&icons).expect("resources/icons").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    png.sort();
    for p in png {
        let n = p.file_name().expect("Name").to_string_lossy().into_owned();
        liste.push((format!("icons/{n}"), p));
    }
    let mut t = String::from(
        "// Erzeugt von build.rs (ressourcen_liste).\n\
         /// Eintrag wie `struct eingebettet` in eingebettet.c.\n\
         #[repr(C)]\n\
         pub struct Datei {\n    pub name: *const core::ffi::c_char,\n    pub daten: *const u8,\n    pub laenge: usize,\n}\n\
         // SAFETY: zeigt nur auf unveränderliche Daten des Abbilds.\n\
         unsafe impl Sync for Datei {}\n",
    );
    // je Datei eine Konstante (einmal eingebettet), dann die Tabelle
    for (i, (_, quelle)) in liste.iter().enumerate() {
        assert!(quelle.exists(), "Ressource fehlt: {}", quelle.display());
        println!("cargo:rerun-if-changed={}", quelle.display());
        t.push_str(&format!("const D{i}: &[u8] = include_bytes!({:?});\n", quelle.to_string_lossy().replace('\\', "/")));
    }
    t.push_str(&format!("pub static DATEIEN: [Datei; {}] = [\n", liste.len()));
    for (i, (ziel, _)) in liste.iter().enumerate() {
        t.push_str(&format!("    Datei {{ name: c{ziel:?}.as_ptr(), daten: D{i}.as_ptr(), laenge: D{i}.len() }},\n"));
    }
    t.push_str("];\n");
    let aus = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(aus.join("ressourcen.rs"), t).expect("ressourcen.rs");
}

/// `frontends/framebuffer/gui.c` als geprüfte Kopie in `ziel` (B2):
/// - die Adressleiste deutet Eingaben über `ns_eingabe_zu_url` (c/eingabe.c):
///   Adresse oder – bei Wörtern – Suche beim eingestellten Anbieter;
/// - nach `netsurf_init` die Suchanbieter aus der Ressource `SearchEngines`
///   (eigene Liste in netsurf/res, DuckDuckGo-HTML zuerst).
/// Jede Ersetzung prüft den Originaltext; das Original bleibt unverändert.
fn gui_kopie(q: &Path, ziel: &Path) -> PathBuf {
    let quelle = q.join("netsurf/frontends/framebuffer/gui.c");
    println!("cargo:rerun-if-changed={}", quelle.display());
    let mut t = std::fs::read_to_string(&quelle).expect("gui.c").replace("\r\n", "\n");
    for (alt, neu) in [
        (
            "#include \"framebuffer/local_history.h\"\n",
            "#include \"framebuffer/local_history.h\"\n#include \"desktop/searchweb.h\"\n\
             /* ESP32-OS (B2): Eingabe der Adressleiste → Adresse oder Suche */\n\
             nserror ns_eingabe_zu_url(const char *text, struct nsurl **url);\n\
             /* ESP32-OS (B4): Verlauf und Lesezeichen speichern (gui_quit) */\n\
             void ns_daten_ende(void);\n\
             /* ESP32-OS (B5): Touch-Scrollen und Pinch-Zoom (c/touch.c) */\n\
             int ns_touch_scroll_nehmen(void);\n\
             bool ns_touch_zoom_nehmen(double *faktor);\n\
             /* ESP32-OS (B8): Zwischenablage des Systems (c/ablage.c) */\n\
             extern struct gui_clipboard_table ns_ablage_tabelle;\n\
             unsigned ns_ablage_zaehler(void);\n\
             /* ESP32-OS (B8): Tabs (c/tabs.c) */\n\
             void ns_tab_knopf_setzen(struct gui_window *gw, char art, fbtk_widget_t *knopf);\n\
             fbtk_widget_t *ns_tab_knopf(struct gui_window *gw, char art);\n\
             struct gui_window *ns_tab_aktiv(void);\n\
             struct browser_window *ns_tab_bw(void);\n\
             void ns_tab_neu(struct gui_window *gw);\n\
             void ns_tab_weg(struct gui_window *gw);\n\
             int ns_tabs_klick(fbtk_widget_t *widget, fbtk_callback_info *cbi);\n\
             #define NS_TABS_BREITE 60\n\
             /* ESP32-OS (B8-Review): beim Schliessen eines Tabs geplante Aufrufe streichen */\n\
             static void throbber_advance(void *pw);\n\
             void ns_ohne_js_fenster_weg(struct browser_window *bw);\n\
             /* ESP32-OS (B8): Downloads (c/download.c) */\n\
             extern struct gui_download_table ns_download_tabelle;\n",
        ),
        // B8: Tabs – NetSurfs Knopf „Fenster schliessen“ (das Fenster hat
        // einen eigenen) wird zum Knopf „Tabs n“ mit der Tab-Liste (c/tabs.c)
        (
            "\t\t\twidget = fbtk_create_button(toolbar,\n\t\t\t\t\t\t    (xdir == 1)?xpos : \n\t\t\t\t\t\t     xpos - stop_image_g.width,\n\t\t\t\t\t\t    padding,\n\t\t\t\t\t\t    stop_image_g.width,\n\t\t\t\t\t\t    -padding,\n\t\t\t\t\t\t    frame_col,\n\t\t\t\t\t\t    &stop_image_g,\n\t\t\t\t\t\t    fb_close_click,\n\t\t\t\t\t\t    gw->bw);\n\t\t\tgw->close = widget;\n",
            "\t\t\t/* ESP32-OS (B8): Knopf „Tabs n“ (c/tabs.c) */\n\
             \t\t\twidget = fbtk_create_text_button(toolbar,\n\
             \t\t\t\t\t\t(xdir == 1) ? xpos : xpos - NS_TABS_BREITE,\n\
             \t\t\t\t\t\tpadding, NS_TABS_BREITE, -padding,\n\
             \t\t\t\t\t\tframe_col, FB_COLOUR_BLACK,\n\
             \t\t\t\t\t\tns_tabs_klick, gw);\n\
             \t\t\tfbtk_set_text(widget, \"Tabs 1\");\n\
             \t\t\tgw->close = widget;\n",
        ),
        (
            "\t\t\twidget = gw->close;\n\t\t\tx = (xdir == 1) ? xpos : xpos - stop_image_g.width;\n\t\t\ty = padding;\n\t\t\tw = stop_image_g.width;\n",
            "\t\t\twidget = gw->close;\n\t\t\tx = (xdir == 1) ? xpos : xpos - NS_TABS_BREITE;\n\t\t\ty = padding;\n\t\t\tw = NS_TABS_BREITE;\n",
        ),
        (
            "\t/* Add it to the window list */\n\tgui_window_add_to_window_list(gw);\n",
            "\t/* Add it to the window list */\n\tgui_window_add_to_window_list(gw);\n\
             \t/* ESP32-OS (B8): neues Fenster = neuer, aktiver Tab */\n\tns_tab_neu(gw);\n",
        ),
        (
            "\tfbtk_destroy_widget(gw->window);\n\n\tfree(gw);\n",
            "\t/* ESP32-OS (B8-Review): geplante Aufrufe mit diesem Fenster\n\
             \t * streichen – der Throbber läuft weiter, wenn ein ladender Tab\n\
             \t * geschlossen wird; Rückfallebene ohne JS (c/ohne_js.c) */\n\
             \tframebuffer_schedule(-1, throbber_advance, gw);\n\
             \tns_ohne_js_fenster_weg(gw->bw);\n\
             \tfbtk_destroy_widget(gw->window);\n\
             \t/* ESP32-OS (B8): anderen Tab zeigen */\n\tns_tab_weg(gw);\n\n\tfree(gw);\n",
        ),
        // B5 (Board): Messung Planer (Zeitgeber, Skripte, Layout)
        ("\t\ttimeout = schedule_run();\n", "\t\tNS_MESS(ns_mess_planer_us, timeout = schedule_run());\n"),
        // B8: Downloads in den Download-Ordner (c/download.c)
        ("\t\t.layout = framebuffer_layout_table,\n\t};\n", "\t\t.layout = framebuffer_layout_table,\n\t\t.download = &ns_download_tabelle,\n\t};\n"),
        // B8: Zwischenablage des Systems statt der eigenen von framebuffer/
        ("\t\t.clipboard = framebuffer_clipboard_table,\n", "\t\t.clipboard = &ns_ablage_tabelle,\n"),
        // B8: Markieren per Touch (Halten, dann Ziehen) kopiert den Text gleich
        // – die Bildschirmtastatur hat keine Strg-Taste
        (
            "\t\t\t\t/* Tell core */\n\t\t\t\tbrowser_window_mouse_track(gw->bw, 0, x, y);\n\t\t\t\tbreak;\n\t\t\t}\n\
             \t\t\t/* This is a click;\n\t\t\t * clear PRESSED state and pass to core */\n\
             \t\t\tgui_drag.state = GUI_DRAG_NONE;\n\t\t\tmouse = BROWSER_MOUSE_CLICK_1;\n",
            "\t\t\t\t/* Tell core */\n\t\t\t\tbrowser_window_mouse_track(gw->bw, 0, x, y);\n\
             \t\t\t\t/* ESP32-OS (B8): Markiertes gleich in die Zwischenablage */\n\
             \t\t\t\t/* (die Auswahl der Seite – auch wenn das Fenster sie\n\
             \t\t\t\t * noch nicht kennt; ohne Auswahl wird nichts kopiert) */\n\
             \t\t\t\t{\n\t\t\t\t\tunsigned vorher = ns_ablage_zaehler();\n\
             \t\t\t\t\tbrowser_window_key_press(gw->bw, NS_KEY_COPY_SELECTION);\n\
             \t\t\t\t\tif (ns_ablage_zaehler() != vorher)\n\
             \t\t\t\t\t\tfbtk_set_text(gw->status, getenv(\"LANGUAGE\") != NULL && strncmp(getenv(\"LANGUAGE\"), \"de\", 2) == 0 ? \"Text kopiert\" : \"Text copied\");\n\
             \t\t\t\t}\n\
             \t\t\t\tbreak;\n\t\t\t}\n\
             \t\t\t/* This is a click;\n\t\t\t * clear PRESSED state and pass to core */\n\
             \t\t\tgui_drag.state = GUI_DRAG_NONE;\n\t\t\tmouse = BROWSER_MOUSE_CLICK_1;\n",
        ),
        ("\terror = nsurl_create(text, &url);\n", "\terror = ns_eingabe_zu_url(text, &url);\n"),
        // B4: Adressleiste – erstes Antippen leert sie (Touch: kein Markieren;
        // die Bildschirmtastatur hat keine Ende-Taste)
        (
            "/* reload icon click routine */\n",
            "/* ESP32-OS (B4): erstes Antippen der Adressleiste leert sie, damit
             sich gleich eine neue Adresse tippen lässt; ein weiteres Antippen
             setzt die Schreibmarke wie gewohnt */\n\
             static fbtk_callback ns_url_klick_alt;\n\n\
             static int\nns_url_klick(fbtk_widget_t *widget, fbtk_callback_info *cbi)\n{\n\
             \tint x, y, h;\n\
             \tif (!fbtk_get_caret(widget, &x, &y, &h))\n\t\tfbtk_set_text(widget, \"\");\n\
             \treturn ns_url_klick_alt != NULL ? ns_url_klick_alt(widget, cbi) : 0;\n}\n\n\
             /* reload icon click routine */\n",
        ),
        (
            "\t\t\t\tgw->url = widget; /* keep reference */\n",
            "\t\t\t\tgw->url = widget; /* keep reference */\n\
             \t\t\t\tns_url_klick_alt = fbtk_set_handler(widget, FBTK_CBT_CLICK, ns_url_klick, NULL);\n",
        ),
        // B5: Wischen scrollt um den Fingerweg (gesammelte Pixel aus
        // c/touch.c), ein echtes Mausrad weiter um 100
        (
            "\t\tcase NSFB_KEY_MOUSE_4:\n\t\t\t/* scroll up */\n\t\t\tif (browser_window_scroll_at_point(gw->bw,\n\t\t\t\t\t\t\t   x, y,\n\t\t\t\t\t\t\t   0, -100) == false)\n\t\t\t\twidget_scroll_y(gw, -100, false);\n\t\t\tbreak;\n\n\t\tcase NSFB_KEY_MOUSE_5:\n\t\t\t/* scroll down */\n\t\t\tif (browser_window_scroll_at_point(gw->bw,\n\t\t\t\t\t\t\t   x, y,\n\t\t\t\t\t\t\t   0, 100) == false)\n\t\t\t\twidget_scroll_y(gw, 100, false);\n\t\t\tbreak;\n",
            "\t\tcase NSFB_KEY_MOUSE_4:\n\t\tcase NSFB_KEY_MOUSE_5: {\n\
             \t\t\t/* ESP32-OS (B5): Fingerweg aus c/touch.c, sonst Mausrad */\n\
             \t\t\tint d = ns_touch_scroll_nehmen();\n\
             \t\t\tif (d == 0)\n\
             \t\t\t\td = cbi->event->value.keycode == NSFB_KEY_MOUSE_4 ? -100 : 100;\n\
             \t\t\t/* Innerhalb einer Wischbewegung (Schritte weniger als 300 ms\n\
             \t\t\t * auseinander) bleibt das Ziel: hat der erste Schritt die\n\
             \t\t\t * Seite gescrollt, scrollen die weiteren sie direkt – die\n\
             \t\t\t * Suche nach einem scrollbaren Bereich unter dem Finger geht\n\
             \t\t\t * durch den ganzen Box-Baum (Wikipedia am Board: ~40 ms) */\n\
             \t\t\t{\n\t\t\t\tstatic uint64_t ns_wisch_ms;\n\t\t\t\tstatic bool ns_wisch_seite;\n\
             \t\t\t\tuint64_t ns_jetzt;\n\
             \t\t\t\tnsu_getmonotonic_ms(&ns_jetzt);\n\
             \t\t\t\tif (ns_wisch_seite && ns_jetzt - ns_wisch_ms < 300) {\n\
             \t\t\t\t\tNS_MESS(ns_mess_scroll_us, widget_scroll_y(gw, d, false));\n\
             \t\t\t\t} else {\n\
             \t\t\t\t\tNS_MESS(ns_mess_scroll_us, ns_wisch_seite = !browser_window_scroll_at_point(gw->bw, x, y, 0, d));\n\
             \t\t\t\t\tif (ns_wisch_seite)\n\t\t\t\t\t\twidget_scroll_y(gw, d, false);\n\
             \t\t\t\t}\n\
             \t\t\t\tns_wisch_ms = ns_jetzt;\n\t\t\t}\n\
             \t\t\tbreak;\n\t\t}\n",
        ),
        // B5: Pinch-Zoom nach jedem Ereignis-Durchlauf (0,5- bis 3-fach)
        (
            "\t\tfbtk_redraw(fbtk);\n\t}\n}\n",
            "\t\t/* ESP32-OS (B5): Pinch-Zoom (c/touch.c) */\n\
             \t\t{\n\t\t\tdouble f;\n\
             \t\t\tif (ns_touch_zoom_nehmen(&f) && ns_tab_bw() != NULL) {\n\
             \t\t\t\tfloat s = browser_window_get_scale(ns_tab_bw()) * (float)f;\n\
             \t\t\t\tif (s < 0.5f)\n\t\t\t\t\ts = 0.5f;\n\
             \t\t\t\tif (s > 3.0f)\n\t\t\t\t\ts = 3.0f;\n\
             \t\t\t\tbrowser_window_set_scale(ns_tab_bw(), s, true);\n\
             \t\t\t}\n\t\t}\n\
             \t\tNS_MESS(ns_mess_zeichnen_us, fbtk_redraw(fbtk));\n\t}\n}\n",
        ),
        // B3: Zwischenspeicher am Board begrenzen (vor netsurf_init)
        (
            "\tnsoption_commandline(&argc, argv, nsoptions);\n",
            "\tnsoption_commandline(&argc, argv, nsoptions);\n\
             #ifdef NS_SPEICHER_CACHE\n\
             \t/* ESP32-OS (B3): Zwischenspeicher begrenzt (Heap des Browsers 10 MB) */\n\
             \tnsoption_set_int(memory_cache_size, NS_SPEICHER_CACHE);\n\
             #endif\n\
             \t/* ESP32-OS (B3, PC-Tests): Grenze per Umgebung */\n\
             \t{\n\t\tconst char *grenze = getenv(\"NS_CACHE_GRENZE\");\n\
             \t\tif (grenze != NULL && grenze[0] != '\\0')\n\
             \t\t\tnsoption_set_int(memory_cache_size, atoi(grenze));\n\t}\n\
             \t/* ESP32-OS (B4): Datenordner, Cookie-Dateien */\n\
             \tns_daten_optionen();\n",
        ),
        // Knopf „Ohne JS“/„Mit JS“ (c/ohne_js.c) in der Werkzeugleiste
        ("#define NSFB_TOOLBAR_DEFAULT_LAYOUT \"blfsrutc\"", "#define NSFB_TOOLBAR_DEFAULT_LAYOUT \"blfsrjmvutc\""),
        (
            "\tbrowser_window_reload(bw, true);\n\treturn 1;\n}\n",
            "\tbrowser_window_reload(bw, true);\n\treturn 1;\n}\n\n\
             /* ESP32-OS: Seite ohne bzw. wieder mit JavaScript laden (c/ohne_js.c) */\n\
             void ns_ohne_js_umschalten(struct browser_window *bw);\n\
             const char *ns_ohne_js_knopftext(struct nsurl *url);\n\
             void ns_ohne_js_ladestart(struct browser_window *bw);\n\
             #define NS_JS_KNOPF_BREITE 70\n\
             static fbtk_widget_t *ns_js_knopf;\n\nvoid ns_js_knopf_text(const char *text)\n{\n\tfbtk_widget_t *k = ns_tab_knopf(ns_tab_aktiv(), 'j');\n\tif (k != NULL)\n\t\tfbtk_set_text(k, text);\n}\n\n\
             /* ESP32-OS (B4): Lesezeichen und Verlauf (c/lesezeichen.c) */\n\
             void ns_lesezeichen_umschalten(struct browser_window *bw);\n\
             const char *ns_merken_knopftext(struct nsurl *url);\n\
             void ns_liste_oeffnen(struct browser_window *bw);\n\
             void ns_daten_optionen(void);\n\
             void ns_daten_start(void);\n\
             void ns_daten_ende(void);\n\
             #define NS_MERKEN_BREITE 70\n\
             #define NS_LISTE_BREITE 54\n\
             static fbtk_widget_t *ns_merken_knopf;\n\n\
             void ns_merken_knopf_text(const char *text)\n{\n\
             \tfbtk_widget_t *k = ns_tab_knopf(ns_tab_aktiv(), 'm');\n\
             \tif (k != NULL)\n\t\tfbtk_set_text(k, text);\n}\n\n\
             static int\nns_merken_klick(fbtk_widget_t *widget, fbtk_callback_info *cbi)\n{\n\
             \tif (cbi->event->type != NSFB_EVENT_KEY_UP)\n\t\treturn 0;\n\
             \tns_lesezeichen_umschalten(cbi->context);\n\treturn 1;\n}\n\n\
             static int\nns_liste_klick(fbtk_widget_t *widget, fbtk_callback_info *cbi)\n{\n\
             \tif (cbi->event->type != NSFB_EVENT_KEY_UP)\n\t\treturn 0;\n\
             \tns_liste_oeffnen(cbi->context);\n\treturn 1;\n}\n\n\
             static int\nns_js_knopf_klick(fbtk_widget_t *widget, fbtk_callback_info *cbi)\n{\n\
             \tif (cbi->event->type != NSFB_EVENT_KEY_UP)\n\t\treturn 0;\n\
             \tns_ohne_js_umschalten(cbi->context);\n\treturn 1;\n}\n",
        ),
        (
            "\t\tcase 't': /* throbber/activity indicator */\n\t\t\twidget = fbtk_create_bitmap(",
            "\t\tcase 'j': /* ESP32-OS: ohne/mit JavaScript */\n\
             \t\t\twidget = fbtk_create_text_button(toolbar,\n\
             \t\t\t\t\t\t(xdir == 1) ? xpos : xpos - NS_JS_KNOPF_BREITE,\n\
             \t\t\t\t\t\tpadding, NS_JS_KNOPF_BREITE, -padding,\n\
             \t\t\t\t\t\tframe_col, FB_COLOUR_BLACK,\n\
             \t\t\t\t\t\tns_js_knopf_klick, gw->bw);\n\
             \t\t\tfbtk_set_text(widget, \"Ohne JS\");\n\
             \t\t\tns_js_knopf = widget;\n\t\t\tns_tab_knopf_setzen(gw, 'j', widget);\n\t\t\tbreak;\n\n\
             \t\tcase 't': /* throbber/activity indicator */\n\t\t\twidget = fbtk_create_bitmap(",
        ),
        (
            "\t\tcase 't': /* throbber/activity indicator */\n\t\t\twidget = fbtk_create_bitmap(",
            "\t\tcase 'm': /* ESP32-OS (B4): Lesezeichen merken */\n\
             \t\t\twidget = fbtk_create_text_button(toolbar,\n\
             \t\t\t\t\t\t(xdir == 1) ? xpos : xpos - NS_MERKEN_BREITE,\n\
             \t\t\t\t\t\tpadding, NS_MERKEN_BREITE, -padding,\n\
             \t\t\t\t\t\tframe_col, FB_COLOUR_BLACK,\n\
             \t\t\t\t\t\tns_merken_klick, gw->bw);\n\
             \t\t\tfbtk_set_text(widget, ns_merken_knopftext(NULL));\n\
             \t\t\tns_merken_knopf = widget;\n\t\t\tns_tab_knopf_setzen(gw, 'm', widget);\n\t\t\tbreak;\n\n\
             \t\tcase 'v': /* ESP32-OS (B4): Lesezeichen und Verlauf */\n\
             \t\t\twidget = fbtk_create_text_button(toolbar,\n\
             \t\t\t\t\t\t(xdir == 1) ? xpos : xpos - NS_LISTE_BREITE,\n\
             \t\t\t\t\t\tpadding, NS_LISTE_BREITE, -padding,\n\
             \t\t\t\t\t\tframe_col, FB_COLOUR_BLACK,\n\
             \t\t\t\t\t\tns_liste_klick, gw->bw);\n\
             \t\t\tfbtk_set_text(widget, getenv(\"LANGUAGE\") != NULL && strncmp(getenv(\"LANGUAGE\"), \"de\", 2) == 0 ? \"Liste\" : \"List\");\n\
             \t\t\tbreak;\n\n\
             \t\tcase 't': /* throbber/activity indicator */\n\t\t\twidget = fbtk_create_bitmap(",
        ),
        (
            "\t\tcase 't': /* throbber/activity indicator */\n\t\t\twidget = gw->throbber;\n",
            "\t\tcase 'j': /* ESP32-OS: ohne/mit JavaScript */\n\
             \t\t\twidget = ns_tab_knopf(gw, 'j');\n\
             \t\t\tx = (xdir == 1) ? xpos : xpos - NS_JS_KNOPF_BREITE;\n\
             \t\t\ty = padding;\n\t\t\tw = NS_JS_KNOPF_BREITE;\n\t\t\th = -padding;\n\t\t\tbreak;\n\n\
             \t\tcase 't': /* throbber/activity indicator */\n\t\t\twidget = gw->throbber;\n",
        ),
        (
            "\t\tcase 'j': /* ESP32-OS: ohne/mit JavaScript */\n\t\t\twidget = ns_tab_knopf(gw, 'j');\n",
            "\t\tcase 'm': /* ESP32-OS (B4) */\n\
             \t\t\twidget = ns_tab_knopf(gw, 'm');\n\
             \t\t\tx = (xdir == 1) ? xpos : xpos - NS_MERKEN_BREITE;\n\
             \t\t\ty = padding;\n\t\t\tw = NS_MERKEN_BREITE;\n\t\t\th = -padding;\n\t\t\tbreak;\n\n\
             \t\tcase 'v': /* ESP32-OS (B4): Knopf ohne Merker – nicht verschieben */\n\
             \t\t\twidget = NULL;\n\
             \t\t\tw = NS_LISTE_BREITE;\n\t\t\tbreak;\n\n\
             \t\tcase 'j': /* ESP32-OS: ohne/mit JavaScript */\n\t\t\twidget = ns_tab_knopf(gw, 'j');\n",
        ),
        (
            "\tfbtk_set_text(g->url, nsurl_access(url));\n\treturn NSERROR_OK;\n",
            "\tfbtk_set_text(g->url, nsurl_access(url));\n\
             \tif (ns_tab_knopf(g, 'j') != NULL)\n\t\tfbtk_set_text(ns_tab_knopf(g, 'j'), ns_ohne_js_knopftext(url));\n\
             \tif (ns_tab_knopf(g, 'm') != NULL)\n\t\tfbtk_set_text(ns_tab_knopf(g, 'm'), ns_merken_knopftext(url));\n\
             \treturn NSERROR_OK;\n",
        ),
        (
            "\tg->throbber_index = 0;\n\tframebuffer_schedule(100, throbber_advance, g);\n",
            "\tg->throbber_index = 0;\n\tframebuffer_schedule(100, throbber_advance, g);\n\
             \t/* ESP32-OS: Rückfall ohne JavaScript, wenn die Seite nicht erscheint */\n\
             \tns_ohne_js_ladestart(g->bw);\n",
        ),
        (
            "\tret = netsurf_init(NULL);\n\tif (ret != NSERROR_OK) {\n\t\tdie(\"NetSurf failed to initialise\");\n\t}\n",
            "\tret = netsurf_init(NULL);\n\tif (ret != NSERROR_OK) {\n\t\tdie(\"NetSurf failed to initialise\");\n\t}\n\n\
             \t/* ESP32-OS (B2): Suchanbieter (SearchEngines, DuckDuckGo-HTML zuerst) */\n\
             \t{\n\t\tchar *anbieter = filepath_find(respaths, \"SearchEngines\");\n\
             \t\tsearch_web_init(anbieter);\n\t\tfree(anbieter);\n\
             \t\tsearch_web_select_provider(nsoption_int(search_provider));\n\t}\n\
             \t/* ESP32-OS (B4): Adressen (Verlauf) und Lesezeichen laden */\n\
             \tns_daten_start();\n",
        ),
        (
            "\turldb_save_cookies(nsoption_charp(cookie_jar));\n\n\tframebuffer_finalise();\n",
            "\t/* ESP32-OS (B4): Verlauf und Lesezeichen speichern */\n\
             \tns_daten_ende();\n\
             \turldb_save_cookies(nsoption_charp(cookie_jar));\n\n\tframebuffer_finalise();\n",
        ),
        // B4: nach jeder neuen Adresse gibt die Adressleiste die Schreibmarke
        // ab – das nächste Antippen leert sie wieder (sonst landete neuer Text
        // mitten in der alten Adresse)
        (
            "\tif (ns_tab_knopf(g, 'm') != NULL)\n\t\tfbtk_set_text(ns_tab_knopf(g, 'm'), ns_merken_knopftext(url));\n",
            "\tif (ns_tab_knopf(g, 'm') != NULL)\n\t\tfbtk_set_text(ns_tab_knopf(g, 'm'), ns_merken_knopftext(url));\n\
             \t{\n\t\tint cx, cy, ch;\n\
             \t\tif (fbtk_get_caret(g->url, &cx, &cy, &ch))\n\
             \t\t\tfbtk_set_caret(g->url, false, 0, 0, 0, NULL);\n\t}\n",
        ),
        // B5 (Board): Zeitmessung je Bild beim Scrollen – Verschieben
        // (nsfb_plot_copy), Seite zeichnen, an die Fläche geben
        (
            "static void\nfb_pan(fbtk_widget_t *widget,\n",
            "#ifdef ESP_PLATFORM\n\
             #include \"esp_timer.h\"\n\
             /* ESP32-OS (B5): Messung, ausgegeben von c/flaeche_esp32os.c */\n\
             int64_t ns_mess_kopie_us, ns_mess_seite_us, ns_mess_ausgabe_us;\n\
             int64_t ns_mess_planer_us, ns_mess_zeichnen_us, ns_mess_scroll_us;\n\
             unsigned ns_mess_seite_n, ns_mess_seite_px;\n\
             #define NS_MESS(v, x) do { int64_t ns_t0 = esp_timer_get_time(); x; v += esp_timer_get_time() - ns_t0; } while (0)\n\
             #else\n\
             #define NS_MESS(v, x) x\n\
             #endif\n\n\
             static void\nfb_pan(fbtk_widget_t *widget,\n",
        ),
        (
            "\t\t/* move part that remains visible up */\n\t\tnsfb_plot_copy(nsfb, &srcbox, nsfb, &dstbox);\n",
            "\t\t/* move part that remains visible up */\n\t\tNS_MESS(ns_mess_kopie_us, nsfb_plot_copy(nsfb, &srcbox, nsfb, &dstbox));\n",
        ),
        (
            "\t\t/* move part that remains visible down */\n\t\tnsfb_plot_copy(nsfb, &srcbox, nsfb, &dstbox);\n",
            "\t\t/* move part that remains visible down */\n\t\tNS_MESS(ns_mess_kopie_us, nsfb_plot_copy(nsfb, &srcbox, nsfb, &dstbox));\n",
        ),
        (
            "\tbrowser_window_redraw(bw,\n\t\t\tx - bwidget->scrollx,\n\t\t\ty - bwidget->scrolly,\n\t\t\t&clip, &ctx);\n",
            "#ifdef ESP_PLATFORM\n\
             \tns_mess_seite_n++;\n\
             \tns_mess_seite_px += (unsigned)((clip.x1 - clip.x0) * (clip.y1 - clip.y0));\n\
             #endif\n\
             \tNS_MESS(ns_mess_seite_us, browser_window_redraw(bw,\n\t\t\tx - bwidget->scrollx,\n\t\t\ty - bwidget->scrolly,\n\t\t\t&clip, &ctx));\n",
        ),
        (
            "\tnsfb_update(fbtk_get_nsfb(widget), &bwidget->redraw_box);\n\n\tbwidget->redraw_box.y0 = bwidget->redraw_box.x0 = INT_MAX;\n",
            "\tNS_MESS(ns_mess_ausgabe_us, nsfb_update(fbtk_get_nsfb(widget), &bwidget->redraw_box));\n\n\tbwidget->redraw_box.y0 = bwidget->redraw_box.x0 = INT_MAX;\n",
        ),
        (
            // B7: Rollen ist im Framebuffer nur vorgemerkt (pan, beim
            // nächsten Zeichnen) – die Lage zählt das Vorgemerkte mit, sonst
            // sähe scrollTo → scrollY die alte Lage
            "\t*sx = bwidget->scrollx;\n\t*sy = bwidget->scrolly;\n",
            "\t/* ESP32-OS (B7): vorgemerktes Rollen mitzählen */\n\t*sx = bwidget->scrollx + bwidget->panx;\n\t*sy = bwidget->scrolly + bwidget->pany;\n",
        ),
    ] {
        assert_eq!(t.matches(alt).count(), 1, "gui.c: Originaltext nicht (genau einmal) gefunden: {alt:?}");
        t = t.replacen(alt, neu, 1);
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (gui_kopie) aus frontends/framebuffer/gui.c. */\n{t}");
    let datei = ziel.join("gui.c");
    std::fs::create_dir_all(ziel).expect("Ordner");
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, t).expect("gui.c");
    }
    datei
}

/// `content/handlers/html/html.c` als geprüfte Kopie: eine Seite auf der
/// Liste „ohne JavaScript“ (c/ohne_js.c, Rückfallebene und Knopf) wird ohne
/// Skripte angelegt (`enable_scripting`, wirkt auf Parser, Skripte und
/// `<noscript>`).
fn html_kopie(q: &Path, ziel: &Path) -> PathBuf {
    let quelle = q.join("netsurf/content/handlers/html/html.c");
    println!("cargo:rerun-if-changed={}", quelle.display());
    let mut t = std::fs::read_to_string(&quelle).expect("html.c").replace("\r\n", "\n");
    for (alt, neu) in [
        (
            "#include \"html/layout.h\"\n",
            "#include \"html/layout.h\"\n\
             /* ESP32-OS: Seiten ohne JavaScript (c/ohne_js.c) */\n\
             bool ns_ohne_js(struct nsurl *url);\n",
        ),
        (
            "\tc->enable_scripting = nsoption_bool(enable_javascript);\n",
            "\tc->enable_scripting = nsoption_bool(enable_javascript) && !ns_ohne_js(content_get_url(&c->base));\n",
        ),
        (
            // B7: DOMContentLoaded früh – html_begin_conversion wartet auf alle
            // Abrufe (auch Bilder); DCL gehört ans Ende des Parsens (nach den
            // aufgeschobenen Skripten). Den Parse wie dort abschliessen, falls
            // kein synchrones Skript mehr aussteht.
            "static bool html_convert(struct content *c)\n{\n",
            "/* ESP32-OS (B7): DOMContentLoaded, sobald das Parsen fertig ist und alle\n\
             \x20* aufgeschobenen Skripte da sind (nicht erst nach allen Bildern). */\n\
             static void ns_dcl_frueh(html_content *htmlc)\n{\n\
             \tunsigned int i;\n\
             \tif (htmlc->jsthread == NULL)\n\t\treturn;\n\
             \tif (htmlc->parse_completed == false) {\n\
             \t\tdom_hubbub_error e = dom_hubbub_parser_completed(htmlc->parser);\n\
             \t\tif (e != DOM_HUBBUB_OK)\n\
             \t\t\treturn; /* synchrones Skript steht aus: später in html_begin_conversion */\n\
             \t\thtmlc->parse_completed = true;\n\t}\n\
             \tfor (i = 0; i != htmlc->scripts_count; i++) {\n\
             \t\tstruct html_script *s = &htmlc->scripts[i];\n\
             \t\tif (s->already_started || s->type != HTML_SCRIPT_DEFER)\n\t\t\tcontinue;\n\
             \t\tif (s->data.handle == NULL)\n\t\t\treturn;\n\
             \t\tif (content_get_status(s->data.handle) != CONTENT_STATUS_DONE &&\n\
             \t\t    content_get_status(s->data.handle) != CONTENT_STATUS_ERROR)\n\
             \t\t\treturn; /* aufgeschobenes Skript lädt noch */\n\t}\n\
             \thtml_script_exec(htmlc, true);\n\
             \tjs_fire_event(htmlc->jsthread, \"DOMContentLoaded\", htmlc->document, NULL);\n}\n\n\
             static bool html_convert(struct content *c)\n{\n",
        ),
        (
            "\thtmlc->base.active--; /* the html fetch is no longer active */\n\tNSLOG(netsurf, INFO, \"%d fetches active (%p)\", htmlc->base.active, c);\n",
            "\thtmlc->base.active--; /* the html fetch is no longer active */\n\tNSLOG(netsurf, INFO, \"%d fetches active (%p)\", htmlc->base.active, c);\n\
             \tns_dcl_frueh(htmlc); /* ESP32-OS (B7) */\n",
        ),
        (
            // B7: DOMContentLoaded – NetSurf hat an dieser Stelle nur den Kommentar
            "\t/* fire a simple event that bubbles named DOMContentLoaded at\n\t * the Document.\n\t */\n",
            "\t/* fire a simple event that bubbles named DOMContentLoaded at\n\t * the Document.\n\t */\n\
             \tif (htmlc->jsthread != NULL) /* ESP32-OS (B7) */\n\
             \t\tjs_fire_event(htmlc->jsthread, \"DOMContentLoaded\", htmlc->document, NULL);\n",
        ),
    ] {
        assert_eq!(t.matches(alt).count(), 1, "html.c: Originaltext nicht (genau einmal) gefunden: {alt:?}");
        t = t.replacen(alt, neu, 1);
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (html_kopie) aus content/handlers/html/html.c. */\n{t}");
    let datei = ziel.join("html.c");
    std::fs::create_dir_all(ziel).expect("Ordner");
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, t).expect("html.c");
    }
    datei
}

/// `frontends/framebuffer/fbtk/fbtk.c` als geprüfte Kopie (B6): jede
/// Schreibmarke – Formularfeld einer Seite oder Adressleiste – läuft über
/// `fbtk_set_caret`; dort meldet die Kopie dem Kern, ob ein Textfeld den
/// Fokus hat und wo es liegt (Bildschirmtastatur erscheint, verdeckt das
/// Feld nicht, verschwindet wieder).
fn fbtk_kopie(q: &Path, ziel: &Path) -> PathBuf {
    let quelle = q.join("netsurf/frontends/framebuffer/fbtk/fbtk.c");
    println!("cargo:rerun-if-changed={}", quelle.display());
    let mut t = std::fs::read_to_string(&quelle).expect("fbtk.c").replace("\r\n", "\n");
    for (alt, neu) in [
        (
            "#include \"widget.h\"\n",
            "#include \"widget.h\"\n\n\
             /* ESP32-OS (B6): Textfeld-Fokus an den Kern (netsurf/src/lib.rs) */\n\
             void ns_tastatur_fokus(int an, int x, int y, int hoehe);\n",
        ),
        (
            "\t} else {\n\t\troot->u.root.caret.owner = NULL;\n\t\troot->u.root.caret.remove_cb = NULL;\n\t}\n}\n",
            "\t} else {\n\t\troot->u.root.caret.owner = NULL;\n\t\troot->u.root.caret.remove_cb = NULL;\n\t}\n\
             \t/* ESP32-OS (B6): Bildschirmtastatur – Lage im Fenster */\n\
             \tns_tastatur_fokus(set, fbtk_get_absx(widget) + x, fbtk_get_absy(widget) + y, height);\n}\n",
        ),
        // B8-Review: Tabs zerstören Fenster im Betrieb (upstream nur beim
        // Beenden) – die Wurzel darf danach nicht mehr auf das Widget zeigen
        // (Eingabefokus, Zeigergriff, letztes Widget unter dem Zeiger,
        // Schreibmarke)
        (
            "\t\t\tparent->last_child = widget->prev;\n\t\t}\n\n\t\tfree(widget);\n",
            "\t\t\tparent->last_child = widget->prev;\n\t\t}\n\n\
             \t\t/* ESP32-OS (B8): Verweise der Wurzel auf das Widget löschen */\n\
             \t\t{\n\t\t\tfbtk_widget_t *wurzel = fbtk_get_root_widget(parent);\n\
             \t\t\tif (wurzel != NULL) {\n\
             \t\t\t\tif (wurzel->u.root.input == widget)\n\t\t\t\t\twurzel->u.root.input = NULL;\n\
             \t\t\t\tif (wurzel->u.root.grabbed == widget)\n\t\t\t\t\twurzel->u.root.grabbed = NULL;\n\
             \t\t\t\tif (wurzel->u.root.prev == widget)\n\t\t\t\t\twurzel->u.root.prev = NULL;\n\
             \t\t\t\tif (wurzel->u.root.caret.owner == widget) {\n\
             \t\t\t\t\twurzel->u.root.caret.owner = NULL;\n\
             \t\t\t\t\twurzel->u.root.caret.remove_cb = NULL;\n\
             \t\t\t\t\tns_tastatur_fokus(0, 0, 0, 0);\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\
             \t\tfree(widget);\n",
        ),
    ] {
        assert_eq!(t.matches(alt).count(), 1, "fbtk.c: Originaltext nicht (genau einmal) gefunden: {alt:?}");
        t = t.replacen(alt, neu, 1);
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (fbtk_kopie) aus frontends/framebuffer/fbtk/fbtk.c. */\n{t}");
    let datei = ziel.join("fbtk.c");
    std::fs::create_dir_all(ziel).expect("Ordner");
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, t).expect("fbtk.c");
    }
    datei
}

/// Upstream-Datei `rel` (unter `netsurf/`) mit geprüften Ersetzungen nach
/// `ziel/name` – jeder Originaltext muss genau einmal vorkommen.
fn kopie_mit(q: &Path, ziel: &Path, rel: &str, name: &str, ersetzungen: &[(&str, &str)]) -> PathBuf {
    kopie_aus(&q.join("netsurf"), ziel, rel, name, ersetzungen)
}

/// Wie `kopie_mit`, Datei `rel` unter dem Ordner `wurzel` (z. B. libcss).
fn kopie_aus(wurzel: &Path, ziel: &Path, rel: &str, name: &str, ersetzungen: &[(&str, &str)]) -> PathBuf {
    let quelle = wurzel.join(rel);
    println!("cargo:rerun-if-changed={}", quelle.display());
    let mut t = std::fs::read_to_string(&quelle).unwrap_or_else(|_| panic!("{rel}")).replace("\r\n", "\n");
    for (alt, neu) in ersetzungen {
        assert_eq!(t.matches(alt).count(), 1, "{rel}: Originaltext nicht (genau einmal) gefunden: {alt:?}");
        t = t.replacen(alt, neu, 1);
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (kopie_mit) aus {rel}. */\n{t}");
    let datei = ziel.join(name);
    std::fs::create_dir_all(ziel).expect("Ordner");
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, t).expect(name);
    }
    datei
}

/// `libcss/src/parse/language.c` als geprüfte Kopie (Q3): jede verworfene
/// Deklaration an c/css_luecken.c melden – unbekannte Eigenschaft (Art 0),
/// Wert mit unbekannter Funktion wie `var()`/`calc()` (Art 1) oder sonst
/// ungültiger Wert (Art 2). So entsteht je Seite die Liste der fehlenden
/// CSS-Funktionen nach Häufigkeit.
fn css_sprache_kopie(libcss: &Path, ziel: &Path) -> PathBuf {
    kopie_aus(
        libcss,
        ziel,
        "src/parse/language.c",
        "css_language.c",
        &[
            (
                "#include \"utils/utils.h\"
",
                "#include \"utils/utils.h\"

void ns_css_luecke(int art, const char *name, size_t len);

/* ESP32-OS Q3: verworfenen Wert melden – erste Funktion im Wert, sonst
 * die Eigenschaft */
static void luecke_wert(const css_token *property, const parserutils_vector *vector, int32_t anfang)
{
	int32_t i = anfang;
	const css_token *t;
	while ((t = parserutils_vector_iterate(vector, &i)) != NULL) {
		if (t->type == CSS_TOKEN_FUNCTION && t->idata != NULL) {
			ns_css_luecke(1, lwc_string_data(t->idata), lwc_string_length(t->idata));
			return;
		}
	}
	ns_css_luecke(2, lwc_string_data(property->idata), lwc_string_length(property->idata));
}
",
            ),
            (
                "	if (i == LAST_PROP + 1)
		return CSS_INVALID;
",
                "	if (i == LAST_PROP + 1) {
		ns_css_luecke(0, lwc_string_data(property->idata), lwc_string_length(property->idata));
		return CSS_INVALID;
	}
",
            ),
            (
                "	/* Call the handler */
	error = handler(c, vector, ctx, style);
	if (error != CSS_OK) {
",
                "	/* Call the handler */
	anfang = *ctx;
	error = handler(c, vector, ctx, style);
	if (error != CSS_OK) {
		if (error == CSS_INVALID)
			luecke_wert(property, vector, anfang);
",
            ),
            (
                "	uint8_t flags = 0;
	css_style *style = NULL;
	const css_token *token;
",
                "	uint8_t flags = 0;
	css_style *style = NULL;
	const css_token *token;
	int32_t anfang;
",
            ),
            (
                "	if (token != NULL) {
		/* Trailing junk, so discard declaration */
",
                "	if (token != NULL) {
		/* Trailing junk, so discard declaration */
		luecke_wert(property, vector, anfang);
",
            ),
        ],
    )
}

/// `content/handlers/css/css.c` als geprüfte Kopie (Q3): jedes Stylesheet
/// (Datei und `<style>`) geht durch c/css_vorbereitung.c – eigene
/// Eigenschaften, `var()`, `calc()`, `rgb()`-Schreibweise, logische
/// Eigenschaften. Teilstücke bis zur letzten vollständigen Regel, der Rest
/// beim Abschluss; verworfene Stylesheets geben ihren Rest frei.
fn css_inhalt_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "content/handlers/css/css.c",
        "css_inhalt.c",
        &[
            (
                "#include \"css/internal.h\"\n",
                "#include \"css/internal.h\"\n\n/* ESP32-OS Q3: c/css_vorbereitung.c */\n#include <stdlib.h>\nchar *ns_css_stueck(const void *k, const char *s, size_t n, size_t *aus_n);\nchar *ns_css_ende(const void *k, size_t *aus_n);\nvoid ns_css_vergessen(const void *k);\n",
            ),
            (
                "\treturn css_stylesheet_append_data(c->sheet,\n\t\t\t(const uint8_t *) data, size);\n}\n",
                "\tsize_t n;\n\tcss_error e;\n\tchar *t = ns_css_stueck(c, data, size, &n);\n\tif (t == NULL)\n\t\treturn n == 0 ? CSS_NEEDDATA : css_stylesheet_append_data(c->sheet, (const uint8_t *) data, size);\n\te = n > 0 ? css_stylesheet_append_data(c->sheet, (const uint8_t *) t, n) : CSS_NEEDDATA;\n\tfree(t);\n\treturn e;\n}\n",
            ),
            (
                "\terror = css_stylesheet_data_done(c->sheet);\n",
                "\t{\n\t\t/* ESP32-OS Q3: vorbereiteter Rest des letzten Stücks */\n\t\tsize_t n;\n\t\tchar *t = ns_css_ende(c, &n);\n\t\tif (t != NULL) {\n\t\t\tif (n > 0)\n\t\t\t\t(void) css_stylesheet_append_data(c->sheet, (const uint8_t *) t, n);\n\t\t\tfree(t);\n\t\t}\n\t}\n\terror = css_stylesheet_data_done(c->sheet);\n",
            ),
            (
                "static void nscss_destroy_css_data(struct content_css_data *c)\n{\n\tuint32_t i;\n",
                "static void nscss_destroy_css_data(struct content_css_data *c)\n{\n\tuint32_t i;\n\n\tns_css_vergessen(c);\n",
            ),
        ],
    )
}

/// `desktop/browser_window.c` als geprüfte Kopie (Q3): eine neue Seite im
/// Hauptfenster (nicht in einem Rahmen) leert die Tabelle der eigenen
/// CSS-Eigenschaften (c/css_vorbereitung.c) – sonst färbten die Variablen
/// der vorigen Seite die nächste (Board 26.11.: nach der DuckDuckGo-App
/// fehlte auf html.duckduckgo das Logo).
fn browser_window_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "desktop/browser_window.c",
        "browser_window.c",
        &[(
            "\tNSLOG(netsurf, INFO, \"bw %p, url %s\", bw, nsurl_access(url));\n",
            "\tNSLOG(netsurf, INFO, \"bw %p, url %s\", bw, nsurl_access(url));\n\n\t/* ESP32-OS Q3: neue Seite im Hauptfenster: CSS-Variablen leeren */\n\tif (bw->parent == NULL) {\n\t\textern void ns_css_variablen_leeren(void);\n\t\tns_css_variablen_leeren();\n\t}\n",
        )],
    )
}

/// `content/handlers/css/select.c` als geprüfte Kopie (Q3): style-Attribute
/// gehen durch c/css_vorbereitung.c wie die Stylesheets.
fn css_select_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "content/handlers/css/select.c",
        "css_select.c",
        &[
            (
                "\terror = css_stylesheet_append_data(sheet, data, len);\n",
                "\t{\n\t\t/* ESP32-OS Q3: var(), calc() … im style-Attribut */\n\t\tsize_t vn = 0;\n\t\tchar *v = ns_css_inline((const char *) data, len, &vn);\n\t\tif (v == NULL)\n\t\t\terror = css_stylesheet_append_data(sheet, data, len);\n\t\telse\n\t\t\terror = vn > 0 ? css_stylesheet_append_data(sheet, (const uint8_t *) v, vn) : CSS_OK;\n\t\tfree(v);\n\t}\n",
            ),
            (
                "#include \"css/select.h\"\n",
                "#include \"css/select.h\"\n\n/* ESP32-OS Q3: c/css_vorbereitung.c */\n#include <stdlib.h>\nchar *ns_css_inline(const char *s, size_t n, size_t *aus_n);\n",
            ),
        ],
    )
}

/// `content/handlers/html/script.c` als geprüfte Kopie (Q3): NetSurf
/// führte `<script type="module">` als klassisches Skript aus – am Board 40
/// der 61 JS-Fehler der 30 Testseiten („Unexpected token '{'“ bei `import`,
/// „unsupported keyword: export“). Ein Browser ohne ES-Module überspringt
/// solche Skripte (und Import-Maps) und führt `nomodule`-Skripte aus, wie es
/// NetSurf ohnehin tut.
fn script_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "content/handlers/html/script.c",
        "html_script.c",
        &[(
            "	exc = dom_element_get_attribute(node, corestring_dom_type, &mimetype);
	if (exc != DOM_NO_ERR || mimetype == NULL) {
		mimetype = dom_string_ref(corestring_dom_text_javascript);
	}
",
            "	exc = dom_element_get_attribute(node, corestring_dom_type, &mimetype);
	if (exc != DOM_NO_ERR || mimetype == NULL) {
		mimetype = dom_string_ref(corestring_dom_text_javascript);
	}

\
             	/* ESP32-OS Q3: ES-Module und Import-Maps nicht ausführen */
\
             	if ((dom_string_byte_length(mimetype) == 6 &&
\
             	     strncasecmp(dom_string_data(mimetype), \"module\", 6) == 0) ||
\
             	    (dom_string_byte_length(mimetype) == 9 &&
\
             	     strncasecmp(dom_string_data(mimetype), \"importmap\", 9) == 0)) {
\
             		NSLOG(jserrors, WARNING, \"Modul-Skript übersprungen\");
\
             		dom_string_unref(mimetype);
\
             		return DOM_HUBBUB_OK;
\
             	}
",
        )],
    )
}

/// `frontends/framebuffer/schedule.c` als geprüfte Kopie (BR2): Bei vollem
/// Heap lieferte calloc NULL und `gettimeofday(&nscb->tv, …)` schrieb an
/// Adresse 8 (am Board: Absturz in `_gettimeofday_r`). Jetzt NSERROR_NOMEM.
fn schedule_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "frontends/framebuffer/schedule.c",
        "fb_schedule.c",
        &[(
            "\tnscb = calloc(1, sizeof(struct nscallback));\n\n\tgettimeofday(&nscb->tv, NULL);",
            "\tnscb = calloc(1, sizeof(struct nscallback));\n\tif (nscb == NULL) {\n\t\treturn NSERROR_NOMEM; /* ESP32-OS (BR2) */\n\t}\n\n\tgettimeofday(&nscb->tv, NULL);",
        )],
    )
}

/// `frontends/framebuffer/fbtk/event.c` als geprüfte Kopie (B8): Tastencodes
/// ab 0x10000 sind schon Unicode (Zeichen der Bildschirmtastatur, auch
/// Grossbuchstaben, Umlaute, „%“ und „{|}~“, für die nsfb keine Codes hat;
/// c/flaeche_esp32os.c).
fn event_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "frontends/framebuffer/fbtk/event.c",
        "fbtk_event.c",
        &[(
            "fbtk_keycode_to_ucs4(int code, fbtk_modifier_type mods)\n{\n\tint ucs4 = -1;\n",
            "fbtk_keycode_to_ucs4(int code, fbtk_modifier_type mods)\n{\n\tint ucs4 = -1;\n\n\
             \t/* ESP32-OS (B8): schon Unicode (Bildschirmtastatur) */\n\
             \tif (code >= 0x10000)\n\t\treturn code - 0x10000;\n",
        )],
    )
}

/// `frontends/framebuffer/fbtk/text.c` als geprüfte Kopie (B8): Textfelder
/// des Frontends (Adressleiste) speichern UTF-8 – Einfügen schreibt das
/// Zeichen als 1–4 Byte (vorher ein Byte, unbekannte Tasten als 0xFF),
/// Rücktaste und Pfeiltasten gehen um ganze Zeichen.
fn text_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "frontends/framebuffer/fbtk/text.c",
        "fbtk_text.c",
        &[
            (
                "\tcase NSFB_KEY_BACKSPACE:\n\t\tif (widget->u.text.idx <= 0)\n\t\t\tbreak;\n\
                 \t\tmemmove(widget->u.text.text + widget->u.text.idx - 1,\n\
                 \t\t\t\twidget->u.text.text + widget->u.text.idx,\n\
                 \t\t\t\twidget->u.text.len - widget->u.text.idx);\n\
                 \t\twidget->u.text.idx--;\n\t\twidget->u.text.len--;\n",
                "\tcase NSFB_KEY_BACKSPACE: {\n\
                 \t\t/* ESP32-OS (B8): ein ganzes UTF-8-Zeichen */\n\
                 \t\tint n = 1;\n\
                 \t\tif (widget->u.text.idx <= 0)\n\t\t\tbreak;\n\
                 \t\twhile (n < widget->u.text.idx && (widget->u.text.text[widget->u.text.idx - n] & 0xC0) == 0x80)\n\
                 \t\t\tn++;\n\
                 \t\tmemmove(widget->u.text.text + widget->u.text.idx - n,\n\
                 \t\t\t\twidget->u.text.text + widget->u.text.idx,\n\
                 \t\t\t\twidget->u.text.len - widget->u.text.idx);\n\
                 \t\twidget->u.text.idx -= n;\n\t\twidget->u.text.len -= n;\n",
            ),
            (
                "\t\tcaret_moved = true;\n\t\tbreak;\n\n\tcase NSFB_KEY_RETURN:\n",
                "\t\tcaret_moved = true;\n\t\tbreak;\n\t}\n\n\tcase NSFB_KEY_RETURN:\n",
            ),
            (
                "\t\t\tif (modifier == FBTK_MOD_CLEAR)\n\t\t\t\twidget->u.text.idx++;\n",
                "\t\t\tif (modifier == FBTK_MOD_CLEAR) {\n\
                 \t\t\t\tdo\n\t\t\t\t\twidget->u.text.idx++;\n\
                 \t\t\t\twhile (widget->u.text.idx < widget->u.text.len &&\n\
                 \t\t\t\t       (widget->u.text.text[widget->u.text.idx] & 0xC0) == 0x80);\n\
                 \t\t\t}\n",
            ),
            (
                "\t\t\tif (modifier == FBTK_MOD_CLEAR)\n\t\t\t\twidget->u.text.idx--;\n",
                "\t\t\tif (modifier == FBTK_MOD_CLEAR) {\n\
                 \t\t\t\tdo\n\t\t\t\t\twidget->u.text.idx--;\n\
                 \t\t\t\twhile (widget->u.text.idx > 0 &&\n\
                 \t\t\t\t       (widget->u.text.text[widget->u.text.idx] & 0xC0) == 0x80);\n\
                 \t\t\t}\n",
            ),
            (
                "\t\t/* allow for new character and null */\n\
                 \t\ttemp = realloc(widget->u.text.text, widget->u.text.len + 2);\n\
                 \t\tif (temp == NULL) {\n\t\t\tbreak;\n\t\t}\n\n\
                 \t\twidget->u.text.text = temp;\n\
                 \t\tmemmove(widget->u.text.text + widget->u.text.idx + 1,\n\
                 \t\t\t\twidget->u.text.text + widget->u.text.idx,\n\
                 \t\t\t\twidget->u.text.len - widget->u.text.idx);\n\
                 \t\twidget->u.text.text[widget->u.text.idx] =\n\
                 \t\t\t\tfbtk_keycode_to_ucs4(value, modifier);\n\
                 \t\twidget->u.text.idx++;\n\t\twidget->u.text.len++;\n",
                "\t\t{\n\
                 \t\t\t/* ESP32-OS (B8): Zeichen als UTF-8; unbekannte Tasten nicht */\n\
                 \t\t\tint u = fbtk_keycode_to_ucs4(value, modifier);\n\
                 \t\t\tchar b[4];\n\t\t\tint n;\n\
                 \t\t\tif (u < 32 || u > 0x10FFFF)\n\t\t\t\tbreak;\n\
                 \t\t\tif (u < 0x80) {\n\t\t\t\tb[0] = (char)u;\n\t\t\t\tn = 1;\n\
                 \t\t\t} else if (u < 0x800) {\n\
                 \t\t\t\tb[0] = (char)(0xC0 | (u >> 6));\n\t\t\t\tb[1] = (char)(0x80 | (u & 0x3F));\n\t\t\t\tn = 2;\n\
                 \t\t\t} else if (u < 0x10000) {\n\
                 \t\t\t\tb[0] = (char)(0xE0 | (u >> 12));\n\t\t\t\tb[1] = (char)(0x80 | ((u >> 6) & 0x3F));\n\
                 \t\t\t\tb[2] = (char)(0x80 | (u & 0x3F));\n\t\t\t\tn = 3;\n\
                 \t\t\t} else {\n\
                 \t\t\t\tb[0] = (char)(0xF0 | (u >> 18));\n\t\t\t\tb[1] = (char)(0x80 | ((u >> 12) & 0x3F));\n\
                 \t\t\t\tb[2] = (char)(0x80 | ((u >> 6) & 0x3F));\n\t\t\t\tb[3] = (char)(0x80 | (u & 0x3F));\n\t\t\t\tn = 4;\n\
                 \t\t\t}\n\
                 \t\t\ttemp = realloc(widget->u.text.text, widget->u.text.len + n + 1);\n\
                 \t\t\tif (temp == NULL)\n\t\t\t\tbreak;\n\
                 \t\t\twidget->u.text.text = temp;\n\
                 \t\t\tmemmove(widget->u.text.text + widget->u.text.idx + n,\n\
                 \t\t\t\twidget->u.text.text + widget->u.text.idx,\n\
                 \t\t\t\twidget->u.text.len - widget->u.text.idx);\n\
                 \t\t\tmemcpy(widget->u.text.text + widget->u.text.idx, b, n);\n\
                 \t\t\twidget->u.text.idx += n;\n\t\t\twidget->u.text.len += n;\n\t\t}\n",
            ),
        ],
    )
}

/// `content/handlers/html/box_textarea.c` als geprüfte Kopie (B7): nach
/// einem Tastendruck, der den Text eines Feldes ändert, „input“ ans Feld;
/// vor dem Abschicken mit der Eingabetaste „submit“ ans Formular, das ein
/// Skript abbrechen kann (c/formular.c).
fn box_textarea_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "content/handlers/html/box_textarea.c",
        "box_textarea.c",
        &[
            (
                "nserror box_textarea_keypress(html_content *html, struct box *box, uint32_t key)\n{\n",
                "/* ESP32-OS (B7): Formular-Ereignisse für Skripte (c/formular.c) */\n\
                 bool ns_formular_absenden_erlaubt(struct form *form);\n\
                 void ns_feld_eingabe_melden(struct form_control *g, const char *vorher);\n\
                 void ns_feld_vorgabe_sichern(dom_node *n);\n\
                 static nserror ns_tastendruck(html_content *html, struct box *box, uint32_t key);\n\
                 \n\
                 nserror box_textarea_keypress(html_content *html, struct box *box, uint32_t key)\n{\n\
                 \tstruct form_control *g = box->gadget;\n\
                 \tchar *vorher = g->value != NULL ? strdup(g->value) : NULL;\n\
                 \tnserror res;\n\
                 \tif (g->type == GADGET_TEXTBOX || g->type == GADGET_PASSWORD)\n\
                 \t\tns_feld_vorgabe_sichern(g->node);\n\
                 \tres = ns_tastendruck(html, box, key);\n\
                 \t/* Eingabetaste im einzeiligen Feld ändert den Text nicht;\n\
                 \t * was ein submit-Hörer setzt, ist keine Eingabe */\n\
                 \tif (g->type == GADGET_TEXTAREA || (key != NS_KEY_NL && key != NS_KEY_CR))\n\
                 \t\tns_feld_eingabe_melden(g, vorher);\n\
                 \tfree(vorher);\n\
                 \treturn res;\n\
                 }\n\
                 \n\
                 static nserror ns_tastendruck(html_content *html, struct box *box, uint32_t key)\n{\n",
            ),
            (
                "\t\tif (form) {\n\t\t\tres = form_submit(content_get_url(c),\n",
                "\t\tif (form && ns_formular_absenden_erlaubt(form)) {\n\t\t\tres = form_submit(content_get_url(c),\n",
            ),
        ],
    )
}

/// `content/handlers/html/form.c` als geprüfte Kopie (B7): Auswahl in der
/// Liste eines select per Tippen → „input“ und „change“ (c/formular.c).
fn form_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "content/handlers/html/form.c",
        "form.c",
        &[
            (
                "static void form_select_menu_clicked(struct form_control *control, int x, int y)\n{\n",
                "/* ESP32-OS (B7): Formular-Ereignisse für Skripte (c/formular.c) */\n\
                 void ns_bedienung_gewechselt(dom_node *n);\n\
                 \n\
                 static void form_select_menu_clicked(struct form_control *control, int x, int y)\n{\n",
            ),
            (
                "\tif (option != NULL) {\n\t\tform__select_process_selection(html, control, i);\n\t}\n",
                "\tif (option != NULL) {\n\
                 \t\t/* ESP32-OS (B7): Ereignisse nur bei geänderter Auswahl */\n\
                 \t\tbool gleich = !control->data.select.multiple && option->selected;\n\
                 \t\tform__select_process_selection(html, control, i);\n\
                 \t\tif (!gleich)\n\
                 \t\t\tns_bedienung_gewechselt(control->node);\n\
                 \t}\n",
            ),
        ],
    )
}

/// `content/handlers/html/interaction.c` als geprüfte Kopie (B7): bricht
/// ein Skript den Klick ab (preventDefault), folgt NetSurf weder Link noch
/// Absendeknopf und nimmt das Umschalten eines Kästchens zurück, sonst
/// „input“/„change“ an ein umgeschaltetes Kästchen oder Optionsfeld; vor
/// dem Abschicken über einen Knopf „submit“ ans Formular (c/formular.c).
fn interaction_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "content/handlers/html/interaction.c",
        "interaction.c",
        &[
            (
                "#include \"html/interaction.h\"\n",
                "#include \"html/interaction.h\"\n\n\
                 /* ESP32-OS (B7): Formular-Ereignisse für Skripte (c/formular.c) */\n\
                 bool ns_ereignis_frei(dom_string *typ, dom_node *ziel, bool sprudelt, bool abbrechbar);\n\
                 bool ns_formular_absenden_erlaubt(struct form *form);\n\
                 void ns_bedienung_gewechselt(dom_node *n);\n\
                 /* Kästchen/Optionsfeld beim letzten Klick umgeschaltet */\n\
                 static bool ns_umgeschaltet;\n",
            ),
            (
                "\t\tif (mouse & BROWSER_MOUSE_CLICK_1) {\n\t\t\tmas->gadget.control->selected = !mas->gadget.control->selected;\n",
                "\t\tif (mouse & BROWSER_MOUSE_CLICK_1) {\n\t\t\tns_umgeschaltet = true;\n\t\t\tmas->gadget.control->selected = !mas->gadget.control->selected;\n",
            ),
            (
                "\t\tif (mouse & BROWSER_MOUSE_CLICK_1) {\n\t\t\tform_radio_set(mas->gadget.control);\n",
                "\t\tif (mouse & BROWSER_MOUSE_CLICK_1) {\n\t\t\tns_umgeschaltet = !mas->gadget.control->selected;\n\t\t\tform_radio_set(mas->gadget.control);\n",
            ),
            (
                "\tif (mouse & BROWSER_MOUSE_CLICK_1) {\n\t\tfire_generic_dom_event(corestring_dom_click, mas.node, true, true);\n\t}\n",
                "\tif (mouse & BROWSER_MOUSE_CLICK_1) {\n\
                 \t\t/* ESP32-OS (B7): abgebrochener Klick – keine Folgeaktion */\n\
                 \t\tif (!ns_ereignis_frei(corestring_dom_click, mas.node, true, true)) {\n\
                 \t\t\tif (mas.gadget.control != NULL && mas.gadget.control->type == GADGET_CHECKBOX) {\n\
                 \t\t\t\tmas.gadget.control->selected = !mas.gadget.control->selected;\n\
                 \t\t\t\tdom_html_input_element_set_checked(\n\
                 \t\t\t\t\t(dom_html_input_element *)(mas.gadget.control->node),\n\
                 \t\t\t\t\tmas.gadget.control->selected);\n\
                 \t\t\t\thtml__redraw_a_box(html, mas.gadget.box);\n\
                 \t\t\t}\n\
                 \t\t\tmas.result.action = ACTION_NONE;\n\
                 \t\t} else if (ns_umgeschaltet && mas.gadget.control != NULL) {\n\
                 \t\t\tns_bedienung_gewechselt(mas.gadget.control->node);\n\
                 \t\t}\n\
                 \t\tns_umgeschaltet = false;\n\
                 \t}\n",
            ),
            (
                "\tcase ACTION_SUBMIT:\n\t\tres = form_submit(content_get_url(c),\n",
                "\tcase ACTION_SUBMIT:\n\
                 \t\t/* ESP32-OS (B7): „submit“ ans Formular, abbrechbar */\n\
                 \t\tif (!ns_formular_absenden_erlaubt(mas.gadget.control->form))\n\
                 \t\t\tbreak;\n\
                 \t\tres = form_submit(content_get_url(c),\n",
            ),
        ],
    )
}

/// `libdom/src/html/html_text_area_element.c` als geprüfte Kopie (B7):
/// value ist vom Inhalt getrennt wie im Standard (upstream überschrieb
/// `set_value` den Inhalt – der ist aber die Vorgabe, defaultValue/reset();
/// NetSurf liest den Wert nur über `get_value`), und `get_value` gibt den
/// vorigen Text frei, solange der Wert noch dem Inhalt folgt.
/// libdom `html_document.c` als geprüfte Kopie (BR2): Schlägt beim Anlegen
/// eines Dokuments schon `memoised` fehl (Heap voll), räumte der Fehlerweg
/// `doc->elements` auf, bevor es gesetzt war – das Dokument stammt aus
/// malloc, der Zeiger war Müll (am Board: Absturz in
/// `_dom_html_document_initialise`, Zeiger zeigte auf Text).
fn dom_dokument_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "../libdom/src/html/html_document.c",
        "dom_html_document.c",
        &[
            (
                "\tdoc->body = NULL;\n\n\tdoc->memoised = calloc(",
                "\tdoc->body = NULL;\n\t/* ESP32-OS (BR2): vor dem ersten goto out gesetzt */\n\tdoc->memoised = NULL;\n\tdoc->elements = NULL;\n\n\tdoc->memoised = calloc(",
            ),
            ("#include \"html_document_strings.h\"", "#include \"html/html_document_strings.h\""),
        ],
    )
}

fn dom_textarea_kopie(q: &Path, ziel: &Path) -> PathBuf {
    kopie_mit(
        q,
        ziel,
        "../libdom/src/html/html_text_area_element.c",
        "dom_html_text_area_element.c",
        &[
            (
                "\tif (ele->value_set == false) {\n\t\terr = dom_node_get_text_content((dom_node *)ele,\n\t\t\t\t\t\t&ele->value);\n\t\tif (err == DOM_NO_ERR) {\n\t\t\tele->default_value_set = true;\n",
                "\tif (ele->value_set == false) {\n\
                 \t\t/* ESP32-OS (B7): vorigen Text freigeben */\n\
                 \t\tdom_string *inhalt = NULL;\n\
                 \t\terr = dom_node_get_text_content((dom_node *)ele, &inhalt);\n\
                 \t\tif (err == DOM_NO_ERR) {\n\
                 \t\t\tif (ele->value != NULL)\n\
                 \t\t\t\tdom_string_unref(ele->value);\n\
                 \t\t\tele->value = inhalt;\n\
                 \t\t\tele->default_value_set = true;\n",
            ),
            (
                "\treturn dom_node_set_text_content((dom_node *)ele, ele->value);\n}\n",
                "\t/* ESP32-OS (B7): Inhalt (= Vorgabe) bleibt */\n\treturn DOM_NO_ERR;\n}\n",
            ),
        ],
    )
}

/// `content/urldb.c` als Kopie (B7, Cookie-Fix in urldb.c): ist die lokale
/// Datei `urldb_fix.local.txt` da (nicht im Repo; Zeilen: Originaltext,
/// Ersatz, erwartete Anzahl), wird sie angewandt und `cfg(ns_cookie_fix)`
/// gesetzt (zugehöriger Test lokal); sonst unverändert wie bei NetSurf.
fn urldb_kopie(q: &Path, ziel: &Path) -> PathBuf {
    let quelle = q.join("netsurf/content/urldb.c");
    println!("cargo:rerun-if-changed={}", quelle.display());
    println!("cargo:rustc-check-cfg=cfg(ns_cookie_fix)");
    let fix = Path::new(env!("CARGO_MANIFEST_DIR")).join("urldb_fix.local.txt");
    println!("cargo:rerun-if-changed={}", fix.display());
    let mut t = std::fs::read_to_string(&quelle).expect("urldb.c").replace("\r\n", "\n");
    if let Ok(f) = std::fs::read_to_string(&fix) {
        let z: Vec<&str> = f.lines().collect();
        assert!(z.len() >= 3, "urldb_fix.local.txt: drei Zeilen erwartet");
        let anzahl: usize = z[2].trim().parse().expect("urldb_fix.local.txt: Anzahl");
        assert_eq!(t.matches(z[0]).count(), anzahl, "urldb.c: Stellen für urldb_fix.local.txt nicht wie erwartet");
        t = t.replace(z[0], z[1]);
        println!("cargo:rustc-cfg=ns_cookie_fix");
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (urldb_kopie) aus content/urldb.c. */\n{t}");
    let datei = ziel.join("urldb.c");
    std::fs::create_dir_all(ziel).expect("Ordner");
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, t).expect("urldb.c");
    }
    datei
}

/// `content/llcache.c` als geprüfte Kopie (B3): am Ende jedes Aufräumens
/// (`llcache_clean`) Gesamtgrösse, Grenze und den freigebbaren Teil
/// (Objekte ohne Nutzer und ohne laufenden Abruf) festhalten – die Grenze
/// gilt für diesen Teil; was eine Seite gerade nutzt, bleibt.
fn llcache_kopie(q: &Path, ziel: &Path) -> PathBuf {
    let quelle = q.join("netsurf/content/llcache.c");
    println!("cargo:rerun-if-changed={}", quelle.display());
    let mut t = std::fs::read_to_string(&quelle).expect("llcache.c").replace("\r\n", "\n");
    for (alt, neu) in [
        (
            "\nvoid llcache_clean(bool purge)\n{\n",
            "\n/* ESP32-OS (B3): Messwerte des letzten Aufräumens (Board-Log\n \
             * „ns_takt“ bzw. Tests lesen sie über ns_llcache_stand) */\n\
             static uint32_t ns_cache_groesse, ns_cache_grenze, ns_cache_frei;\n\
             void ns_llcache_stand(uint32_t *groesse, uint32_t *grenze, uint32_t *frei)\n{\n\
             \t*groesse = ns_cache_groesse;\n\t*grenze = ns_cache_grenze;\n\t*frei = ns_cache_frei;\n}\n\n\
             /* ESP32-OS (B7): HTTP-Status für fetch/XHR (beim Abruf im Objekt\n \
             * festgehalten, siehe llcache_fetch_callback; 0 = unbekannt) */\n\
             long ns_llcache_http_code(const llcache_handle *h)\n{\n\
             \treturn (h != NULL && h->object != NULL) ? h->object->ns_http_code : 0;\n}\n\n\
             void llcache_clean(bool purge)\n{\n",
        ),
        (
            "\tNSLOG(llcache, DEBUG, \"Size: %\"PRIu32\" (limit: %\"PRIu32\")\", llcache_size, limit);\n}\n",
            "\tNSLOG(llcache, DEBUG, \"Size: %\"PRIu32\" (limit: %\"PRIu32\")\", llcache_size, limit);\n\
             \t/* ESP32-OS (B3): Messwerte festhalten */\n\
             \t{\n\
             \t\tuint32_t frei = 0;\n\
             \t\tfor (object = llcache->cached_objects; object != NULL; object = object->next)\n\
             \t\t\tif (object->users == NULL && object->candidate_count == 0 && object->fetch.fetch == NULL)\n\
             \t\t\t\tfrei += total_object_size(object);\n\
             \t\tfor (object = llcache->uncached_objects; object != NULL; object = object->next)\n\
             \t\t\tif (object->users == NULL && object->candidate_count == 0 && object->fetch.fetch == NULL)\n\
             \t\t\t\tfrei += total_object_size(object);\n\
             \t\tns_cache_groesse = llcache_size;\n\
             \t\tns_cache_grenze = limit;\n\
             \t\tns_cache_frei = frei;\n\
             \t}\n}\n",
        ),
        (
            "\tnsurl *url;\t\t     /**< Post-redirect URL for object */\n",
            "\tnsurl *url;\t\t     /**< Post-redirect URL for object */\n\
             \tlong ns_http_code;\t     /**< ESP32-OS (B7): HTTP-Status des Abrufs */\n",
        ),
        (
            "\tNSLOG(llcache, DEBUG, \"Fetch event %d for %p\", msg->type, object);\n",
            "\tNSLOG(llcache, DEBUG, \"Fetch event %d for %p\", msg->type, object);\n\
             \t/* ESP32-OS (B7): HTTP-Status festhalten, sobald bekannt (fetch/XHR) */\n\
             \tif (object->fetch.fetch != NULL && fetch_http_code(object->fetch.fetch) > 0)\n\
             \t\tobject->ns_http_code = fetch_http_code(object->fetch.fetch);\n",
        ),
    ] {
        assert_eq!(t.matches(alt).count(), 1, "llcache.c: Originaltext nicht (genau einmal) gefunden: {alt:?}");
        t = t.replacen(alt, neu, 1);
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (llcache_kopie) aus content/llcache.c. */\n{t}");
    let datei = ziel.join("llcache.c");
    std::fs::create_dir_all(ziel).expect("Ordner");
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, t).expect("llcache.c");
    }
    datei
}

/// `utils/config.h` fürs Board: NetSurf wählt Funktionen nach der
/// Plattform, ESP-IDF kennt es nicht. Die Kopie (vor dem Upstream-Kopf im
/// Suchpfad) nimmt `ESP_PLATFORM` dort auf, wo NetSurf für Plattformen ohne
/// mmap bzw. ohne dirfd/fstatat/unlinkat schon eigene Wege hat (Datei
/// lesen statt abbilden, Pfade statt Verzeichnis-Deskriptoren). Jede
/// Ersetzung prüft den Originaltext; die Upstream-Quellen bleiben
/// unverändert.
fn board_config(q: &Path, ziel: &Path) {
    let quelle = q.join("netsurf/utils/config.h");
    println!("cargo:rerun-if-changed={}", quelle.display());
    let mut t = std::fs::read_to_string(&quelle).expect("utils/config.h").replace("\r\n", "\n");
    for (alt, neu) in [
        ("#define HAVE_MMAP\n#if (defined(_WIN32) ||", "#define HAVE_MMAP\n#if (defined(ESP_PLATFORM) || defined(_WIN32) ||"),
        (
            "#define HAVE_FSTATAT\n#if (defined(_WIN32) ||",
            "#define HAVE_FSTATAT\n#if (defined(ESP_PLATFORM) || defined(_WIN32) ||",
        ),
        // keine Signale unter ESP-IDF (lwIP meldet geschlossene Sockets als
        // Fehler, kein SIGPIPE)
        ("#define HAVE_SIGPIPE\n#if (defined(_WIN32))", "#define HAVE_SIGPIPE\n#if (defined(ESP_PLATFORM) || defined(_WIN32))"),
    ] {
        assert_eq!(t.matches(alt).count(), 1, "utils/config.h: Originaltext nicht (genau einmal) gefunden: {alt:?}");
        t = t.replacen(alt, neu, 1);
    }
    let t = format!("/* Erzeugt von esp32/netsurf/build.rs (board_config) aus utils/config.h. */\n{t}");
    std::fs::create_dir_all(ziel.parent().expect("Ordner")).expect("Ordner");
    if std::fs::read_to_string(ziel).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(ziel, t).expect("config.h");
    }
}

/// CMake-Liste der Gruppen fürs Board: `<Quellen>/erzeugt/netsurf_quellen.cmake`
/// (Variablen NS_QUELLEN, NS_GRUPPE_<name>; Pfade und Defines je Datei).
fn board_liste(q: &Path) {
    let ziel = q.join("erzeugt/board");
    let hb = ziel.join("include/dom/bindings/hubbub");
    std::fs::create_dir_all(&hb).expect("Ordner");
    for h in ["parser.h", "errors.h"] {
        std::fs::copy(q.join("libdom/bindings/hubbub").join(h), hb.join(h)).expect("Kopie");
    }
    vorspann_erzeugen(&ziel.join("vorspann.h"));
    board_config(q, &ziel.join("include/utils/config.h"));
    let hier = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let pfad = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut t = String::from(
        "# Erzeugt von esp32/netsurf/build.rs – nicht von Hand ändern.\n\
         # Gleiche Quellgruppen wie die PC-Tests; Board: compat/gemeinsam,\n\
         # ns_programm.h vorangestellt (Heap des Programms).\n",
    );
    t.push_str(&format!("set(NS_COMPAT \"{}\")\nset(NS_QUELLEN \"\")\n", pfad(&hier.join("compat/gemeinsam"))));
    for g in gruppen(q, &ziel, true) {
        let n = g.name.to_uppercase();
        t.push_str(&format!("set(NS_GRUPPE_{n}\n"));
        for f in &g.dateien {
            t.push_str(&format!("    \"{}\"\n", pfad(f)));
        }
        t.push_str(")\n");
        // compat/board: was newlib fehlt (sys/utsname.h); dann gemeinsame
        let pfade: Vec<String> = [pfad(&hier.join("compat/board")), pfad(&hier.join("compat/gemeinsam"))]
            .into_iter()
            .chain(g.pfade.iter().map(|p| pfad(p)))
            .collect();
        let defines: Vec<String> = g
            .defines
            .iter()
            .map(|(n, w)| match w {
                Some(w) => format!("{n}={}", w.replace('\\', "\\\\").replace('"', "\\\"")),
                None => n.clone(),
            })
            .collect();
        let optionen = match g.modus {
            Modus::Voll => "-include;ns_programm.h",
            Modus::Abschnitt | Modus::Ohne => "",
        };
        t.push_str(&format!(
            "set_source_files_properties(${{NS_GRUPPE_{n}}} PROPERTIES\n    INCLUDE_DIRECTORIES \"{}\"\n    COMPILE_DEFINITIONS \"{}\"\n    COMPILE_OPTIONS \"{}\")\nlist(APPEND NS_QUELLEN ${{NS_GRUPPE_{n}}})\n",
            pfade.join(";"),
            defines.join(";"),
            optionen
        ));
    }
    let datei = q.join("erzeugt/netsurf_quellen.cmake");
    // nur schreiben, wenn sich etwas ändert (sonst baute ESP-IDF jedes Mal neu)
    if std::fs::read_to_string(&datei).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(&datei, &t).expect("netsurf_quellen.cmake");
    }
    // Stand der erzeugten Dateien in den Komponentenordner: cargo beobachtet
    // für den Board-Bau nur die Komponentenordner, nicht C:\ns – ohne diese
    // Datei bliebe eine geänderte Liste oder ein geänderter Kopf unbemerkt
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mischen = |b: &[u8]| {
        for &x in b {
            h = (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3);
        }
    };
    mischen(t.as_bytes());
    for f in ["vorspann.h", "include/utils/config.h", "include/dom/bindings/hubbub/parser.h", "include/dom/bindings/hubbub/errors.h"] {
        mischen(&std::fs::read(ziel.join(f)).expect("erzeugte Datei"));
    }
    // auch der Inhalt aller Quellen (eigene in netsurf/c, erzeugte Bilder …)
    // und der eigenen Köpfe
    for g in gruppen(q, &ziel, true) {
        for f in &g.dateien {
            mischen(&std::fs::read(f).expect("Quelle"));
        }
    }
    // c/programm_daten.c baut die Komponente direkt (ausserhalb der Gruppen)
    mischen(&std::fs::read(hier.join("c/programm_daten.c")).expect("programm_daten.c"));
    for d in ["compat/gemeinsam", "compat/board", "compat/board/sys", "compat/gemeinsam/curl"] {
        let mut k: Vec<PathBuf> = std::fs::read_dir(hier.join(d)).expect("compat").flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        k.sort();
        for f in k {
            mischen(&std::fs::read(f).expect("Kopf"));
        }
    }
    let stand = hier.join("../firmware/komponenten/netsurf/quellen_stand.txt");
    let neu = format!("{h:016x}\n");
    if std::fs::read_to_string(&stand).ok().as_deref() != Some(neu.as_str()) {
        std::fs::write(&stand, neu).expect("quellen_stand.txt");
    }
}

/// Quellen des NetSurf-Kerns (Ordner, Dateien) – aus content/Makefile,
/// content/fetchers/…, content/handlers/…, utils/…, desktop/Makefile.
const KERN: &[(&str, &str)] = &[
    ("content", "content.c content_factory.c hlcache.c mimesniff.c textsearch.c no_backing_store.c"),
    ("content/fetchers", "data.c resource.c"),
    (
        "content/fetchers/about",
        "about.c blank.c certificate.c chart.c choices.c config.c imagecache.c nscolours.c query.c query_auth.c query_fetcherror.c \
         query_privacy.c query_timeout.c testament.c",
    ),
    ("content/fetchers/file", "dirlist.c file.c"),
    ("content/handlers/css", "css.c dump.c internal.c hints.c select.c"),
    // JavaScript: QuickJS-ng statt Duktape (c/js_quickjs.c, STATUS
    // „Browser-Architektur“); content.c = Inhaltstyp für externe Skripte
    ("content/handlers/javascript", "fetcher.c content.c"),
    (
        "content/handlers/html",
        "box_construct.c box_inspect.c box_manipulate.c box_normalise.c box_special.c css.c css_fetcher.c \
         dom_event.c font.c forms.c imagemap.c layout.c layout_flex.c object.c redraw.c \
         redraw_border.c script.c table.c textselection.c",
    ),
    ("content/handlers/text", "textplain.c"),
    ("content/handlers/image", "image.c image_cache.c bmp.c gif.c ico.c"),
    (
        "utils",
        "bloom.c corestrings.c file.c filename.c filepath.c hashmap.c hashtable.c idna.c libdom.c log.c messages.c nscolour.c \
         nsoption.c punycode.c ssl_certs.c talloc.c time.c url.c useragent.c utf8.c utils.c",
    ),
    (
        "utils/http",
        "challenge.c generics.c primitives.c parameter.c cache-control.c content-disposition.c content-type.c \
         strict-transport-security.c www-authenticate.c",
    ),
    ("utils/nsurl", "nsurl.c parse.c"),
    (
        "desktop",
        "cookie_manager.c knockout.c hotlist.c mouse.c plot_style.c print.c search.c searchweb.c scrollbar.c textarea.c \
         version.c system_colour.c local_history.c global_history.c treeview.c page-info.c \
         bitmap.c browser.c browser_window.c browser_history.c download.c frames.c netsurf.c cw_helper.c \
         save_text.c selection.c textinput.c gui_factory.c save_pdf.c font_haru.c",
        // save_complete.c fehlt absichtlich: braucht POSIX-Regex (weder MSVC
        // noch newlib), und das Framebuffer-Frontend bietet „komplett
        // speichern“ nicht an (PLAN, Abweichung „B1 komplett speichern“)
    ),
];

/// `js/vorspann.js` als nullterminiertes C-Feld (QuickJS verlangt das
/// Nullbyte; `sizeof - 1` ist die Länge).
fn vorspann_erzeugen(ziel: &Path) {
    let js = std::fs::read("js/vorspann.js").expect("js/vorspann.js");
    let mut t = String::from("/* erzeugt von build.rs aus js/vorspann.js */\nstatic const char ns_vorspann[] = {\n");
    for z in js.chunks(24) {
        for b in z {
            t.push_str(&format!("{b},"));
        }
        t.push('\n');
    }
    t.push_str("0};\n");
    if std::fs::read_to_string(ziel).ok().as_deref() != Some(t.as_str()) {
        std::fs::write(ziel, t).expect("vorspann.h");
    }
}
