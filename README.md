# sqlite-fleet

`sqlite-fleet` は、多数の SQLite データベースファイルをまとめて管理するための Rust 製 CLI / ライブラリです。

マルチテナント、ユーザーごとDB、ワークスペースごとDB、店舗ごとDBなど、SQLite ファイルをデータ境界として大量に持つプロジェクトで使うことを想定しています。

## インストール

```bash
cargo install sqlite-fleet --locked
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
sqlite-fleet gui
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
sqlite-fleet migrate --backup
sqlite-fleet migrate --group canary --limit 1 --backup
sqlite-fleet backup --database tenant-a
sqlite-fleet restore --database tenant-a --from ./backups/tenant-a/1770000000000000000_tenant-a.db
sqlite-fleet drift
sqlite-fleet check
sqlite-fleet doctor
sqlite-fleet --json doctor
sqlite-fleet gui
sqlite-fleet gui --host 127.0.0.1 --port 8765
```

`--config`、`--json`、`--parallel` はグローバルオプションです。サブコマンドより前に指定します。

```bash
sqlite-fleet --config sqlite-fleet.toml --json status
sqlite-fleet --config sqlite-fleet.toml --parallel 8 migrate
```

## GUI

ローカルブラウザでDB管理画面を開くには `gui` を実行します。

```bash
sqlite-fleet --config sqlite-fleet.toml gui
```

デフォルトでは `127.0.0.1:8765` でHTTPサーバを起動します。GUIはローカル操作用のため、必要なときだけ起動し、`--host 127.0.0.1` でループバックアドレスに閉じる運用を推奨します。`--host` にはループバックアドレスだけ指定でき、`--host 0.0.0.0` などでGUIを公開する運用はサポートされません。外部から接続させる必要がある場合も、GUIはローカルにbindしたまま、SSHトンネル、VPN、Zero Trustアクセスなどを利用者側の責任で構成してください。

By default, the GUI starts an HTTP server on `127.0.0.1:8765`. The GUI is intended for local operation, so start it only when needed and prefer `--host 127.0.0.1` to keep it bound to a loopback address. `--host` only accepts loopback addresses, and publishing the GUI with `--host 0.0.0.0` or another non-loopback address is not supported. If remote access is required, keep the GUI bound locally and configure SSH tunnels, VPN, Zero Trust access, or equivalent controls on the user/operator side.

管理画面は「何を適用するか」と「どこへ適用するか」を分けて表示します。

- マイグレーション: ファイル名単位でSQL内容、所属グループ、適用済DB、未適用DBを確認します。グループに分けたい場合だけ、画面上でマイグレーショングループを作成します。
- DB: DB単位で対象マイグレーショングループ、適用済み件数、未適用マイグレーション、チェックサム不一致などを確認します。DBグループはDBをまとめて操作したい場合にだけ使います。
- オーバービュー: 最新マイグレーション、未適用があるDB、失敗・不整合の有無をまとめて確認します。

画面上から `check`、`migrate --dry-run`、個別DBまたは全DBへの `migrate`、バックアップを実行できます。デフォルトでは読み取り系の `check` だけを許可し、DBを書き換える操作、バックアップ、SQL適用、マイグレーション編集は `[gui]` の `allow_*` を明示的に `true` にした場合だけ有効になります。GUIの操作権限は `sqlite-fleet.toml` の `[gui]` で必要最小限に絞ってください。現在のGUIにはrestore実行画面はないため、復元はCLIの `sqlite-fleet restore` を使ってください。マイグレーション詳細のDB行には「そのSQLだけ適用する」ボタンは出しません。`migrate` は対象DBの未適用マイグレーションを順番に適用する操作だからです。

From the GUI, you can run `check`, `migrate --dry-run`, `migrate` for one or all databases, and backup operations. By default, only the read-only `check` operation is allowed. Operations that write databases, backup, SQL apply, and migration editing are enabled only when the corresponding `[gui]` `allow_*` setting is explicitly set to `true`. Restrict GUI operation permissions to the minimum necessary in `sqlite-fleet.toml` under `[gui]`. The current GUI does not provide a restore screen, so use `sqlite-fleet restore` from the CLI for restores. The migration detail database rows do not include an "apply only this SQL" button because `migrate` applies the pending migrations for the target database in order.

`新規` では、まだマイグレーション化していないSQLを選択DBに対してDry runまたは適用できます。SQLファイルの読み込み、SQLite SQLスニペットの挿入、スキーマ変更SQLの生成、SQLファイル保存もできます。GUI SQL適用は通常のSQLをatomic transactionで実行し、途中で失敗した場合はrollbackします。atomicにできない `VACUUM` / `PRAGMA journal_mode` は単独SQLとしてだけ適用できます。外部ファイルや外部DBへ影響する `VACUUM INTO` / `ATTACH` / `DETACH` はGUI SQLでは拒否されます。実際にDBを変更する操作はブラウザ側で確認ダイアログを表示します。

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

