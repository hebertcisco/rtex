//! Running a job: the engine with the typesetter, in memory or on disk.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rtex_core::{CatCode, CharMode, Config, Diagnostic, Engine, FileSystem, History, Interaction, Loc, Token, Value};

use crate::document::Document;
use crate::error::CompileError;
use crate::kpse::Program;
use crate::paper::Paper;
use crate::tex::Typesetter;
use crate::texfs::TexFs;

/// Which TeX dialect the engine presents to the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineKind {
    /// pdfTeX: 8-bit characters and pdfTeX's primitives (the default).
    #[default]
    PdfTeX,
    /// XeTeX: Unicode input and XeTeX's file lookup.
    XeTeX,
    /// pdfLaTeX: LaTeX's file lookup with the pdfTeX engine.
    PdfLaTeX,
    /// XeLaTeX: LaTeX's file lookup with the XeTeX engine.
    XeLaTeX,
}

impl EngineKind {
    /// The kpathsea program name this engine searches as.
    pub fn program(self) -> Program {
        match self {
            EngineKind::PdfTeX => Program::PlainTex,
            EngineKind::XeTeX => Program::XeTeX,
            EngineKind::PdfLaTeX => Program::LaTeX,
            EngineKind::XeLaTeX => Program::XeLaTeX,
        }
    }

    /// The character model this engine reads with.
    pub fn char_mode(self) -> CharMode {
        match self {
            EngineKind::PdfTeX | EngineKind::PdfLaTeX => CharMode::EightBit,
            EngineKind::XeTeX | EngineKind::XeLaTeX => CharMode::Unicode,
        }
    }

    /// The name used on the command line.
    pub fn name(self) -> &'static str {
        match self {
            EngineKind::PdfTeX => "pdftex",
            EngineKind::XeTeX => "xetex",
            EngineKind::PdfLaTeX => "pdflatex",
            EngineKind::XeLaTeX => "xelatex",
        }
    }

    /// Whether this engine targets a LaTeX format.
    pub fn is_latex(self) -> bool {
        matches!(self, EngineKind::PdfLaTeX | EngineKind::XeLaTeX)
    }

    /// Files read through `\everyjob` before the document when not starting from a format image.
    ///
    /// Plain TeX engines preload pdfTeX's settings and plain TeX; LaTeX engines load `latex.ltx`.
    pub fn default_preload(self) -> Vec<String> {
        match self {
            EngineKind::PdfTeX | EngineKind::XeTeX => PDFTEX_PRELOAD.iter().map(|s| (*s).to_owned()).collect(),
            EngineKind::PdfLaTeX | EngineKind::XeLaTeX => LATEX_PRELOAD.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// Default inputs for [`build_format`] when none are given.
    pub fn default_format_inputs(self) -> Vec<String> {
        match self {
            EngineKind::PdfTeX | EngineKind::XeTeX => vec!["plain".to_owned()],
            EngineKind::PdfLaTeX | EngineKind::XeLaTeX => vec!["latex.ltx".to_owned()],
        }
    }
}

/// How to typeset a job.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// The paper size.
    pub paper: Paper,
    /// The PDF name reported in the transcript; defaults to `<job>.pdf`.
    pub output_name: Option<String>,
    /// Files read through `\everyjob` before the document, such as [`PDFTEX_PRELOAD`]; empty runs as INITEX.
    pub preload: Vec<String>,
    /// The engine dialect.
    pub engine: EngineKind,
    /// How TeX stops for errors.
    pub interaction: Interaction,
    /// A format image produced by [`build_format`]; when set the engine starts from it.
    pub format: Option<Vec<u8>>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            paper: Paper::A4,
            output_name: None,
            preload: Vec::new(),
            engine: EngineKind::PdfTeX,
            interaction: Interaction::Scroll,
            format: None,
        }
    }
}

/// What `rtex` reads before a document unless asked to run as INITEX: pdfTeX's settings, then plain TeX.
pub const PDFTEX_PRELOAD: [&str; 2] = ["pdftexconfig", "plain"];

/// Inputs read before a LaTeX document when no format image is supplied.
pub const LATEX_PRELOAD: [&str; 1] = ["latex.ltx"];

