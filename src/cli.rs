//! The `rtex` command line.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, SystemTime};

fn prog_name() -> String {
    std::env::args_os()
        .next()
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("rtex")
        .to_owned()
}

use clap::{Args, Parser, Subcommand, ValueEnum};
use rtex_core::{Diagnostic, History, Interaction};

use crate::compile::{
    CompileOptions, CompileReport, EngineKind, RenderOptions, Rendering, build_format, compile, render,
};
use crate::epub;
use crate::paper::Paper;
use crate::texfs::TexFs;

/// Typesets TeX documents into PDF and EPUB.
#[derive(Debug, Parser)]
#[command(
    name = "rtex",
    version,
    about,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    build: BuildArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Typeset a TeX file into PDF (the default command).
    Build(BuildArgs),
    /// Parse and typeset without writing files, reporting diagnostics only.
    Check(CheckArgs),
    /// Build and inspect format images.
    Format {
        /// What to do with the format.
        #[command(subcommand)]
        command: FormatCommand,
    },
    /// Rebuild whenever the input or a `.tex` file beside it changes.
    Watch(BuildArgs),
    /// Rasterize a page of a PDF into a PNG or JPEG image.
    Rasterize(RasterizeArgs),
}

#[derive(Debug, Subcommand)]
enum FormatCommand {
    /// Dump a primed engine as a format image.
    Build(FormatBuildArgs),
}

