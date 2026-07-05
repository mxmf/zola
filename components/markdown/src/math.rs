use config::{Math, MathSyntax};
use errors::{Result, anyhow, bail};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use typst::diag::{FileError, PackageError};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::layout::Abs;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};
use typst_html::{HtmlDocument, HtmlOptions};
use typst_kit::downloader::SystemDownloader;
use typst_kit::packages::{FsPackages, UniversePackages};
use typst_layout::PagedDocument;

const CACHE_VERSION: u8 = 1;
const DEFAULT_MITEX_VERSION: &str = "0.2.7";

static FONTS: LazyLock<Vec<Font>> =
    LazyLock::new(|| typst_assets::fonts().flat_map(|data| Font::iter(Bytes::new(data))).collect());
static FONT_BOOK: LazyLock<LazyHash<FontBook>> =
    LazyLock::new(|| LazyHash::new(FontBook::from_fonts(FONTS.iter())));
static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| {
    LazyHash::new(Library::builder().with_features(Features::from_iter([Feature::Html])).build())
});
static CACHE: LazyLock<RwLock<HashMap<CacheKey, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static UNIVERSE_PACKAGES: LazyLock<UniversePackages> =
    LazyLock::new(|| UniversePackages::new(SystemDownloader::new("zola")));

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Mode {
    Inline,
    Display,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    version: u8,
    formula: String,
    mode: Mode,
    syntax: MathSyntax,
    preamble: String,
    mitex_version: String,
}

struct TypstWorld {
    main: FileId,
    source: Source,
    import_root: Option<PathBuf>,
    package_cache: Option<PathBuf>,
}

impl TypstWorld {
    fn new(source: String, math: &Math) -> Result<Self> {
        Self::new_at(source, math, None)
    }

    fn new_for_page(source: String, math: &Math, page_path: Option<&Path>) -> Result<Self> {
        Self::new_at(source, math, page_path)
    }

    fn new_at(source: String, math: &Math, page_path: Option<&Path>) -> Result<Self> {
        let path = RootedPath::new(
            VirtualRoot::Project,
            main_virtual_path(math.import_root.as_deref(), page_path)?,
        );
        let main = FileId::unique(path);
        Ok(Self {
            main,
            source: Source::new(main, source),
            import_root: math.import_root.clone(),
            package_cache: math.package_cache_dir.clone(),
        })
    }

    fn path(&self, id: FileId) -> std::result::Result<PathBuf, FileError> {
        match id.root() {
            VirtualRoot::Project => self.project_path(id),
            VirtualRoot::Package(package) => self.package_path(id, package),
        }
    }

    fn project_path(&self, id: FileId) -> std::result::Result<PathBuf, FileError> {
        let Some(root) = &self.import_root else {
            return Err(FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())));
        };
        let path = id.vpath().realize(root).map_err(FileError::Realize)?;
        let path = path.canonicalize().map_err(|e| FileError::from_io(e, &path))?;
        if path.starts_with(root) { Ok(path) } else { Err(FileError::AccessDenied) }
    }

    fn package_path(
        &self,
        id: FileId,
        package: &typst::syntax::package::PackageSpec,
    ) -> std::result::Result<PathBuf, FileError> {
        let Some(cache) = &self.package_cache else {
            return Err(FileError::Package(PackageError::NotFound(package.clone())));
        };
        let root = package_root(cache, package)?;
        let path = id.vpath().realize(&root).map_err(FileError::Realize)?;
        let path = path.canonicalize().map_err(|e| FileError::from_io(e, &path))?;
        if path.starts_with(&root) { Ok(path) } else { Err(FileError::AccessDenied) }
    }
}

fn main_virtual_path(import_root: Option<&Path>, page_path: Option<&Path>) -> Result<VirtualPath> {
    let Some((root, page_path)) = import_root.zip(page_path) else {
        return VirtualPath::new("/zola-typst.typ").map_err(|e| anyhow!(e.to_string()));
    };

    let page_path = page_path.canonicalize().map_err(|e| anyhow!(e.to_string()))?;
    let relative = page_path.strip_prefix(root).map_err(|e| anyhow!(e.to_string()))?;
    let mut typst_path = PathBuf::from("/");
    for component in relative.components() {
        match component {
            Component::Normal(component) => typst_path.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("Typst SVG source path must stay within the site root");
            }
        }
    }
    typst_path.set_extension("typ");

    let path = typst_path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    VirtualPath::new(&path).map_err(|e| anyhow!(e.to_string()))
}

fn package_root(
    cache: &std::path::Path,
    package: &typst::syntax::package::PackageSpec,
) -> std::result::Result<PathBuf, FileError> {
    let fs = FsPackages::new(cache.to_path_buf());
    if let Some(root) = fs.obtain(package) {
        return Ok(root.path().to_path_buf());
    }

    if package.namespace == UniversePackages::NAMESPACE {
        let mut archive = UNIVERSE_PACKAGES.package(package).map_err(FileError::Package)?;
        fs.store(package, |tempdir| {
            archive
                .unpack(tempdir)
                .map_err(|err| PackageError::MalformedArchive(Some(format!("{err}").into())))
        })
        .map_err(FileError::Package)?;

        if let Some(root) = fs.obtain(package) {
            return Ok(root.path().to_path_buf());
        }
    }

    Err(FileError::Package(PackageError::NotFound(package.clone())))
}

