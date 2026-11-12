//! Finding TeX Live files the way kpathsea does: TEXMF trees, `ls-R` databases and search paths.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What kind of file is wanted; each kind has its own subtrees, like kpathsea's formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// TeX input (`.tex`, `.sty`, `.cls`, …).
    Tex,
    /// TeX font metrics.
    Tfm,
    /// Virtual fonts.
    Vf,
    /// Type 1 font programs (`.pfb`).
    Type1,
    /// PostScript encoding vectors (`.enc`).
    Enc,
    /// Font map files (`pdftex.map`).
    Map,
    /// OpenType fonts.
    OpenType,
    /// TrueType fonts.
    TrueType,
    /// Adobe font metrics.
    Afm,
    /// Images for `\includegraphics` and `\pdfximage`.
    Image,
}

/// Which format's input paths to use, as `-progname` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Program {
    /// `pdftex` with plain TeX.
    #[default]
    PlainTex,
    /// `pdflatex`.
    LaTeX,
    /// `xetex` with plain TeX.
    XeTeX,
    /// `xelatex`.
    XeLaTeX,
}

impl Program {
    fn tex_subtrees(self) -> &'static [&'static str] {
        match self {
            Program::PlainTex => &["tex/plain/", "tex/generic/", "tex/"],
            Program::LaTeX => &["tex/latex/", "tex/generic/", "tex/"],
            Program::XeTeX => &["tex/xetex/", "tex/plain/", "tex/generic/", "tex/"],
            Program::XeLaTeX => &["tex/xelatex/", "tex/xetex/", "tex/latex/", "tex/generic/", "tex/"],
        }
    }
}

impl FileKind {
    fn subtrees(self, program: Program) -> &'static [&'static str] {
        match self {
            FileKind::Tex | FileKind::Image => program.tex_subtrees(),
            FileKind::Tfm => &["fonts/tfm/"],
            FileKind::Vf => &["fonts/vf/"],
            FileKind::Type1 => &["fonts/type1/"],
            FileKind::Enc => &["fonts/enc/pdftex/", "fonts/enc/dvips/", "fonts/enc/"],
            FileKind::Map => &["fonts/map/pdftex/", "fonts/map/dvips/", "fonts/map/"],
            FileKind::OpenType => &["fonts/opentype/"],
            FileKind::TrueType => &["fonts/truetype/"],
            FileKind::Afm => &["fonts/afm/"],
        }
    }

    fn suffixes(self) -> &'static [&'static str] {
        match self {
            FileKind::Tex => &[".tex"],
            FileKind::Tfm => &[".tfm"],
            FileKind::Vf => &[".vf"],
            FileKind::Type1 => &[".pfb", ".pfa"],
            FileKind::Enc => &[".enc"],
            FileKind::Map => &[".map"],
            FileKind::OpenType => &[".otf"],
            FileKind::TrueType => &[".ttf", ".ttc"],
            FileKind::Afm => &[".afm"],
            FileKind::Image => &[],
        }
    }
}

#[derive(Debug)]
struct Tree {
    root: PathBuf,
    index: HashMap<String, Vec<String>>,
}

impl Tree {
    fn load(root: PathBuf) -> Option<Tree> {
        if !root.is_dir() {
            return None;
        }
        let index = match fs::read_to_string(root.join("ls-R")) {
            Ok(db) => parse_ls_r(&db),
            Err(_) => walk(&root),
        };
        Some(Tree { root, index })
    }

    fn find(&self, file: &str, subtree: &str) -> Option<PathBuf> {
        let dirs = self.index.get(file)?;
        dirs.iter()
            .find(|d| format!("{d}/").starts_with(subtree))
            .map(|d| self.root.join(d).join(file))
            .filter(|p| p.is_file())
    }
}

fn parse_ls_r(db: &str) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();
    let mut dir = String::new();
    for line in db.lines() {
        if line.is_empty() || line.starts_with('%') {
            continue;
        }
        if let Some(d) = line.strip_suffix(':') {
            dir = d.trim_start_matches("./").trim_end_matches('/').to_owned();
            continue;
        }
        index.entry(line.to_owned()).or_default().push(dir.clone());
    }
    for dirs in index.values_mut() {
        dirs.sort();
    }
    index
}

fn walk(root: &Path) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(entries) = fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = rel.join(&name);
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path);
            } else {
                let dir = rel.to_string_lossy().replace('\\', "/");
                index.entry(name).or_default().push(dir);
            }
        }
    }
    for dirs in index.values_mut() {
        dirs.sort();
    }
    index
}

/// The TEXMF trees and a lookup cache.
#[derive(Debug, Default)]
pub struct Kpathsea {
    trees: Vec<Tree>,
    program: Program,
    cache: HashMap<(FileKind, String), Option<PathBuf>>,
}

const TREE_VARIABLES: [&str; 7] = [
    "TEXMFCONFIG",
    "TEXMFVAR",
    "TEXMFHOME",
    "TEXMFLOCAL",
    "TEXMFSYSCONFIG",
    "TEXMFSYSVAR",
    "TEXMFDIST",
];