[migration_groups]
main = ["001_create_items.sql", "002_add_item_index.sql"]
premium = ["001_create_items.sql", "002_add_item_index.sql", "101_create_subscription.sql"]

[database_migration_groups]
tenant-a = ["main", "premium"]
tenant-b = ["main"]

[execution]
parallel = 4
lock_timeout_ms = 5000
continue_on_error = true

[report]
format = "json"
path = "./sqlite-fleet-report.json"

[backup]
dir = "./backups"
before_migrate = false
keep_last = 10

[audit]
path = "./sqlite-fleet-audit.jsonl"

[security]
allowed_roots = [
  ".",
  "./data",
  "/var/lib/myapp"
]

[gui]
allow_check = true
allow_migrate = false
allow_backup = false
allow_restore = false
allow_sql_apply = false
allow_migration_edit = false

[db_groups]
canary = ["tenant-a", "tenant-b"]
```

`migrate`、`backup`、`restore` はDBファイルごとに `*.sqlite-fleet.lock` を作り、同じDBに対する sqlite-fleet 経由の変更操作を直列化します。別プロセスやGUIから同じDBへ同時操作した場合、後続操作は `execution.lock_timeout_ms` まで待機し、取得できなければ `DBは別のsqlite-fleet操作中です` として失敗します。処理が正常終了または通常のエラーで戻る場合、ロックファイルは自動削除されます。

`lock_timeout_ms` はSQLiteの `busy_timeout` と sqlite-fleet のDB操作ロック待ち時間の両方に使います。外部アプリや `sqlite3` が直接DBを書く操作はこのロックを見ないため、完全に止めたい場合はアプリ側でも同じ運用ルールに揃えてください。

設定ファイルでは未知フィールドを拒否します。`path_globb` のような typo は無視せずエラーになります。

設定由来のパスはデフォルトでは設定ファイルのディレクトリ配下に限定されます。外部DBディレクトリ、外部マイグレーションディレクトリ、外部report/backup先を使う場合は `[security].allowed_roots` に明示します。`.` は設定ファイルのディレクトリを表し、相対パスは設定ファイルのディレクトリ基準、絶対パスも指定できます。前後空白、親ディレクトリ成分 `..`、シンボリックリンクによる許可ルート外への脱出は安全側で拒否します。

`allowed_roots` は「どこを触れるか」を決める設定です。`allow_sql_apply`、`allow_migrate`、`allow_restore` など `[gui]` の `allow_*` は「何をできるか」を決める設定で、互いに独立しています。GUI の Settings では許可ルートを一覧、追加、削除でき、resolved path、存在しないroot、広すぎるrootの警告を表示します。保存時には、指定ディレクトリ配下の DB / migration / report を操作できるようになることを確認します。

## 既存プロジェクトへの導入

sqlite-fleet は、新規プロジェクト専用ではなく、既存のSQLite DBと既存のマイグレーションファイルを取り込んで管理する前提です。

DBは設定ファイルからの相対パス、または `[security].allowed_roots` 配下の絶対パスで検出します。DBファイルがディレクトリに並んでいる場合は `databases.discovery = "glob"` と `databases.path_glob` を使います。親DBや管理DBにtenant一覧がある場合は `databases.discovery = "query"` を使い、`source`、`query`、`id_column`、`path_column` または `path_template` で対象DBを列挙します。GUIの設定画面では、この仕組みを「DB検出」と表示し、保存前の入力内容でDB検出プレビューを実行できます。

既存のマイグレーションディレクトリは `migrations.dir` に指定します。複数ディレクトリに分かれている場合は、基本のディレクトリを `migrations.dir` に置き、追加ディレクトリをマイグレーショングループの `dir` として登録できます。

```toml
[migrations]
dir = "./db/migrate"

[migration_groups.legacy_admin]
dir = "./engines/admin/db/migrate"

