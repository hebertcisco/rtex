# rtex

[![CI](https://github.com/hebertcisco/rtex/actions/workflows/ci.yml/badge.svg)](https://github.com/hebertcisco/rtex/actions/workflows/ci.yml)

A TeX engine in safe Rust that turns `.tex` files into PDF and EPUB 3.

rtex reads TeX with [rtex-core](https://crates.io/crates/rtex-core), a
faithful implementation of the mouth of Knuth's TeX (tokenizer, macro
expansion, conditionals, registers, grouping, TeX-format transcripts). On top
of it this crate adds a port of TeX's stomach (every mode, Knuth–Plass line
breaking, hyphenation, alignments, the page builder and output routines, and
math), TeX Live file lookup, TFM and Type 1 fonts from `pdftex.map`, a PDF
writer, EPUB 3 output and the `rtex` command line. Its transcripts match
`pdftex -ini -etex` byte for byte on the test suite.

```text
$ rtex examples/hello.tex
(./hello.tex (/usr/local/texlive/2026basic/texmf-dist/tex/plain/base/plain.tex
...
[1] )
Output written on examples/hello.pdf (1 page, 3778 bytes).
Transcript written on hello.log.
```

## Install

### Homebrew

On macOS and Linux, install from the
[hebertcisco/rtex](https://github.com/hebertcisco/homebrew-rtex) tap:

```text
brew tap hebertcisco/rtex
brew install rtex
```

The formula builds `rtex` from the release's source archive. Check the
installation with `rtex --version`, and upgrade with:

```text
brew update
brew upgrade rtex
```

### Cargo

```text
cargo install --git https://github.com/hebertcisco/rtex
```

The crate needs Rust 1.88 or newer. It is not published on crates.io: the name
`rtex` belongs to another project there.

## Usage

```text
rtex [build] <file.tex> [options]
rtex check <file.tex> [options]        # typeset without writing files
rtex format build <file.fmt>           # dump a primed engine, plain TeX by default
rtex watch <file.tex> [options]        # rebuild when a .tex file changes

  -o, --output <FILE>       where to write the PDF [default: <input name>.pdf next to the input]
      --paper <SIZE>        a4 (default) or letter
      --ini                 run as INITEX, without loading plain TeX first
      --to <FORMATS>        pdf (default), epub, or pdf,epub
      --epub-layout <L>     reflow (default) or fixed
      --title/--author/--lang/--identifier   EPUB metadata
      --engine <NAME>       pdftex (default), xetex, pdflatex or xelatex
      --format <FILE>       start from a format image
      --interaction <MODE>  batchmode, nonstopmode (default), scrollmode or errorstopmode
      --jobname <NAME>      override \jobname and the output names
      --output-directory <DIR>  where auxiliary files and outputs go
      --max-passes <N>      reruns while the auxiliary files change [default: 5]
      --profile             print the wall time of every pass
      --json                print a JSON report with every diagnostic instead of the transcript
  -q, --quiet               do not echo the transcript
```

`--to epub` writes a reflowable EPUB by default and a pre-paginated book with
`--epub-layout fixed`, one SVG page per shipped page. Both layouts pass
epubcheck 5.2.1 with zero errors on `examples/hello.tex` and
`examples/book/book.tex`.

The transcript is written next to the PDF with the extension `.log`. The exit
code is 0 when the run was clean or only produced warnings, 1 when TeX
reported errors (the PDF is still written, as TeX does), and 2 for a fatal
error, a missing input or an I/O failure.

`--json` prints one object:

```json
{"pdf":"doc.pdf","log":"doc.log","epub":null,"pages":1,"passes":1,"pass_ms":["18.4"],"history":"error","diagnostics":[{"severity":"error","message":"Undefined control sequence","file":"./doc.tex","line":1,"column":21,"help":["..."],"expansion":[]}]}
```

### As a library

```rust
use rtex::{RenderOptions, render};
use rtex_core::MemoryFs;

let fs = MemoryFs::new().with("hello.tex", r"Hello, world! \bye");
let rendering = render(Box::new(fs), "hello.tex", &RenderOptions::default());
assert_eq!(rendering.pages, 1);
let pdf: Vec<u8> = rendering.pdf.unwrap();
```

`rtex::compile` does the same for files on disk and writes the results.

## Writing documents

Documents are plain TeX. Unless `--ini` is given, rtex reads `plain.tex`
before the document (through `\everyjob`, so `\jobname` stays the
document's name). Input files, fonts and hyphenation patterns come from the
installed TeX Live, found through its `ls-R` databases as kpathsea finds them.

```tex
\centerline{\bf Notes}
\beginsection Introduction\par
TeX's ``space factor'' puts a little more space after a sentence,
and $\sum_{k=1}^n k = {n(n+1)\over2}$ is set as TeX sets it.
\bye
```

See [examples/hello.tex](examples/hello.tex) and the multi-file
[examples/book/](examples/book/book.tex).

### Fonts

`\font` loads TFM files from TeX Live, scaled exactly as TeX scales them, with
ligatures, kerns and the extensible recipes math needs. The PDF writer
follows `pdftex.map`: it finds each font's Type 1 program, subclasses the map's
encoding subset and embeds a subset with deterministic six-letter tags.
`pdffonts` reports the result as an embedded, subset Type 1 font. ToUnicode
CMaps and OpenType/CFF programs are not written yet.

## What the typesetter supports

A port of `tex.web` with e-TeX's and pdfTeX's changes, checked against
`pdftex -ini -etex` transcripts (`tests/golden/`):

- Every mode, `\everypar`/`\everyhbox`/`\everyvbox`/`\everymath`/`\everydisplay`,
  characters with ligatures and kerns, `\accent`, `\discretionary`, leaders,
  `\vadjust`, `\insert`, `\mark` and e-TeX's `\marks`, whatsits.
- Boxes: `\hbox`, `\vbox`, `\vtop`, box registers with `\setbox`, `\box`,
  `\copy`, `\unhbox`, `\unvbox`, `\lastbox`, `\vsplit`, `\wd`/`\ht`/`\dp`,
  `\raise`, `\lower`, `\moveleft`, `\moveright`.
- Knuth–Plass line breaking with `\parshape`, `\hangindent`, `\looseness`,
  `\emergencystretch` and e-TeX's penalty arrays; hyphenation with
  `\patterns`, `\hyphenation` and languages.
- `\halign` and `\valign`, including alignments in displays.
- The page builder with inserts, marks, `\output`, `\holdinginserts` and
  `\vsplit`.
- Math: families, sub- and superscripts, fractions, radicals, `\left`,
  `\middle` and `\right`, `\mathchoice`, `\vcenter`, accents, limits,
  displays with `\eqno` and `\leqno`.
- Diagnostics: `\showbox`, `\showlists`, `\tracingparagraphs`,
  `\tracingpages`, `\tracingoutput`, `\tracinglostchars`.

## Not implemented yet

Documented deviations, kept here and in
[rtex-core's COMPAT.md](../rtex-core/docs/COMPAT.md), with milestone
evidence in [docs/STATUS.md](docs/STATUS.md):

- OpenType and CFF fonts, virtual fonts, and `\font\x="Name:features"`; the
  engine reads TFM metrics and embeds Type 1 subsets only.
- XeTeX's Unicode text mode: `--engine xetex` selects Unicode input and
  XeTeX's search paths, but text is still set through 8-bit TFM fonts.
- LaTeX formats (`latex.ltx`), TRIP and etrip.
- Unicode math through OpenType MATH tables; math is set with the classic
  Computer Modern families.
- EPUB semantics beyond the text: MathML, footnotes as `epub:type`, tables,
  images and embedded book fonts. The reflow layout reconstructs paragraphs
  from the shipped pages; the fixed layout positions text as SVG `<text>`
  instead of glyph outlines.
- Images through `\pdfximage` and PDF inclusion beyond the implemented
  helpers; link annotations and outlines are written by the `\pdfstartlink`,
  `\pdfoutline` and `\pdfdest` primitives.
- e-TeX's `\lastlinefit`, TeXXeT (`\TeXXeTstate`, `\beginL`...) and
  `\predisplaydirection` other than zero; `\savinghyphcodes`.

## Development

```text
cargo test                                   # unit, integration, CLI and doc tests
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo run -- examples/book/book.tex          # writes examples/book/book.pdf
```

## Relation to TeX

TeX is a trademark of the American Mathematical Society, and Knuth asks that
modified versions of his program not be called TeX. rtex is an independent
reimplementation of the algorithms described in *TeX: The Program*; it is not
TeX and does not claim to be.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