/// The outcome of a job.
#[derive(Clone, Debug)]
pub struct Rendering {
    /// The PDF, when at least one page was shipped.
    pub pdf: Option<Vec<u8>>,
    /// Number of pages shipped.
    pub pages: usize,
    /// The transcript (`.log`).
    pub log: Vec<u8>,
    /// What TeX printed on the terminal.
    pub terminal: Vec<u8>,
    /// Every error and warning, structured.
    pub diagnostics: Vec<Diagnostic>,
    /// The worst thing that happened.
    pub history: History,
    /// The pages and fonts a reflowable or fixed EPUB is built from.
    pub document: Document,
    /// How many passes the driver ran; a single [`render`] call is one pass.
    pub passes: usize,
    /// Wall time of each pass, in order.
    pub pass_times: Vec<Duration>,
}

impl Rendering {
    fn from_engine(mut engine: Engine<Typesetter>, history: History) -> Self {
        Rendering {
            pdf: engine.stomach.take_pdf(),
            pages: engine.stomach.document().pages().len(),
            log: engine.log().to_vec(),
            terminal: engine.terminal().to_vec(),
            diagnostics: engine.diagnostics().to_vec(),
            history,
            document: engine.stomach.document().clone(),
            passes: 1,
            pass_times: Vec::new(),
        }
    }
}

/// Typesets `input`, read from `fs`, once, as `rtex <input>` would.
pub fn render(fs: Box<dyn FileSystem>, input: &str, options: &RenderOptions) -> Rendering {
    let page_size = options.paper.size();
    let output_name = options
        .output_name
        .clone()
        .unwrap_or_else(|| format!("{}.pdf", job_name(input)));
    let config = engine_config(options.engine, options.interaction);
    let mut engine = match &options.format {
        Some(data) => match Engine::load_format(config, fs, Typesetter::new(page_size, output_name.clone()), data) {
            Ok(engine) => engine,
            Err(error) => {
                let mut engine = Engine::with_stomach(
                    engine_config(options.engine, options.interaction),
                    Box::new(rtex_core::MemoryFs::new()),
                    Typesetter::new(page_size, output_name.clone()),
                );
                let message = format!("cannot load the format: {error}");
                engine.print_err(&message);
                let _ = engine.error();
                return Rendering::from_engine(engine, History::FatalErrorStop);
            }
        },
        None => {
            let mut engine = Engine::with_stomach(config, fs, Typesetter::new(page_size, output_name.clone()));
            crate::tex::install(&mut engine);
            engine
        }
    };
    if options.preload.is_empty() {
        crate::tex::set_page_size(&mut engine, page_size);
    } else {
        let other = |s: &str| {
            s.chars()
                .map(|c| Token::char(CatCode::Other, c.into()))
                .collect::<Vec<_>>()
        };
        let space = Token::char(CatCode::Space, u32::from(' '));
        let mut tokens = Vec::new();
        for file in &options.preload {
            tokens.push(Token::cs(engine.cs("input")));
            tokens.extend(other(file));
            tokens.push(space);
        }
        for (name, v) in [("pdfpagewidth", page_size.width), ("pdfpageheight", page_size.height)] {
            tokens.push(Token::cs(engine.cs(name)));
            tokens.extend(other(&format!("={v}sp")));
            tokens.push(Token::cs(engine.cs("relax")));
        }
        let every_job = Loc::ToksPar(rtex_core::params::EVERY_JOB as u16);
        if engine
            .eq_define(every_job, Value::Toks(Some(Arc::from(tokens))), true)
            .is_err()
        {
            let history = engine.history();
            return Rendering::from_engine(engine, history);
        }
    }
    let history = engine.run_file(&first_line(input));
    Rendering::from_engine(engine, history)
}

/// What to compile and where to write the results.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// The TeX file.
    pub input: PathBuf,
    /// The PDF to write; defaults to `<job>.pdf` in the output directory.
    pub output: Option<PathBuf>,
    /// The paper size.
    pub paper: Paper,
    /// Files read through `\everyjob` before the document; empty runs as INITEX.
    pub preload: Vec<String>,
    /// The engine dialect.
    pub engine: EngineKind,
    /// How TeX stops for errors.
    pub interaction: Interaction,
    /// Overrides `\jobname` and the output names.
    pub job_name: Option<String>,
    /// Where auxiliary files and outputs go; defaults to the input's directory.
    pub output_directory: Option<PathBuf>,
    /// A format image to start from.
    pub format: Option<Vec<u8>>,
    /// The most passes the driver runs while the auxiliary files change.
    pub max_passes: usize,
    /// Whether to write the PDF; the pages are still typeset for an EPUB.
    pub pdf: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            input: PathBuf::new(),
            output: None,
            paper: Paper::A4,
            preload: Vec::new(),
            engine: EngineKind::PdfTeX,
            interaction: Interaction::Nonstop,
            job_name: None,
            output_directory: None,
            format: None,
            max_passes: 5,
            pdf: true,
        }
    }
}

