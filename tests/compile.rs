//! End-to-end typesetting through the library API: plain TeX documents to parsed PDFs.

use std::fs;
use std::path::Path;

use rtex::kpse::{FileKind, Kpathsea, Program};
use rtex::texfs::TexFs;
use rtex::{Paper, RenderOptions, Rendering, render};
use rtex_core::{History, MemoryFs};

fn tex_live() -> bool {
    Kpathsea::discover(Program::PlainTex)
        .find("plain.tex", FileKind::Tex)
        .is_some()
}

fn render_plain(source: &str) -> Option<Rendering> {
    render_plain_with(source, RenderOptions::default())
}

fn render_plain_with(source: &str, options: RenderOptions) -> Option<Rendering> {
    if !tex_live() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("doc.tex"), source).unwrap();
    let fs = TexFs::new(dir.path(), dir.path(), Program::PlainTex);
    let options = RenderOptions {
        preload: rtex::PDFTEX_PRELOAD.iter().map(|s| s.to_string()).collect(),
        ..options
    };
    Some(render(Box::new(fs), "doc.tex", &options))
}

fn parse(rendering: &Rendering) -> lopdf::Document {
    let pdf = rendering.pdf.as_ref().expect("a PDF was produced");
    lopdf::Document::load_mem(pdf).expect("the PDF parses")
}

fn transcript(rendering: &Rendering) -> String {
    String::from_utf8_lossy(&rendering.log).into_owned()
}

fn media_box(document: &lopdf::Document) -> Vec<f32> {
    let (_, page_id) = document.get_pages().into_iter().next().expect("one page");
    let page = document.get_dictionary(page_id).expect("page dictionary");
    page.get(b"MediaBox")
        .and_then(|b| b.as_array())
        .expect("media box")
        .iter()
        .map(|v| v.as_float().expect("number"))
        .collect()
}

#[test]
fn a_plain_paragraph_becomes_a_one_page_pdf() {
    let Some(r) = render_plain("Typesetting with rtex is simple.\\bye") else {
        return;
    };
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));
    assert_eq!(r.pages, 1);
    assert_eq!(parse(&r).get_pages().len(), 1);
    assert!(transcript(&r).contains("Output written on doc.pdf (1 page,"));
}

#[test]
fn eject_starts_a_new_page_with_plain_page_numbers() {
    let Some(r) = render_plain("First page.\\vfill\\eject Second page.\\vfill\\eject Third.\\bye") else {
        return;
    };
    assert_eq!(r.pages, 3);
    let log = transcript(&r);
    assert!(
        log.contains("[1") && log.contains("[2]") && log.contains("[3]"),
        "{log}"
    );
}

#[test]
fn long_documents_break_into_pages() {
    let paragraph = "This sentence is repeated to fill several pages of text. ".repeat(30);
    let source = format!("{}\\bye", format!("{paragraph}\\par ").repeat(30));
    let Some(r) = render_plain(&source) else { return };
    assert!(r.pages > 3, "only {} pages", r.pages);
    assert_eq!(parse(&r).get_pages().len(), r.pages);
}

#[test]
fn hyphenation_patterns_are_used() {
    let source = "\\hsize=1in \\tracingparagraphs=1 Incomprehensibilities characterization.\\par\\bye";
    let Some(r) = render_plain(source) else { return };
    assert!(transcript(&r).contains("@secondpass"), "{}", transcript(&r));
    assert!(transcript(&r).contains("char-ac-ter-i-za-tion."), "{}", transcript(&r));
}

#[test]
fn unknown_fonts_produce_a_diagnostic() {
    let Some(r) = render_plain("\\font\\x=nosuchfont \\x Text.\\bye") else {
        return;
    };
    assert_eq!(r.history, History::ErrorMessageIssued);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("nosuchfont") && d.message.contains("not loadable"))
    );
}

#[test]
fn paper_size_sets_the_media_box() {
    let letter = RenderOptions {
        paper: Paper::Letter,
        ..RenderOptions::default()
    };
    let Some(r) = render_plain_with("Letter.\\bye", letter) else {
        return;
    };
    let media = media_box(&parse(&r));
    assert!(
        (media[2] - 612.0).abs() < 0.01 && (media[3] - 792.0).abs() < 0.01,
        "{media:?}"
    );
}

