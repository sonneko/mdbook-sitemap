//! mdbook-sitemap: A sitemap.xml generator backend for mdBook
//!
//! This tool is an mdBook backend that automatically generates a sitemap.xml
//! file when `mdbook build` is run. It reads the book structure from stdin
//! (as JSON provided by mdbook) and writes sitemap.xml to the output directory.
//!
//! # Usage in book.toml
//!
//! ```toml
//! [output.html]
//!
//! [output.sitemap]
//! base-url = "https://example.com/docs/"
//! # Optional settings:
//! # change-freq = "weekly"          # always|hourly|daily|weekly|monthly|yearly|never
//! # priority = 0.7                  # 0.0 - 1.0
//! # output-filename = "sitemap.xml" # output filename
//! # include-draft = false           # whether to include draft chapters
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

// ─── mdBook JSON data structures ────────────────────────────────────────────

/// Top-level JSON sent by mdbook to the backend via stdin.
#[derive(Debug, Deserialize)]
struct RenderContext {
    version: String,
    root: PathBuf,
    book: Book,
    config: serde_json::Value,
    destination: PathBuf,
}

impl RenderContext {
    fn from_json<R: Read>(reader: R) -> Result<Self> {
        serde_json::from_reader(reader).context("Failed to parse RenderContext from JSON")
    }