#[derive(Debug, Args)]
struct FormatBuildArgs {
    /// The format image to write.
    output: PathBuf,
    /// Files to `\input` before `\dump` [default: plain].
    #[arg(long = "input")]
    inputs: Vec<String>,
    /// The engine dialect.
    #[arg(long, value_enum, default_value_t = EngineArg::Pdftex)]
    engine: EngineArg,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// The TeX file to check.
    #[arg(required = true)]
    input: Option<PathBuf>,
    /// Run as INITEX, without loading plain TeX first.
    #[arg(long)]
    ini: bool,
    /// The engine dialect.
    #[arg(long, value_enum, default_value_t = EngineArg::Pdftex)]
    engine: EngineArg,
    /// How TeX stops for errors.
    #[arg(long, value_enum, default_value_t = InteractionArg::Nonstopmode)]
    interaction: InteractionArg,
    /// Print a JSON report with every diagnostic.
    #[arg(long)]
    json: bool,
    /// Do not echo the transcript.
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum EngineArg {
    /// pdfTeX with 8-bit input (the default).
    #[value(name = "pdftex")]
    Pdftex,
    /// XeTeX with Unicode input.
    #[value(name = "xetex")]
    Xetex,
    /// pdfLaTeX file lookup.
    #[value(name = "pdflatex")]
    Pdflatex,
    /// XeLaTeX file lookup.
    #[value(name = "xelatex")]
    Xelatex,
}

impl From<EngineArg> for EngineKind {
    fn from(value: EngineArg) -> Self {
        match value {
            EngineArg::Pdftex => EngineKind::PdfTeX,
            EngineArg::Xetex => EngineKind::XeTeX,
            EngineArg::Pdflatex => EngineKind::PdfLaTeX,
            EngineArg::Xelatex => EngineKind::XeLaTeX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum InteractionArg {
    /// No terminal output, no stops.
    #[value(name = "batchmode")]
    Batchmode,
    /// No stops.
    #[value(name = "nonstopmode")]
    Nonstopmode,
    /// No error stops.
    #[value(name = "scrollmode")]
    Scrollmode,
    /// Stop at every error.
    #[value(name = "errorstopmode")]
    Errorstopmode,
}

impl From<InteractionArg> for Interaction {
    fn from(value: InteractionArg) -> Self {
        match value {
            InteractionArg::Batchmode => Interaction::Batch,
            InteractionArg::Nonstopmode => Interaction::Nonstop,
            InteractionArg::Scrollmode => Interaction::Scroll,
            InteractionArg::Errorstopmode => Interaction::ErrorStop,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum EpubLayoutArg {
    /// Text reflows in the reader (the default).
    #[value(name = "reflow")]
    Reflow,
    /// One pre-paginated page per shipped page.
    #[value(name = "fixed")]
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageFormat {
    Png,
    Jpg,
}

impl ImageFormat {
    fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpg => "jpg",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OutputFormats {
    pdf: bool,
    epub: bool,
    image: Option<ImageFormat>,
}

impl OutputFormats {
    fn parse(value: &str) -> Option<Self> {
        let mut out = OutputFormats::default();
        for part in value.split(',') {
            match part.trim() {
                "pdf" => out.pdf = true,
                "epub" => out.epub = true,
                "png" => out.image = Some(ImageFormat::Png),
                "jpg" | "jpeg" => out.image = Some(ImageFormat::Jpg),
                _ => return None,
            }
        }
        if out.pdf || out.epub || out.image.is_some() {
            Some(out)
        } else {
            None
        }
    }

    fn pdf(self) -> bool {
        self.pdf
    }

    fn epub(self) -> bool {
        self.epub
    }

    fn image(self) -> Option<ImageFormat> {
        self.image
    }
}

#[derive(Clone, Debug, Args)]
struct BuildArgs {
    /// The TeX file to typeset.
    #[arg(required = true)]
    input: Option<PathBuf>,
    /// Where to write the PDF [default: <input name>.pdf next to the input].
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Paper size: a4 or letter.
    #[arg(long, default_value_t = Paper::A4, value_name = "SIZE")]
    paper: Paper,
    /// Run as INITEX, without loading plain TeX first.
    #[arg(long)]
    ini: bool,
    /// What to write: pdf, epub, or pdf,epub [default: pdf].
    #[arg(long, value_name = "FORMATS", default_value = "pdf")]
    to: String,
    /// EPUB layout: reflow or fixed.
    #[arg(long, value_enum, default_value_t = EpubLayoutArg::Reflow)]
    epub_layout: EpubLayoutArg,
    /// `dc:title` of the EPUB.
    #[arg(long)]
    title: Option<String>,
    /// `dc:creator` of the EPUB.
    #[arg(long)]
    author: Option<String>,
    /// `dc:language` of the EPUB.
    #[arg(long)]
    lang: Option<String>,
    /// `dc:identifier` of the EPUB.
    #[arg(long)]
    identifier: Option<String>,
    /// The engine dialect.
    #[arg(long, value_enum, default_value_t = EngineArg::Pdftex)]
    engine: EngineArg,
    /// A format image to start from.
    #[arg(long, value_name = "FILE")]
    format: Option<PathBuf>,
    /// How TeX stops for errors.
    #[arg(long, value_enum, default_value_t = InteractionArg::Nonstopmode)]
    interaction: InteractionArg,
    /// Overrides `\jobname` and the output names.
    #[arg(long)]
    jobname: Option<String>,
    /// Where auxiliary files and outputs go.
    #[arg(long, value_name = "DIR")]
    output_directory: Option<PathBuf>,
    /// The most passes to run while the auxiliary files change.
    #[arg(long, default_value_t = 5, value_name = "N")]
    max_passes: u8,
    /// Print how long each pass took.
    #[arg(long)]
    profile: bool,
    /// Print a JSON report with every diagnostic instead of the transcript.
    #[arg(long)]
    json: bool,
    /// Do not echo the transcript.
    #[arg(short, long)]
    quiet: bool,
    /// Page to rasterize when `--to` includes png/jpg (1-based).
    #[arg(long, default_value_t = 1, value_name = "N")]
    image_page: u32,
    /// Rasterization density in DPI when `--to` includes png/jpg.
    #[arg(long, default_value_t = 150, value_name = "DPI")]
    image_dpi: u32,
    /// Target raster width in pixels (keeps aspect when height omitted).
    #[arg(long, value_name = "PX")]
    image_width: Option<u32>,
    /// Target raster height in pixels (keeps aspect when width omitted).
    #[arg(long, value_name = "PX")]
    image_height: Option<u32>,
    /// JPEG quality (1-100) when writing jpg/jpeg.
    #[arg(long, default_value_t = 92, value_name = "N")]
    image_quality: u8,
    /// Where to write a raster image output.
    #[arg(long, value_name = "FILE")]
    image_output: Option<PathBuf>,
}

impl Cli {
    #[cfg(test)]
    fn into_build_args(self) -> BuildArgs {
        match self.command {
            Some(Command::Build(args)) | Some(Command::Watch(args)) => args,
            _ => self.build,
        }
    }
}

/// Parses the process arguments and runs the command.
pub fn main() -> ExitCode {
    run(Cli::parse())
}

#[derive(Debug, Args)]
struct RasterizeArgs {
    /// The PDF file to rasterize.
    input: PathBuf,
    /// Where to write the image [default: <input name>.<ext>].
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Image format: png or jpg/jpeg.
    #[arg(long, default_value = "png")]
    to: String,
    /// Page to rasterize (1-based).
    #[arg(long, default_value_t = 1, value_name = "N")]
    page: u32,
    /// Rasterization density in DPI.
    #[arg(long, default_value_t = 150, value_name = "DPI")]
    dpi: u32,
    /// Target width in pixels.
    #[arg(long, value_name = "PX")]
    width: Option<u32>,
    /// Target height in pixels.
    #[arg(long, value_name = "PX")]
    height: Option<u32>,
    /// JPEG quality (1-100).
    #[arg(long, default_value_t = 92, value_name = "N")]
    quality: u8,
}

/// Runs a parsed command line and returns the process exit code.
pub fn run(cli: Cli) -> ExitCode {
    match cli.command {
        Some(Command::Check(args)) => run_check(&args),
        Some(Command::Format {
            command: FormatCommand::Build(args),
        }) => run_format_build(&args),
        Some(Command::Watch(mut args)) => {
            args.quiet = false;
            run_watch(&args)
        }
        Some(Command::Rasterize(args)) => run_rasterize(&args),
        Some(Command::Build(args)) => run_build(&args),
        None => run_build(&cli.build),
    }
}

fn run_rasterize(args: &RasterizeArgs) -> ExitCode {
    let format = match args.to.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpg,
        _ => {
            eprintln!("{}: --to takes png or jpg/jpeg", prog_name());
            return ExitCode::from(2);
        }
    };
    if !args.input.is_file() {
        eprintln!("{}: input file not found: {}", prog_name(), args.input.display());
        return ExitCode::from(2);
    }
    let out = args
        .output
        .clone()
        .unwrap_or_else(|| args.input.with_extension(format.extension()));
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("{}: cannot write {}: {e}", prog_name(), out.display());
            return ExitCode::from(2);
        }
    }
    let page = args.page.max(1).to_string();
    let dpi = args.dpi.max(1).to_string();
    let quality = args.quality.clamp(1, 100).to_string();
    let Some(tool) = find_raster_tool() else {
        eprintln!(
            "{}: no PDF rasterizer found (need pdftoppm, ImageMagick, or sips)",
            prog_name()
        );
        return ExitCode::from(2);
    };
    let status = match tool {
        "pdftoppm" => {
            let out_base = out.with_extension("");
            let mut cmd = std::process::Command::new("pdftoppm");
            cmd.arg("-f")
                .arg(&page)
                .arg("-l")
                .arg(&page)
                .arg("-singlefile")
                .arg("-r")
                .arg(&dpi);
            if let Some(w) = args.width {
                cmd.arg("-scale-to-x").arg(w.to_string());
            }
            if let Some(h) = args.height {
                cmd.arg("-scale-to-y").arg(h.to_string());
            }
            match format {
                ImageFormat::Png => {
                    cmd.arg("-png");
                }
                ImageFormat::Jpg => {
                    cmd.arg("-jpeg").arg("-jpegopt").arg(format!("quality={quality}"));
                }
            }
            cmd.arg(&args.input).arg(&out_base);
            match cmd.status() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{}: cannot run pdftoppm: {e}", prog_name());
                    return ExitCode::from(2);
                }
            }
        }
        "magick" | "convert" => {
            let mut cmd = std::process::Command::new(tool);
            cmd.arg("-density")
                .arg(&dpi)
                .arg(format!("{}[{}]", args.input.display(), args.page.saturating_sub(1)));
            cmd.arg("-background")
                .arg("white")
                .arg("-alpha")
                .arg("remove")
                .arg("-alpha")
                .arg("off")
                .arg("-flatten");
            if let (Some(w), Some(h)) = (args.width, args.height) {
                cmd.arg("-resize").arg(format!("{w}x{h}!"));
            } else if let Some(w) = args.width {
                cmd.arg("-resize").arg(format!("{w}x"));
            } else if let Some(h) = args.height {
                cmd.arg("-resize").arg(format!("x{h}"));
            }
            if matches!(format, ImageFormat::Jpg) {
                cmd.arg("-quality").arg(&quality);
            }
            cmd.arg(&out);
            match cmd.status() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{}: cannot run {tool}: {e}", prog_name());
                    return ExitCode::from(2);
                }
            }
        }
        "sips" => {
            let mut cmd = std::process::Command::new("sips");
            cmd.arg("-s").arg("format").arg(match format {
                ImageFormat::Png => "png",
                ImageFormat::Jpg => "jpeg",
            });
            if matches!(format, ImageFormat::Jpg) {
                cmd.arg("-s").arg("formatOptions").arg(&quality);
            }
            if let Some(w) = args.width {
                cmd.arg("--resampleWidth").arg(w.to_string());
            } else if let Some(h) = args.height {
                cmd.arg("--resampleHeight").arg(h.to_string());
            }
            cmd.arg(&args.input).arg("--out").arg(&out);
            match cmd.status() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{}: cannot run sips: {e}", prog_name());
                    return ExitCode::from(2);
                }
            }
        }
        _ => {
            eprintln!("{}: unsupported rasterizer", prog_name());
            return ExitCode::from(2);
        }
    };
    if !status.success() {
        eprintln!("{}: rasterizing {} failed", prog_name(), args.input.display());
        return ExitCode::from(2);
    }
    if !out.exists() {
        let alt = match format {
            ImageFormat::Jpg => out.with_extension("jpeg"),
            ImageFormat::Png => out.clone(),
        };
        if alt.exists() {
            if let Err(e) = fs::rename(&alt, &out) {
                eprintln!("{}: cannot move {}: {e}", prog_name(), alt.display());
                return ExitCode::from(2);
            }
        } else {
            eprintln!("{}: image was not generated: {}", prog_name(), out.display());
            return ExitCode::from(2);
        }
    }
    println!("{}: Image written on {}.", prog_name(), out.display());
    ExitCode::SUCCESS
}