#[test]
fn the_job_name_is_the_document() {
    let Some(r) = render_plain("\\message{[\\jobname]}\\bye") else {
        return;
    };
    assert!(transcript(&r).contains("[doc]"));
}

#[test]
fn initex_runs_without_any_distribution() {
    let fs = MemoryFs::new().with("ini.tex", "\\catcode`\\{=1 \\catcode`\\}=2 \\message{ini}\\end");
    let r = render(Box::new(fs), "ini.tex", &RenderOptions::default());
    assert_eq!(r.history, History::Spotless);
    assert_eq!(r.pages, 0);
    assert!(r.pdf.is_none());
    assert!(transcript(&r).contains("No pages of output."));
    assert!(transcript(&r).contains("ini"));
}

#[test]
fn examples_compile() {
    if !tex_live() {
        return;
    }
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    for (dir, file, min_pages) in [(".", "hello.tex", 1), ("book", "book.tex", 3)] {
        let root = examples.join(dir);
        let fs = TexFs::new(&root, std::env::temp_dir(), Program::PlainTex);
        let options = RenderOptions {
            preload: rtex::PDFTEX_PRELOAD.iter().map(|s| s.to_string()).collect(),
            ..RenderOptions::default()
        };
        let r = render(Box::new(fs), file, &options);
        assert_eq!(r.history, History::Spotless, "{file}: {}", transcript(&r));
        assert!(r.pages >= min_pages, "{file}: {} pages", r.pages);
    }
}

#[test]
fn a_format_image_restores_fonts_and_hyphenation() {
    if !tex_live() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let image = rtex::build_format(dir.path(), &[], rtex::EngineKind::PdfTeX).expect("plain TeX is dumped");
    assert!(image.len() > 10_000, "{} bytes", image.len());
    fs::write(dir.path().join("plain.fmt"), &image).unwrap();
    fs::write(
        dir.path().join("doc.tex"),
        "Loaded from a format: incomprehensibilities.\\bye\n",
    )
    .unwrap();
    let fs = TexFs::new(dir.path(), dir.path(), Program::PlainTex);
    let options = RenderOptions {
        format: Some(image),
        ..RenderOptions::default()
    };
    let r = render(Box::new(fs), "doc.tex", &options);
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));
    assert_eq!(r.pages, 1);
    let pdf = r.pdf.expect("a PDF was produced");
    assert!(pdf.len() > 5_000, "the format kept its fonts: {} bytes", pdf.len());
}

fn latex_available() -> bool {
    rtex::kpse::Kpathsea::discover(Program::LaTeX)
        .find("latex.ltx", FileKind::Tex)
        .is_some()
}

#[test]
fn builds_a_latex_format_and_renders_a_minimal_document() {
    if !latex_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let image = rtex::build_format(dir.path(), &[], rtex::EngineKind::PdfLaTeX).expect("latex.ltx is dumped");
    assert!(image.len() > 100_000, "{} bytes", image.len());

    fs::write(
        dir.path().join("doc.tex"),
        "\\documentclass{article}\\begin{document}Hello LaTeX.\\end{document}\n",
    )
    .unwrap();
    let fs = TexFs::new(dir.path(), dir.path(), Program::LaTeX);
    let options = RenderOptions {
        engine: rtex::EngineKind::PdfLaTeX,
        format: Some(image),
        ..RenderOptions::default()
    };
    let r = render(Box::new(fs), "doc.tex", &options);
    assert!(r.pages >= 1, "{}", transcript(&r));
    let pdf = r.pdf.expect("a PDF was produced");
    assert!(pdf.starts_with(b"%PDF-"));
}

#[test]
fn compile_auto_builds_latex_format_for_pdflatex_engine() {
    if !latex_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("doc.tex");
    fs::write(
        &input,
        "\\documentclass{article}\\begin{document}Auto format.\\end{document}\n",
    )
    .unwrap();
    let report = rtex::compile(&rtex::CompileOptions {
        input,
        engine: rtex::EngineKind::PdfLaTeX,
        preload: rtex::EngineKind::PdfLaTeX.default_preload(),
        interaction: rtex_core::Interaction::Nonstop,
        ..Default::default()
    })
    .expect("compile succeeds");
    assert!(report.rendering.pages >= 1, "{}", transcript(&report.rendering));
    assert!(report.pdf_path.as_ref().expect("pdf path").is_file());
    assert!(report.log_path.is_file());
}