/// The files a compilation wrote and the job's outcome.
#[derive(Clone, Debug)]
pub struct CompileReport {
    /// The PDF written, or `None` when no page was shipped.
    pub pdf_path: Option<PathBuf>,
    /// The transcript written.
    pub log_path: PathBuf,
    /// Where the EPUB was written when one was asked for.
    pub epub_path: Option<PathBuf>,
    /// The job's outcome.
    pub rendering: Rendering,
}

/// Compiles a file on disk, writing the PDF, the transcript and, when
/// `epub` is set, the EPUB. TeX runs again until the auxiliary files stop
/// changing, up to `max_passes`.
pub fn compile(options: &CompileOptions) -> Result<CompileReport, CompileError> {
    let input = &options.input;
    if !input.is_file() {
        return Err(CompileError::InputNotFound(input.clone()));
    }
    let file_name = input
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CompileError::InvalidInput(input.clone()))?;
    let input_parent = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Search from the current working directory so project-relative paths (e.g.
    // `\input{shared/core}` from a book root) resolve as they do with pdflatex.
    let directory = Path::new(".");
    let job = options
        .job_name
        .clone()
        .unwrap_or_else(|| job_name(file_name).to_owned());
    let output_directory = options
        .output_directory
        .clone()
        .unwrap_or_else(|| input_parent.to_owned());
    let pdf_path = options
        .output
        .clone()
        .unwrap_or_else(|| output_directory.join(format!("{job}.pdf")));
    let log_path = pdf_path.with_extension("log");
    let source = if options.job_name.is_some() {
        Some(fs::read(input).map_err(|source| CompileError::Read {
            path: input.clone(),
            source,
        })?)
    } else {
        None
    };
    let run_name = source
        .as_ref()
        .map_or_else(|| input.to_string_lossy().into_owned(), |_| format!("{job}.tex"));

    // LaTeX engines need a format image; loading `latex.ltx` as a runtime preload
    // ends in `\dump`. If none is provided, build the default LaTeX format once.
    let mut format = options.format.clone();
    let mut preload = options.preload.clone();
    if format.is_none() && options.engine.is_latex() && !options.preload.is_empty() {
        format = Some(build_format(directory, &[], options.engine)?);
        preload.clear();
    }

    let mut passes = 0;
    let mut times = Vec::new();
    let mut previous: Option<u64> = None;
    let mut rendering = None;
    for pass in 0..options.max_passes.max(1) {
        let mut texfs = TexFs::new(directory, &output_directory, options.engine.program());
        if let Some(data) = &source {
            texfs.add_virtual(&run_name, data.clone());
        }
        let render_options = RenderOptions {
            paper: options.paper,
            output_name: Some(pdf_path.display().to_string()),
            preload: preload.clone(),
            engine: options.engine,
            interaction: options.interaction,
            format: format.clone(),
        };
        let started = Instant::now();
        let outcome = render(Box::new(texfs), &run_name, &render_options);
        times.push(started.elapsed());
        passes = pass + 1;
        let digest = aux_digest(&output_directory, &job);
        let finished = rendering.is_some() && previous == Some(digest);
        let has_aux = aux_files_exist(&output_directory, &job);
        rendering = Some(outcome);
        previous = Some(digest);
        if !has_aux || finished || matches!(rendering.as_ref().unwrap().history, History::FatalErrorStop) {
            break;
        }
    }
    let mut rendering = rendering.expect("at least one pass ran");
    rendering.passes = passes;
    rendering.pass_times = times;
    write_file(&log_path, &rendering.log)?;
    let pdf_path = match &rendering.pdf {
        Some(pdf) if options.pdf => {
            write_file(&pdf_path, pdf)?;
            Some(pdf_path)
        }
        _ => None,
    };
    Ok(CompileReport {
        pdf_path,
        log_path,
        epub_path: None,
        rendering,
    })
}

fn aux_files_exist(directory: &Path, job: &str) -> bool {
    ["aux", "toc", "lof", "lot", "idx", "bbl"]
        .iter()
        .any(|extension| directory.join(format!("{job}.{extension}")).is_file())
}

