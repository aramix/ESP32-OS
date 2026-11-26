"""NetSurf-Quellen für den Browser (B1) holen und vorbereiten.

    python esp32/netsurf/werkzeuge/quellen_holen.py

* Lädt `netsurf-all-3.11.tar.gz` (NetSurf und alle eigenen Bibliotheken,
  offizielle Quelle download.netsurf-browser.org) und prüft die SHA-256 –
  stimmt sie nicht, wird nichts entpackt.
* Entpackt ohne Testdaten (Dateinamen mit „:“ gehen unter Windows nicht) nach
  `C:\\ns\\3.11` (kurzer Pfad wie `C:\\ec` für esp_hosted).
* Lädt zlib 1.4 (NetSurf braucht sie für gepackte Messages/Hash-Tabellen;
  offizielle Quelle zlib.net, SHA-256 wie dort veröffentlicht) nach
  `C:\\ns\\zlib-1.4`.
* Erzeugt, was die Upstream-Makefiles sonst erzeugen (es gibt hier kein
  make): Zeichensatz-Aliase (libparserutils, perl), HTML-Entitäten
  (libhubbub, perl), CSS-Eigenschafts-Parser (libcss: Generator für den PC
  übersetzen und je Eigenschaft ausführen).
* Merker `C:\\ns\\3.11\\.fertig` mit SHA und Regelstand: Zweiter Aufruf tut
  nichts; ein neuer Regelstand erzeugt nur neu (die Generatoren
  überschreiben). Gelöscht wird nie etwas.

Das entpackte Archiv sind fremde Daten: Ausgeführt werden nur die beiden
perl-Skripte und der C-Generator aus dem geprüften Archiv, sonst nichts.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request

URL = "https://download.netsurf-browser.org/netsurf/releases/source-full/netsurf-all-3.11.tar.gz"
SHA = "4dea880ff3c2f698bfd62c982b259340f9abcd7f67e6c8eb2b32c61f71644b7b"
# Regelstand: erhöhen, wenn sich die Vorbereitung ändert
REGELN = "7"
# JavaScript (Auftrag 13.9): QuickJS-ng als Amalgam, SHA-256 wie von GitHub
# zum Release veröffentlicht
QJS_URL = "https://github.com/quickjs-ng/quickjs/releases/download/v0.18.0/quickjs-amalgam.zip"
QJS_SHA = "6c05066a6b62b483e2ba0f83d7a16688c6958182b214bfd693529434cef84a9e"
ZLIB_URL = "https://zlib.net/fossils/zlib-1.4.tar.gz"
# veröffentlicht auf https://zlib.net (zlib 1.4, tar.gz)
ZLIB_SHA = "02dd159783677a749c04adbdd02b55b8a2bcc7e054f51b76b549199a916ad584"
# Windows: C:\\ns; elsewhere in the home directory (then set NETSURF_QUELLEN to
# <root>/3.11 when building).
WURZEL = r"C:\ns" if os.name == "nt" else os.path.expanduser("~/.cache/esp32os/ns")
ZIEL = os.path.join(WURZEL, "3.11")
ARCHIV = os.path.join(WURZEL, "dl", "netsurf-all-3.11.tar.gz")
ZLIB_ARCHIV = os.path.join(WURZEL, "dl", "zlib-1.4.tar.gz")
ZLIB_ZIEL = os.path.join(WURZEL, "zlib-1.4")
QJS_ARCHIV = os.path.join(WURZEL, "dl", "quickjs-amalgam-0.18.0.zip")
QJS_ZIEL = os.path.join(WURZEL, "quickjs-0.18.0")
MERKER = os.path.join(ZIEL, ".fertig")
PERL = shutil.which("perl") or r"C:\Program Files\Git\usr\bin\perl.exe"
CLANG = shutil.which("clang") or r"C:\Program Files\LLVM\bin\clang.exe"


def sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def laden(url=URL, sha=SHA, archiv=ARCHIV):
    os.makedirs(os.path.dirname(archiv), exist_ok=True)
    if not os.path.exists(archiv) or sha256(archiv) != sha:
        print("lade", url)
        tmp = archiv + ".teil"
        with urllib.request.urlopen(url, timeout=120) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f)
        if sha256(tmp) != sha:
            sys.exit(f"SHA-256 von {tmp} stimmt nicht – abgebrochen, nichts entpackt")
        os.replace(tmp, archiv)


def qjs_entpacken():
    """QuickJS-ng-Amalgam (wenige Dateien) entpacken; nur, wenn noch nicht da."""
    import zipfile
    if os.path.exists(os.path.join(QJS_ZIEL, "quickjs.h")) or os.path.exists(os.path.join(QJS_ZIEL, "quickjs-amalgam.c")):
        return
    os.makedirs(QJS_ZIEL, exist_ok=True)
    with zipfile.ZipFile(QJS_ARCHIV) as z:
        for n in z.namelist():
            ziel = os.path.normpath(os.path.join(QJS_ZIEL, n))
            if not ziel.startswith(os.path.normpath(QJS_ZIEL) + os.sep) or n.endswith("/"):
                continue
            os.makedirs(os.path.dirname(ziel), exist_ok=True)
            with z.open(n) as q, open(ziel, "wb") as f:
                shutil.copyfileobj(q, f)
    print("QuickJS-ng 0.18.0: entpackt", sorted(os.listdir(QJS_ZIEL)))


def zlib_entpacken():
    """zlib ohne Tests/Beispiele/Fremdes; nur, wenn noch nicht da."""
    if os.path.exists(os.path.join(ZLIB_ZIEL, "zlib.h")):
        return
    tmp = ZLIB_ZIEL + ".teil"
    if os.path.exists(tmp):
        sys.exit(f"{tmp} ist von einem abgebrochenen Lauf übrig – bitte von Hand löschen")
    os.makedirs(tmp)
    with tarfile.open(ZLIB_ARCHIV) as t:
        for m in t.getmembers():
            teile = m.name.split("/")
            if len(teile) > 1 and teile[1] in ("test", "examples", "contrib", "doc", "os400", "qnx", "watcom", "amiga", "msdos", "nintendods", "old", "win32"):
                continue
            if any(c in m.name for c in ':*?"<>|'):
                continue
            t.extract(m, tmp, filter="data")
    os.replace(os.path.join(tmp, "zlib-1.4"), ZLIB_ZIEL)
    os.rmdir(tmp)
    print("zlib 1.4: entpackt", ZLIB_ZIEL)


def entpacken():
    """Nur, wenn es den Ordner noch nicht gibt – das Skript löscht nichts
    (ein angefangener Ordner `.teil` muss dann von Hand weg)."""
    if os.path.exists(os.path.join(ZIEL, "netsurf")):
        return
    tmp = ZIEL + ".teil"
    if os.path.exists(tmp):
        sys.exit(f"{tmp} ist von einem abgebrochenen Lauf übrig – bitte von Hand löschen")
    os.makedirs(tmp)
    n = 0
    with tarfile.open(ARCHIV) as t:
        for m in t.getmembers():
            teile = m.name.split("/")
            if "test" in teile[2:3] or any(c in m.name for c in ':*?"<>|'):
                continue
            t.extract(m, tmp, filter="data")
            n += 1
    os.replace(os.path.join(tmp, "netsurf-all-3.11"), ZIEL)
    os.rmdir(tmp)
    print("entpackt:", n, "Dateien")


def erzeugen():
    # libparserutils: src/charset/aliases.inc
    subprocess.run([PERL, "build/make-aliases.pl"], cwd=os.path.join(ZIEL, "libparserutils"), check=True)
    # libhubbub: src/tokeniser/entities.inc
    subprocess.run([PERL, "build/make-entities.pl"], cwd=os.path.join(ZIEL, "libhubbub"), check=True)
    # libcss: Eigenschafts-Parser
    pdir = os.path.join(ZIEL, "libcss", "src", "parse", "properties")
    gen = os.path.join(WURZEL, "gen_parser.exe")
    subprocess.run([CLANG, "-O1", "-w", "-o", gen, os.path.join(pdir, "css_property_parser_gen.c")], check=True)
    n = 0
    with open(os.path.join(pdir, "properties.gen"), encoding="utf-8") as f:
        for zeile in f:
            zeile = zeile.rstrip("\n")
            if not zeile or zeile.startswith("#") or ":" not in zeile:
                continue
            name = zeile.split(":", 1)[0]
            subprocess.run([gen, "-o", os.path.join(pdir, f"autogenerated_{name}.c"), zeile], check=True)
            n += 1
    print("libcss: Parser erzeugt:", n)
    element_typen()
    links_aufloesen()
    fb_bilder()
    fb_schrift()
    fb_messages()


def fb_messages():
    """Messages (Texte der Oberfläche) je Sprache aus resources/FatMessages,
    gefiltert für das Framebuffer-Frontend – wie Makefile.macros
    split_messages, mit der perl-Fassung tools/split-messages.pl; ungepackt
    (NetSurf liest beides)."""
    for sprache in ("en", "de"):
        ziel = os.path.join(ERZEUGT, "res", sprache)
        os.makedirs(ziel, exist_ok=True)
        aus = os.path.join(ziel, "Messages")
        # split-messages.pl legt die Ausgabe exklusiv an (O_EXCL): eine
        # frühere Fassung beiseitelegen statt löschen
        if os.path.exists(aus):
            os.replace(aus, aus + ".vorher")
        subprocess.run([PERL, "tools/split-messages.pl", "-l", sprache, "-p", "fb", "-f", "messages",
                        "-o", aus, "resources/FatMessages"], cwd=os.path.join(ZIEL, "netsurf"), check=True)
    print("Framebuffer: Messages erzeugt (en, de)")


def links_aufloesen():
    """Symbolische Links des Archivs: tarfile legt unter Windows für
    Datei-Links Kopien an, Ordner-Links (z. B. framebuffer/res/throbber →
    gtk/res/throbber) fehlen. Hier als Kopie nachholen (nur, was fehlt und
    dessen Ziel existiert; Links auf erst beim Bauen erzeugte Dateien wie
    Messages bleiben offen)."""
    n = 0
    with tarfile.open(ARCHIV) as t:
        for m in t.getmembers():
            if not m.issym():
                continue
            rel = m.name.split("/", 1)[1]
            pfad = os.path.join(ZIEL, *rel.split("/"))
            ziel = os.path.normpath(os.path.join(os.path.dirname(pfad), *m.linkname.split("/")))
            if os.path.exists(pfad) or not os.path.exists(ziel):
                continue
            if not os.path.realpath(ziel).startswith(os.path.realpath(ZIEL)):
                continue
            if os.path.isdir(ziel):
                shutil.copytree(ziel, pfad)
            else:
                shutil.copy2(ziel, pfad)
            n += 1
    print("Links als Kopie ergänzt:", n)


# Framebuffer-Frontend: Ausgabeordner für erzeugte Quellen
ERZEUGT = os.path.join(ZIEL, "erzeugt")
HIER = os.path.dirname(os.path.abspath(__file__))

# frontends/framebuffer/Makefile: FB_IMAGE_<name> := <Datei>
FB_BILDER = {
    "left_arrow": "icons/back.png", "right_arrow": "icons/forward.png", "reload": "icons/reload.png",
    "stop_image": "icons/stop.png", "history_image": "icons/history.png", "left_arrow_g": "icons/back_g.png",
    "right_arrow_g": "icons/forward_g.png", "reload_g": "icons/reload_g.png", "stop_image_g": "icons/stop_g.png",
    "history_image_g": "icons/history_g.png", "scrolll": "icons/scrolll.png", "scrollr": "icons/scrollr.png",
    "scrollu": "icons/scrollu.png", "scrolld": "icons/scrolld.png", "osk_image": "icons/osk.png",
    "pointer_image": "pointers/default.png", "hand_image": "pointers/point.png", "caret_image": "pointers/caret.png",
    "menu_image": "pointers/menu.png", "progress_image": "pointers/progress.png", "move_image": "pointers/move.png",
    "throbber0": "throbber/throbber0.png", "throbber1": "throbber/throbber1.png", "throbber2": "throbber/throbber2.png",
    "throbber3": "throbber/throbber3.png", "throbber4": "throbber/throbber4.png", "throbber5": "throbber/throbber5.png",
    "throbber6": "throbber/throbber6.png", "throbber7": "throbber/throbber7.png", "throbber8": "throbber/throbber8.png",
}


def fb_bilder():
    """Wie tools/convert_image.c (upstream mit libpng): PNG → RGBA-Bytes als
    C-Feld + `struct fbtk_bitmap`. Zeiger (Markierung in erster Zeile und
    Spalte: genau ein grünes Pixel je Richtung) ohne Markierungszeile/-spalte,
    Hotspot daraus. Gamma: Bildschirm 2,2 gegen Datei 1/2,2 = unverändert –
    die Symbole haben kein abweichendes gAMA (geprüft)."""
    from PIL import Image

    os.makedirs(ERZEUGT, exist_ok=True)
    res = os.path.join(ZIEL, "netsurf", "frontends", "framebuffer", "res")
    for name, datei in FB_BILDER.items():
        pfad = os.path.join(res, datei)
        im = Image.open(pfad)
        g = im.info.get("gamma")
        if g is not None and abs(g - 0.45455) > 0.001:
            sys.exit(f"{datei}: Gamma {g} – Umrechnung fehlt")
        im = im.convert("RGBA")
        w, h = im.size
        px = im.load()
        # detect_hotspot() aus convert_image.c
        cursor, gruen, hx, hy = True, 0, 0, 0
        for i in range(w):
            r, gg, b, a = px[i, 0]
            if a == 255:
                if gg == 255:
                    gruen += 1
                    hx = i
                if b != 0 or r != 0:
                    cursor = False
                    break
            elif a != 0:
                cursor = False
                break
        if cursor and gruen != 1:
            cursor = False
        if cursor:
            for i in range(h):
                r, gg, b, a = px[0, i]
                if a == 255:
                    if gg == 255:
                        gruen += 1
                        hy = i
                    if b != 0 or r != 0:
                        cursor = False
                        break
                elif a != 0:
                    cursor = False
                    break
            if cursor and gruen != 2:
                cursor = False
        ow, oh = (w - 1, h - 1) if cursor else (w, h)
        dx = 1 if cursor else 0
        z = [
            f"/* This file is auto-generated from {datei}",
            " *",
            " * Do not edit this file directly.",
            " * (ESP32-OS: erzeugt von esp32/netsurf/werkzeuge/quellen_holen.py",
            " * wie tools/convert_image.c) */",
            "",
            "#include <sys/types.h>",
            "",
            "#include <stdint.h>",
            "",
            "#include <stdbool.h>",
            "",
            "#include <libnsfb.h>",
            "",
            '#include "netsurf/plot_style.h"',
            '#include "framebuffer/gui.h"',
            '#include "framebuffer/fbtk.h"',
            "",
            # const (Abweichung zu tools/convert_image.c): die Bilder werden nie
            # beschrieben und lägen sonst am Board im knappen internen RAM
            f"static const uint8_t {name}_pixdata[] = {{",
        ]
        for y in range(oh):
            werte = []
            for x in range(ow):
                werte.extend(px[x + dx, y + dx])
            z.append("\t" + "".join(f"0x{v:02x}, " for v in werte))
        z += [
            "};",
            "",
            f"struct fbtk_bitmap {name} = {{",
            f"\t.width\t\t= {ow},",
            f"\t.height\t\t= {oh},",
            f"\t.hot_x\t\t= {hx - 1 if cursor else 0},",
            f"\t.hot_y\t\t= {hy - 1 if cursor else 0},",
            f"\t.pixdata\t= {name}_pixdata,",
            "};",
            "",
        ]
        with open(os.path.join(ERZEUGT, f"image-{name}.c"), "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(z))
    print("Framebuffer: Bilder erzeugt:", len(FB_BILDER))


def fb_schrift():
    """Interne Schrift (frontends/framebuffer/Makefile, FB_FONT_internal_
    ns-sans): upstream tools/convert_font.c, für den PC übersetzt (getopt-
    Ersatz c/getopt.c, POSIX-Header aus compat und compat/gemeinsam)."""
    os.makedirs(ERZEUGT, exist_ok=True)
    werkzeug = os.path.join(WURZEL, "convert_font.exe")
    befehl = [CLANG, "-O1", "-w", "-o", werkzeug, os.path.join(ZIEL, "netsurf", "tools", "convert_font.c")]
    if os.name == "nt":
        # POSIX shims for Windows only; other systems have all of it
        befehl += ["-D_CRT_SECURE_NO_WARNINGS", "-I", os.path.join(HIER, "..", "compat"),
                   "-I", os.path.join(HIER, "..", "compat", "gemeinsam"), os.path.join(HIER, "..", "c", "getopt.c")]
    subprocess.run(befehl, check=True)
    glyphen = os.path.join(ZIEL, "netsurf", "frontends", "framebuffer", "res", "fonts", "glyph_data")
    subprocess.run([werkzeug, "-q", "-H", os.path.join(ERZEUGT, "font-ns-sans.h"), glyphen,
                    os.path.join(ERZEUGT, "font-ns-sans.c")], check=True)
    print("Framebuffer: Schrift erzeugt")


def element_typen():
    """libhubbub: `autogenerated-element-type.c` – upstream aus
    `element-type.gperf` mit gperf (fehlt hier). Gleiche Schnittstelle und
    Semantik (`%ignore-case`, exakte Länge, globale Tabelle `wordlist`), statt
    perfektem Hash Binärsuche über die sortierten Namen."""
    tdir = os.path.join(ZIEL, "libhubbub", "src", "treebuilder")
    text = open(os.path.join(tdir, "element-type.gperf"), encoding="utf-8").read()
    teile = text.split("%%")
    eintraege = []
    for zeile in teile[1].splitlines():
        zeile = zeile.strip()
        if not zeile or zeile.startswith("#"):
            continue
        name, typ = [x.strip() for x in zeile.split(",", 1)]
        eintraege.append((name.lower(), typ))
    eintraege.sort()
    namen = [n for n, _ in eintraege]
    assert len(set(namen)) == len(namen), "doppelte Elementnamen"
    z = [
        "/* Erzeugt von esp32/netsurf/werkzeuge/quellen_holen.py aus",
        " * element-type.gperf (Ersatz für gperf, gleiche Schnittstelle). */",
        "#include <string.h>",
        "#include \"treebuilder/element-type.h\"",
        "",
        "static const struct element_type_map wordlist[] = {",
    ]
    z += [f'\t{{"{n}", {t}}},' for n, t in eintraege]
    z += [
        "};",
        "",
        "static int esp32os_vergleich(const char *a, size_t la, const char *b)",
        "{",
        "\tsize_t lb = strlen(b);",
        "\tsize_t n = la < lb ? la : lb;",
        "\tfor (size_t i = 0; i < n; i++) {",
        "\t\tunsigned char x = (unsigned char)a[i], y = (unsigned char)b[i];",
        "\t\tif (x >= 'A' && x <= 'Z')",
        "\t\t\tx = (unsigned char)(x - 'A' + 'a');",
        "\t\tif (x != y)",
        "\t\t\treturn x < y ? -1 : 1;",
        "\t}",
        "\treturn la < lb ? -1 : la > lb ? 1 : 0;",
        "}",
        "",
        "static const struct element_type_map *",
        "hubbub_element_type_generated_lookup(const char *str, size_t len)",
        "{",
        "\tsize_t lo = 0, hi = sizeof(wordlist) / sizeof(wordlist[0]);",
        "\twhile (lo < hi) {",
        "\t\tsize_t m = lo + (hi - lo) / 2;",
        "\t\tint c = esp32os_vergleich(str, len, wordlist[m].name);",
        "\t\tif (c == 0)",
        "\t\t\treturn &wordlist[m];",
        "\t\tif (c < 0)",
        "\t\t\thi = m;",
        "\t\telse",
        "\t\t\tlo = m + 1;",
        "\t}",
        "\treturn NULL;",
        "}",
        "",
    ]
    with open(os.path.join(tdir, "autogenerated-element-type.c"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(z))
    print("libhubbub: Elementtypen:", len(eintraege))


def main():
    stand = f"{SHA}:{ZLIB_SHA}:{QJS_SHA}:{REGELN}"
    if os.path.exists(MERKER) and open(MERKER, encoding="utf-8").read() == stand:
        print("NetSurf 3.11: vorhanden", ZIEL)
        return
    laden()
    laden(ZLIB_URL, ZLIB_SHA, ZLIB_ARCHIV)
    laden(QJS_URL, QJS_SHA, QJS_ARCHIV)
    entpacken()
    zlib_entpacken()
    qjs_entpacken()
    erzeugen()
    with open(MERKER, "w", encoding="utf-8") as f:
        f.write(stand)
    print("NetSurf 3.11: bereit", ZIEL)


if __name__ == "__main__":
    main()
