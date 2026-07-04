use errors::{Result, anyhow, bail};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use typst::diag::FileError;
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};
use typst_html::{HtmlDocument, HtmlOptions};

static TYPST_FONTS: LazyLock<Vec<Font>> =
    LazyLock::new(|| typst_assets::fonts().flat_map(|data| Font::iter(Bytes::new(data))).collect());

static TYPST_FONT_BOOK: LazyLock<LazyHash<FontBook>> =
    LazyLock::new(|| LazyHash::new(FontBook::from_fonts(TYPST_FONTS.iter())));

static TYPST_LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| {
    LazyHash::new(Library::builder().with_features(Features::from_iter([Feature::Html])).build())
});

const CACHE_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MathCacheKey {
    version: u8,
    formula: String,
    mode: MathMode,
    preamble: String,
    local_import_root: Option<PathBuf>,
}

static MATH_CACHE: LazyLock<RwLock<HashMap<MathCacheKey, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MathMode {
    Inline,
    Display,
}

impl MathMode {
    fn wrapper_class(self) -> &'static str {
        match self {
            MathMode::Inline => "zola-math-inline",
            MathMode::Display => "zola-math-display",
        }
    }

    fn wrap_source(self, formula: &str, preamble: Option<&str>) -> String {
        let formula = match self {
            MathMode::Inline => format!("${formula}$"),
            MathMode::Display => format!("$ {formula} $"),
        };

        match preamble {
            Some(preamble) if !preamble.trim().is_empty() => format!("{preamble}\n{formula}"),
            _ => formula,
        }
    }

    fn wrap_html(self, mathml: String) -> String {
        match self {
            MathMode::Inline => {
                format!(r#"<span class="zola-math {}">{mathml}</span>"#, self.wrapper_class())
            }
            MathMode::Display => {
                format!(r#"<div class="zola-math {}">{mathml}</div>"#, self.wrapper_class())
            }
        }
    }
}

struct MathWorld {
    main: FileId,
    source: Source,
    local_import_root: Option<PathBuf>,
}

impl MathWorld {
    fn new(source: String, local_import_root: Option<&Path>) -> Result<Self> {
        let path = RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("/zola-math.typ").map_err(|e| anyhow!(e.to_string()))?,
        );
        let main = FileId::unique(path);
        let source = Source::new(main, source);
        Ok(Self { main, source, local_import_root: local_import_root.map(Path::to_path_buf) })
    }

    fn resolve_local_path(&self, id: FileId) -> std::result::Result<PathBuf, FileError> {
        if id.root() != &VirtualRoot::Project {
            return Err(FileError::AccessDenied);
        }

        let Some(root) = &self.local_import_root else {
            return Err(FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())));
        };

        let path = id.vpath().realize(root).map_err(FileError::Realize)?;
        let canonical = path.canonicalize().map_err(|err| FileError::from_io(err, &path))?;
        if !canonical.starts_with(root) {
            return Err(FileError::AccessDenied);
        }
        Ok(canonical)
    }
}

