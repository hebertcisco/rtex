//! The `rtex` binary: files written, output printed and exit codes.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rtex() -> Command {
    Command::cargo_bin("rtex").expect("the rtex binary is built")
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn write_tex(dir: &TempDir, name: &str, source: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, source).expect("the input is written");
    path
}

fn page_count(pdf: &Path) -> usize {
    lopdf::Document::load(pdf).expect("the PDF parses").get_pages().len()
}

#[test]
fn builds_the_hello_example_into_a_pdf_and_a_log() {
    let out = TempDir::new().unwrap();
    let pdf = out.path().join("nested").join("hello.pdf");
    rtex()
        .arg("build")
        .arg(examples().join("hello.tex"))
        .arg("-o")
        .arg(&pdf)
        .assert()
        .success()
        .stdout(predicate::str::contains("Output written on"));
    assert!(fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    assert_eq!(page_count(&pdf), 1);
    let log = fs::read_to_string(pdf.with_extension("log")).unwrap();
    assert!(log.starts_with("This is rtex"));
}

#[test]
fn builds_the_book_example_with_its_chapters() {
    let out = TempDir::new().unwrap();
    let pdf = out.path().join("book.pdf");
    rtex()
        .arg(examples().join("book").join("book.tex"))
        .arg("--output")
        .arg(&pdf)
        .arg("--quiet")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
    assert!(page_count(&pdf) >= 3);
}

#[test]
fn writes_next_to_the_input_by_default() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "note.tex", "A short note.\\bye\n");
    rtex().arg(&input).arg("-q").assert().success();
    assert!(dir.path().join("note.pdf").is_file());
    assert!(dir.path().join("note.log").is_file());
}

#[test]
fn json_reports_files_pages_and_diagnostics() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "bad.tex", "Text \\undefinedmacro more.\\bye\n");
    rtex()
        .arg(&input)
        .arg("--json")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"pages\":1"))
        .stdout(predicate::str::contains("\"history\":\"error\""))
        .stdout(predicate::str::contains("Undefined control sequence"));
}

#[test]
fn letter_paper_is_accepted_and_unknown_paper_is_rejected() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "page.tex", "Letter.\\bye\n");
    rtex().arg(&input).args(["--paper", "letter", "-q"]).assert().success();
    rtex().arg(&input).args(["--paper", "a5"]).assert().failure();
}

#[test]
fn a_missing_input_fails_with_a_message() {
    rtex()
        .arg("does-not-exist.tex")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("input file not found"));
}

#[test]
fn no_arguments_print_usage() {
    rtex().assert().failure().stderr(predicate::str::contains("Usage"));
}

#[test]
fn a_job_without_pages_writes_no_pdf() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "empty.tex", "\\message{nothing}\\end\n");
    rtex()
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains("No pages of output."));
    assert!(!dir.path().join("empty.pdf").exists());
}

#[test]
fn an_epub_is_written_in_both_layouts() {
    let out = TempDir::new().unwrap();
    let pdf = out.path().join("hello.pdf");
    rtex()
        .arg(examples().join("hello.tex"))
        .arg("-o")
        .arg(&pdf)
        .args(["--to", "pdf,epub", "--title", "Hello", "--author", "Hebert", "-q"])
        .assert()
        .success();
    let epub = out.path().join("hello.epub");
    let data = fs::read(&epub).unwrap();
    assert!(data.starts_with(b"PK\x03\x04"));
    let names: Vec<String> = rtex::epub::entries(&data).into_iter().map(|(name, _)| name).collect();
    assert!(names.contains(&"OEBPS/content.opf".to_owned()));
    assert!(names.contains(&"OEBPS/nav.xhtml".to_owned()));
    assert!(
        names
            .iter()
            .any(|name| name.ends_with(".xhtml") && name.contains("page"))
    );

    let fixed = out.path().join("fixed.epub");
    rtex()
        .arg(examples().join("hello.tex"))
        .arg("-o")
        .arg(out.path().join("fixed.pdf"))
        .args(["--to", "epub", "--epub-layout", "fixed", "-q"])
        .assert()
        .success();
    assert!(!out.path().join("fixed.pdf").exists());
    let data = fs::read(&fixed).unwrap();
    let names: Vec<String> = rtex::epub::entries(&data).into_iter().map(|(name, _)| name).collect();
    assert!(names.iter().any(|name| name.ends_with(".svg")));
    let opf = rtex::epub::entries(&data)
        .into_iter()
        .find(|(name, _)| name == "OEBPS/content.opf")
        .map(|(_, data)| String::from_utf8(data).unwrap())
        .unwrap();
    assert!(opf.contains("pre-paginated"));
}

#[test]
fn check_reports_json_without_writing_a_pdf() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "check.tex", "Fine.\\bye\n");
    rtex()
        .arg("check")
        .arg(&input)
        .args(["--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"history\":\"spotless\""));
    assert!(!dir.path().join("check.pdf").exists());
}

