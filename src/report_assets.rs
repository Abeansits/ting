//! Embedded renderer assets keep exported reports usable without a CDN.

pub const SECURITY_POLICY: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'">"#;

const MARKED: &str = include_str!("../vendor/report/marked.min.js");
const PURIFY: &str = include_str!("../vendor/report/purify.min.js");
const MARKED_LICENSE: &str = include_str!("../vendor/report/marked-LICENSE.md");
const PURIFY_LICENSE: &str = include_str!("../vendor/report/dompurify-LICENSE");

/// Shared sanitizer for standalone reports and the local dashboard.
pub const RENDER_JS: &str = r#"
function renderMarkdown(source) {
  return DOMPurify.sanitize(marked.parse(source), {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ['img', 'style', 'video', 'audio', 'source', 'form', 'input', 'button'],
    FORBID_ATTR: ['style']
  });
}
document.querySelectorAll('.md-render').forEach(el => {
  const source = el.querySelector('.md-src');
  if (source) el.innerHTML = renderMarkdown(source.value);
});
"#;

/// Include pinned libraries and notices in every exported artifact.
pub fn scripts() -> String {
    // Developer tools must not try to fetch unbundled source maps either.
    let purify = PURIFY
        .lines()
        .filter(|line| !line.starts_with("//# sourceMappingURL="))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<script>{MARKED}</script>\n<script>{purify}</script>\n<script>{RENDER_JS}</script>\n<!--\nThird-party notices\n{MARKED_LICENSE}\n{PURIFY_LICENSE}\n-->"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_are_safe_for_inline_script_elements() {
        for script in [MARKED, PURIFY, RENDER_JS] {
            assert!(!script.to_ascii_lowercase().contains("</script"));
        }
        for license in [MARKED_LICENSE, PURIFY_LICENSE] {
            assert!(!license.contains("-->"));
        }
        let html = scripts();
        assert!(!html.contains("<script src="));
        assert!(!html.contains("sourceMappingURL="));
        assert!(html.contains("Permission is hereby granted"));
        assert!(html.contains("Apache License"));
    }

    #[test]
    fn evaluation_export_embeds_the_same_assets_and_policy() {
        let dir = std::env::temp_dir().join(format!("ting-offline-eval-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, content) in [
            (
                "meta.toml",
                "[eval]\ntopic = \"Offline evaluation\"\nbaseline = \"sample\"\njudge = \"sample\"\n",
            ),
            (
                "scores.toml",
                "[baseline]\naverage = 5\n[forum]\naverage = 7\n",
            ),
            ("baseline.md", "# Baseline\nA sample response."),
            (
                "comparison.md",
                "# Comparison\nThe forum adds useful dissent.",
            ),
        ] {
            std::fs::write(dir.join(name), content).unwrap();
        }
        let html = crate::eval::generate_eval_html(&dir).unwrap();
        assert!(html.contains(SECURITY_POLICY));
        assert!(!html.contains("<script src="));
        assert!(html.contains("Third-party notices"));
        assert!(html.contains("The forum adds useful dissent."));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn forum_export_is_self_contained_and_escapes_model_text() {
        let dir =
            std::env::temp_dir().join(format!("ting-offline-report-{}", uuid::Uuid::new_v4()));
        crate::demo::create(&dir, "ting-demo-export").unwrap();
        std::fs::write(
            dir.join("final/dissent.md"),
            "</textarea><script>alert('unsafe')</script>",
        )
        .unwrap();
        let cfg = crate::config::load(&dir.join("meta.toml")).unwrap();
        let html = crate::report::generate_html_report(&cfg, &dir).unwrap();
        assert!(html.contains(SECURITY_POLICY));
        assert!(!html.contains("<script src="));
        assert!(!html.contains("https://cdn.jsdelivr.net"));
        assert!(html.contains("&lt;/textarea&gt;&lt;script&gt;alert('unsafe')&lt;/script&gt;"));
        assert!(html.contains("https://github.com/Abeansits/ting"));
        assert!(!html.contains("https://github.com/Abeansits/agora"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