#[test]
fn compile_resolves_project_relative_inputs_from_cwd() {
    if !tex_live() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("part.tex"), "Included from project root.").unwrap();
    let nested = dir.path().join("editions").join("1");
    fs::create_dir_all(&nested).unwrap();
    let input = nested.join("main.tex");
    fs::write(&input, "\\input{shared/part}\\bye\n").unwrap();

    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let result = rtex::compile(&rtex::CompileOptions {
        input: Path::new("editions/1/main.tex").to_path_buf(),
        preload: rtex::PDFTEX_PRELOAD.iter().map(|s| s.to_string()).collect(),
        interaction: rtex_core::Interaction::Nonstop,
        ..Default::default()
    });
    std::env::set_current_dir(cwd).unwrap();

    let report = result.expect("compile succeeds");
    assert_eq!(report.rendering.pages, 1);
    assert!(transcript(&report.rendering).contains("Included from project root.") || report.pdf_path.is_some());
}

#[test]
fn letterspacefont_and_pdfcopyfont_restrictions_are_enforced() {
    if !tex_live() {
        return;
    }
    let ok = render_plain("\\font\\base=cmr10\n\\letterspacefont\\ls=\\base by 100\n\\ls Letterspaced text.\n\\bye");
    let r = ok.expect("render");
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));

    let nolig =
        render_plain("\\font\\base=cmr10\n\\letterspacefont\\ls=\\base by 100 nolig\n\\ls Letterspaced text.\n\\bye");
    let r = nolig.expect("render with nolig");
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));

    let copied = render_plain("\\font\\base=cmr10\n\\pdfcopyfont\\cp=\\base\n\\cp Copied text.\n\\bye");
    let r = copied.expect("render of a plain copy");
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));

    let refused =
        render_plain("\\font\\base=cmr10\n\\letterspacefont\\ls=\\base by 100\n\\pdfcopyfont\\cp=\\ls\n\\bye");
    let r = refused.expect("render refuses to copy a letterspaced font");
    assert_ne!(r.history, History::Spotless, "{}", transcript(&r));
    assert!(
        transcript(&r).contains("cannot copy a letterspaced font"),
        "{}",
        transcript(&r)
    );

    let expanded = render_plain("\\font\\base=cmr10\n\\pdffontexpand\\base 20 20 1\n\\pdfcopyfont\\cp=\\base\n\\bye");
    let r = expanded.expect("render refuses to copy an expanded font");
    assert_ne!(r.history, History::Spotless, "{}", transcript(&r));
    assert!(
        transcript(&r).contains("cannot copy an expanded font"),
        "{}",
        transcript(&r)
    );
}

#[test]
fn letterspaced_font_ships_packets_through_the_base_font() {
    if !tex_live() {
        return;
    }
    let src = "\\font\\base=cmr10\n\\letterspacefont\\ls=\\base by 1000\n\\ls AV TeX.\n\\bye";
    let r = render_plain(src).expect("render");
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));
    let pdf = r.pdf.as_ref().expect("a PDF was produced");
    let bytes = pdf.as_slice();
    let text = String::from_utf8_lossy(bytes);
    assert!(
        !text.contains("1000LS") && !text.contains("+1000ls"),
        "the virtual font must not appear as a PDF font:\n{}",
        transcript(&r)
    );
    assert!(
        text.contains("/BaseFont /") && text.contains("CMR10"),
        "the base font must be embedded: no /BaseFont CMR10 found"
    );
    assert!(text.contains("/FontFile"), "the base font program must be embedded");
}

#[test]
fn pdffontexpand_rejects_conflicting_redefinition_but_allows_identical() {
    if !tex_live() {
        return;
    }
    let same = render_plain(
        "\\font\\base=cmr10\n\\pdffontexpand\\base 40 20 2 autoexpand\n\\pdffontexpand\\base 40 20 2 autoexpand\nWidth check.\\bye",
    );
    let r = same.expect("render");
    assert_eq!(r.history, History::Spotless, "{}", transcript(&r));

    let different = render_plain(
        "\\font\\base=cmr10\n\\pdffontexpand\\base 40 20 2\n\\pdffontexpand\\base 30 20 2\nWidth check.\\bye",
    );
    let r = different.expect("render");
    assert_ne!(r.history, History::Spotless, "{}", transcript(&r));
    let flat = transcript(&r).replace(['\n', '\r'], "");
    assert!(flat.contains("different parameters"), "{flat}");
}