fn aux_digest(directory: &Path, job: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    for extension in ["aux", "toc", "lof", "lot", "idx", "bbl"] {
        let path = directory.join(format!("{job}.{extension}"));
        if let Ok(data) = fs::read(path) {
            extension.hash(&mut hasher);
            data.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Builds a format image by running `\dump` after `input`s, as INITEX does.
///
/// With no `input` this primes the engine's default format (plain TeX or `latex.ltx`).
pub fn build_format(directory: &Path, inputs: &[String], engine_kind: EngineKind) -> Result<Vec<u8>, CompileError> {
    let fs = TexFs::new(directory, directory, engine_kind.program());
    let mut engine = Engine::with_stomach(
        engine_config(engine_kind, Interaction::Nonstop),
        Box::new(fs),
        Typesetter::new(Paper::A4.size(), "format.fmt"),
    );
    crate::tex::install(&mut engine);
    let inputs: Vec<String> = if inputs.is_empty() {
        engine_kind.default_format_inputs()
    } else {
        inputs.to_vec()
    };
    let mut text = String::new();
    for input in &inputs {
        text.push_str("\\input ");
        text.push_str(input);
        text.push(' ');
    }
    text.push_str("\\dump");
    engine.file_system_mut().add_virtual("rtexfmt.tex", text.into_bytes());
    engine.run_file("rtexfmt.tex");
    engine.take_format().ok_or_else(|| CompileError::Format {
        message: String::from_utf8_lossy(engine.terminal()).into_owned(),
    })
}

fn engine_config(engine: EngineKind, interaction: Interaction) -> Config {
    Config {
        banner: concat!("This is rtex, Version ", env!("CARGO_PKG_VERSION")).into(),
        char_mode: engine.char_mode(),
        interaction,
        ..Config::default()
    }
}

fn job_name(input: &str) -> &str {
    Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("texput")
}

fn first_line(input: &str) -> String {
    if input.contains(' ') {
        format!("\"{input}\"")
    } else {
        input.to_owned()
    }
}

fn write_file(path: &Path, data: &[u8]) -> Result<(), CompileError> {
    let write = || {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, data)
    };
    write().map_err(|source| CompileError::Write {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_names_drop_directories_and_extensions() {
        assert_eq!(job_name("book.tex"), "book");
        assert_eq!(job_name("chapters/one"), "one");
    }

    #[test]
    fn names_with_spaces_are_quoted() {
        assert_eq!(first_line("my book.tex"), "\"my book.tex\"");
        assert_eq!(first_line("book.tex"), "book.tex");
    }

    #[test]
    fn an_unchanged_auxiliary_file_stops_the_driver() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("job.aux"), b"same").unwrap();
        let first = aux_digest(dir.path(), "job");
        assert_eq!(first, aux_digest(dir.path(), "job"));
        fs::write(dir.path().join("job.toc"), b"new").unwrap();
        assert_ne!(first, aux_digest(dir.path(), "job"));
    }

    #[test]
    fn engine_defaults_for_plain_and_latex() {
        assert!(!EngineKind::PdfTeX.is_latex());
        assert!(!EngineKind::XeTeX.is_latex());
        assert!(EngineKind::PdfLaTeX.is_latex());
        assert!(EngineKind::XeLaTeX.is_latex());

        assert_eq!(EngineKind::PdfTeX.default_preload(), vec!["pdftexconfig", "plain"]);
        assert_eq!(EngineKind::XeTeX.default_preload(), vec!["pdftexconfig", "plain"]);
        assert_eq!(EngineKind::PdfLaTeX.default_preload(), vec!["latex.ltx"]);
        assert_eq!(EngineKind::XeLaTeX.default_preload(), vec!["latex.ltx"]);

        assert_eq!(EngineKind::PdfTeX.default_format_inputs(), vec!["plain"]);
        assert_eq!(EngineKind::PdfLaTeX.default_format_inputs(), vec!["latex.ltx"]);

        assert_eq!(EngineKind::PdfTeX.name(), "pdftex");
        assert_eq!(EngineKind::XeTeX.name(), "xetex");
        assert_eq!(EngineKind::PdfLaTeX.name(), "pdflatex");
        assert_eq!(EngineKind::XeLaTeX.name(), "xelatex");

        assert_eq!(EngineKind::PdfTeX.program(), Program::PlainTex);
        assert_eq!(EngineKind::XeTeX.program(), Program::XeTeX);
        assert_eq!(EngineKind::PdfLaTeX.program(), Program::LaTeX);
        assert_eq!(EngineKind::XeLaTeX.program(), Program::XeLaTeX);
    }
}