impl World for TypstWorld {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &FONT_BOOK
    }
    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> std::result::Result<Source, FileError> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            let path = self.path(id)?;
            let text = std::fs::read_to_string(&path).map_err(|e| FileError::from_io(e, &path))?;
            Ok(Source::new(id, text))
        }
    }

    fn file(&self, id: FileId) -> std::result::Result<Bytes, FileError> {
        if id == self.main {
            Ok(Bytes::from_string(self.source.text().to_owned()))
        } else {
            let path = self.path(id)?;
            std::fs::read(&path).map(Bytes::new).map_err(|e| FileError::from_io(e, &path))
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.get(index).cloned()
    }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

pub fn render_inline(formula: &str, math: &Math) -> Result<String> {
    render_math(formula, Mode::Inline, math)
}

pub fn render_display(formula: &str, math: &Math) -> Result<String> {
    render_math(formula, Mode::Display, math)
}

fn render_math(formula: &str, mode: Mode, math: &Math) -> Result<String> {
    let preamble = math.preamble.as_deref().filter(|p| !p.trim().is_empty()).unwrap_or("");
    let mitex_version = math.mitex_version.as_deref().unwrap_or(DEFAULT_MITEX_VERSION);
    let key = CacheKey {
        version: CACHE_VERSION,
        formula: formula.to_owned(),
        mode,
        syntax: math.syntax,
        preamble: preamble.to_owned(),
        mitex_version: mitex_version.to_owned(),
    };
    let cacheable = math.import_root.is_none() && math.package_cache_dir.is_none();
    if cacheable && let Some(html) = CACHE.read().unwrap().get(&key).cloned() {
        return Ok(html);
    }

    let source = formula_source(formula, mode, math.syntax, preamble, mitex_version);
    let world = TypstWorld::new(source, math)?;
    let doc = typst::compile::<HtmlDocument>(&world).output.map_err(|diagnostics| {
        anyhow!(
            "Typst failed to compile math formula `{}`: {}",
            formula,
            diagnostics.iter().map(|d| d.message.to_string()).collect::<Vec<_>>().join("; ")
        )
    })?;
    let html = typst_html::html(&doc, &HtmlOptions::default()).map_err(|diagnostics| {
        anyhow!(
            "Typst failed to encode math formula `{}` as HTML: {}",
            formula,
            diagnostics.iter().map(|d| d.message.to_string()).collect::<Vec<_>>().join("; ")
        )
    })?;
    let html = wrap_math(extract_mathml(&html)?, mode);
    if cacheable {
        CACHE.write().unwrap().insert(key, html.clone());
    }
    Ok(html)
}

pub fn render_svg(source: &str, math: &Math, page_path: Option<&Path>) -> Result<String> {
    let world = TypstWorld::new_for_page(source.to_owned(), math, page_path)?;
    let doc = typst::compile::<PagedDocument>(&world).output.map_err(|diagnostics| {
        anyhow!(
            "Typst failed to compile SVG code block: {}",
            diagnostics.iter().map(|d| d.message.to_string()).collect::<Vec<_>>().join("; ")
        )
    })?;
    let svg = typst_svg::svg_merged(&doc, &typst_svg::SvgOptions::default(), Abs::zero());
    Ok(format!(r#"<div class="zola-typst-svg">{svg}</div>"#))
}

fn formula_source(
    formula: &str,
    mode: Mode,
    syntax: MathSyntax,
    preamble: &str,
    mitex: &str,
) -> String {
    let body = match (syntax, mode) {
        (MathSyntax::Typst, Mode::Inline) => format!("${formula}$"),
        (MathSyntax::Typst, Mode::Display) => format!("$ {formula} $"),
        (MathSyntax::Latex, Mode::Inline) => {
            format!("#import \"@preview/mitex:{mitex}\": mi\n#mi({})", typst_raw(formula))
        }
        (MathSyntax::Latex, Mode::Display) => {
            format!("#import \"@preview/mitex:{mitex}\": mitex\n#mitex({})", typst_raw(formula))
        }
    };
    if preamble.is_empty() { body } else { format!("{preamble}\n{body}") }
}

fn typst_raw(value: &str) -> String {
    let mut ticks = 1;
    let mut run = 0;
    for c in value.chars() {
        if c == '`' {
            run += 1;
            ticks = ticks.max(run + 1);
        } else {
            run = 0;
        }
    }
    let delimiter = "`".repeat(ticks);
    format!("{delimiter}{value}{delimiter}")
}

fn extract_mathml(html: &str) -> Result<String> {
    let start = html
        .find("<math")
        .ok_or_else(|| anyhow!("Typst HTML output did not contain a <math> element"))?;
    let after = &html[start..];
    let end = after
        .find("</math>")
        .map(|idx| start + idx + "</math>".len())
        .ok_or_else(|| anyhow!("Typst HTML output contained an unterminated <math> element"))?;
    let mathml = html[start..end].to_owned();
    if mathml.trim().is_empty() {
        bail!("Typst HTML output contained an empty <math> element");
    }
    Ok(mathml)
}

fn wrap_math(mathml: String, mode: Mode) -> String {
    match mode {
        Mode::Inline => format!(r#"<span class="zola-math zola-math-inline">{mathml}</span>"#),
        Mode::Display => format!(r#"<div class="zola-math zola-math-display">{mathml}</div>"#),
    }
}