#[test]
fn format_build_writes_an_image() {
    let found =
        rtex::kpse::Kpathsea::discover(rtex::kpse::Program::PlainTex).find("plain.tex", rtex::kpse::FileKind::Tex);
    if found.is_none() {
        return;
    }
    let out = TempDir::new().unwrap();
    let image = out.path().join("plain.fmt");
    rtex()
        .arg("format")
        .arg("build")
        .arg(&image)
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));
    assert!(fs::metadata(&image).unwrap().len() > 10_000);
}

fn latex_available() -> bool {
    rtex::kpse::Kpathsea::discover(rtex::kpse::Program::LaTeX)
        .find("latex.ltx", rtex::kpse::FileKind::Tex)
        .is_some()
}

#[test]
fn build_accepts_all_common_parameters() {
    let out = TempDir::new().unwrap();
    let input = write_tex(&out, "params.tex", "Parameter test.\\bye\n");
    let pdf = out.path().join("out").join("custom-job.pdf");
    rtex()
        .arg("build")
        .arg(&input)
        .args(["-o", pdf.to_str().unwrap()])
        .args(["--paper", "letter"])
        .args(["--to", "pdf,epub"])
        .args(["--epub-layout", "fixed"])
        .args(["--title", "Title"])
        .args(["--author", "Author"])
        .args(["--lang", "pt-BR"])
        .args(["--identifier", "urn:test:id"])
        .args(["--engine", "pdftex"])
        .args(["--interaction", "nonstopmode"])
        .args(["--jobname", "custom-job"])
        .args(["--output-directory", out.path().join("out").to_str().unwrap()])
        .args(["--max-passes", "2"])
        .arg("--profile")
        .arg("--quiet")
        .assert()
        .success();
    assert!(pdf.is_file());
    assert!(out.path().join("out").join("custom-job.log").is_file());
    assert!(out.path().join("out").join("custom-job.epub").is_file());
}

#[test]
fn build_supports_json_and_ini_and_formats_flags() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "ini.tex", "\\catcode`\\{=1 \\catcode`\\}=2 \\end\n");
    rtex()
        .arg(&input)
        .arg("--ini")
        .arg("--json")
        .args(["--engine", "pdftex"])
        .args(["--interaction", "batchmode"])
        .args(["--to", "pdf"])
        .args(["--paper", "a4"])
        .args(["--max-passes", "1"])
        .assert()
        .code(predicate::in_iter([0i32, 1, 2]))
        .stdout(predicate::str::contains("\"history\""));
}

#[test]
fn check_accepts_all_parameters() {
    let dir = TempDir::new().unwrap();
    let input = write_tex(&dir, "check2.tex", "Fine.\\bye\n");
    rtex()
        .arg("check")
        .arg(&input)
        .args(["--engine", "pdftex"])
        .args(["--interaction", "nonstopmode"])
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"history\":\"spotless\""));

    let ini = write_tex(&dir, "checkini.tex", "\\catcode`\\{=1 \\catcode`\\}=2 \\end\n");
    rtex()
        .arg("check")
        .arg(&ini)
        .arg("--ini")
        .args(["--engine", "xetex"])
        .args(["--interaction", "scrollmode"])
        .arg("--quiet")
        .assert()
        .code(predicate::in_iter([0i32, 1, 2]));
}

#[test]
fn format_build_accepts_engine_and_input_parameters() {
    let found =
        rtex::kpse::Kpathsea::discover(rtex::kpse::Program::PlainTex).find("plain.tex", rtex::kpse::FileKind::Tex);
    if found.is_none() {
        return;
    }
    let out = TempDir::new().unwrap();
    let image = out.path().join("plain-input.fmt");
    rtex()
        .arg("format")
        .arg("build")
        .arg(&image)
        .args(["--engine", "pdftex"])
        .args(["--input", "plain"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));
    assert!(image.is_file());
}

#[test]
fn build_uses_a_provided_format_image() {
    let found =
        rtex::kpse::Kpathsea::discover(rtex::kpse::Program::PlainTex).find("plain.tex", rtex::kpse::FileKind::Tex);
    if found.is_none() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let image = dir.path().join("plain.fmt");
    rtex().arg("format").arg("build").arg(&image).assert().success();

    let input = write_tex(&dir, "fromfmt.tex", "Loaded.\\bye\n");
    rtex()
        .arg(&input)
        .args(["--format", image.to_str().unwrap()])
        .args(["--engine", "pdftex"])
        .arg("-q")
        .assert()
        .success();
    assert!(dir.path().join("fromfmt.pdf").is_file());
}

#[test]
fn format_build_and_compile_support_latex_engine() {
    if !latex_available() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let fmt = dir.path().join("latex.fmt");
    rtex()
        .arg("format")
        .arg("build")
        .arg(&fmt)
        .args(["--engine", "pdflatex"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));
    assert!(fmt.is_file());

    let input = write_tex(
        &dir,
        "doc.tex",
        "\\documentclass{article}\\begin{document}Hello LaTeX.\\end{document}\n",
    );
    rtex()
        .arg(&input)
        .args(["--engine", "pdflatex"])
        .args(["--format", fmt.to_str().unwrap()])
        .args(["--interaction", "nonstopmode"])
        .arg("-q")
        .assert()
        .code(predicate::in_iter([0i32, 1]))
        .stdout(predicate::str::is_empty());
    assert!(dir.path().join("doc.pdf").is_file());
}