impl Kpathsea {
    /// Finds the installed TeX distribution: `TEXMF*` environment variables, then
    /// `kpsewhich -var-value`, then the usual installation directories.
    pub fn discover(program: Program) -> Self {
        let mut roots = Vec::new();
        for var in TREE_VARIABLES {
            if let Some(dir) = env::var_os(var).map(PathBuf::from).or_else(|| kpsewhich_var(var)) {
                roots.push(dir);
            }
        }
        if roots.is_empty() {
            roots = default_roots();
        }
        Self::with_trees(roots, program)
    }

    /// Lookup over explicit tree roots, in search order.
    pub fn with_trees(roots: Vec<PathBuf>, program: Program) -> Self {
        let mut seen = Vec::new();
        let trees = roots
            .into_iter()
            .filter(|r| {
                let new = !seen.contains(r);
                seen.push(r.clone());
                new
            })
            .filter_map(Tree::load)
            .collect();
        Kpathsea {
            trees,
            program,
            cache: HashMap::new(),
        }
    }

    /// Whether any tree was found.
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// Changes the input paths, as `-progname` does.
    pub fn set_program(&mut self, program: Program) {
        if self.program != program {
            self.program = program;
            self.cache.clear();
        }
    }

    /// Finds `name` of the given kind, trying the kind's suffix first as kpathsea does.
    pub fn find(&mut self, name: &str, kind: FileKind) -> Option<PathBuf> {
        let key = (kind, name.to_owned());
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let found = self.search(name, kind);
        self.cache.insert(key, found.clone());
        found
    }

    fn search(&self, name: &str, kind: FileKind) -> Option<PathBuf> {
        if name.contains('/') || name.contains('\\') {
            return None;
        }
        for candidate in candidates(name, kind) {
            for subtree in kind.subtrees(self.program) {
                for tree in &self.trees {
                    if let Some(p) = tree.find(&candidate, subtree) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
}

/// The names to try for `name`: with the kind's suffix first, then as given.
pub fn candidates(name: &str, kind: FileKind) -> Vec<String> {
    let suffixes = kind.suffixes();
    if suffixes.iter().any(|s| name.ends_with(s)) || suffixes.is_empty() {
        return vec![name.to_owned()];
    }
    let mut names: Vec<String> = suffixes.iter().map(|s| format!("{name}{s}")).collect();
    names.push(name.to_owned());
    names
}

fn kpsewhich_var(var: &str) -> Option<PathBuf> {
    let output = Command::new("kpsewhich")
        .arg(format!("-var-value={var}"))
        .output()
        .ok()?;
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty() && output.status.success()).then(|| PathBuf::from(value))
}

fn default_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for base in ["/usr/local/texlive", "/opt/texlive", "C:\\texlive"] {
        if let Ok(entries) = fs::read_dir(base) {
            let mut years: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            years.sort();
            if let Some(latest) = years.last() {
                roots.push(latest.join("texmf-var"));
                roots.push(latest.join("texmf-dist"));
            }
        }
    }
    for dir in ["/usr/share/texlive/texmf-dist", "/usr/share/texmf"] {
        roots.push(PathBuf::from(dir));
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_r_lists_files_by_directory() {
        let index =
            parse_ls_r("% ls-R\n./:\nls-R\n\n./tex/plain/base:\nplain.tex\n\n./tex/generic/hyphen:\nhyphen.tex\n");
        assert_eq!(index["plain.tex"], vec!["tex/plain/base".to_string()]);
        assert_eq!(index["hyphen.tex"], vec!["tex/generic/hyphen".to_string()]);
    }

    #[test]
    fn suffixes_are_tried_first() {
        assert_eq!(candidates("plain", FileKind::Tex), vec!["plain.tex", "plain"]);
        assert_eq!(
            candidates("article.cls", FileKind::Tex),
            vec!["article.cls.tex", "article.cls"]
        );
        assert_eq!(candidates("cmr10.tfm", FileKind::Tfm), vec!["cmr10.tfm"]);
    }

    #[test]
    fn finds_files_in_a_tree_respecting_subtrees() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("tex/latex/base");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("article.cls"), "%").unwrap();
        let mut kpse = Kpathsea::with_trees(vec![root.path().to_owned()], Program::PlainTex);
        assert_eq!(kpse.find("article.cls", FileKind::Tex), Some(dir.join("article.cls")));
        assert_eq!(kpse.find("article.cls", FileKind::Tfm), None);
    }

    #[test]
    fn finds_tex_live_files_like_kpsewhich() {
        let mut kpse = Kpathsea::discover(Program::PlainTex);
        if kpse.is_empty() {
            return;
        }
        for (name, kind) in [
            ("plain.tex", FileKind::Tex),
            ("cmr10", FileKind::Tfm),
            ("hyphen", FileKind::Tex),
        ] {
            let ours = kpse.find(name, kind);
            let format = if kind == FileKind::Tfm { "tfm" } else { "tex" };
            let theirs = Command::new("kpsewhich")
                .args([format!("-format={format}"), name.to_owned()])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| PathBuf::from(s.trim()));
            if let Some(theirs) = theirs.filter(|p| !p.as_os_str().is_empty()) {
                assert_eq!(ours, Some(theirs), "{name}");
            }
        }
    }
}
