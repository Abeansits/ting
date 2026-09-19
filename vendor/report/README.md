# Offline report assets

These upstream files are embedded in the Ting binary and in exported HTML reports.
They replace runtime CDN requests. Reports also include the required upstream
notices; Ting uses DOMPurify under its Apache-2.0 option. The upstream alternate
MPL license is preserved here for completeness.

| Dependency | Version | Upstream |
| --- | --- | --- |
| Marked | 15.0.12 | https://github.com/markedjs/marked |
| DOMPurify | 3.4.15 | https://github.com/cure53/DOMPurify |

`manifest.json` records exact archive integrity and file SHA-256 hashes. Files are
unmodified from their public npm packages. The exporter removes the source-map
reference from its embedded copy, so opening developer tools does not require a
separate map file.

To reproduce the vendored files, run `python3 scripts/vendor-report-assets.py`.
To update, explicitly review and change the pinned version/integrity pair, run the
script, then validate export rendering and sanitization. Review sanitizer security
updates regularly. No npm installation or build step is required for users.
