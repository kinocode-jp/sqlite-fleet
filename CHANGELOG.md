# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Treat migration file names as the migration identity instead of bare versions.
- Keep `main` as the implicit default migration group when no explicit migration groups are configured.
- Read legacy `version` primary-key history tables and upgrade them to `filename` primary-key history after legacy entries resolve to local migration files and checksum validation passes.

## [0.1.3](https://github.com/0809android/sqlite-fleet/compare/v0.1.1...v0.1.3) - 2026-05-09

### Other

- release v0.1.3
- Add local GUI database manager

## [0.1.1](https://github.com/0809android/sqlite-fleet/compare/v0.1.0...v0.1.1) - 2026-05-09

### Other

- Point crate metadata to repository