fn run_build(args: &BuildArgs) -> ExitCode {
    let Some(input) = args.input.clone() else {
        eprintln!("{}: missing input file", prog_name());
        return ExitCode::from(2);
    };
    let Some(formats) = OutputFormats::parse(&args.to) else {
        eprintln!(
            "{}: --to takes pdf, epub, png, jpg/jpeg or a comma-separated combination",
            prog_name()
        );
        return ExitCode::from(2);
    };
    let format = match &args.format {
        Some(path) => match fs::read(path) {
            Ok(data) => Some(data),
            Err(error) => {
                eprintln!("{}: cannot read {}: {error}", prog_name(), path.display());
                return ExitCode::from(2);
            }
        },
        None => None,
    };
    let engine: EngineKind = args.engine.into();
    let need_pdf = formats.pdf() || formats.image().is_some();
    let options = CompileOptions {
        input,
        output: args.output.clone(),
        paper: args.paper,
        preload: if args.ini || format.is_some() {
            Vec::new()
        } else {
            engine.default_preload()
        },
        engine,
        interaction: args.interaction.into(),
        job_name: args.jobname.clone(),
        output_directory: args.output_directory.clone(),
        format,
        max_passes: usize::from(args.max_passes.max(1)),
        pdf: need_pdf,
    };
    match compile(&options) {
        Ok(mut report) => {
            let epub_path = if formats.epub() {
                match write_epub(args, &report) {
                    Ok(path) => path,
                    Err(error) => {
                        eprintln!("{}: {error}", prog_name());
                        return ExitCode::from(2);
                    }
                }
            } else {
                None
            };
            let image_path = if let Some(image_format) = formats.image() {
                match write_raster_image(args, &report, image_format) {
                    Ok(path) => path,
                    Err(error) => {
                        eprintln!("{}: {error}", prog_name());
                        return ExitCode::from(2);
                    }
                }
            } else {
                None
            };
            if !formats.pdf() {
                if let Some(pdf_path) = report.pdf_path.take() {
                    let _ = fs::remove_file(pdf_path);
                }
            }
            if formats.pdf() || formats.epub() {
                print_report(args, &report, epub_path.as_deref(), image_path.as_deref());
            } else {
                print_aux_only(args, &report, image_path.as_deref());
            }
            exit_code(report.rendering.history)
        }
        Err(error) => {
            eprintln!("{}: {error}", prog_name());
            ExitCode::from(2)
        }
    }
}