[database_migration_groups]
admin = ["main", "legacy_admin"]
```

すでに本番DBへ適用済みのmigrationを初回導入時に再実行したくない場合は、GUIの実行画面で対象DBを選び、`対象を読み取り済みにする` を実行します。この操作はSQLを実行せず、未適用として見えているmigrationを履歴テーブルへ登録します。DBの実スキーマがmigration内容と一致していることを確認してから使ってください。

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

## マイグレーションファイル

マイグレーションファイルは `migrations.dir` に `<version>_<name>.sql` または `<name>_<version>.sql` 形式で置きます。通常は `[migration_groups]` を書かず、全ファイルを暗黙の `main` グループとして使えます。分岐したい場合だけ `[migration_groups]` にファイル名リストを書きます。既存プロジェクトで複数ディレクトリに分かれている場合は、`[migration_groups.<name>] dir = "..."` で追加ディレクトリを読み込めます。

sqlite-fleet では、マイグレーションの同一性は `version` ではなくファイル名全体です。同じ `001` や日付バージョンを持つファイルが複数あっても、ファイル名が違えば別マイグレーションとして扱います。同じファイル名が複数グループに含まれる場合は、DBごとに1回だけ適用されます。

```text
migrations/
  001_create_items.sql
  002_add_item_index.sql
  101_create_subscription.sql
