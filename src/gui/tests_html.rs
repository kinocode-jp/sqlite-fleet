    #[test]
    fn html_does_not_use_inline_event_handlers() {
        assert!(!INDEX_HTML.contains("onclick="));
        assert!(INDEX_HTML.contains("data-action=\"migrate\""));
    }

    #[test]
    fn html_script_uses_nonce_placeholder() {
        assert!(INDEX_HTML.contains("<script nonce=\"__SCRIPT_NONCE__\">"));
        assert!(INDEX_HTML.contains("<style nonce=\"__SCRIPT_NONCE__\">"));
    }

    #[test]
    fn html_api_client_validates_json_envelope() {
        assert!(
            INDEX_HTML.contains("const { headers: optionHeaders = {}, ...fetchOptions } = options")
        );
        assert!(INDEX_HTML.contains("Object.assign({}, optionHeaders)"));
        assert!(INDEX_HTML.contains("Object.assign({}, fetchOptions, { headers })"));
        assert!(INDEX_HTML.contains("response.headers.get('content-type')"));
        assert!(INDEX_HTML
            .contains("const mediaType = contentType.split(';', 1)[0].trim().toLowerCase()"));
        assert!(INDEX_HTML.contains("mediaType !== 'application/json'"));
        assert!(INDEX_HTML.contains("JSON.parse(text)"));
        assert!(INDEX_HTML.contains("typeof payload.ok !== 'boolean'"));
        assert!(!INDEX_HTML.contains("response.json()"));
    }

    #[test]
    fn html_keeps_buttons_disabled_until_all_api_requests_finish() {
        assert!(INDEX_HTML.contains("let activeRequests = 0"));
        assert!(INDEX_HTML.contains(
            "activeRequests = value ? activeRequests + 1 : Math.max(0, activeRequests - 1)"
        ));
        assert!(INDEX_HTML.contains("function syncControlState()"));
        assert!(INDEX_HTML.contains("const disabled = activeRequests > 0"));
        assert!(INDEX_HTML.contains("button, input, select, textarea"));
        assert!(INDEX_HTML.contains("control.disabled = disabled"));
        assert!(!INDEX_HTML.contains("let busy = false"));
    }

    #[test]
    fn html_preserves_migrate_completion_message_after_refresh() {
        assert!(
            INDEX_HTML.contains("const showLoadedMessage = options.showLoadedMessage !== false")
        );
        assert!(INDEX_HTML.contains("await load({ showLoadedMessage: false })"));
        assert!(INDEX_HTML.contains("const selectedDatabase = $('sqlDatabase').value"));
        assert!(INDEX_HTML.contains(
            "if (!dryRun && selectedDatabase && (!database || selectedDatabase === database))"
        ));
    }

    #[test]
    fn html_preserves_sql_completion_message_after_schema_refresh() {
        assert!(INDEX_HTML.contains("async function loadSchema(options = {})"));
        assert!(INDEX_HTML.contains("await refreshAfterSqlRun(database, dryRun)"));
        assert!(INDEX_HTML.contains("async function refreshAfterSqlRun(database, dryRun)"));
        assert!(INDEX_HTML.contains("if (dryRun) return"));
        assert!(INDEX_HTML.contains("if (showLoadedMessage) message(t('schemaLoaded', database))"));
        assert!(INDEX_HTML.contains("if ($('sqlDatabase').value !== database) return"));
        assert!(INDEX_HTML.contains("nextSchemaState.database.id !== database"));
        assert!(INDEX_HTML
            .contains("throw new Error('schema response database does not match selected DB')"));
        assert!(INDEX_HTML.contains("if ($('sqlDatabase').value === database) clearSchema()"));
        assert!(INDEX_HTML.contains("await loadSchema({ showLoadedMessage: false })"));
        assert!(INDEX_HTML.contains(
            "await load({ showLoadedMessage: false });\n      if ($('sqlDatabase').value === database)"
        ));
    }

    #[test]
    fn html_disables_database_selector_when_no_databases_exist() {
        assert!(INDEX_HTML.contains("if (!plans.length)"));
        assert!(INDEX_HTML.contains(r#"`<option value="">${escapeHtml(t('noDb'))}</option>`"#));
        assert!(INDEX_HTML.contains("$('sqlDatabase').disabled = true"));
        assert!(INDEX_HTML.contains("$('sqlDatabase').disabled = activeRequests > 0"));
        assert!(INDEX_HTML.contains("clearSchema()"));
        assert!(INDEX_HTML.contains("$('sqlDatabase').addEventListener('change', clearSchema)"));
        assert!(INDEX_HTML.contains("function clearSchema()"));
        assert!(INDEX_HTML.contains("selectDbForSchema"));
        assert!(INDEX_HTML.contains("const selected = $('sqlDatabase').value"));
        assert!(INDEX_HTML.contains(
            "schemaState && schemaState.database && schemaState.database.id !== selected"
        ));
    }

    #[test]
    fn html_escapes_dynamic_table_values_consistently() {
        assert!(INDEX_HTML.contains("return String(value).replace"));
        assert!(INDEX_HTML.contains("<td>${escapeHtml(plan.applied_count)}</td>"));
        assert!(!INDEX_HTML.contains("<td>${plan.applied_count}</td>"));
    }

    #[test]
    fn html_escapes_dynamic_attribute_values_consistently() {
        assert!(INDEX_HTML.contains(r#"data-database="${escapeHtml(plan.database.id)}""#));
        assert!(INDEX_HTML.contains(r#"<option value="${escapeHtml(plan.database.id)}">"#));
        assert!(INDEX_HTML.contains(r#"<option value="${escapeHtml(key)}">"#));
        assert!(!INDEX_HTML.contains(r#"data-database="${plan.database.id}""#));
        assert!(!INDEX_HTML.contains(r#"<option value="${plan.database.id}">"#));
    }

    #[test]
    fn html_uses_sidebar_layout() {
        assert!(INDEX_HTML.contains(r#"<div class="layout">"#));
        assert!(INDEX_HTML.contains(r#"<aside class="sidebar">"#));
        assert!(INDEX_HTML.contains(r#"<main class="content">"#));
        assert!(INDEX_HTML.contains("grid-template-columns:272px minmax(0, 1fr)"));
        assert!(INDEX_HTML.contains(r#"<header class="topbar">"#));
        assert!(INDEX_HTML.contains(".sidebar-nav a[hidden] { display:none; }"));
        assert!(INDEX_HTML.contains(r#"<h2 id="pageHeading">実行</h2>"#));
        assert!(!INDEX_HTML.contains("実行計画から管理するSQLite fleet"));
        assert!(!INDEX_HTML.contains(
            "「何を適用するか」と「どこへ適用するか」を分けて確認し、DB群へ安全に展開します。"
        ));
        assert!(INDEX_HTML.contains("const pageTitles = {"));
        assert!(INDEX_HTML.contains("function updatePageHeading(page)"));
        assert!(INDEX_HTML
            .contains(r#"<section class="summary page active" data-page="execute" id="summary""#));
        assert!(INDEX_HTML.contains(
            r#"<section class="panel page active" data-page="execute" id="command-center">"#
        ));
        assert!(INDEX_HTML.contains(r#"<a href='#db-groups-panel' data-page-link="db-groups" data-conditional-nav="db-groups">"#));
        assert!(INDEX_HTML.contains(r#"<a href='#databases-panel' data-page-link="databases"><span class="nav-icon">DB</span>DB一覧</a>"#));
        assert!(INDEX_HTML.contains(r#"<a href='#migration-groups-panel' data-page-link="migration-groups" data-conditional-nav="migration-groups">"#));
        let migration_group_nav = INDEX_HTML
            .find(r#"<a href='#migration-groups-panel' data-page-link="migration-groups" data-conditional-nav="migration-groups">"#)
            .expect("migration group nav link exists");
        let migration_nav = INDEX_HTML
            .find(r#"<a href='#migrations-panel' data-page-link="migrations">"#)
            .expect("migration nav link exists");
        let db_group_nav = INDEX_HTML
            .find(r#"<a href='#db-groups-panel' data-page-link="db-groups" data-conditional-nav="db-groups">"#)
            .expect("db group nav link exists");
        assert!(migration_group_nav < migration_nav);
        assert!(migration_nav < db_group_nav);
        assert!(INDEX_HTML.contains("function updateConditionalNav()"));
        assert!(INDEX_HTML.contains("const hasMigrationGroups = (state.migration_groups || []).some((group) => group.name !== 'main')"));
        assert!(INDEX_HTML.contains(
            r#"<section class="panel page" data-page="migration-groups" id="migration-groups-panel">"#
        ));
        assert!(INDEX_HTML.contains(
            r#"<section class="panel page" data-page="db-groups" id="db-groups-panel">"#
        ));
    }

    #[test]
    fn html_includes_help_page_linked_from_sidebar() {
        assert!(INDEX_HTML.contains(
            r#"<a href='#help' data-page-link="help"><span class="nav-icon">?</span>ヘルプ</a>"#
        ));
        assert!(INDEX_HTML.contains(r#"<section class="panel page" data-page="help" id="help">"#));
        assert!(INDEX_HTML.contains("基本の考え方"));
        assert!(INDEX_HTML.contains("マイグレーションファイルは <code>migrations.dir</code>"));
        assert!(INDEX_HTML.contains("選択中の1ファイルだけを飛ばして適用する操作ではありません"));
        assert!(INDEX_HTML.contains("audit.path"));
    }

    #[test]
    fn html_supports_english_default_and_japanese_toggle() {
        assert!(INDEX_HTML.contains(r#"<html lang="en">"#));
        assert!(INDEX_HTML
            .contains(r#"<div class="language-toggle" role="group" aria-label="Language">"#));
        assert!(INDEX_HTML
            .contains(r#"<button type="button" data-locale-button="en">English</button>"#));
        assert!(
            INDEX_HTML.contains(r#"<button type="button" data-locale-button="ja">日本語</button>"#)
        );
        assert!(INDEX_HTML.contains("let currentLocale = localStorage.getItem('sqlite-fleet-locale') === 'ja' ? 'ja' : 'en'"));
        assert!(INDEX_HTML.contains("function translateStaticDom()"));
        assert!(INDEX_HTML.contains("function setLocale(locale)"));
        assert!(INDEX_HTML.contains("'実行': 'Run'"));
        assert!(INDEX_HTML.contains("'グループ分け': 'Split into groups'"));
        assert!(INDEX_HTML.contains("'Run': '実行'"));
        assert!(INDEX_HTML.contains("'SQL runner': 'SQL実行'"));
        assert!(INDEX_HTML.contains("'ヘルプ': 'Help'"));
        assert!(INDEX_HTML.contains("const staticJapaneseTranslations = {"));
        assert!(INDEX_HTML.contains("'Migration Groups': 'マイグレーショングループ'"));
        assert!(INDEX_HTML
            .contains("document.querySelectorAll('[data-locale-button]').forEach((button) =>"));
        assert!(INDEX_HTML.contains(
            "button.addEventListener('click', () => setLocale(button.dataset.localeButton))"
        ));
        assert!(INDEX_HTML.contains(
            "translateStaticDom();\n    openPage(pageFromHash());\n    renderSqlTemplates();\n    load();"
        ));
    }

    #[test]
    fn html_localizes_dynamic_render_labels() {
        assert!(INDEX_HTML.contains("[t('migrationGroupsLabel'), state.migration_groups.length]"));
        assert!(INDEX_HTML.contains("[t('dbGroupsLabel'), state.db_groups.length]"));
        assert!(INDEX_HTML.contains("[t('upToDate'), s.up_to_date]"));
        assert!(INDEX_HTML.contains("[t('pendingDb'), s.pending]"));
        assert!(INDEX_HTML.contains("${t('migrationsLabel')}"));
        assert!(INDEX_HTML.contains("t('migrationFilesLabel')"));
        assert!(INDEX_HTML.contains("escapeHtml(t('selectorsLabel'))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('previewLabel'))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('allowed'))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('disabled'))"));
        assert!(INDEX_HTML.contains("[t('lockTimeout'), `${settings.lock_timeout_ms} ms`]"));
        assert!(INDEX_HTML.contains("message(adminSuccessMessage(path, result.message))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('column'))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('objectDefinitions'))"));
    }

    #[test]
    fn html_uses_modal_for_migration_group_creation() {
        assert!(INDEX_HTML.contains(r#"<button id="openMigrationGroupModal" class="primary">グループ作成</button>"#));
        assert!(INDEX_HTML.contains(r#"id="migrationGroupModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"role="dialog" aria-modal="true" aria-labelledby="migrationGroupModalTitle""#));
        assert!(INDEX_HTML.contains(r#"<input id="manageMigrationGroupName" placeholder="premium">"#));
        assert!(INDEX_HTML.contains("function openMigrationGroupModal()"));
        assert!(INDEX_HTML.contains("function closeMigrationGroupModal()"));
        assert!(INDEX_HTML.contains("function renderMigrationGroupEditor()"));
        assert!(INDEX_HTML.contains("function saveMigrationGroupMembership()"));
        assert!(INDEX_HTML.contains("function uniqueMigrationsByVersion(migrations)"));
        assert!(INDEX_HTML.contains("const byVersion = new Map()"));
        assert!(INDEX_HTML.contains("if (!byVersion.has(migration.version)) byVersion.set(migration.version, migration)"));
        assert!(INDEX_HTML.contains(r#"<div id="migrationGroupChecklist" class="migration-checklist"></div>"#));
        assert!(INDEX_HTML.contains(r#"<button id="saveMigrationGroupMembership" class="primary">保存</button>"#));
        assert!(INDEX_HTML.contains(r#"<button id="openMigrationGroupModalInline" class="primary" hidden>グループ分け</button>"#));
        assert!(INDEX_HTML.contains(r#"<button id="openMigrationGroupModalFromMigrations" class="primary" hidden>グループ分け</button>"#));
        assert!(INDEX_HTML.contains(r#"<div id="migrationAssignmentEditor" class="migration-assignment-editor">"#));
        assert!(INDEX_HTML.contains(r#"id="migrationGroupSimpleNote" class="simple-mode-note" hidden"#));
        assert!(INDEX_HTML.contains("const simpleMode = groups.length === 1 && groups[0].name === 'main' && rules.length === 0"));
        assert!(INDEX_HTML.contains(r#"<button id="saveDatabaseRule" class="primary">割当を保存</button>"#));
        assert!(INDEX_HTML.contains("$('openMigrationGroupModal').addEventListener('click', openMigrationGroupModal)"));
        assert!(INDEX_HTML.contains("$('openMigrationGroupModalInline').addEventListener('click', openMigrationGroupModal)"));
        assert!(INDEX_HTML.contains("$('openMigrationGroupModalFromMigrations').addEventListener('click', openMigrationGroupModal)"));
        assert!(INDEX_HTML.contains("$('openMigrationGroupModalFromMigrations').hidden = hasMigrationGroups"));
        assert!(INDEX_HTML.contains("$('migrationGroupCards').addEventListener('click', (event) =>"));
        assert!(INDEX_HTML.contains("const name = requireValue('manageMigrationGroupName', 'Group name')"));
        assert!(INDEX_HTML.contains("(state.migration_groups || []).some((group) => group.name === name)"));
        assert!(INDEX_HTML.contains("throw new Error(t('duplicateMigrationGroup', name))"));
        assert!(INDEX_HTML.contains("duplicateMigrationGroup: (name) => `Migration Group already exists: ${name}`"));
        assert!(INDEX_HTML.contains("body: JSON.stringify({ name, versions: [] })"));
        assert!(INDEX_HTML.contains(".map((input) => input.dataset.migrationVersion)"));
        assert!(!INDEX_HTML.contains("manageMigrationGroupVersions"));
    }

    #[test]
    fn html_includes_contextual_tooltips_for_admin_fields() {
        assert!(INDEX_HTML.contains(r#"class="tool-tip""#));
        assert!(INDEX_HTML.contains(".panel { margin-bottom:18px; border:1px solid var(--line); border-radius:8px; background:var(--surface); box-shadow:var(--shadow); overflow:visible; }"));
        assert!(INDEX_HTML.contains(".tool-tip:hover, .tool-tip:focus { z-index:40; }"));
        assert!(INDEX_HTML.contains(r#"data-tip="DBグループはどのDBへ実行するか"#));
        assert!(INDEX_HTML.contains(r#"data-tip="db_groups/groups に定義した対象DBのまとまりです"#));
        assert!(INDEX_HTML
            .contains(r#"data-tip="SQL apply は自動的にatomic transactionで実行されます"#));
        assert!(INDEX_HTML.contains(r#"data-tip="UTF-8の.sqlまたはテキストファイルを読み込みます"#));
        assert!(INDEX_HTML.contains(r#"data-tip="履歴テーブルはversion主キーなので"#));
    }

    #[test]
    fn html_supports_sql_templates_upload_edit_and_download() {
        assert!(INDEX_HTML.contains("const sqlTemplates = {"));
        assert!(INDEX_HTML.contains("const maxSqlBytes = 2 * 1024 * 1024"));
        assert!(INDEX_HTML.contains("create_table_strict"));
        assert!(INDEX_HTML.contains("create_table_generated"));
        assert!(INDEX_HTML.contains("create_table_stored_generated"));
        assert!(INDEX_HTML.contains("create_table_foreign_key"));
        assert!(INDEX_HTML.contains("create_table_constraints"));
        assert!(INDEX_HTML.contains("create_table_as"));
        assert!(INDEX_HTML.contains("create_partial_index"));
        assert!(INDEX_HTML.contains("create_expression_index"));
        assert!(INDEX_HTML.contains("create_temp_view"));
        assert!(INDEX_HTML.contains("create_instead_of_trigger"));
        assert!(INDEX_HTML.contains("create_temp_trigger"));
        assert!(INDEX_HTML.contains("virtual_fts5"));
        assert!(INDEX_HTML.contains("virtual_rtree"));
        assert!(INDEX_HTML.contains("insert_select"));
        assert!(INDEX_HTML.contains("insert_returning"));
        assert!(INDEX_HTML.contains("insert_or_replace"));
        assert!(INDEX_HTML.contains("update_returning"));
        assert!(INDEX_HTML.contains("delete_returning"));
        assert!(INDEX_HTML.contains("BEFORE INSERT ON"));
        assert!(INDEX_HTML.contains(r#"WHEN NEW."name" IS NULL"#));
        assert!(INDEX_HTML.contains("savepoint"));
        assert!(INDEX_HTML.contains("recursive_cte"));
        assert!(INDEX_HTML.contains("explain_query_plan"));
        assert!(INDEX_HTML.contains("PRAGMA integrity_check"));
        assert!(INDEX_HTML.contains("PRAGMA quick_check"));
        assert!(INDEX_HTML.contains("PRAGMA optimize"));
        assert!(INDEX_HTML.contains("PRAGMA wal_checkpoint(PASSIVE)"));
        assert!(INDEX_HTML.contains("PRAGMA journal_mode (single apply only)"));
        assert!(INDEX_HTML.contains("PRAGMA journal_modeはatomic transactionでは実行できないため"));
        assert!(INDEX_HTML.contains("VACUUM INTO (single apply only)"));
        assert!(INDEX_HTML.contains("VACUUM INTOは外部ファイルを作成するため"));
        assert!(INDEX_HTML.contains("ATTACH DATABASE (external tool)"));
        assert!(INDEX_HTML.contains("ATTACH/DETACHは外部DBへ影響するため"));
        assert!(INDEX_HTML.contains("基本的にはdry-runで確認してから適用してください"));
        assert!(INDEX_HTML.contains("GUI applyは自動的にatomic transactionで実行されます"));
        assert!(INDEX_HTML.contains("PRAGMA journal_modeは単独SQLとしてだけ適用できます"));
        assert!(INDEX_HTML.contains(r#"<select id="sqlTemplate">"#));
        assert!(INDEX_HTML.contains(r#"<input id="sqlFile" type="file""#));
        assert!(INDEX_HTML.contains(r#"<textarea id="sqlInput""#));
        assert!(INDEX_HTML.contains(r#"<button id="downloadSql">SQLファイル保存</button>"#));
        assert!(INDEX_HTML.contains("new Blob([sql]"));
        assert!(INDEX_HTML.contains("sql.includes('\\u0000')"));
        assert!(INDEX_HTML.contains("SQLにNUL文字は指定できません"));
        assert!(INDEX_HTML.contains("SQLファイルにNUL文字は指定できません"));
        assert!(INDEX_HTML.contains("file.size > maxSqlBytes"));
        assert!(INDEX_HTML.contains("new TextEncoder().encode(sql).length > maxSqlBytes"));
        assert!(INDEX_HTML.contains("const maxSqlRequestBytes = maxSqlBytes + 4 * 1024"));
        assert!(INDEX_HTML.contains("const sqlBody = JSON.stringify({ sql })"));
        assert!(
            INDEX_HTML.contains("new TextEncoder().encode(sqlBody).length > maxSqlRequestBytes")
        );
        assert!(INDEX_HTML.contains("body: sqlBody"));
        assert!(INDEX_HTML.contains("file.arrayBuffer()"));
        assert!(INDEX_HTML.contains("new TextDecoder('utf-8', { fatal: true })"));
        assert!(INDEX_HTML.contains("SQLファイルをUTF-8として読み込めません"));
        assert!(!INDEX_HTML.contains("file.text()"));
    }

    #[test]
    fn html_sanitizes_download_sql_file_name() {
        assert!(INDEX_HTML.contains("function sanitizeSqlFileName(value)"));
        assert!(INDEX_HTML.contains("$('sqlFileName').value = sanitizeSqlFileName(file.name)"));
        assert!(INDEX_HTML.contains("replace(/^\\.+$/, '')"));
        assert!(INDEX_HTML.contains("replace(/^_+$/, '')"));
        assert!(INDEX_HTML.contains("name.toLowerCase().endsWith('.sql')"));
        assert!(INDEX_HTML.contains("const url = URL.createObjectURL(blob)"));
        assert!(INDEX_HTML.contains("URL.revokeObjectURL(url)"));
    }

    #[test]
    fn html_displays_hidden_schema_columns() {
        assert!(INDEX_HTML.contains("escapeHtml(table.type || t('table'))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('hidden'))"));
        assert!(INDEX_HTML.contains("hiddenColumnLabel(column.hidden)"));
        assert!(INDEX_HTML.contains("generated virtual"));
        assert!(INDEX_HTML.contains("generated stored"));
    }

    #[test]
    fn html_displays_schema_object_definitions() {
        assert!(INDEX_HTML.contains("objectDefinitions: 'Object Definitions'"));
        assert!(INDEX_HTML.contains("escapeHtml(t('objectDefinitions'))"));
        assert!(INDEX_HTML.contains("schemaState.objects"));
        assert!(INDEX_HTML.contains("object.table_name"));
        assert!(INDEX_HTML.contains("schema-sql"));
    }
