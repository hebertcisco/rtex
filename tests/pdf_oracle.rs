//! PDF comparison against pdfTeX: each document is typeset by `pdftex` (plain format) and by
//! rtex, then compared word by word with `pdftotext -bbox` and pixel by pixel with `pdftoppm`.
//!
//! Needs TeX Live and poppler-utils; set `RTEX_SKIP_GOLDEN=1` to skip.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rtex::{CompileOptions, Paper, compile};

const POSITION_TOLERANCE_BP: f64 = 0.05;
const CENTER_TOLERANCE_BP: f64 = 1.0;
const PIXEL_DPI: &str = "100";
const PIXEL_THRESHOLD: u8 = 64;
const MAX_DIFFERING_PIXELS: f64 = 0.0005;

#[derive(Debug, Clone, PartialEq)]
struct Word {
    page: usize,
    text: String,
    bbox: [f64; 4],
}

fn tools_available() -> bool {
    std::env::var_os("RTEX_SKIP_GOLDEN").is_none()
        && ["pdftex", "pdftotext", "pdftoppm"].iter().all(|t| {
            Command::new(t)
                .arg("-v")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        })
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pdf-oracle-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("ref")).unwrap();
    fs::create_dir_all(dir.join("ours")).unwrap();
    dir
}

fn attr(tag: &str, name: &str) -> f64 {
    let key = format!("{name}=\"");
    let at = tag.find(&key).map(|i| i + key.len()).unwrap_or(0);
    tag[at..]
        .split('"')
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::NAN)
}

fn words(pdf: &Path) -> Vec<Word> {
    let out = Command::new("pdftotext")
        .args(["-bbox"])
        .arg(pdf)
        .arg("-")
        .output()
        .unwrap();
    let html = String::from_utf8_lossy(&out.stdout);
    let mut page = 0;
    let mut v = Vec::new();
    for line in html.lines() {
        let line = line.trim();
        if line.starts_with("<page ") {
            page += 1;
        } else if let Some(rest) = line.strip_prefix("<word ") {
            let (tag, text) = rest.split_once('>').unwrap();
            let text = text.trim_end_matches("</word>").to_owned();
            v.push(Word {
                page,
                text,
                bbox: [
                    attr(tag, "xMin"),
                    attr(tag, "yMin"),
                    attr(tag, "xMax"),
                    attr(tag, "yMax"),
                ],
            });
        }
    }
    v
}

fn pgm(pdf: &Path, prefix: &Path) -> Vec<(usize, usize, Vec<u8>)> {
    Command::new("pdftoppm")
        .args(["-r", PIXEL_DPI, "-gray"])
        .arg(pdf)
        .arg(prefix)
        .status()
        .unwrap();
    let dir = prefix.parent().unwrap();
    let stem = prefix.file_name().unwrap().to_string_lossy().into_owned();
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "pgm") && p.file_name().unwrap().to_string_lossy().starts_with(&stem)
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| {
            let data = fs::read(f).unwrap();
            let mut fields = Vec::new();
            let mut i = 0;
            while fields.len() < 4 {
                while data[i].is_ascii_whitespace() {
                    i += 1;
                }
                let s = i;
                while !data[i].is_ascii_whitespace() {
                    i += 1;
                }
                fields.push(String::from_utf8_lossy(&data[s..i]).into_owned());
            }
            let (w, h): (usize, usize) = (fields[1].parse().unwrap(), fields[2].parse().unwrap());
            (w, h, data[i + 1..].to_vec())
        })
        .collect()
}

fn compare(name: &str, source: &str) {
    compare_with(name, source, &[]);
}

/// A link annotation's page and rectangle.
type LinkRect = (usize, [f64; 4]);

