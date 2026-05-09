# sqlite-fleet

`sqlite-fleet` は、多数の SQLite データベースファイルをまとめて管理するための Rust 製 CLI / ライブラリです。

マルチテナント、ユーザーごとDB、ワークスペースごとDB、店舗ごとDBなど、SQLite ファイルをデータ境界として大量に持つプロジェクトで使うことを想定しています。

## インストール

crates.io 公開後:

```bash
cargo install sqlite-fleet --locked
```

ローカルビルド:

```bash
cargo install --path . --locked
```

単一バイナリとして組み込みたい場合は、CIで `cargo build --release` を実行し、`target/release/sqlite-fleet` をデプロイ成果物に含めます。

## クイックスタート

```bash
sqlite-fleet init
mkdir -p data migrations
sqlite-fleet discover
sqlite-fleet migrate --dry-run
sqlite-fleet migrate
sqlite-fleet status
```

本番適用前は、少なくとも `doctor`、`plan`、`migrate --dry-run` を通してください。

```bash
sqlite-fleet --config sqlite-fleet.toml doctor
sqlite-fleet --config sqlite-fleet.toml plan
sqlite-fleet --config sqlite-fleet.toml migrate --dry-run
sqlite-fleet --config sqlite-fleet.toml migrate
```

## コマンド

```bash
sqlite-fleet init
sqlite-fleet discover
sqlite-fleet status
sqlite-fleet --json status
sqlite-fleet plan
sqlite-fleet migrate
sqlite-fleet migrate --dry-run
sqlite-fleet --parallel 8 migrate --continue-on-error
sqlite-fleet check
sqlite-fleet doctor
sqlite-fleet --json doctor
```

`--config`、`--json`、`--parallel` はグローバルオプションです。サブコマンドより前に指定します。

```bash
sqlite-fleet --config sqlite-fleet.toml --json status
sqlite-fleet --config sqlite-fleet.toml --parallel 8 migrate
```

## 設定例

```toml
[project]
name = "my-project"

[databases]
discovery = "glob"
path_glob = "./data/**/*.db"

[migrations]
dir = "./migrations"
table = "_sqlite_fleet_migrations"

[execution]
parallel = 4
lock_timeout_ms = 5000
continue_on_error = true

[report]
format = "json"
path = "./sqlite-fleet-report.json"
```

設定ファイルでは未知フィールドを拒否します。`path_globb` のような typo は無視せずエラーになります。

設定由来のパスは設定ファイルのディレクトリ配下に限定されます。前後空白、親ディレクトリ成分 `..`、シンボリックリンクによる `base_dir` 外への脱出は安全側で拒否します。

## 親DBから対象DBを列挙する例

```toml
[databases]
discovery = "query"
source = "./data/shared.db"
query = "SELECT id FROM tenants WHERE is_active = 1"
id_column = "id"
path_template = "./data/tenants/{id:08:split2}.db"
```

`{id:08:split2}` は `1234` を `00/00/00001234` のように展開します。

## migration ファイル

`migrations` ディレクトリに `<version>_<name>.sql` 形式で置きます。

```text
migrations/
  001_create_items.sql
  002_add_item_index.sql
```

各DBには `_sqlite_fleet_migrations` が作成され、適用済み version、name、checksum、適用時刻、実行時間が保存されます。

`status`、`plan`、`check`、`migrate --dry-run` は読み取り系コマンドとして扱い、対象DBに管理テーブルを作成しません。管理テーブルは実際に `migrate` で適用するときだけ作成します。

## doctor とレポート

`report.path` を設定している場合、CLI は `status`、`plan`、`migrate`、`check`、`doctor` のJSONレポートを書き出します。

`status`、`plan`、`migrate`、`check` でDB不整合やmigration失敗などの業務エラーとレポート書き込みエラーが同時に起きた場合、CLI の終了理由は業務エラーを優先します。`--json` 指定時は標準出力JSONを先に返し、その後にレポート書き込み失敗があれば非0終了します。

