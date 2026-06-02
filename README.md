# mdbook-sitemap

Tool to generate a sitemap.xml file for an mdbook project

This tool is an mdBook backend that automatically generates a sitemap.xml

file when `mdbook build` is run. It reads the book structure from stdin
(as JSON provided by mdbook) and writes sitemap.xml to the output directory.

# Usage in book.toml

```toml
[output.html]

[output.sitemap]
base-url = "https://example.com/docs/"
# Optional settings:
# change-freq = "weekly"          # always|hourly|daily|weekly|monthly|yearly|never
# priority = 0.7                  # 0.0 - 1.0
# output-filename = "sitemap.xml" # output filename
# include-draft = false           # whether to include draft chapters
```

# Licence

MIT license