fn run_check(args: &CheckArgs) -> ExitCode {
    let Some(input) = args.input.clone() else {
        eprintln!("{}: missing input file", prog_name());
        return ExitCode::from(2);
    };
    let Some(file_name) = input.file_name().and_then(|n| n.to_str()) else {
        eprintln!(
            "{}: input path has no usable file name: {}",
            prog_name(),
            input.display()
        );
        return ExitCode::from(2);
    };
    let directory = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let engine = EngineKind::from(args.engine);
    let texfs = TexFs::new(directory, directory, engine.program());
    let options = RenderOptions {
        paper: Paper::A4,
        output_name: Some(format!("{file_name}.pdf")),
        preload: if args.ini { Vec::new() } else { engine.default_preload() },
        engine,
        interaction: args.interaction.into(),
        format: None,
    };
    let rendering = render(Box::new(texfs), file_name, &options);
    if args.json {
        println!("{}", check_json(&rendering));
    } else if !args.quiet {
        print_diagnostics(&rendering.diagnostics);
        let _ = io::stdout().write_all(&rendering.terminal);
    }
    exit_code(rendering.history)
}

fn run_format_build(args: &FormatBuildArgs) -> ExitCode {
    let engine: EngineKind = args.engine.into();
    match build_format(Path::new("."), &args.inputs, engine) {
        Ok(image) => match fs::write(&args.output, image) {
            Ok(()) => {
                println!("{}: Wrote {}", prog_name(), args.output.display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{}: cannot write {}: {error}", prog_name(), args.output.display());
                ExitCode::from(2)
            }
        },
        Err(error) => {
            eprintln!("{}: {error}", prog_name());
            ExitCode::from(2)
        }
    }
}

fn run_watch(args: &BuildArgs) -> ExitCode {
    let Some(input) = args.input.clone() else {
        eprintln!("{}: missing input file", prog_name());
        return ExitCode::from(2);
    };
    let mut stamp = newest_tex(&input);
    loop {
        let code = run_build(args);
        println!("{}: Watching {} (Ctrl-C to stop)", prog_name(), input.display());
        loop {
            thread::sleep(Duration::from_millis(300));
            let next = newest_tex(&input);
            if next != stamp {
                stamp = next;
                break;
            }
            if code != ExitCode::SUCCESS {
                break;
            }
        }
    }
}

fn newest_tex(input: &Path) -> SystemTime {
    let mut newest = fs::metadata(input)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if let Some(directory) = input.parent()
        && let Ok(entries) = fs::read_dir(directory)
    {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "tex")
                && let Ok(modified) = entry.metadata().and_then(|m| m.modified())
            {
                newest = newest.max(modified);
            }
        }
    }
    newest
}

