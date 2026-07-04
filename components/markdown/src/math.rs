use errors::{Result, anyhow, bail};

use std::path::PathBuf;
use std::sync::LazyLock;
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

#[derive(Debug, Clone, Copy)]
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

    fn wrap_source(self, formula: &str) -> String {
        match self {
            MathMode::Inline => format!("${formula}$"),
            MathMode::Display => format!("$ {formula} $"),
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
}

impl MathWorld {
    fn new(source: String) -> Result<Self> {
        let path = RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("/zola-math.typ").map_err(|e| anyhow!(e.to_string()))?,
        );
        let main = FileId::unique(path);
        let source = Source::new(main, source);
        Ok(Self { main, source })
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
            Err(FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())))
        }
    }

    fn file(&self, id: FileId) -> std::result::Result<Bytes, FileError> {
        if id == self.main {
            Ok(Bytes::from_string(self.source.text().to_owned()))
        } else {
            Err(FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())))
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        TYPST_FONTS.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

fn render_math(formula: &str, mode: MathMode) -> Result<String> {
    let source = mode.wrap_source(formula);
    let world = MathWorld::new(source)?;
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
    Ok(mode.wrap_html(mathml))
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

pub fn render_inline_math(formula: &str) -> Result<String> {
    render_math(formula, MathMode::Inline)
}

pub fn render_display_math(formula: &str) -> Result<String> {
    render_math(formula, MathMode::Display)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_inline_mathml() {
        let html = render_inline_math("a^2 + b^2 = c^2").unwrap();
        assert!(html.contains("zola-math-inline"));
        assert!(html.contains("<math"));
        assert!(html.contains("</math>"));
    }

    #[test]
    fn renders_display_mathml() {
        let html = render_display_math("integral_0^1 x^2 dif x = 1 / 3").unwrap();
        assert!(html.contains("zola-math-display"));
        assert!(html.contains("<math"));
        assert!(html.contains("display=\"block\""));
        assert!(html.contains("</math>"));
    }

    #[test]
    fn invalid_formula_returns_error() {
        let error = render_inline_math("sqrt(").unwrap_err();
        assert!(error.to_string().contains("Typst failed to compile math formula"));
    }
}
