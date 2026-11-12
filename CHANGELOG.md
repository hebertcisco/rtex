# Changelog

All notable changes to rtex are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- EPUB 3 output: `--to pdf,epub`, `--epub-layout reflow|fixed`, and
  `--title`/`--author`/`--lang`/`--identifier`. Packaging is deterministic:
  stored `mimetype`, OCF container, OPF 3, `nav.xhtml`, `toc.ncx`, and one
  spine item per shipped page. The reflow layout reconstructs paragraphs,
  emphasis and monospace runs from the shipped pages; the fixed layout emits
  one SVG `<text>` page per shipped page and marks the book
  `rendition:layout pre-paginated`. Both layouts pass epubcheck 5.2.1 with no
  errors on the examples.
- `rtex check <file.tex>`: typeset without writing files, reporting TeX's
  diagnostics (or JSON with `--json`).
- `rtex format build <file.fmt>`: dump a primed engine as a format image, and
  `--format <file.fmt>` to start a job from it. The image carries the mouth's
  tables, the typesetter's fonts (name and size, with TFM metrics re-read at
  load time) and the hyphenation patterns.
- `rtex watch <file.tex>`: rebuild whenever the input or a `.tex` file beside
  it changes.
- Driver and options: `--engine pdftex|xetex|pdflatex|xelatex`,
  `--interaction`, `--jobname`, `--output-directory`, `--max-passes` and
  `--profile`. The driver reruns TeX until `.aux`/`.toc`/`.lof`/`.lot`/`.idx`/
  `.bbl` stop changing. `--json` now reports `passes`, `pass_ms` and `epub`.
- Font embedding: `pdftex.map` lookup, Type 1 PFB parsing, subsetting with
  deterministic tags and PDF `FontFile` embedding, so `pdffonts` shows
  embedded Type 1 subsets instead of base-14 substitutes.
- pdfTeX primitives for images, links, destinations, outlines and info, plus
  the extended `\pdf*` parameter set already covered by the tests.
- Integration tests for EPUB in both layouts, `check --json`, `format build`
  and a format round trip that restores fonts and hyphenation.

### Changed

- `CompileOptions` gained `engine`, `interaction`, `format` and `max_passes`;
  `Rendering` gained `document`, `passes` and `pass_times`. Both default, so
  existing literals keep working with `..Default::default()`.

### Fixed

- `\dump` now serializes the typesetter's fonts and hyphenation patterns, so
  a format loaded by a later run produces the same text instead of an empty
  page.

## [0.1.0] - 2026-11-11

### Added

- The `rtex` command: `rtex [build] <file.tex> [-o out.pdf] [--paper a4|letter]
  [--no-prelude] [--json] [--quiet]`, writing the PDF and the `.log` transcript,
  with exit codes 0 (clean or warnings), 1 (TeX errors) and 2 (fatal or I/O).
- A typesetting stomach for [rtex-core](https://crates.io/crates/rtex-core)
  0.1.0: characters with TeX's space factor, interword glue, `\hskip`,
  `\vskip`, kerns, penalties, rules, `\hbox` (natural, `to`, `spread`),
  `\indent`/`\noindent`, `\unskip`/`\unkern`/`\unpenalty`, first-fit line
  breaking with `\leftskip`/`\rightskip`/`\parfillskip`, and a page builder
  with `\baselineskip`, `\lineskip`, `\topskip`, `\parskip`, forced breaks and
  `\count0` page numbers.
- Overfull, underfull and tight box reports in TeX's wording.
- The twelve text faces of the standard PDF fonts (Times, Helvetica, Courier)
  with Adobe's metrics, WinAnsi encoding, Computer Modern names mapped onto
  them (`cmr10`, `cmbx12`, `cmtt10`, …), and Computer Modern's text
  ligatures: en and em dashes from `--` and `---`, curly double quotes from
  two grave accents or two apostrophes, and inverted `!` and `?`.
- UTF-8 input for every character WinAnsi covers.
- A PDF writer with Flate-compressed content streams.
- The rtex prelude, a small plain-TeX-like environment read through
  `\everyjob`: catcodes, fonts (`\rm`, `\bf`, `\it`, `\bi`, `\sf`, `\tt`),
  `\bye`, `\eject`, `\newpage`, skips, `\centerline` and friends,
  `\beginsection`, `\chapter` and `\title`.
- Library API: `render` (any `FileSystem`) and `compile` (files on disk).
- Examples (`examples/hello.tex`, `examples/book/`), unit, integration and
  command-line tests, and CI on Linux, macOS and Windows.

[Unreleased]: https://github.com/hebertcisco/rtex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hebertcisco/rtex/releases/tag/v0.1.0