fn write_epub(args: &BuildArgs, report: &CompileReport) -> Result<Option<PathBuf>, String> {
    if report.rendering.pages == 0 {
        return Ok(None);
    }
    let base = report
        .pdf_path
        .clone()
        .unwrap_or_else(|| report.log_path.with_extension("epub"));
    let path = base.with_extension("epub");
    let job = args.jobname.clone().unwrap_or_else(|| {
        args.input
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|s| s.to_str())
            .unwrap_or("document")
            .to_owned()
    });
    let options = epub::Options {
        layout: match args.epub_layout {
            EpubLayoutArg::Reflow => epub::Layout::Reflow,
            EpubLayoutArg::Fixed => epub::Layout::Fixed,
        },
        metadata: epub::Metadata {
            title: args.title.clone().unwrap_or_else(|| job.clone()),
            author: args.author.clone(),
            language: args.lang.clone().unwrap_or_else(|| "en".to_owned()),
            identifier: args.identifier.clone().unwrap_or_else(|| format!("urn:rtex:{job}")),
        },
    };
    let book = epub::build(&report.rendering.document, &options);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    fs::write(&path, book).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(Some(path))
}

fn raster_output_path(args: &BuildArgs, report: &CompileReport, format: ImageFormat) -> PathBuf {
    if let Some(path) = &args.image_output {
        return path.clone();
    }
    let base = report
        .pdf_path
        .clone()
        .unwrap_or_else(|| report.log_path.with_extension(format.extension()));
    base.with_extension(format.extension())
}

