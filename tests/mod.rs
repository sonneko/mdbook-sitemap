use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

trait Tester: Sized {
    fn init_book_toml() -> &'static str;

    fn init_mock_contents(src_dir: &Path);

    fn validate_sitemap(book_dir: &Path);

    fn run_test() {
        // 1. initialize virtual file system
        let dir = tempdir().unwrap();
        let book_dir = dir.path();

        // 2. make virtual book.toml
        let book_toml_content = Self::init_book_toml();
        fs::write(book_dir.join("book.toml"), book_toml_content).unwrap();

        // 3. make src/ SUMARRY.md and each sections
        let src_dir = book_dir.join("src");
        fs::create_dir(&src_dir).unwrap();
        Self::init_mock_contents(&src_dir);

        // 4. run mdBook build
        let target_debug_dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("debug");
        let path_sep = if cfg!(windows) { ";" } else { ":" };
        let status = Command::new("mdbook")
            .arg("build")
            .current_dir(book_dir)
            .env(
                "PATH",
                format!(
                    "{}{}{}",
                    target_debug_dir.display(),
                    path_sep,
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .status()
            .expect("Failed to execute mdbook build. Is mdbook installed globally?");

        println!("{}", status);
        assert!(status.success(), "mdbook build failed");

        // 5. validate generated sitemap.xml
        Self::validate_sitemap(book_dir);
    }
}

struct BasicSitemapTest;

impl Tester for BasicSitemapTest {
    fn init_book_toml() -> &'static str {
        r#"
[book]
title = "Basic Test Book"

[output.html]

[output.sitemap]
base-url = "https://example.com/docs/"
"#
    }

    fn init_mock_contents(src_dir: &Path) {
        fs::write(
            src_dir.join("SUMMARY.md"),
            "# Summary\n\n- [Home](README.md)\n- [First](first.md)",
        )
        .unwrap();
        fs::write(src_dir.join("README.md"), "# Home").unwrap();
        fs::write(src_dir.join("first.md"), "# First Chapter").unwrap();
    }

    fn validate_sitemap(book_dir: &Path) {
        let sitemap_path = book_dir.join("book/html/sitemap.xml");
        assert!(sitemap_path.exists(), "sitemap.xml was not generated!");

        let sitemap = fs::read_to_string(sitemap_path).unwrap();

        assert!(sitemap.contains("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(sitemap.contains("<urlset"));
        assert!(sitemap.contains("<loc>https://example.com/docs/index.html</loc>"));
        assert!(sitemap.contains("<loc>https://example.com/docs/first.html</loc>"));
    }
}

#[test]
fn test_basic_sitemap() {
    BasicSitemapTest::run_test();
}

struct CustomOptionsTest;

impl Tester for CustomOptionsTest {
    fn init_book_toml() -> &'static str {
        r#"
[book]
title = "Custom Options Book"

[output.html]

[output.sitemap]
base-url = "https://example.com/docs/"
change-freq = "monthly"
priority = 0.8
output-filename = "custom_sitemap.xml"
"#
    }

    fn init_mock_contents(src_dir: &Path) {
        fs::write(src_dir.join("SUMMARY.md"), "# Summary\n\n- [Page](page.md)").unwrap();
        fs::write(src_dir.join("page.md"), "# Page").unwrap();
    }

    fn validate_sitemap(book_dir: &Path) {
        let sitemap_path = book_dir.join("book/html/custom_sitemap.xml");
        assert!(
            sitemap_path.exists(),
            "custom_sitemap.xml was not generated!"
        );

        let sitemap = fs::read_to_string(sitemap_path).unwrap();

        assert!(sitemap.contains("<changefreq>monthly</changefreq>"));
        assert!(sitemap.contains("<priority>0.8</priority>"));
    }
}

#[test]
fn test_custom_options_sitemap() {
    CustomOptionsTest::run_test();
}

struct DraftExclusionTest;

impl Tester for DraftExclusionTest {
    fn init_book_toml() -> &'static str {
        r#"
[book]
title = "Draft Test Book"

[output.html]

[output.sitemap]
base-url = "https://example.com/docs/"
include-draft = false
"#
    }

    fn init_mock_contents(src_dir: &Path) {
        // Draft Chapterはファイル名（パス）が割り当てられていない、または中身がない想定
        fs::write(
            src_dir.join("SUMMARY.md"),
            "# Summary\n\n- [Published](pub.md)\n- [Draft Chapter]()", // もしくは存在しないファイルへのリンク
        )
        .unwrap();
        fs::write(src_dir.join("pub.md"), "# Published Page").unwrap();
    }

    fn validate_sitemap(book_dir: &Path) {
        let sitemap_path = book_dir.join("book/html/sitemap.xml");
        assert!(sitemap_path.exists(), "sitemap.xml was not generated!");

        let sitemap = fs::read_to_string(sitemap_path).unwrap();

        assert!(sitemap.contains("<loc>https://example.com/docs/pub.html</loc>"));
        assert!(!sitemap.contains("Draft"));
    }
}

#[test]
fn test_draft_exclusion() {
    DraftExclusionTest::run_test();
}
