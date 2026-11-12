//! The file system TeX reads through: the job's directory first, then the TeX distribution.

use std::fs;
use std::path::{Path, PathBuf};

use rtex_core::{FileSystem, FoundFile, MemoryFs};

use crate::kpse::{FileKind, Kpathsea, Program};

/// Reads from a working directory, then from TeX Live through [`Kpathsea`]; writes to the output directory.
pub struct TexFs {
    root: PathBuf,
    output: PathBuf,
    kpse: Kpathsea,
    overlay: MemoryFs,
}

impl TexFs {
    /// A file system rooted at `root`, writing into `output`, searching the installed distribution.
    pub fn new(root: impl Into<PathBuf>, output: impl Into<PathBuf>, program: Program) -> Self {
        Self::with_kpathsea(root, output, Kpathsea::discover(program))
    }

    /// A file system over an explicit [`Kpathsea`].
    pub fn with_kpathsea(root: impl Into<PathBuf>, output: impl Into<PathBuf>, kpse: Kpathsea) -> Self {
        TexFs {
            root: root.into(),
            output: output.into(),
            kpse,
            overlay: MemoryFs::new(),
        }
    }

    /// The path of `name` of the given kind, as `kpsewhich` would report it.
    pub fn locate(&mut self, name: &str, kind: FileKind) -> Option<PathBuf> {
        for candidate in crate::kpse::candidates(name, kind) {
            for dir in [&self.output, &self.root] {
                let path = dir.join(&candidate);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        self.kpse.find(name, kind)
    }

    fn display(&self, path: &Path, name: &str) -> String {
        if path.starts_with(&self.root) || path.starts_with(&self.output) {
            format!("./{name}")
        } else {
            path.display().to_string()
        }
    }
}

/// The kind of file a name with this extension is.
pub fn kind_of(name: &str) -> FileKind {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("tfm") => FileKind::Tfm,
        Some("vf") => FileKind::Vf,
        Some("pfb" | "pfa") => FileKind::Type1,
        Some("enc") => FileKind::Enc,
        Some("map") => FileKind::Map,
        Some("otf") => FileKind::OpenType,
        Some("ttf" | "ttc") => FileKind::TrueType,
        Some("afm") => FileKind::Afm,
        Some("png" | "jpg" | "jpeg" | "pdf" | "jbig2") => FileKind::Image,
        _ => FileKind::Tex,
    }
}

impl FileSystem for TexFs {
    fn find(&mut self, name: &str) -> Option<FoundFile> {
        if let Some(f) = self.overlay.find(name) {
            return Some(f);
        }
        let direct = Path::new(name);
        if direct.is_absolute() && direct.is_file() {
            let data = fs::read(direct).ok()?;
            return Some(FoundFile {
                display_name: name.to_owned(),
                data: data.into(),
            });
        }
        for candidate in crate::kpse::candidates(name, kind_of(name)) {
            for dir in [&self.output, &self.root] {
                let path = dir.join(&candidate);
                if path.is_file() {
                    let data = fs::read(&path).ok()?;
                    let display_name = if direct.is_absolute() {
                        path.display().to_string()
                    } else {
                        format!("./{candidate}")
                    };
                    return Some(FoundFile {
                        display_name,
                        data: data.into(),
                    });
                }
            }
        }
        let path = self.kpse.find(name, kind_of(name))?;
        let data = fs::read(&path).ok()?;
        Some(FoundFile {
            display_name: self.display(&path, name),
            data: data.into(),
        })
    }

    fn write(&mut self, name: &str, data: Vec<u8>) {
        let path = self.output.join(name);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, &data);
        self.overlay.write(name, data);
    }

    fn add_virtual(&mut self, name: &str, data: Vec<u8>) {
        self.overlay.add_virtual(name, data);
    }

    fn written(&self, name: &str) -> Option<&[u8]> {
        self.overlay.written(name)
    }
}

impl TexFs {
    /// Switches the input paths, as `-progname` does.
    pub fn set_program(&mut self, program: Program) {
        self.kpse.set_program(program);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_choose_the_search_path() {
        assert_eq!(kind_of("cmr10.tfm"), FileKind::Tfm);
        assert_eq!(kind_of("story.tex"), FileKind::Tex);
        assert_eq!(kind_of("article.cls"), FileKind::Tex);
        assert_eq!(kind_of("figure.PNG"), FileKind::Image);
    }

    #[test]
    fn the_working_directory_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("plain.tex"), "local").unwrap();
        let mut fs = TexFs::with_kpathsea(dir.path(), dir.path(), Kpathsea::default());
        let found = fs.find("plain.tex").unwrap();
        assert_eq!(found.display_name, "./plain.tex");
        assert_eq!(&*found.data, b"local");
        assert!(fs.find("missing.tex").is_none());
    }

    #[test]
    fn writes_go_to_the_output_directory() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let mut fs = TexFs::with_kpathsea(dir.path(), &out, Kpathsea::default());
        fs.write("job.aux", b"\\relax".to_vec());
        assert_eq!(std::fs::read(out.join("job.aux")).unwrap(), b"\\relax");
        assert!(fs.find("job.aux").is_some());
    }
}