fn find_raster_tool() -> Option<&'static str> {
    for tool in ["pdftoppm", "magick", "convert", "sips"] {
        if std::process::Command::new("sh")
            .arg("-lc")
            .arg(format!("command -v {tool} >/dev/null 2>&1"))
            .status()
            .is_ok_and(|s| s.success())
        {
            return Some(tool);
        }
    }
    None
}

fn write_raster_image(
    args: &BuildArgs,
    report: &CompileReport,
    format: ImageFormat,
) -> Result<Option<PathBuf>, String> {
    if report.rendering.pages == 0 {
        return Ok(None);
    }
    let pdf_path = report
        .pdf_path
        .as_ref()
        .ok_or_else(|| "no PDF was produced for rasterization".to_owned())?;
    let out = raster_output_path(args, report, format);
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    }
    let page = args.image_page.max(1).to_string();
    let dpi = args.image_dpi.max(1).to_string();
    let quality = args.image_quality.clamp(1, 100).to_string();
    let tool =
        find_raster_tool().ok_or_else(|| "no PDF rasterizer found (need pdftoppm, ImageMagick, or sips)".to_owned())?;

    let status = match tool {
        "pdftoppm" => {
            let out_base = out.with_extension("");
            let mut cmd = std::process::Command::new("pdftoppm");
            cmd.arg("-f")
                .arg(&page)
                .arg("-l")
                .arg(&page)
                .arg("-singlefile")
                .arg("-r")
                .arg(&dpi);
            if let Some(w) = args.image_width {
                cmd.arg("-scale-to-x").arg(w.to_string());
            }
            if let Some(h) = args.image_height {
                cmd.arg("-scale-to-y").arg(h.to_string());
            }
            match format {
                ImageFormat::Png => {
                    cmd.arg("-png");
                }
                ImageFormat::Jpg => {
                    cmd.arg("-jpeg").arg("-jpegopt").arg(format!("quality={quality}"));
                }
            }
            cmd.arg(pdf_path).arg(&out_base);
            cmd.status().map_err(|e| format!("cannot run pdftoppm: {e}"))?
        }
        "magick" | "convert" => {
            let mut cmd = std::process::Command::new(tool);
            cmd.arg("-density")
                .arg(&dpi)
                .arg(format!("{}[{}]", pdf_path.display(), args.image_page.saturating_sub(1)));
            cmd.arg("-background")
                .arg("white")
                .arg("-alpha")
                .arg("remove")
                .arg("-alpha")
                .arg("off")
                .arg("-flatten");
            if let (Some(w), Some(h)) = (args.image_width, args.image_height) {
                cmd.arg("-resize").arg(format!("{w}x{h}!"));
            } else if let Some(w) = args.image_width {
                cmd.arg("-resize").arg(format!("{w}x"));
            } else if let Some(h) = args.image_height {
                cmd.arg("-resize").arg(format!("x{h}"));
            }
            if matches!(format, ImageFormat::Jpg) {
                cmd.arg("-quality").arg(&quality);
            }
            cmd.arg(&out);
            cmd.status().map_err(|e| format!("cannot run {tool}: {e}"))?
        }
        "sips" => {
            // sips works well on macOS for basic conversion; render via pdf first page by converting the pdf file.
            let mut cmd = std::process::Command::new("sips");
            cmd.arg("-s").arg("format").arg(match format {
                ImageFormat::Png => "png",
                ImageFormat::Jpg => "jpeg",
            });
            if matches!(format, ImageFormat::Jpg) {
                cmd.arg("-s").arg("formatOptions").arg(&quality);
            }
            if let Some(w) = args.image_width {
                cmd.arg("--resampleWidth").arg(w.to_string());
            } else if let Some(h) = args.image_height {
                cmd.arg("--resampleHeight").arg(h.to_string());
            }
            cmd.arg(pdf_path).arg("--out").arg(&out);
            cmd.status().map_err(|e| format!("cannot run sips: {e}"))?
        }
        _ => return Err("unsupported rasterizer".to_owned()),
    };

    if !status.success() {
        return Err(format!("rasterizing {} failed", pdf_path.display()));
    }
    if !out.exists() {
        // pdftoppm with certain versions/extensions may name differently; try common variant
        let alt = match format {
            ImageFormat::Jpg => out.with_extension("jpeg"),
            ImageFormat::Png => out.clone(),
        };
        if alt.exists() {
            fs::rename(&alt, &out).map_err(|e| format!("cannot move {}: {e}", alt.display()))?;
        } else {
            return Err(format!("image was not generated: {}", out.display()));
        }
    }
    Ok(Some(out))
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!(
            "{:?}: {}:{}: {}",
            diagnostic.severity,
            diagnostic.file.as_deref().unwrap_or("-"),
            diagnostic.line,
            diagnostic.message
        );
    }
}

