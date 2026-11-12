//! Golden tests for the typesetter: each `tests/golden/<name>.tex` must produce the
//! transcript that `pdftex -ini -etex -interaction=scrollmode` writes for it, with
//! boxes (`\showbox`, `\showlists`), paragraphs (`\tracingparagraphs`), pages
//! (`\tracingpages`) and output (`\tracingoutput`) shown in full.
//!
//! Transcripts are generated on demand from the installed TeX Live into ignored
//! `tests/golden/<name>.log` files, again whenever the input is newer; set
//! `RTEX_SKIP_GOLDEN=1` to skip these tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rtex::kpse::Program;
use rtex::tex::Typesetter;
use rtex::texfs::TexFs;
use rtex::{PageSize, Paper};
use rtex_core::{Config, Engine, Interaction};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn inputs() -> Vec<PathBuf> {
    fs::read_dir(golden_dir())
        .expect("golden directory")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect()
}

fn normalize(log: &[u8]) -> String {
    let text = String::from_utf8_lossy(log);
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with('(') {
            break;
        }
        start += line.len();
    }
    text[start..]
        .lines()
        .map(|l| {
            if l.starts_with("Output written on ") {
                "Output written on <output>."
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_difference(expected: &str, actual: &str) -> String {
    let (e, a): (Vec<_>, Vec<_>) = (expected.lines().collect(), actual.lines().collect());
    let n = e.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let from = n.saturating_sub(3);
    let show = |v: &[&str]| {
        v.iter()
            .skip(from)
            .take(10)
            .map(|l| format!("    |{l}|\n"))
            .collect::<String>()
    };
    format!(
        "first difference at line {}\n  expected:\n{}  actual:\n{}",
        n + 1,
        show(&e),
        show(&a)
    )
}

fn scratch(name: &str, tag: &str) -> PathBuf {
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("golden-{tag}-{name}"));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).expect("scratch directory");
    for path in inputs() {
        fs::copy(&path, work.join(path.file_name().expect("file name"))).expect("copy golden input");
    }
    work
}

fn generate_expected(name: &str, dest: &Path) {
    let work = scratch(name, "oracle");
    Command::new("pdftex")
        .args(["-ini", "-etex", "-interaction=scrollmode", &format!("{name}.tex")])
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|err| panic!("{name}: pdftex could not run ({err}); set RTEX_SKIP_GOLDEN=1 to skip"));
    let log = fs::read(work.join(format!("{name}.log"))).expect("pdftex wrote a transcript");
    fs::write(dest, normalize(&log)).expect("write expected transcript");
    let _ = fs::remove_dir_all(&work);
}

fn is_stale(input: &Path, expected: &Path) -> bool {
    let modified = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    match (modified(input), modified(expected)) {
        (Some(i), Some(e)) => i > e,
        _ => true,
    }
}

fn run_golden(name: &str) {
    if std::env::var_os("RTEX_SKIP_GOLDEN").is_some() {
        return;
    }
    let expected_path = golden_dir().join(format!("{name}.log"));
    if is_stale(&golden_dir().join(format!("{name}.tex")), &expected_path) {
        generate_expected(name, &expected_path);
    }
    let expected = fs::read_to_string(&expected_path).expect("expected transcript");
    let work = scratch(name, "rtex");
    let fs = TexFs::new(&work, &work, Program::PlainTex);
    let config = Config {
        interaction: Interaction::Scroll,
        ..Config::default()
    };
    let size: PageSize = Paper::A4.size();
    let mut engine = Engine::with_stomach(config, Box::new(fs), Typesetter::new(size, format!("{name}.pdf")));
    rtex::tex::install(&mut engine);
    engine.run_file(&format!("{name}.tex"));
    let actual = normalize(engine.log());
    let _ = fs::remove_dir_all(&work);
    assert!(actual == expected, "{name}: {}", first_difference(&expected, &actual));
}

macro_rules! golden {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_golden(stringify!($name));
            }
        )*
        const LISTED: &[&str] = &[$(stringify!($name)),*];
    };
}

golden!(
    alignments, boxes, math, matherrors, pages, paragraphs, pdfprims, plainstory
);

#[test]
fn every_golden_input_is_listed() {
    let mut found: Vec<String> = inputs()
        .iter()
        .filter_map(|p| Some(p.file_stem()?.to_string_lossy().into_owned()))
        .filter(|stem| !stem.starts_with('_'))
        .collect();
    found.sort();
    assert_eq!(found, LISTED, "tests/golden.rs must list every input in tests/golden");
}