    /// Get the sitemap-specific config section from book.toml.
    fn sitemap_config(&self) -> SitemapConfig {
        let val = self
            .config
            .get("output")
            .and_then(|o| o.get("sitemap"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        serde_json::from_value(val).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
struct Book {
    sections: Vec<BookItem>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BookItem {
    Chapter(Chapter),
    Separator,
    PartTitle(String),
}

#[derive(Debug, Deserialize)]
struct Chapter {
    name: String,
    /// Relative path from the book's source directory (e.g. "chapter_1.md").
    /// `None` for draft chapters.
    path: Option<PathBuf>,
    /// Nested sub-chapters.
    #[serde(default)]
    sub_items: Vec<BookItem>,
    /// Whether the chapter is a draft (no path).
    #[serde(default)]
    is_draft: bool,
}

// ─── Sitemap configuration ───────────────────────────────────────────────────

/// Settings read from `[output.sitemap]` in book.toml.
#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct SitemapConfig {
    /// Base URL of the published book (required, e.g. "https://example.com/").
    /// Trailing slash is added automatically if missing.
    base_url: Option<String>,
    /// Default change frequency for all pages (default: "weekly").
    change_freq: ChangeFreq,
    /// Default priority for all pages (0.0–1.0, default: 0.7).
    priority: f64,
    /// Filename to write (default: "sitemap.xml").
    output_filename: String,
    /// Whether to include draft chapters in the sitemap (default: false).
    include_draft: bool,
    /// Whether to include a lastmod date using today's date (default: true).
    include_lastmod: bool,
}

impl Default for SitemapConfig {
    fn default() -> Self {
        SitemapConfig {
            base_url: None,
            change_freq: ChangeFreq::Weekly,
            priority: 0.7,
            output_filename: "sitemap.xml".to_string(),
            include_draft: false,
            include_lastmod: true,
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ChangeFreq {
    Always,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Never,
}

impl std::fmt::Display for ChangeFreq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChangeFreq::Always => "always",
            ChangeFreq::Hourly => "hourly",
            ChangeFreq::Daily => "daily",
            ChangeFreq::Weekly => "weekly",
            ChangeFreq::Monthly => "monthly",
            ChangeFreq::Yearly => "yearly",
            ChangeFreq::Never => "never",
        };
        write!(f, "{}", s)
    }
}

impl Default for ChangeFreq {
    fn default() -> Self {
        ChangeFreq::Weekly
    }
}

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name = "mdbook-sitemap",
    version,
    about = "An mdBook backend that generates a sitemap.xml file",
    long_about = "mdbook-sitemap reads an mdBook's structure and generates a \
                  sitemap.xml file for SEO purposes.\n\n\
                  Normally invoked automatically by `mdbook build`. \
                  Configure it in your book.toml:\n\n\
                  [output.html]\n\
                  [output.sitemap]\n\
                  base-url = \"https://example.com/docs/\"\n"
)]
struct Cli {
    /// Check if this backend supports the given renderer (called by mdbook).
    #[arg(long, value_name = "RENDERER")]
    supports: Option<String>,
}

// ─── Sitemap generation ──────────────────────────────────────────────────────

/// Collect all chapter URLs recursively.
fn collect_urls(
    items: &[BookItem],
    base_url: &str,
    include_draft: bool,
    urls: &mut Vec<String>,
) {
    for item in items {
        match item {
            BookItem::Chapter(ch) => {
                // Only add chapters that have a path (i.e. not draft-only).
                if !ch.is_draft || include_draft {
                    if let Some(ref path) = ch.path {
                        let html_path = path_to_html(path);
                        let url = format!("{}{}", base_url, html_path);
                        urls.push(url);
                    }
                }
                // Recurse into sub-chapters.
                if !ch.sub_items.is_empty() {
                    collect_urls(&ch.sub_items, base_url, include_draft, urls);
                }
            }
            BookItem::Separator | BookItem::PartTitle(_) => {}
        }
    }
}

/// Convert a .md file path to the corresponding .html path that mdbook outputs.
/// e.g. "chapter_1.md" → "chapter_1.html"
///      "sub/README.md" → "sub/index.html"  (mdbook converts README.md → index.html)
fn path_to_html(path: &PathBuf) -> String {
    let s = path.to_string_lossy();

    // Handle Windows-style path separators.
    let normalized = s.replace('\\', "/");

    // mdBook converts README.md to index.html.
    if normalized.ends_with("/README.md") || normalized == "README.md" {
        let without_readme = normalized.trim_end_matches("README.md");
        return format!("{}index.html", without_readme);
    }

    // Replace .md extension with .html.
    if normalized.ends_with(".md") {
        return format!("{}.html", &normalized[..normalized.len() - 3]);
    }

    normalized.into_owned()
}

/// Render the sitemap XML string.
fn build_sitemap(urls: &[String], cfg: &SitemapConfig) -> String {
    let today = get_today();
    let mut xml = String::with_capacity(1024 + urls.len() * 128);

    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    xml.push_str(
        r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
    );
    xml.push('\n');

    for url in urls {
        xml.push_str("  <url>\n");
        xml.push_str(&format!("    <loc>{}</loc>\n", escape_xml(url)));
        if cfg.include_lastmod {
            xml.push_str(&format!("    <lastmod>{}</lastmod>\n", today));
        }
        xml.push_str(&format!(
            "    <changefreq>{}</changefreq>\n",
            cfg.change_freq
        ));
        xml.push_str(&format!("    <priority>{:.1}</priority>\n", cfg.priority));
        xml.push_str("  </url>\n");
    }

    xml.push_str("</urlset>\n");
    xml
}

/// Escape XML special characters in a URL.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Return today's date in YYYY-MM-DD format without depending on chrono.
fn get_today() -> String {
    // Use SystemTime to get current date.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple calculation: days since epoch → year/month/day.
    epoch_secs_to_date(secs)
}

/// Convert UNIX epoch seconds to "YYYY-MM-DD" string.
/// This avoids a dependency on chrono for a simple date formatting task.
fn epoch_secs_to_date(secs: u64) -> String {
    let days = secs / 86400;
    // Using the algorithm from https://www.researchgate.net/publication/316558298
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

// ─── Main logic ──────────────────────────────────────────────────────────────

fn run() -> Result<()> {
    let cli = Cli::parse();

    // mdBook calls us with `--supports <renderer>` to check compatibility.
    if let Some(renderer) = cli.supports {
        // We only support the "html" renderer (we generate alongside HTML).
        // However, we are also invoked as an output backend ourselves,
        // so returning 0 (supported) for all renderers is typical.
        // We skip for "test" and "markdown" since sitemap only makes sense for HTML.
        if renderer == "html" || renderer == "sitemap" {
            std::process::exit(0);
        } else {
            // For any other renderer, we still support being a co-backend.
            std::process::exit(0);
        }
    }

    // Normal invocation: read RenderContext from stdin.
    let mut stdin = io::stdin();
    let ctx = RenderContext::from_json(&mut stdin)
        .context("Failed to read RenderContext from stdin.\n\
                  This program is meant to be invoked by `mdbook build`.\n\
                  Add `[output.sitemap]` to your book.toml to use it.")?;

    eprintln!(
        "[mdbook-sitemap] mdBook version: {}, root: {}",
        ctx.version,
        ctx.root.display()
    );

    let cfg = ctx.sitemap_config();

    // Validate base_url.
    let base_url = match &cfg.base_url {
        Some(u) => {
            let u = u.trim();
            if u.is_empty() {
                anyhow::bail!(
                    "mdbook-sitemap: `base-url` in [output.sitemap] is empty.\n\
                     Please set it to your site's base URL, e.g.:\n\
                     [output.sitemap]\n\
                     base-url = \"https://example.com/docs/\""
                );
            }
            // Ensure trailing slash.
            if u.ends_with('/') {
                u.to_string()
            } else {
                format!("{}/", u)
            }
        }
        None => {
            anyhow::bail!(
                "mdbook-sitemap: `base-url` is required in [output.sitemap].\n\
                 Example book.toml:\n\
                 \n\
                 [output.sitemap]\n\
                 base-url = \"https://example.com/docs/\""
            );
        }
    };

    // Collect all chapter URLs.
    let mut urls: Vec<String> = Vec::new();
    collect_urls(&ctx.book.sections, &base_url, cfg.include_draft, &mut urls);

    if urls.is_empty() {
        eprintln!(
            "[mdbook-sitemap] Warning: no chapters found. \
             The sitemap will be empty."
        );
    }

    // Build the XML.
    let sitemap_xml = build_sitemap(&urls, &cfg);

    // Write to destination directory.
    fs::create_dir_all(&ctx.destination).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            ctx.destination.display()
        )
    })?;