fn print_report(args: &BuildArgs, report: &CompileReport, epub: Option<&Path>, image: Option<&Path>) {
    let mut stdout = io::stdout().lock();
    let written = if args.json {
        writeln!(stdout, "{}", json_report(report, epub, image))
    } else if !args.quiet {
        stdout
            .write_all(&report.rendering.terminal)
            .and_then(|_| writeln!(stdout))
    } else {
        Ok(())
    };
    if let Err(error) = written {
        eprintln!("{}: cannot write to standard output: {error}", prog_name());
    }
    if !args.json && !args.quiet {
        if let Some(path) = image {
            let _ = writeln!(stdout, "{}: Image written on {}.", prog_name(), path.display());
        }
    }
    if args.profile && !args.json {
        print_profile(report);
    }
}

fn print_aux_only(args: &BuildArgs, report: &CompileReport, image: Option<&Path>) {
    if args.json {
        println!("{}", json_report(report, None, image));
    } else if let Some(path) = image {
        println!("{}: Image written on {}.", prog_name(), path.display());
    }
    if args.profile && !args.json {
        print_profile(report);
    }
}

fn print_profile(report: &CompileReport) {
    for (index, time) in report.rendering.pass_times.iter().enumerate() {
        eprintln!(
            "{}: pass {}: {:.1} ms",
            prog_name(),
            index + 1,
            time.as_secs_f64() * 1000.0
        );
    }
    let total: f64 = report.rendering.pass_times.iter().map(|t| t.as_secs_f64()).sum();
    eprintln!(
        "{}: {} pass(es): {:.1} ms",
        prog_name(),
        report.rendering.passes,
        total * 1000.0
    );
}

fn exit_code(history: History) -> ExitCode {
    match history {
        History::Spotless | History::WarningIssued => ExitCode::SUCCESS,
        History::ErrorMessageIssued => ExitCode::from(1),
        History::FatalErrorStop => ExitCode::from(2),
    }
}

fn history_name(history: History) -> &'static str {
    match history {
        History::Spotless => "spotless",
        History::WarningIssued => "warning",
        History::ErrorMessageIssued => "error",
        History::FatalErrorStop => "fatal",
    }
}

fn json_report(report: &CompileReport, epub: Option<&Path>, image: Option<&Path>) -> String {
    let pdf = report
        .pdf_path
        .as_ref()
        .map_or_else(|| "null".to_owned(), |p| json_string(&p.display().to_string()));
    let epub = epub.map_or_else(|| "null".to_owned(), |p| json_string(&p.display().to_string()));
    let image = image.map_or_else(|| "null".to_owned(), |p| json_string(&p.display().to_string()));
    let diagnostics: Vec<String> = report.rendering.diagnostics.iter().map(Diagnostic::to_json).collect();
    let passes: Vec<String> = report
        .rendering
        .pass_times
        .iter()
        .map(|t| format!("{:.1}", t.as_secs_f64() * 1000.0))
        .collect();
    format!(
        "{{\"pdf\":{pdf},\"log\":{},\"epub\":{epub},\"image\":{image},\"pages\":{},\"passes\":{},\"pass_ms\":[{}],\"history\":\"{}\",\"diagnostics\":[{}]}}",
        json_string(&report.log_path.display().to_string()),
        report.rendering.pages,
        report.rendering.passes,
        passes.join(","),
        history_name(report.rendering.history),
        diagnostics.join(",")
    )
}

