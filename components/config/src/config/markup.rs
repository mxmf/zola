use giallo::{HighlightOptions, Registry, ThemeVariant};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use errors::{Context, Result, bail};
use utils::fs::read_file;
use utils::types::InsertAnchor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HighlightStyle {
    Inline,
    Class,
}

impl Default for HighlightStyle {
    fn default() -> HighlightStyle {
        HighlightStyle::Inline
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HighlightConfig {
    Single { theme: String },
    Dual { light_theme: String, dark_theme: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Highlighting {
    /// Emit an error for missing highlight languages. Defaults to false
    #[serde(default)]
    pub error_on_missing_language: bool,
    #[serde(default)]
    pub style: HighlightStyle,
    #[serde(flatten)]
    pub theme: HighlightConfig,
    #[serde(default)]
    pub extra_grammars: Vec<String>,
    #[serde(default)]
    pub extra_themes: Vec<String>,
    #[serde(skip, default)]
    pub registry: Registry,
}

impl Highlighting {
    pub fn init(&mut self, config_dir: &Path) -> Result<()> {
        let mut registry = Registry::builtin()?;

        for grammar in &self.extra_grammars {
            registry.add_grammar_from_path(config_dir.join(grammar))?;
        }

        for theme in &self.extra_themes {
            registry.add_theme_from_path(config_dir.join(theme))?;
        }

        registry.link_grammars();

        match &self.theme {
            HighlightConfig::Single { theme } => {
                if !registry.contains_theme(&theme) {
                    bail!("Theme `{theme}` does not exist");
                }
            }
            HighlightConfig::Dual { light_theme, dark_theme } => {
                if !registry.contains_theme(&light_theme) {
                    bail!("Theme `{light_theme}` does not exist");
                }

                if !registry.contains_theme(&dark_theme) {
                    bail!("Theme `{dark_theme}` does not exist");
                }
            }
        }

        self.registry = registry;

        Ok(())
    }

    pub fn uses_classes(&self) -> bool {
        self.style == HighlightStyle::Class
    }

    pub fn generate_themes_css(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();

        if self.style == HighlightStyle::Inline {
            return out;
        }

        // we know themes are present so unwrap
        match &self.theme {
            HighlightConfig::Single { theme } => {
                out.push((
                    "giallo.css",
                    self.registry.generate_css(theme, "z-").expect("theme to be present"),
                ));
            }
            HighlightConfig::Dual { light_theme, dark_theme } => {
                out.push((
                    "giallo-light.css",
                    self.registry.generate_css(light_theme, "z-").expect("theme to be present"),
                ));
                out.push((
                    "giallo-dark.css",
                    self.registry.generate_css(dark_theme, "z-").expect("theme to be present"),
                ));
            }
        }

        out
    }

    pub fn highlight_options<'a>(&'a self, lang: &'a str) -> HighlightOptions {
        let mut opt = match &self.theme {
            HighlightConfig::Single { theme } => {
                HighlightOptions::new(lang, ThemeVariant::Single(theme))
            }
            HighlightConfig::Dual { light_theme, dark_theme } => HighlightOptions::new(
                lang,
                ThemeVariant::Dual { light: light_theme, dark: dark_theme },
            ),
        };

        if !self.error_on_missing_language {
            opt = opt.fallback_to_plain(true);
        }

        opt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MathEngine {
    Typst,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Math {
    /// Whether to render Markdown math formulas with the configured engine.
    pub enabled: bool,
    /// The renderer to use for Markdown math formulas.
    pub engine: MathEngine,
    /// Typst-specific math rendering configuration.
    pub typst: TypstMath,
}

impl Default for Math {
    fn default() -> Self {
        Self { enabled: false, engine: MathEngine::Typst, typst: TypstMath::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypstMath {
    /// Typst code prepended before every formula.
    pub preamble: Option<String>,
    /// Path to a Typst preamble file, relative to the config file directory.
    pub preamble_file: Option<String>,
    /// Whether Typst math can import files from the site directory.
    pub allow_local_imports: bool,
    /// Whether Typst math can import packages from the Typst Universe.
    pub packages: bool,
    /// Directory used to cache downloaded Typst packages, relative to the config file directory.
    pub package_cache: Option<String>,
    /// Canonical root used to resolve local imports.
    #[serde(skip)]
    pub local_import_root: Option<PathBuf>,
    /// Directory used to resolve cached Typst Universe packages.
    #[serde(skip)]
    pub package_cache_dir: Option<PathBuf>,
}

impl TypstMath {
    pub fn init(&mut self, config_dir: &Path) -> Result<()> {
        if let Some(preamble_file) = &self.preamble_file {
            let file_content = read_file(&config_dir.join(preamble_file))
                .with_context(|| format!("Failed to load Typst math preamble `{preamble_file}`"))?;

            self.preamble = Some(match self.preamble.take() {
                Some(inline) if !inline.trim().is_empty() => format!("{inline}\n{file_content}"),
                _ => file_content,
            });
        }

        if self.allow_local_imports {
            self.local_import_root = Some(config_dir.canonicalize().with_context(|| {
                format!("Failed to canonicalize Typst math import root `{}`", config_dir.display())
            })?);
        }

        if self.packages {
            let package_cache = self.package_cache.as_deref().unwrap_or(".zola/typst-packages");
            self.package_cache_dir = Some(config_dir.join(package_cache));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Markdown {
    /// Syntax highlighting option
    pub highlighting: Option<Highlighting>,
    /// Whether to render emoji aliases (e.g.: :smile: => 😄) in the markdown files
    pub render_emoji: bool,
    /// CSS class to add to external links
    pub external_links_class: Option<String>,
    /// Whether external links are to be opened in a new tab
    /// If this is true, a `rel="noopener"` will always automatically be added for security reasons
    pub external_links_target_blank: bool,
    /// Whether to set rel="nofollow" for all external links
    pub external_links_no_follow: bool,
    /// Whether to set rel="noreferrer" for all external links
    pub external_links_no_referrer: bool,
    /// Whether to set rel="external" for all external links
    pub external_links_external: bool,
    /// Whether smart punctuation is enabled (changing quotes, dashes, dots etc in their typographic form)
    pub smart_punctuation: bool,
    /// Whether parsing of definition lists is enabled
    pub definition_list: bool,
    /// Whether footnotes are rendered at the bottom in the style of GitHub.
    pub bottom_footnotes: bool,
    /// Add loading="lazy" decoding="async" to img tags. When turned on, the alt text must be plain text. Defaults to false
    pub lazy_async_image: bool,
    /// Whether to insert a link for each header like the ones you can see in this site if you hover one
    /// The default template can be overridden by creating a `anchor-link.html` in the `templates` directory
    pub insert_anchor_links: InsertAnchor,
    /// Whether to enable GitHub-style alerts
    pub github_alerts: bool,
    /// Math rendering configuration.
    pub math: Math,
}

impl Markdown {
    pub fn validate_external_links_class(&self) -> Result<()> {
        // Validate external link class doesn't contain quotes which would break HTML and aren't valid in CSS
        if let Some(class) = &self.external_links_class
            && (class.contains('"') || class.contains('\''))
        {
            bail!("External link class '{}' cannot contain quotes", class)
        }
        Ok(())
    }

    pub fn has_external_link_tweaks(&self) -> bool {
        self.external_links_target_blank
            || self.external_links_no_follow
            || self.external_links_no_referrer
            || self.external_links_external
            || self.external_links_class.is_some()
    }

    pub fn construct_external_link_tag(&self, url: &str, title: &str) -> String {
        let mut rel_opts = Vec::new();
        let mut target = "".to_owned();
        let title = if title.is_empty() { "".to_owned() } else { format!("title=\"{}\" ", title) };

        let class = self
            .external_links_class
            .as_ref()
            .map_or("".to_owned(), |c| format!("class=\"{}\" ", c));

        if self.external_links_target_blank {
            // Security risk otherwise
            rel_opts.push("noopener");
            target = "target=\"_blank\" ".to_owned();
        }
        if self.external_links_no_follow {
            rel_opts.push("nofollow");
        }
        if self.external_links_no_referrer {
            rel_opts.push("noreferrer");
        }
        if self.external_links_external {
            rel_opts.push("external");
        }
        let rel = if rel_opts.is_empty() {
            "".to_owned()
        } else {
            format!("rel=\"{}\" ", rel_opts.join(" "))
        };

        format!("<a {}{}{}{}href=\"{}\">", class, rel, target, title, url)
    }
}

impl Default for Markdown {
    fn default() -> Markdown {
        Markdown {
            highlighting: None,
            render_emoji: false,
            external_links_class: None,
            external_links_target_blank: false,
            external_links_no_follow: false,
            external_links_no_referrer: false,
            external_links_external: true,
            smart_punctuation: false,
            definition_list: false,
            bottom_footnotes: false,
            lazy_async_image: false,
            insert_anchor_links: InsertAnchor::None,
            github_alerts: false,
            math: Math::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_is_disabled_by_default() {
        let markdown = Markdown::default();
        assert!(!markdown.math.enabled);
        assert_eq!(markdown.math.engine, MathEngine::Typst);
        assert_eq!(markdown.math.typst, TypstMath::default());
    }

    #[test]
    fn can_enable_typst_math() {
        let markdown: Markdown = toml::from_str(
            r#"
            [math]
            enabled = true
            engine = "typst"
            "#,
        )
        .unwrap();

        assert!(markdown.math.enabled);
        assert_eq!(markdown.math.engine, MathEngine::Typst);
    }

    #[test]
    fn can_configure_typst_math_preamble() {
        let markdown: Markdown = toml::from_str(
            r##"
            [math]
            enabled = true

            [math.typst]
            preamble = "#let sq(x) = $ #x^2 $"
            preamble_file = "math.typ"
            allow_local_imports = true
            packages = true
            package_cache = ".cache/typst"
            "##,
        )
        .unwrap();

        assert_eq!(markdown.math.typst.preamble.as_deref(), Some("#let sq(x) = $ #x^2 $"));
        assert_eq!(markdown.math.typst.preamble_file.as_deref(), Some("math.typ"));
        assert!(markdown.math.typst.allow_local_imports);
        assert!(markdown.math.typst.packages);
        assert_eq!(markdown.math.typst.package_cache.as_deref(), Some(".cache/typst"));
    }

    #[test]
    fn loads_typst_math_preamble_file() {
        let unique = format!(
            "zola-typst-math-test-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("math.typ"), "#let cube(x) = $ #x^3 $").unwrap();

        let mut typst = TypstMath {
            preamble: Some("#let sq(x) = $ #x^2 $".to_string()),
            preamble_file: Some("math.typ".to_string()),
            allow_local_imports: false,
            packages: false,
            package_cache: None,
            local_import_root: None,
            package_cache_dir: None,
        };
        typst.init(&dir).unwrap();

        assert_eq!(
            typst.preamble.as_deref(),
            Some("#let sq(x) = $ #x^2 $\n#let cube(x) = $ #x^3 $")
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn math_rejects_unknown_fields() {
        let error = toml::from_str::<Markdown>(
            r#"
            [math]
            enabled = true
            engine = "typst"
            unexpected = true
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn typst_math_rejects_unknown_fields() {
        let error = toml::from_str::<Markdown>(
            r#"
            [math.typst]
            unexpected = true
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }
}
