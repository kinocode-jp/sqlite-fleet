# Security Policy

## Supported Versions

`sqlite-fleet` is currently pre-1.0. Security fixes are expected to be released on the latest `0.x` line.

## Security Model

`sqlite-fleet` is an operator-controlled CLI/library for managing many SQLite database files.

It assumes the following inputs are trusted and reviewed by project operators:

- `sqlite-fleet.toml`
- migration SQL files
- discovery SQL queries
- the directory containing target database files

Do not allow untrusted users to write configuration, discovery queries, or migration SQL.

## Built-in Safeguards

- Unknown TOML fields are rejected to avoid typo-driven defaults.
- Configured paths must stay under the configuration file directory.
- Surrounding whitespace and `..` path components are rejected for configured paths.
- Symlink escapes are rejected for database paths, migration files, and report output.
- Missing database files are not created by migration/status/check operations.
- Read-only commands do not create the migration history table.
- Migration SQL is rejected when it contains explicit transaction control, `ATTACH`, `DETACH`, `VACUUM`, selected dangerous PRAGMAs, or direct writes/DDL against the history table.
- Applied migration checksums are verified before further migrations run.

## Operational Guidance

- Back up production databases before running `migrate`.
- Run `doctor`, `plan`, and `migrate --dry-run` before production migration.
- Store JSON reports from CI/CD runs.
- Keep migration SQL in source control and require code review.
- Use conservative `--parallel` values on busy systems or slow disks.
- Treat any non-zero exit code as deployment failure.

## Reporting a Vulnerability

If the repository is public, report suspected vulnerabilities through GitHub Issues.

If the issue includes sensitive exploit details, data exposure steps, or a working proof of concept, do not post full details publicly. Open an issue with a short summary first so maintainers can coordinate a safer disclosure path.

Once GitHub Security Advisories and private vulnerability reporting are enabled, prefer that GitHub flow for sensitive reports.

Please include:

- affected version or commit
- impact summary
- reproduction steps
- whether the issue can alter, delete, or exfiltrate data
- suggested fix, if known

Do not publish exploit details before maintainers have had a reasonable chance to respond.