fn check_json(rendering: &Rendering) -> String {
    let diagnostics: Vec<String> = rendering.diagnostics.iter().map(Diagnostic::to_json).collect();
    format!(
        "{{\"history\":\"{}\",\"diagnostics\":[{}]}}",
        history_name(rendering.history),
        diagnostics.join(",")
    )
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<BuildArgs, clap::Error> {
        Cli::try_parse_from(args).map(Cli::into_build_args)
    }

    #[test]
    fn command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn input_alone_means_build() {
        let args = parse(&["rtex", "doc.tex"]).unwrap();
        assert_eq!(args.input, Some(PathBuf::from("doc.tex")));
        assert_eq!(args.paper, Paper::A4);
        assert_eq!(args.to, "pdf");
    }

    #[test]
    fn build_subcommand_takes_options() {
        let args = parse(&[
            "rtex",
            "build",
            "doc.tex",
            "-o",
            "out/doc.pdf",
            "--paper",
            "letter",
            "--json",
            "--engine",
            "xetex",
            "--interaction",
            "batchmode",
            "--jobname",
            "book",
            "--output-directory",
            "out",
            "--to",
            "pdf,epub",
            "--epub-layout",
            "fixed",
            "--profile",
        ])
        .unwrap();
        assert_eq!(args.output, Some(PathBuf::from("out/doc.pdf")));
        assert_eq!(args.paper, Paper::Letter);
        assert_eq!(args.engine, EngineArg::Xetex);
        assert_eq!(args.interaction, InteractionArg::Batchmode);
        assert_eq!(args.jobname.as_deref(), Some("book"));
        assert_eq!(args.output_directory, Some(PathBuf::from("out")));
        assert_eq!(
            OutputFormats::parse(&args.to),
            Some(OutputFormats {
                pdf: true,
                epub: true,
                image: None
            })
        );
        assert_eq!(args.epub_layout, EpubLayoutArg::Fixed);
        assert!(args.json);
        assert!(args.profile);
    }

    #[test]
    fn input_is_required() {
        assert!(parse(&["rtex"]).is_err());
        assert!(parse(&["rtex", "build"]).is_err());
        assert!(parse(&["rtex", "doc.tex", "--paper", "a5"]).is_err());
    }

    #[test]
    fn output_formats_are_parsed() {
        assert_eq!(
            OutputFormats::parse("pdf"),
            Some(OutputFormats {
                pdf: true,
                epub: false,
                image: None
            })
        );
        assert_eq!(
            OutputFormats::parse("epub"),
            Some(OutputFormats {
                pdf: false,
                epub: true,
                image: None
            })
        );
        assert_eq!(
            OutputFormats::parse("pdf,epub"),
            Some(OutputFormats {
                pdf: true,
                epub: true,
                image: None
            })
        );
        assert_eq!(
            OutputFormats::parse("epub,pdf"),
            Some(OutputFormats {
                pdf: true,
                epub: true,
                image: None
            })
        );
        assert_eq!(OutputFormats::parse("docx"), None);
    }

    #[test]
    fn check_and_format_build_parse() {
        let Cli {
            command: Some(Command::Check(args)),
            ..
        } = Cli::try_parse_from(["rtex", "check", "doc.tex", "--json"]).unwrap()
        else {
            panic!("check parses");
        };
        assert_eq!(args.input, Some(PathBuf::from("doc.tex")));
        let Cli {
            command: Some(Command::Format {
                command: FormatCommand::Build(args),
            }),
            ..
        } = Cli::try_parse_from(["rtex", "format", "build", "plain.fmt"]).unwrap()
        else {
            panic!("format build parses");
        };
        assert_eq!(args.output, PathBuf::from("plain.fmt"));
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn exit_codes_follow_the_history() {
        assert_eq!(exit_code(History::WarningIssued), ExitCode::SUCCESS);
        assert_eq!(exit_code(History::ErrorMessageIssued), ExitCode::from(1));
        assert_eq!(exit_code(History::FatalErrorStop), ExitCode::from(2));
    }
}