/// Link rectangles and targets, outline titles and destination names of a PDF.
fn structure(pdf: &Path) -> (Vec<LinkRect>, Vec<String>, Vec<String>) {
    let doc = lopdf::Document::load(pdf).unwrap();
    let num = |o: &lopdf::Object| match o {
        lopdf::Object::Integer(i) => *i as f64,
        lopdf::Object::Real(r) => f64::from(*r),
        _ => f64::NAN,
    };
    let mut rects = Vec::new();
    for (n, (_, id)) in doc.get_pages().iter().enumerate() {
        let page = doc.get_dictionary(*id).unwrap();
        let Ok(annots) = page.get(b"Annots") else { continue };
        let annots = match annots {
            lopdf::Object::Reference(r) => doc.get_object(*r).unwrap().as_array().unwrap().clone(),
            o => o.as_array().unwrap().clone(),
        };
        for a in annots {
            let a = doc.get_dictionary(a.as_reference().unwrap()).unwrap();
            let r = a.get(b"Rect").unwrap().as_array().unwrap();
            rects.push((n + 1, [num(&r[0]), num(&r[1]), num(&r[2]), num(&r[3])]));
        }
    }
    let catalog = doc.catalog().unwrap();
    let mut titles = Vec::new();
    if let Ok(o) = catalog.get(b"Outlines") {
        let mut stack = vec![
            doc.get_dictionary(o.as_reference().unwrap())
                .unwrap()
                .get(b"First")
                .unwrap()
                .as_reference()
                .unwrap(),
        ];
        while let Some(id) = stack.pop() {
            let d = doc.get_dictionary(id).unwrap();
            let title = match d.get(b"Title") {
                Ok(lopdf::Object::Reference(r)) => doc.get_object(*r).ok(),
                Ok(o) => Some(o),
                Err(_) => None,
            };
            if let Some(lopdf::Object::String(t, _)) = title {
                titles.push(String::from_utf8_lossy(t).into_owned());
            }
            if let Ok(n) = d.get(b"Next") {
                stack.push(n.as_reference().unwrap());
            }
            if let Ok(f) = d.get(b"First") {
                stack.push(f.as_reference().unwrap());
            }
        }
    }
    titles.sort();
    let mut dests = Vec::new();
    if let Ok(names) = catalog.get(b"Names") {
        let names = doc.get_dictionary(names.as_reference().unwrap()).unwrap();
        if let Ok(d) = names.get(b"Dests") {
            let mut stack = vec![d.as_reference().unwrap()];
            while let Some(id) = stack.pop() {
                let node = doc.get_dictionary(id).unwrap();
                if let Ok(kids) = node.get(b"Kids") {
                    stack.extend(kids.as_array().unwrap().iter().map(|k| k.as_reference().unwrap()));
                }
                if let Ok(n) = node.get(b"Names") {
                    for pair in n.as_array().unwrap().chunks(2) {
                        if let lopdf::Object::String(s, _) = &pair[0] {
                            dests.push(String::from_utf8_lossy(s).into_owned());
                        }
                    }
                }
            }
        }
    }
    dests.sort();
    (rects, titles, dests)
}

