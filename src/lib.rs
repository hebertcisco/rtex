//! rtex: a TeX engine that turns plain-TeX-style documents into PDF.
//!
//! The mouth (tokenizing, expansion, assignments) is [`rtex_core`]; this
//! crate adds a port of TeX's stomach with pdfTeX's primitives, TeX Live file
//! lookup, fonts, images, a PDF writer and the `rtex` command line.
//!
//! ```
//! use rtex::{RenderOptions, render};
//! use rtex_core::MemoryFs;
//!
//! let fs = MemoryFs::new().with("hello.tex", r"\message{hello}\end");
//! let rendering = render(Box::new(fs), "hello.tex", &RenderOptions::default());
//! assert_eq!(rendering.pages, 0);
//! assert!(String::from_utf8_lossy(&rendering.log).contains("hello"));
//! ```

#![forbid(unsafe_code)]

pub mod cli;
mod compile;
pub mod document;
pub mod epub;
mod error;
pub mod fonts;
pub mod images;
pub mod kpse;
mod paper;
pub mod pdf;
pub mod tex;
pub mod texfs;
mod units;

pub use compile::{
    CompileOptions, CompileReport, EngineKind, LATEX_PRELOAD, PDFTEX_PRELOAD, RenderOptions, Rendering, build_format,
    compile, render,
};
pub use error::CompileError;
pub use paper::{PageSize, Paper, UnknownPaper};