```

```toml
[migration_groups]
main = ["001_create_items.sql", "002_add_item_index.sql"]
premium = ["001_create_items.sql", "002_add_item_index.sql", "101_create_subscription.sql"]
```

`[migration_groups]` を使わない設定では、`migrations.dir` 配下の全ファイルが `main` グループとして扱われます。GUIで新規グループを作ると、最初は `main` と同じマイグレーションを含む分岐として作られ、チェックを外すことで空グループにもできます。既存ディレクトリをそのまま取り込む場合は、マイグレーショングループ作成時にマイグレーションディレクトリを指定できます。新しく整理する設定では、基本ディレクトリ + ファイル名リスト形式を推奨します。

古い設定向けに `001` のようなversion指定も、単一ファイルに一致する場合だけ読み込めます。同じversionのファイルが複数ある場合は曖昧として拒否されるため、新しい設定では必ず `001_create_items.sql` のようなファイル名で指定してください。

各DBには `_sqlite_fleet_migrations` が作成され、適用済みファイル名、version、name、checksum、適用時刻、実行時間が保存されます。

旧バージョンで作成された `version` 主キーの履歴テーブルは、`status` / `check` / `migrate --dry-run` では読み取り互換として扱います。実際に `migrate` を実行すると、全ての旧履歴がローカルマイグレーションファイルへ解決でき、checksum検証も通った場合だけ、`filename` 主キーの新スキーマへ自動移行します。解決できない旧履歴がある場合はDBを書き換えずに失敗します。

`status`、`plan`、`check`、`migrate --dry-run` は読み取り系コマンドとして扱い、対象DBに管理テーブルを作成しません。管理テーブルは実際に `migrate` で適用するときだけ作成します。

## doctor とレポート

`report.path` を設定している場合、CLI は `status`、`plan`、`migrate`、`check`、`doctor` のJSONレポートを書き出します。

`status`、`plan`、`migrate`、`check` でDB不整合やマイグレーション失敗などの業務エラーとレポート書き込みエラーが同時に起きた場合、CLI の終了理由は業務エラーを優先します。`--json` 指定時は標準出力JSONを先に返し、その後にレポート書き込み失敗があれば非0終了します。

`doctor --json` は、設定ファイルの検証エラーや TOML 解析エラーも構造化されたJSONとして標準出力へ返します。`report.path` への書き込みに失敗しても、標準出力の診断結果を優先します。

## バックアップ、復元、監査

`backup` は対象DBの一貫したSQLiteコピーを `backup.dir` 配下へ作成します。`migrate --backup` または `backup.before_migrate = true` を使うと、本適用前にDBごとのbackupを取得します。

```bash
sqlite-fleet backup
sqlite-fleet backup --database tenant-a
sqlite-fleet migrate --backup
```

`restore` は復元前に現在のDBをbackupしてから、指定したbackupファイルでDBを置き換えます。

```bash
sqlite-fleet restore --database tenant-a --from ./backups/tenant-a/1770000000000000000_tenant-a.db
```

`audit.path` を設定すると、`migrate`、`backup`、`restore`、`drift` とGUI経由の主要操作がJSONLで追記されます。

## マイグレーショングループ、DBグループ、カナリア

`[migration_groups]` で「どのマイグレーションファイルがどのグループに属するか」を定義します。`[database_migration_groups]` でDB IDまたはDBパスselectorごとに、対象マイグレーショングループを指定します。未指定DBは `main` が対象です。

`[db_groups]` でDB IDまたはDBパスselectorのまとまりを定義できます。DB Groupの標準は全DBを表す `all` です。`--limit` と組み合わせるとカナリア適用に使えます。旧設定名 `[groups]` も互換のため読み込めます。

```bash
sqlite-fleet migrate --group canary --limit 1 --backup
sqlite-fleet migrate --group canary --backup
```

この場合、「何を適用するか」はマイグレーショングループ、「どこへ適用するか」はDBグループで分かれます。

```text
tenant-a -> main + premium
tenant-b -> main
canary   -> tenant-a, tenant-b
```

注意: 履歴テーブルはマイグレーションファイル名を主キーにします。同じ `001` や日付バージョンを複数のファイル名で使えますが、同じファイル名を別内容で重複させる構成は拒否されます。適用順は `version_number`、`version`、`group`、`name`、`filename` の順で安定化されます。

## スキーマdrift

`drift` は最初のDBをbaselineとして、他DBのtable/index/view/trigger定義差分を検出します。

```bash
sqlite-fleet drift
sqlite-fleet drift --group canary
```

## 終了コード

- `0`: コマンドが成功した
- 非0: 設定不備、DB検出失敗、マイグレーション失敗、チェックサム不一致、DB検査失敗、またはレポート書き込み失敗

CI/CD では `migrate`、`check`、`doctor` の非0終了を失敗として扱ってください。

## 本番運用チェックリスト

- 本番DBのバックアップを取得してから `migrate` を実行する
- `migrate --backup` または `backup.before_migrate = true` で本適用前backupを自動化する
- `sqlite-fleet doctor` で設定、DB検出、migration ファイルを検証する
- `sqlite-fleet plan` で対象DBと適用予定を確認する
- `sqlite-fleet migrate --dry-run` で読み取り専用の事前確認を行う
- 旧 `version` 主キーの履歴テーブルを使っているDBでは、初回 `migrate` 前に `status` / `check` で全ての旧履歴がローカルmigrationへ解決できることを確認する
- `report.path` を設定し、JSONレポートをCI/CD成果物として保存する
- `audit.path` を設定し、GUI/CLIの変更操作を監査ログとして保存する
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

`Config::load()` は `report.format` / `report.path` まで含めて全体検証します。CLI と同じように、レポート出力設定の検証を実際の書き込み時まで遅らせたい場合は `Config::load_for_operation()` を使います。DB検出だけを行う場合は `Config::load_for_discovery()` を使います。

`doctor` と `doctor_with_overrides` は `report.path` へ書き込まない診断APIです。レポート書き出しも必要な場合は `doctor_and_write_report` を使います。

## 安全側の制約

- 対象DBが存在しない場合、`migrate` はDBファイルを自動作成しません
- `status`、`plan`、`check`、`migrate --dry-run` は対象DBに管理テーブルを作成しません
- マイグレーションSQL内の明示的な transaction 制御は拒否します
- マイグレーションSQL内の `ATTACH`、`DETACH`、`VACUUM` は拒否します
- マイグレーションSQLとGUI SQLでは危険な `PRAGMA writable_schema` と `PRAGMA journal_mode=OFF` は拒否します
- GUI SQLでは `PRAGMA foreign_keys=OFF` と `PRAGMA ignore_check_constraints=ON` も拒否します
- GUI SQL適用は通常のSQLをatomic transactionで実行し、途中で失敗した場合はrollbackします
- GUI SQLでは明示的な transaction 制御と、外部DBへ影響しうる `ATTACH` / `DETACH` は拒否します
- GUI SQLでは外部ファイルへ影響しうる `VACUUM INTO` を拒否し、`VACUUM` / `PRAGMA journal_mode` は単独SQLとしてだけ許可します
- migration 管理テーブルへの直接変更やDDLは拒否します
- DB検出クエリは読み取り専用の `SELECT` / `WITH` 系だけを許可します
- TOML 設定の未知フィールドは拒否します
- 設定由来パスの前後空白と `..` は拒否します
- `report.path` のシンボリックリンク脱出は拒否し、JSONレポートは一時ファイル経由で置き換えます

## セキュリティ上の前提

`sqlite-fleet` は信頼済みの設定ファイルとマイグレーションSQLを運用者が管理する前提のツールです。信頼できないユーザーに設定ファイル、マイグレーションSQL、DB検出クエリを書かせる用途は想定していません。

防御策として、設定パスの脱出、危険なSQL構文、管理テーブル改変、チェックサム不一致、未知フィールド typo は拒否します。ただし、マイグレーションSQLは最終的に対象DBへDDL/DMLを実行するため、レビュー済みのSQLだけを配置してください。

脆弱性報告やセキュリティ方針は [SECURITY.md](SECURITY.md) を参照してください。

## 開発

```bash
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo publish --locked --dry-run
```

内部計画や作業メモは crate package に含めません。配布対象は `Cargo.toml` の `include` で `README.md`、`CHANGELOG.md`、`LICENSE`、`SECURITY.md`、`src/**`、`tests/**` を中心に絞っています。

## ライセンス

MIT License です。詳細は [LICENSE](LICENSE) を参照してください。