fn compare_with(name: &str, source: &str, files: &[(&str, Vec<u8>)]) {
    if !tools_available() {
        return;
    }
    let dir = scratch(name);
    for sub in ["ref", "ours"] {
        fs::write(dir.join(sub).join(format!("{name}.tex")), source).unwrap();
        for (file, data) in files {
            fs::write(dir.join(sub).join(file), data).unwrap();
        }
    }
    let status = Command::new("pdftex")
        .args(["-interaction=batchmode", &format!("{name}.tex")])
        .current_dir(dir.join("ref"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success() || dir.join("ref").join(format!("{name}.pdf")).exists());
    let ours_tex = dir.join("ours").join(format!("{name}.tex"));
    let options = CompileOptions {
        input: ours_tex,
        output: None,
        paper: Paper::A4,
        preload: rtex::PDFTEX_PRELOAD.iter().map(|s| s.to_string()).collect(),
        ..CompileOptions::default()
    };
    compile(&options).unwrap();
    let ref_pdf = dir.join("ref").join(format!("{name}.pdf"));
    let our_pdf = dir.join("ours").join(format!("{name}.pdf"));

    let expected = words(&ref_pdf);
    let actual = words(&our_pdf);
    assert!(!expected.is_empty(), "{name}: pdfTeX's PDF has no text");
    let mut unmatched = actual.clone();
    let mut worst: f64 = 0.0;
    let mut by_center = 0;
    let center = |b: &[f64; 4]| ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    for w in &expected {
        let same = |a: &Word| a.page == w.page && a.text == w.text;
        let exact = unmatched.iter().position(|a| {
            same(a)
                && a.bbox
                    .iter()
                    .zip(&w.bbox)
                    .all(|(x, y)| (x - y).abs() <= POSITION_TOLERANCE_BP)
        });
        let found = exact.or_else(|| {
            let (wx, wy) = center(&w.bbox);
            let i = unmatched.iter().position(|a| {
                let (ax, ay) = center(&a.bbox);
                same(a) && (ax - wx).abs() <= CENTER_TOLERANCE_BP && (ay - wy).abs() <= CENTER_TOLERANCE_BP
            });
            by_center += usize::from(i.is_some());
            i
        });
        match found {
            Some(i) => {
                let a = unmatched.remove(i);
                if exact.is_some() {
                    for (x, y) in a.bbox.iter().zip(&w.bbox) {
                        worst = worst.max((x - y).abs());
                    }
                }
            }
            None => panic!("{name}: word {w:?} not found within {POSITION_TOLERANCE_BP}bp"),
        }
    }
    assert!(unmatched.is_empty(), "{name}: extra words {unmatched:?}");

    let (ref_rects, ref_titles, ref_dests) = structure(&ref_pdf);
    let (our_rects, our_titles, our_dests) = structure(&our_pdf);
    assert_eq!(ref_titles, our_titles, "{name}: outline titles");
    assert_eq!(ref_dests, our_dests, "{name}: named destinations");
    assert_eq!(
        ref_rects.len(),
        our_rects.len(),
        "{name}: annotations {ref_rects:?} vs {our_rects:?}"
    );
    for ((rp, r), (op, o)) in ref_rects.iter().zip(&our_rects) {
        assert_eq!(rp, op, "{name}: annotation page");
        assert!(
            r.iter().zip(o).all(|(a, b)| (a - b).abs() <= POSITION_TOLERANCE_BP),
            "{name}: annotation {r:?} vs {o:?}"
        );
    }

    let ref_pages = pgm(&ref_pdf, &dir.join("ref").join("page"));
    let our_pages = pgm(&our_pdf, &dir.join("ours").join("page"));
    assert_eq!(ref_pages.len(), our_pages.len(), "{name}: page count");
    let mut worst_pixels: f64 = 0.0;
    for (n, (r, o)) in ref_pages.iter().zip(&our_pages).enumerate() {
        assert_eq!((r.0, r.1), (o.0, o.1), "{name}: page {} size", n + 1);
        let differing =
            r.2.iter()
                .zip(&o.2)
                .filter(|(a, b)| a.abs_diff(**b) > PIXEL_THRESHOLD)
                .count();
        let fraction = differing as f64 / r.2.len() as f64;
        assert!(
            fraction <= MAX_DIFFERING_PIXELS,
            "{name}: page {} differs in {differing} pixels ({:.4}%), word positions within {worst:.4}bp",
            n + 1,
            fraction * 100.0
        );
        worst_pixels = worst_pixels.max(fraction);
    }
    eprintln!(
        "{name}: {} words within {worst:.4}bp ({by_center} by centre), at most {:.4}% of pixels differ",
        expected.len(),
        worst_pixels * 100.0
    );
}

#[test]
fn hello_example_matches_pdftex() {
    compare("hello", include_str!("../examples/hello.tex"));
}

#[test]
fn story_matches_pdftex() {
    compare("storydoc", "\\input story \\bye\n");
}

#[test]
fn math_matches_pdftex() {
    compare(
        "mathpdf",
        r"\hsize=5in
Inline $a^2+b^2=c^2$, $\sqrt{x^2+1}$, $\left(a\over b\right)$, $\sum_{i=1}^n x_i$.
$$\int_0^\infty e^{-x^2}\,dx={\sqrt\pi\over2}\eqno(1)$$
$$\pmatrix{a&b\cr c&d\cr}\quad \bigl(\Bigl(\biggl(\Biggl(x\Biggr)\biggr)\Bigr)\bigr)\quad
\overline{x+y}\,\widehat{abc}\,\underbrace{a+b}_{n}$$
\bye
",
    );
}

fn png_image(alpha: bool) -> Vec<u8> {
    let (w, h) = (40u32, 20u32);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(if alpha {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[(x * 6) as u8, (y * 12) as u8, 128]);
                if alpha {
                    data.push(if (x + y) % 2 == 0 { 255 } else { 64 });
                }
            }
        }
        wr.write_image_data(&data).unwrap();
    }
    out
}