    let output_path = ctx.destination.join(&cfg.output_filename);
    fs::write(&output_path, &sitemap_xml).with_context(|| {
        format!("Failed to write sitemap to: {}", output_path.display())
    })?;

    eprintln!(
        "[mdbook-sitemap] Generated {} ({} URLs) → {}",
        cfg.output_filename,
        urls.len(),
        output_path.display()
    );

    // Also copy sitemap.xml to the HTML output directory if it exists,
    // so it ends up alongside the HTML files at the book root.
    let html_dest = ctx.destination.parent().and_then(|p| {
        let html_dir = p.join("html");
        if html_dir.is_dir() {
            Some(html_dir)
        } else {
            None
        }
    });

    if let Some(html_dir) = html_dest {
        let html_sitemap = html_dir.join(&cfg.output_filename);
        if let Err(e) = fs::copy(&output_path, &html_sitemap) {
            eprintln!(
                "[mdbook-sitemap] Note: could not copy sitemap to html dir: {}",
                e
            );
        } else {
            eprintln!(
                "[mdbook-sitemap] Also copied → {}",
                html_sitemap.display()
            );
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("[mdbook-sitemap] Error: {:?}", e);
        std::process::exit(1);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_html_md() {
        assert_eq!(path_to_html(&PathBuf::from("chapter_1.md")), "chapter_1.html");
    }

    #[test]
    fn test_path_to_html_readme() {
        assert_eq!(
            path_to_html(&PathBuf::from("sub/README.md")),
            "sub/index.html"
        );
        assert_eq!(path_to_html(&PathBuf::from("README.md")), "index.html");
    }

    #[test]
    fn test_path_to_html_root_readme() {
        assert_eq!(path_to_html(&PathBuf::from("README.md")), "index.html");
    }

    #[test]
    fn test_path_to_html_nested() {
        assert_eq!(
            path_to_html(&PathBuf::from("intro/overview.md")),
            "intro/overview.html"
        );
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(
            escape_xml("https://example.com/foo&bar"),
            "https://example.com/foo&amp;bar"
        );
        assert_eq!(escape_xml("normal-url"), "normal-url");
    }

    #[test]
    fn test_epoch_secs_to_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(epoch_secs_to_date(1704067200), "2024-01-01");
        // 2000-01-01 = 946684800
        assert_eq!(epoch_secs_to_date(946684800), "2000-01-01");
        // Unix epoch
        assert_eq!(epoch_secs_to_date(0), "1970-01-01");
    }

    #[test]
    fn test_build_sitemap() {
        let urls = vec![
            "https://example.com/docs/index.html".to_string(),
            "https://example.com/docs/chapter_1.html".to_string(),
        ];
        let cfg = SitemapConfig {
            base_url: Some("https://example.com/docs/".into()),
            include_lastmod: false,
            ..Default::default()
        };
        let xml = build_sitemap(&urls, &cfg);
        assert!(xml.contains(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(xml.contains(r#"xmlns="http://www.sitemaps.org/schemas/sitemap/0.9""#));
        assert!(xml.contains("<loc>https://example.com/docs/index.html</loc>"));
        assert!(xml.contains("<loc>https://example.com/docs/chapter_1.html</loc>"));
        assert!(xml.contains("<changefreq>weekly</changefreq>"));
        assert!(xml.contains("<priority>0.7</priority>"));
        assert!(!xml.contains("<lastmod>"));
    }

    #[test]
    fn test_collect_urls_basic() {
        let items = vec![
            BookItem::Chapter(Chapter {
                name: "Introduction".into(),
                path: Some(PathBuf::from("intro.md")),
                sub_items: vec![],
                is_draft: false,
            }),
            BookItem::Separator,
            BookItem::Chapter(Chapter {
                name: "Chapter 1".into(),
                path: Some(PathBuf::from("chapter1.md")),
                sub_items: vec![BookItem::Chapter(Chapter {
                    name: "Sub Chapter".into(),
                    path: Some(PathBuf::from("chapter1/sub.md")),
                    sub_items: vec![],
                    is_draft: false,
                })],
                is_draft: false,
            }),
        ];
        let mut urls = Vec::new();
        collect_urls(&items, "https://example.com/", false, &mut urls);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], "https://example.com/intro.html");
        assert_eq!(urls[1], "https://example.com/chapter1.html");
        assert_eq!(urls[2], "https://example.com/chapter1/sub.html");
    }

    #[test]
    fn test_collect_urls_skips_draft() {
        let items = vec![
            BookItem::Chapter(Chapter {
                name: "Real Chapter".into(),
                path: Some(PathBuf::from("real.md")),
                sub_items: vec![],
                is_draft: false,
            }),
            BookItem::Chapter(Chapter {
                name: "Draft Chapter".into(),
                path: None,
                sub_items: vec![],
                is_draft: true,
            }),
        ];
        let mut urls = Vec::new();
        collect_urls(&items, "https://example.com/", false, &mut urls);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://example.com/real.html");
    }

    #[test]
    fn test_collect_urls_includes_draft_when_configured() {
        let items = vec![
            BookItem::Chapter(Chapter {
                name: "Draft Chapter".into(),
                path: Some(PathBuf::from("draft.md")),
                sub_items: vec![],
                is_draft: true,
            }),
        ];
        let mut urls_without = Vec::new();
        collect_urls(&items, "https://example.com/", false, &mut urls_without);
        // is_draft=true but path exists: since is_draft check uses is_draft field
        // if include_draft is false, draft chapters are skipped
        assert_eq!(urls_without.len(), 0);

        let mut urls_with = Vec::new();
        collect_urls(&items, "https://example.com/", true, &mut urls_with);
        assert_eq!(urls_with.len(), 1);
    }

    #[test]
    fn test_change_freq_display() {
        assert_eq!(ChangeFreq::Weekly.to_string(), "weekly");
        assert_eq!(ChangeFreq::Daily.to_string(), "daily");
        assert_eq!(ChangeFreq::Monthly.to_string(), "monthly");
    }
}
