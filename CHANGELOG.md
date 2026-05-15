# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/0809android/sqlite-fleet/compare/v0.3.0...v0.4.0) - 2026-05-15

### Other

- Split GUI settings and permission edit flags

## [0.3.0](https://github.com/0809android/sqlite-fleet/compare/v0.2.0...v0.3.0) - 2026-05-15

### Other

- Update README SQL Console documentation
- Auto-load SQL schema without refresh button
- SQLからマイグレーションを作成する導線を追加
- Remove DDL builder page from GUI
- Reorder schema panel safety description
- Localize schema SQL builder labels
- Move create actions into page header
- 重複するページ内見出しを整理
- Move setup description into page heading
- Unify GUI migration and backup terminology
- Localize allowed roots setup text
- Align checklist action buttons with status pills
- Move setup controls into Settings
- Clarify setup checklist status labels
- Add initial setup page and checklist
- Restore white settings section background
- Use app background for settings sections
- Move settings accordion indicator next to label
- Remove duplicate project name summary
- Allow settings tooltips to overflow and wrap
- Add contextual settings tooltips
- Reject parent components in allowed roots
- Add allowed roots and discovery preview settings
- Localize GUI validation and API errors
- Document GUI network exposure guidance in English
- Document GUI loopback-only access guidance

## [0.2.0](https://github.com/0809android/sqlite-fleet/compare/v0.1.3...v0.2.0) - 2026-05-14

### Changed

- Treat migration file names as the migration identity instead of bare versions.
- Keep `main` as the implicit default migration group when no explicit migration groups are configured.
- Add GUI editing for DB discovery, migration/output/backup/audit paths, runtime settings, and GUI permissions.
- Add GUI support for importing existing migration directories through migration group `dir`.
- Add a GUI baseline action that records pending migrations as already applied without executing their SQL.
- Read legacy `version` primary-key history tables and upgrade them to `filename` primary-key history after legacy entries resolve to local migration files and checksum validation passes.
- Harden GUI SQL/file operations by rejecting `VACUUM INTO`, avoiding final symlink writes, and using randomized config save temp files.
- Disable destructive GUI permissions by default and document that GUI restore is not currently available.
- Declare Rust 1.85 as the minimum supported Rust version.

## [0.1.3](https://github.com/0809android/sqlite-fleet/compare/v0.1.1...v0.1.3) - 2026-05-09

### Other

- release v0.1.3
- Add local GUI database manager

## [0.1.1](https://github.com/0809android/sqlite-fleet/compare/v0.1.0...v0.1.1) - 2026-05-09

### Other

- Point crate metadata to repository