fn helper_files() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pdf-oracle-helpers");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).ok()?;
    fs::write(
        dir.join("inc.tex"),
        "\\hsize=2in \\vsize=1in \\pdfpagewidth=3in \\pdfpageheight=2in Included page.\\bye\n",
    )
    .ok()?;
    Command::new("pdftex")
        .args(["-interaction=batchmode", "inc.tex"])
        .current_dir(&dir)
        .stdout(Stdio::null())
        .status()
        .ok()?;
    let pdf = fs::read(dir.join("inc.pdf")).ok()?;
    Command::new("pdftoppm")
        .args(["-jpeg", "-r", "30", "-singlefile", "inc.pdf", "inc"])
        .current_dir(&dir)
        .status()
        .ok()?;
    let jpg = fs::read(dir.join("inc.jpg")).ok()?;
    Some((pdf, jpg))
}

#[test]
fn pdftex_features_match_pdftex() {
    if !tools_available() {
        return;
    }
    let Some((pdf, jpg)) = helper_files() else { return };
    compare_with(
        "features",
        r"\hsize=4in \parindent=0pt
\pdfinfo{/Title (Features)}
\pdfoutline goto name{sec1} count 1 {Section 1}
\pdfoutline goto name{sub} {Sub}
\pdfdest name{sec1} xyz {\bf Section 1}\par
See \pdfstartlink attr{/Border [0 0 1]} goto name{sub}the subsection\pdfendlink{} and
\pdfstartlink user{/Subtype /Link /A << /S /URI /URI (https://example.org) >>}a link that goes
on for long enough to break across two lines of this narrow paragraph\pdfendlink.
\pdfliteral{1 0 0 rg}Red text\pdfliteral{0 g} and back.
\pdfcolorstack0 push{0 0 1 rg}Blue\pdfcolorstack0 pop{} text.\par
\setbox0\hbox{Rotated}\hbox{\pdfsave\pdfsetmatrix{0 1 -1 0}\rlap{\box0}\pdfrestore}\par
\setbox2\hbox{\vrule width 20pt height 10pt\kern2pt X}\pdfxform 2
Form: \pdfrefxform\pdflastxform{} and again \pdfrefxform\pdflastxform.\par
\pdfximage width 1in {img.png}PNG: \pdfrefximage\pdflastximage
\pdfximage height 0.5in {alpha.png}alpha: \pdfrefximage\pdflastximage\par
\pdfximage height 1in {img.jpg}JPEG: \pdfrefximage\pdflastximage\par
\pdfximage width 2in {inc.pdf}PDF: \pdfrefximage\pdflastximage\par
\vfill\eject
\pdfdest name{sub} fith {\bf Sub}\par
\bye
",
        &[
            ("img.png", png_image(false)),
            ("alpha.png", png_image(true)),
            ("img.jpg", jpg),
            ("inc.pdf", pdf),
        ],
    );
}