`doctor --json` は、設定ファイルの検証エラーや TOML 解析エラーも構造化されたJSONとして標準出力へ返します。`report.path` への書き込みに失敗しても、標準出力の診断結果を優先します。

## 終了コード

- `0`: コマンドが成功した
- 非0: 設定不備、DB探索失敗、migration失敗、checksum不一致、DB検査失敗、またはレポート書き込み失敗

CI/CD では `migrate`、`check`、`doctor` の非0終了を失敗として扱ってください。

## 本番運用チェックリスト

- 本番DBのバックアップを取得してから `migrate` を実行する
- `sqlite-fleet doctor` で設定、DB discovery、migration ファイルを検証する
- `sqlite-fleet plan` で対象DBと適用予定を確認する
- `sqlite-fleet migrate --dry-run` で読み取り専用の事前確認を行う
- `report.path` を設定し、JSONレポートをCI/CD成果物として保存する
- `--parallel` はDB数、ディスクI/O、ロック状況に合わせて控えめに設定する
- 失敗時はレポートの `failed_databases` / `databases[].error` を確認し、原因を直して再実行する

## ライブラリAPI

Rust crate としても利用できます。

```rust
use anyhow::Result;
use sqlite_fleet::{doctor, discover_databases, migrate, Config};

fn main() -> Result<()> {
    let discovery_config = Config::load_for_discovery("sqlite-fleet.toml")?;
    let databases = discover_databases(&discovery_config)?;

    let operation_config = Config::load_for_operation("sqlite-fleet.toml")?;
    let report = migrate(&operation_config, false, None)?;

    let diagnosis = doctor("sqlite-fleet.toml");
    Ok(())
}
```

`Config::load()` は `report.format` / `report.path` まで含めて全体検証します。CLI と同じように、レポート出力設定の検証を実際の書き込み時まで遅らせたい場合は `Config::load_for_operation()` を使います。DB探索だけを行う場合は `Config::load_for_discovery()` を使います。

`doctor` と `doctor_with_overrides` は `report.path` へ書き込まない診断APIです。レポート書き出しも必要な場合は `doctor_and_write_report` を使います。

## 安全側の制約

- 対象DBが存在しない場合、`migrate` はDBファイルを自動作成しません
- `status`、`plan`、`check`、`migrate --dry-run` は対象DBに管理テーブルを作成しません
- migration SQL 内の明示的な transaction 制御は拒否します
- `ATTACH`、`DETACH`、`VACUUM` は拒否します
- 危険な `PRAGMA writable_schema` と `PRAGMA journal_mode=OFF` は拒否します
- migration 管理テーブルへの直接変更やDDLは拒否します
- discovery query は読み取り専用の `SELECT` / `WITH` 系だけを許可します
- TOML 設定の未知フィールドは拒否します
- 設定由来パスの前後空白と `..` は拒否します
- `report.path` のシンボリックリンク脱出は拒否し、JSONレポートは一時ファイル経由で置き換えます

## セキュリティ上の前提

`sqlite-fleet` は信頼済みの設定ファイルと migration SQL を運用者が管理する前提のツールです。信頼できないユーザーに設定ファイル、migration SQL、discovery query を書かせる用途は想定していません。

防御策として、設定パスの脱出、危険なSQL構文、管理テーブル改変、checksum不一致、未知フィールド typo は拒否します。ただし、migration SQL は最終的に対象DBへDDL/DMLを実行するため、レビュー済みのSQLだけを配置してください。

脆弱性報告やセキュリティ方針は [SECURITY.md](SECURITY.md) を参照してください。

## 開発

```bash
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
cargo package --allow-dirty
```

内部計画や作業メモは crate package に含めません。配布対象は `Cargo.toml` の `include` で `README.md`、`LICENSE`、`src/**`、`tests/**` を中心に絞っています。

## ライセンス

MIT License です。詳細は [LICENSE](LICENSE) を参照してください。