impl World for MathWorld {
    fn library(&self) -> &LazyHash<Library> {
        &TYPST_LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &TYPST_FONT_BOOK
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> std::result::Result<Source, FileError> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            let path = self.resolve_local_path(id)?;
            let source =
                std::fs::read_to_string(&path).map_err(|err| FileError::from_io(err, &path))?;
            Ok(Source::new(id, source))
        }
    }

    fn file(&self, id: FileId) -> std::result::Result<Bytes, FileError> {
        if id == self.main {
            Ok(Bytes::from_string(self.source.text().to_owned()))
        } else {
            let path = self.resolve_local_path(id)?;
            std::fs::read(&path).map(Bytes::new).map_err(|err| FileError::from_io(err, &path))
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        TYPST_FONTS.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

fn render_math(
    formula: &str,
    mode: MathMode,
    preamble: Option<&str>,
    local_import_root: Option<&Path>,
) -> Result<String> {
    let preamble = preamble.filter(|preamble| !preamble.trim().is_empty()).unwrap_or_default();
    let cache_key = MathCacheKey {
        version: CACHE_VERSION,
        formula: formula.to_owned(),
        mode,
        preamble: preamble.to_owned(),
        local_import_root: local_import_root.map(Path::to_path_buf),
    };
    let use_cache = local_import_root.is_none();

    if use_cache && let Some(cached) = MATH_CACHE.read().unwrap().get(&cache_key).cloned() {
        return Ok(cached);
    }

    let source = mode.wrap_source(formula, Some(preamble));
    let world = MathWorld::new(source, local_import_root)?;
    let warned = typst::compile::<HtmlDocument>(&world);
    let document = warned.output.map_err(|diagnostics| {
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow!("Typst failed to compile math formula `{}`: {}", formula, messages)
    })?;

    let html = typst_html::html(&document, &HtmlOptions::default()).map_err(|diagnostics| {
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow!("Typst failed to encode math formula `{}` as HTML: {}", formula, messages)
    })?;
    let mathml = extract_mathml(&html)?;
    let rendered = mode.wrap_html(mathml);
    if use_cache {
        MATH_CACHE.write().unwrap().insert(cache_key, rendered.clone());
    }
    Ok(rendered)
}

fn extract_mathml(html: &str) -> Result<String> {
    let start = html
        .find("<math")
        .ok_or_else(|| anyhow!("Typst HTML output did not contain a <math> element"))?;
    let after_start = &html[start..];
    let end = after_start
        .find("</math>")
        .map(|idx| start + idx + "</math>".len())
        .ok_or_else(|| anyhow!("Typst HTML output contained an unterminated <math> element"))?;

    let mathml = html[start..end].to_string();
    if mathml.trim().is_empty() {
        bail!("Typst HTML output contained an empty <math> element");
    }
    Ok(mathml)
}

pub fn render_inline_math(
    formula: &str,
    preamble: Option<&str>,
    local_import_root: Option<&Path>,
) -> Result<String> {
    render_math(formula, MathMode::Inline, preamble, local_import_root)
}

pub fn render_display_math(
    formula: &str,
    preamble: Option<&str>,
    local_import_root: Option<&Path>,
) -> Result<String> {
    render_math(formula, MathMode::Display, preamble, local_import_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_contains(formula: &str, mode: MathMode, preamble: Option<&str>) -> bool {
        MATH_CACHE.read().unwrap().contains_key(&MathCacheKey {
            version: CACHE_VERSION,
            formula: formula.to_owned(),
            mode,
            preamble: preamble.unwrap_or_default().to_owned(),
            local_import_root: None,
        })
    }

    #[test]
    fn renders_inline_mathml() {
        let html = render_inline_math("a^2 + b^2 = c^2", None, None).unwrap();
        assert!(html.contains("zola-math-inline"));
        assert!(html.contains("<math"));
        assert!(html.contains("</math>"));
    }

    #[test]
    fn renders_display_mathml() {
        let html = render_display_math("integral_0^1 x^2 dif x = 1 / 3", None, None).unwrap();
        assert!(html.contains("zola-math-display"));
        assert!(html.contains("<math"));
        assert!(html.contains("display=\"block\""));
        assert!(html.contains("</math>"));
    }

    #[test]
    fn invalid_formula_returns_error() {
        let error = render_inline_math("sqrt(", None, None).unwrap_err();
        assert!(error.to_string().contains("Typst failed to compile math formula"));
        assert!(!cache_contains("sqrt(", MathMode::Inline, None));
    }

    #[test]
    fn caches_successful_renders() {
        let formula = "x^2 + y^2 + z^2";
        let first = render_inline_math(formula, None, None).unwrap();
        let second = render_inline_math(formula, None, None).unwrap();

        assert_eq!(first, second);
        assert!(cache_contains(formula, MathMode::Inline, None));
    }

    #[test]
    fn renders_with_preamble() {
        let preamble = "#let sq(x) = $ #x^2 $";
        let html = render_inline_math("sq(a)", Some(preamble), None).unwrap();

        assert!(html.contains("zola-math-inline"));
        assert!(html.contains("<math"));
        assert!(cache_contains("sq(a)", MathMode::Inline, Some(preamble)));
    }

    #[test]
    fn renders_with_local_import() {
        let unique = format!(
            "zola-typst-math-import-test-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("math.typ"), "#let sq(x) = $ #x^2 $").unwrap();

        let html =
            render_inline_math("sq(a)", Some("#import \"math.typ\": sq"), Some(&dir)).unwrap();

        assert!(html.contains("zola-math-inline"));
        assert!(html.contains("<math"));
        assert!(!cache_contains("sq(a)", MathMode::Inline, Some("#import \"math.typ\": sq")));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_local_import_path_traversal() {
        let unique = format!(
            "zola-typst-math-import-traversal-test-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir(&dir).unwrap();

        let error =
            render_inline_math("a", Some("#import \"../outside.typ\": *"), Some(&dir)).unwrap_err();

        assert!(error.to_string().contains("Typst failed to compile math formula"));

        std::fs::remove_dir_all(dir).unwrap();
    }
}
