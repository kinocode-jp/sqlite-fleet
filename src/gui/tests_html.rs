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
        assert!(INDEX_HTML.contains(
            "control.disabled = control.dataset.alwaysDisabled === 'true' || disabled"
        ));
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
        assert!(INDEX_HTML.contains(r#"<option value="${escapeHtml(template[0])}" data-key="${escapeHtml(key)}"></option>"#));
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
        assert!(INDEX_HTML.contains(r#"<div id="messageBar" class="message muted" hidden>"#));
        assert!(INDEX_HTML.contains(r#"<button id="clearMessage" type="button" aria-label="Close">×</button>"#));
        assert!(INDEX_HTML.contains("function clearMessage()"));
        assert!(INDEX_HTML.contains("$('clearMessage').addEventListener('click', clearMessage)"));
        assert!(!INDEX_HTML.contains(r#"<p id="message" class="message muted">読み込み中...</p>"#));
        assert!(INDEX_HTML.contains(".sidebar-nav a[hidden] { display:none; }"));
        assert!(INDEX_HTML.contains(r#"<h2 id="pageHeading">マイグレーション適用</h2>"#));
        assert!(!INDEX_HTML.contains("実行計画から管理するSQLite fleet"));
        assert!(!INDEX_HTML.contains(
            "「何を適用するか」と「どこへ適用するか」を分けて確認し、DB群へ安全に展開します。"
        ));
        assert!(INDEX_HTML.contains("const pageTitles = {"));
        assert!(INDEX_HTML.contains("function updatePageHeading(page)"));
        assert!(INDEX_HTML.contains("function hashForPage(page)"));
        assert!(INDEX_HTML.contains("function navigateToPage(page, options = {})"));
        assert!(INDEX_HTML.contains("history.pushState(null, '', hash)"));
        assert!(INDEX_HTML.contains("window.addEventListener('popstate', () => openPage(pageFromHash()))"));
        assert!(INDEX_HTML
            .contains(r#"<section class="summary page active" data-page="execute" id="summary""#));
        assert!(INDEX_HTML.contains(
            r#"<section class="panel page active" data-page="execute" id="command-center">"#
        ));
        assert!(INDEX_HTML.contains(r#"<a href='#databases-panel' data-page-link="databases"><span class="nav-icon">DB</span>DB</a>"#));
        assert!(INDEX_HTML.contains(r#"<a href='#migration-groups-panel' data-page-link="migration-groups" data-conditional-nav="migration-groups">"#));
        let migration_group_nav = INDEX_HTML
            .find(r#"<a href='#migration-groups-panel' data-page-link="migration-groups" data-conditional-nav="migration-groups">"#)
            .expect("migration group nav link exists");
        let migration_nav = INDEX_HTML
            .find(r#"<a href='#migrations-panel' data-page-link="migrations">"#)
            .expect("migration nav link exists");
        let database_nav = INDEX_HTML
            .find(r#"<a href='#databases-panel' data-page-link="databases">"#)
            .expect("database nav link exists");
        assert!(migration_group_nav < migration_nav);
        assert!(migration_nav < database_nav);
        assert!(INDEX_HTML.contains("function updateConditionalNav()"));
        assert!(INDEX_HTML.contains("if (hash === '#db-groups-panel') return 'databases'"));
        assert!(INDEX_HTML.contains("const hasMigrationGroups = (state.migration_groups || []).some((group) => group.name !== 'main')"));
        assert!(INDEX_HTML.contains(
            r#"<section class="panel page" data-page="migration-groups" id="migration-groups-panel">"#
        ));
        assert!(!INDEX_HTML.contains(r#"data-page-link="db-groups""#));
        assert!(!INDEX_HTML.contains(r#"data-page="db-groups""#));
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
        assert!(INDEX_HTML.contains("'マイグレーション適用': 'Migration Apply'"));
        assert!(INDEX_HTML.contains("'グループ作成': 'Create group'"));
        assert!(INDEX_HTML.contains("'新規作成': 'Create new'"));
        assert!(INDEX_HTML.contains("'新規マイグレーション': 'New migration'"));
        assert!(INDEX_HTML.contains("'Version': 'Version'"));
        assert!(INDEX_HTML.contains("'File name': 'File name'"));
        assert!(INDEX_HTML.contains("'Status': 'Status'"));
        assert!(INDEX_HTML.contains("'Applied': 'Applied'"));
        assert!(INDEX_HTML.contains("'Pending': 'Pending'"));
        assert!(INDEX_HTML.contains("'Path': 'Path'"));
        assert!(INDEX_HTML.contains("'Actions': 'Actions'"));
        assert!(INDEX_HTML.contains("'テンプレート': 'Template'"));
        assert!(INDEX_HTML.contains("'スニペット': 'Snippet'"));
        assert!(INDEX_HTML.contains("'フォーマット': 'Format'"));
        assert!(INDEX_HTML.contains("'クリア': 'Clear'"));
        assert!(INDEX_HTML.contains("'登録がありません': 'No snippets'"));
        assert!(INDEX_HTML.contains("'スニペット削除': 'Delete snippet'"));
        assert!(INDEX_HTML.contains("'テンプレート検索': 'Search templates'"));
        assert!(INDEX_HTML.contains("'現在のSQLをスニペット保存': 'Save current SQL as snippet'"));
        assert!(INDEX_HTML.contains("'Migration Apply': 'マイグレーション適用'"));
        assert!(INDEX_HTML.contains("'SQL Console': 'SQLコンソール'"));
        assert!(INDEX_HTML.contains("'Create new': '新規作成'"));
        assert!(INDEX_HTML.contains("'New migration': '新規マイグレーション'"));
        assert!(INDEX_HTML.contains("'Version': 'バージョン'"));
        assert!(INDEX_HTML.contains("'File name': 'ファイル名'"));
        assert!(INDEX_HTML.contains("'Status': '状態'"));
        assert!(INDEX_HTML.contains("'Applied': '適用済み'"));
        assert!(INDEX_HTML.contains("'Pending': '未適用'"));
        assert!(INDEX_HTML.contains("'Path': 'パス'"));
        assert!(INDEX_HTML.contains("'Actions': '操作'"));
        assert!(!INDEX_HTML.contains("'バージョン': 'Version'"));
        assert!(INDEX_HTML.contains("'Template': 'テンプレート'"));
        assert!(INDEX_HTML.contains("'Format': 'フォーマット'"));
        assert!(INDEX_HTML.contains("'Clear': 'クリア'"));
        assert!(INDEX_HTML.contains("'Search templates': 'テンプレート検索'"));
        assert!(INDEX_HTML.contains("'Save current SQL as snippet': '現在のSQLをスニペット保存'"));
        assert!(INDEX_HTML.contains("'ヘルプ': 'Help'"));
        assert!(INDEX_HTML.contains("'基本': 'Basic'"));
        assert!(INDEX_HTML.contains("'DB探索': 'DB Discovery'"));
        assert!(INDEX_HTML.contains("'出力/保全': 'Output / Safeguards'"));
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
        assert!(INDEX_HTML.contains("escapeHtml(t('allowed'))"));
        assert!(INDEX_HTML.contains("escapeHtml(t('disabled'))"));
        assert!(INDEX_HTML.contains("['lock_timeout_ms', `${settings.lock_timeout_ms} ms`]"));
        assert!(INDEX_HTML.contains("['path_glob', settings.databases_path_glob || t('unset')]"));
        assert!(INDEX_HTML.contains("['format', settings.report_format]"));
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
        assert!(INDEX_HTML.contains("function uniqueMigrationsByFilename(migrations)"));
        assert!(INDEX_HTML.contains("function migrationFileName(migration)"));
        assert!(INDEX_HTML.contains("const byFilename = new Map()"));
        assert!(INDEX_HTML.contains("if (!byFilename.has(filename)) byFilename.set(filename, migration)"));
        assert!(INDEX_HTML.contains("escapeHtml(migrationFileName(migration))"));
        assert!(INDEX_HTML.contains(r#"<div id="migrationGroupChecklist" class="migration-checklist"></div>"#));
        assert!(INDEX_HTML.contains(r#"<button id="saveMigrationGroupMembership" class="primary">保存</button>"#));
        assert!(INDEX_HTML.contains(r#"<button id="openMigrationGroupModalInline" class="primary" hidden>グループ作成</button>"#));
        assert!(INDEX_HTML.contains(r#"<button id="openMigrationGroupModalFromMigrations" hidden>グループ作成</button>"#));
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
        assert!(INDEX_HTML.contains(r#"<div class="panel-actions">"#));
        assert!(!INDEX_HTML.contains("manageMigrationGroupVersions"));
    }

    #[test]
    fn html_uses_modals_for_database_and_db_group_creation() {
        assert!(INDEX_HTML.contains(r#"id="dbGroupModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<h2 id="dbGroupModalTitle">新規DBグループ</h2>"#));
        assert!(!INDEX_HTML.contains(r#"id="dbGroupCards""#));
        assert!(!INDEX_HTML.contains(".db-group-list-item"));
        assert!(INDEX_HTML.contains(".summary, .form-grid, .help, .migration-file-form { grid-template-columns:1fr; }"));
        assert!(INDEX_HTML.contains(r#"<tbody id="databases"></tbody>"#));
        assert!(INDEX_HTML.contains("function openDbGroupModal()"));
        assert!(INDEX_HTML.contains("function closeDbGroupModal()"));
        assert!(!INDEX_HTML.contains("id=\"openDbGroupModal\""));
        assert!(INDEX_HTML.contains(
            "$('openDbGroupModalFromDatabases').addEventListener('click', openDbGroupModal)"
        ));
        assert!(INDEX_HTML.contains("$('saveDbGroup').addEventListener('click', saveDbGroup)"));

        assert!(INDEX_HTML
            .contains(r#"<button id="openDatabaseFileModal" class="primary">新規作成</button>"#));
        assert!(INDEX_HTML.contains(
            r#"<button id="openDbGroupModalFromDatabases">グループ作成</button>"#
        ));
        assert!(INDEX_HTML.contains(r#"id="databaseFileModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<h2 id="databaseFileModalTitle">新規DB</h2>"#));
        assert!(INDEX_HTML.contains("function openDatabaseFileModal()"));
        assert!(INDEX_HTML.contains("function closeDatabaseFileModal()"));
        assert!(INDEX_HTML
            .contains("$('openDatabaseFileModal').addEventListener('click', openDatabaseFileModal)"));
        assert!(INDEX_HTML.contains("$('createDatabaseFile').addEventListener('click', createDatabaseFile)"));
        assert!(INDEX_HTML.contains("path: $('newDatabasePath').value"));
        assert!(!INDEX_HTML.contains("newDatabaseGroup"));
        assert!(!INDEX_HTML.contains("Add to DB group"));
        assert!(INDEX_HTML.contains("closeDatabaseFileModal();"));
        assert!(INDEX_HTML.contains("function renderDatabaseRowsByGroup(plans)"));
        assert!(INDEX_HTML.contains(r#"<tr class="group-divider"><th colspan="7">${escapeHtml(group.name)}</th></tr>"#));
        assert!(INDEX_HTML.contains(r#"<tr class="group-divider"><th colspan="7">${escapeHtml(t('ungroupedDb'))}</th></tr>"#));
        assert!(INDEX_HTML.contains("ungroupedDb: 'Ungrouped DBs'"));
    }

    #[test]
    fn html_uses_modal_for_migration_file_creation() {
        assert!(INDEX_HTML.contains(
            r#"<button id="openMigrationFileModal" class="primary">新規作成</button>"#
        ));
        assert!(INDEX_HTML.contains(r#"id="migrationFileModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<section class="modal wide" role="dialog" aria-modal="true" aria-labelledby="migrationFileModalTitle">"#));
        assert!(INDEX_HTML.contains(r#"<h2 id="migrationFileModalTitle">新規マイグレーション</h2>"#));
        assert!(!INDEX_HTML.contains(r#"id="migrationDetailModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<thead><tr><th>File name</th><th>DB</th><th>Checksum</th></tr></thead>"#));
        assert!(!INDEX_HTML.contains(r#"<thead><tr><th>Version</th><th>Name</th><th>Group</th><th>Checksum</th></tr></thead>"#));
        assert!(INDEX_HTML.contains(".link-button { min-height:0; padding:0; border:0; background:transparent; color:var(--accent-strong); font:inherit; text-align:left; text-decoration:none;"));
        assert!(INDEX_HTML.contains(".chip-link { border:0; background:transparent; color:var(--accent-strong); padding:0; min-height:0; font:inherit; text-decoration:none;"));
        assert!(INDEX_HTML.contains(".checksum-short { display:inline-flex; align-items:center; max-width:120px; padding:0; border:0; background:transparent; color:var(--accent-strong); font:inherit; text-decoration:none;"));
        assert!(INDEX_HTML.contains(".group-divider th { padding:9px 12px;"));
        assert!(INDEX_HTML.contains(r#"id="checksumModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<code id="checksumModalValue"></code>"#));
        assert!(INDEX_HTML.contains(".migration-file-form .file-name-field { grid-column:1 / -1; max-width:640px; }"));
        assert!(INDEX_HTML.contains(".migration-file-form #newMigrationGroupField { grid-column:1 / -1; max-width:240px; }"));
        assert!(INDEX_HTML.contains(".migration-file-form #newMigrationGroupField .field-label { white-space:nowrap; }"));
        assert!(INDEX_HTML.contains(".modal.wide { width:min(1120px, calc(100vw - 36px)); }"));
        assert!(INDEX_HTML.contains(r#"<p id="migrationFileStatus" class="migration-file-status" hidden></p>"#));
        assert!(INDEX_HTML.contains(r#"<label class="field file-name-field"><span class="field-label">File name <span id="newMigrationLatestFilename" class="field-hint"></span></span><input id="newMigrationFilename" placeholder="005_add_feature_flag.sql"></label>"#));
        assert!(INDEX_HTML.contains(r#"<label id="newMigrationGroupField" class="field" hidden><span class="field-label">Add to migration group</span><select id="newMigrationGroup"></select></label>"#));
        assert!(INDEX_HTML.contains(r#"<div id="migrationTemplateRow" class="sql-control-row">"#));
        assert!(INDEX_HTML.contains(r#"<span class="field-label">テンプレート</span>"#));
        assert!(INDEX_HTML.contains(r#"<input id="newMigrationTemplateSearch" list="newMigrationTemplateOptions" placeholder="テンプレート検索" aria-label="SQL template">"#));
        assert!(INDEX_HTML.contains(r#"<datalist id="newMigrationTemplateOptions"></datalist>"#));
        assert!(INDEX_HTML.contains(r#"<input id="newMigrationSqlTemplate" type="hidden">"#));
        assert!(INDEX_HTML.contains(r#"<button id="insertNewMigrationTemplate" type="button">テンプレート挿入</button>"#));
        assert!(INDEX_HTML.contains(r#"<div id="migrationSnippetRow" class="sql-control-row sql-snippet-toolbar">"#));
        assert!(INDEX_HTML.contains(r#"<span class="field-label">スニペット</span>"#));
        assert!(INDEX_HTML.contains(r#"<button id="formatNewMigrationSql" type="button">フォーマット</button>"#));
        assert!(INDEX_HTML.contains(r#"<button id="clearNewMigrationSql" type="button">クリア</button>"#));
        assert!(INDEX_HTML.contains(r#"<input id="newMigrationSnippetName" placeholder="スニペット名">"#));
        assert!(INDEX_HTML.contains(r#"<select id="newMigrationSnippetSelect" aria-label="Snippet"></select>"#));
        assert!(INDEX_HTML.contains(r#"<button id="saveNewMigrationSnippet" type="button">現在のSQLをスニペット保存</button>"#));
        assert!(INDEX_HTML.contains(r#"id="snippetDeleteModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<h2 id="snippetDeleteModalTitle">スニペット削除</h2>"#));
        assert!(INDEX_HTML.contains(r#"<button id="confirmSnippetDelete" type="button" class="danger">スニペットを削除</button>"#));
        assert!(INDEX_HTML.contains(r#"<pre id="newMigrationSqlLines" class="sql-line-numbers" aria-hidden="true">1</pre>"#));
        assert!(INDEX_HTML.contains(".sql-line-numbers { margin:0; padding:10px 8px;"));
        assert!(INDEX_HTML.contains(r#"<pre id="newMigrationSqlHighlight" class="sql-highlight" aria-hidden="true"></pre>"#));
        assert!(INDEX_HTML.contains(r#"<textarea id="newMigrationSql" spellcheck="false" autocomplete="off" autocapitalize="off" placeholder="ALTER TABLE ...;"></textarea>"#));
        assert!(INDEX_HTML.contains(r#"<div id="newMigrationSqlCompletions" class="sql-completion-list" hidden></div>"#));
        assert!(INDEX_HTML.contains(r#"<span id="newMigrationSqlCursor">Ln 1, Col 1</span>"#));
        assert!(INDEX_HTML.contains(r#"<span id="newMigrationSqlStats">0 bytes / 0 lines</span>"#));
        assert!(INDEX_HTML.contains("function openMigrationFileModal()"));
        assert!(INDEX_HTML.contains("function closeMigrationFileModal(force = false)"));
        assert!(INDEX_HTML.contains("function isMigrationSqlDirty()"));
        assert!(INDEX_HTML.contains("const forceClose = force === true"));
        assert!(INDEX_HTML.contains(r#"id="migrationSqlDiscardModal" class="modal-backdrop" hidden"#));
        assert!(INDEX_HTML.contains(r#"<h2 id="migrationSqlDiscardModalTitle">SQL編集内容の破棄</h2>"#));
        assert!(INDEX_HTML.contains("function closeMigrationSqlDiscardModal()"));
        assert!(INDEX_HTML.contains("function confirmMigrationSqlDiscard()"));
        assert!(INDEX_HTML.contains("$('migrationSqlDiscardModal').hidden = false"));
        assert!(INDEX_HTML.contains("let migrationFileInitialSql = ''"));
        assert!(INDEX_HTML.contains("let pendingMigrationFileClose = false"));
        assert!(INDEX_HTML.contains("function renderMigrationFileGroupOptions()"));
        assert!(INDEX_HTML.contains("function nextMigrationFileName()"));
        assert!(INDEX_HTML.contains("function latestMigrationFileName()"));
        assert!(INDEX_HTML.contains("function renderLatestMigrationFilename()"));
        assert!(INDEX_HTML.contains("function openMigrationDetailModal(index)"));
        assert!(INDEX_HTML.contains("function closeMigrationDetailModal()"));
        assert!(INDEX_HTML.contains("function renderMigrationRowsByGroup()"));
        assert!(INDEX_HTML.contains(
            "knownGroups.concat(migrationGroups.filter((group) => !knownGroups.includes(group)))"
        ));
        assert!(INDEX_HTML.contains(r#"<tr class="group-divider"><th colspan="3">${escapeHtml(group)}</th></tr>"#));
        assert!(INDEX_HTML.contains("function shortChecksum(value)"));
        assert!(INDEX_HTML.contains("function migrationDatabaseChips(migration)"));
        assert!(INDEX_HTML.contains("function openDatabaseFromMigration(databaseId)"));
        assert!(INDEX_HTML.contains("function unassignMigrationDatabase(databaseId, migrationGroup)"));
        assert!(INDEX_HTML.contains("function openChecksumModal(checksum)"));
        assert!(INDEX_HTML.contains("migrationFileMode = 'edit'"));
        assert!(INDEX_HTML.contains("const readOnly = appliedDatabases.length > 0 || !(state.gui_permissions && state.gui_permissions.allow_migration_edit)"));
        assert!(INDEX_HTML.contains("$('newMigrationSql').value = migration.sql || ''"));
        assert!(INDEX_HTML.contains("$('newMigrationSql').readOnly = readOnly"));
        assert!(INDEX_HTML.contains("function parseMigrationFilename(value)"));
        assert!(INDEX_HTML.contains("function insertNewMigrationTemplate()"));
        assert!(INDEX_HTML.contains("function formatNewMigrationSql()"));
        assert!(INDEX_HTML.contains("function updateNewMigrationSqlMeta()"));
        assert!(INDEX_HTML.contains("function handleSqlEditorKeydown(event)"));
        assert!(INDEX_HTML.contains("function highlightSql(sql)"));
        assert!(INDEX_HTML.contains("function renderSqlCompletions()"));
        assert!(INDEX_HTML.contains("function saveNewMigrationSnippet()"));
        assert!(INDEX_HTML.contains("function closeSnippetDeleteModal()"));
        assert!(INDEX_HTML.contains("function confirmDeleteMigrationSnippet()"));
        assert!(INDEX_HTML.contains("const migrationSnippetStorageKey = 'sqlite-fleet-migration-sql-snippets'"));
        assert!(INDEX_HTML.contains(": `<option value=\"\">${escapeHtml(t('noSnippets'))}</option>`"));
        assert!(INDEX_HTML.contains("latestMigrationFilename: (filename) => `Latest file name is ${filename}`"));
        assert!(INDEX_HTML.contains("latestMigrationFilename: (filename) => `現在の最新ファイル名は ${filename} です`"));
        assert!(INDEX_HTML.contains("el.textContent = latest ? t('latestMigrationFilename', latest) : t('noLatestMigrationFilename')"));
        assert!(INDEX_HTML.contains("renderLatestMigrationFilename();"));
        assert!(INDEX_HTML.contains("$('newMigrationFilename').value = nextMigrationFileName()"));
        assert!(INDEX_HTML.contains("const filenameParts = parseMigrationFilename($('newMigrationFilename').value)"));
        assert!(INDEX_HTML.contains("filename: filenameParts.filename"));
        assert!(INDEX_HTML.contains("await postAdmin('/api/admin/migration-file/update'"));
        assert!(INDEX_HTML.contains("path: editingMigration.path"));
        assert!(INDEX_HTML.contains(r#"data-migration-detail-index="${escapeHtml(index)}""#));
        assert!(INDEX_HTML.contains(r#"data-open-database="${escapeHtml(databaseId)}""#));
        assert!(INDEX_HTML.contains(r#"data-unassign-migration-db="${escapeHtml(databaseId)}""#));
        assert!(INDEX_HTML.contains(r#"class="disabled-tooltip" data-native-title="${escapeHtml(disabledTitle)}""#));
        assert!(INDEX_HTML.contains(r#"disabled data-always-disabled="true" aria-label="${escapeHtml(disabledTitle)}""#));
        assert!(INDEX_HTML.contains("migrationDbLinkAppliedDisabled: 'すでにマイグレーション済のため削除できない'"));
        assert!(INDEX_HTML.contains("migrationDbLinkAppliedDisabled: 'Already migrated, so it cannot be removed'"));
        assert!(INDEX_HTML.contains(".chip-remove:disabled"));
        assert!(INDEX_HTML.contains(r#"data-checksum="${escapeHtml(migration.checksum)}""#));
        assert!(INDEX_HTML.contains("$('migrations').addEventListener('click', (event) =>"));
        assert!(INDEX_HTML.contains("openDatabaseFromMigration(databaseButton.dataset.openDatabase)"));
        assert!(INDEX_HTML.contains("state.database_migration_assignments || []"));
        assert!(INDEX_HTML.contains("selector: assignment.selector"));
        assert!(INDEX_HTML.contains("unassignMigrationDatabase(unassignButton.dataset.unassignMigrationDb, unassignButton.dataset.unassignMigrationGroup)"));
        assert!(INDEX_HTML.contains("openChecksumModal(checksumButton.dataset.checksum)"));
        assert!(!INDEX_HTML.contains("$('closeMigrationDetailModal').addEventListener('click', closeMigrationDetailModal)"));
        assert!(INDEX_HTML.contains("return `${String(maxVersion + 1).padStart(width, '0')}_new_migration.sql`"));
        assert!(INDEX_HTML.contains("migration file name は <version>_<name>.sql または <name>_<version>.sql 形式で入力してください"));
        assert!(INDEX_HTML.contains("$('newMigrationGroup').value = names.includes('main') ? 'main' : names[0]"));
        assert!(!INDEX_HTML.contains("newMigrationVersion"));
        assert!(!INDEX_HTML.contains("newMigrationName"));
        assert!(INDEX_HTML.contains("$('newMigrationGroupField').hidden = names.length <= 1 && names[0] === 'main'"));
        assert!(INDEX_HTML.contains("$('openMigrationFileModal').addEventListener('click', openMigrationFileModal)"));
        assert!(INDEX_HTML.contains("$('closeMigrationFileModal').addEventListener('click', closeMigrationFileModal)"));
        assert!(INDEX_HTML.contains("$('cancelMigrationFile').addEventListener('click', closeMigrationFileModal)"));
        assert!(INDEX_HTML.contains("$('confirmMigrationSqlDiscard').addEventListener('click', confirmMigrationSqlDiscard)"));
        assert!(INDEX_HTML.contains("$('insertNewMigrationTemplate').addEventListener('click', insertNewMigrationTemplate)"));
        assert!(INDEX_HTML.contains("$('formatNewMigrationSql').addEventListener('click', formatNewMigrationSql)"));
        assert!(INDEX_HTML.contains("$('saveNewMigrationSnippet').addEventListener('click', saveNewMigrationSnippet)"));
        assert!(INDEX_HTML.contains("$('confirmSnippetDelete').addEventListener('click', confirmDeleteMigrationSnippet)"));
        assert!(INDEX_HTML.contains("$('newMigrationSql').addEventListener('scroll', syncSqlEditorScroll)"));
        assert!(INDEX_HTML.contains("$('newMigrationSql').addEventListener('keydown', handleSqlEditorKeydown)"));
        assert!(INDEX_HTML.contains("if (event.target === $('migrationFileModal')) closeMigrationFileModal()"));
        assert!(INDEX_HTML.contains("closeMigrationFileModal(true);"));
        assert!(!INDEX_HTML.contains(r#"<button id="createMigrationFile" class="primary">マイグレーションファイル追加</button>"#));
    }

    #[test]
    fn html_sql_format_does_not_split_semicolons() {
        assert!(INDEX_HTML.contains("function formatSqlText(sql)"));
        assert!(INDEX_HTML.contains(".map((line) => line.trimEnd())"));
        assert!(INDEX_HTML.contains(".replace(/\\n{3,}/g, '\\n\\n')"));
        assert!(!INDEX_HTML.contains(".replace(/;(?!\\s*$)\\s*/g, ';\\n')"));
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
        assert!(INDEX_HTML.contains(r#"<h2 class="panel-title">DB <span class="tool-tip""#));
        assert!(INDEX_HTML.contains(r#"data-tip="DBごとの状態、対象マイグレーショングループ、未適用のマイグレーションを確認します"#));
        assert!(INDEX_HTML.contains(r#"<h2 class="panel-title">マイグレーション一覧 <span class="tool-tip""#));
        assert!(INDEX_HTML.contains(r#"data-tip="読み込まれているマイグレーションファイル、対象DB、checksumを確認できます"#));
        assert!(INDEX_HTML.contains("'読み込まれているマイグレーションファイル、対象DB、checksumを確認できます。ファイル名をクリックすると詳細を表示します。': 'Review loaded migration files, target DBs, and checksums. Click a file name to open details.'"));
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
        assert!(INDEX_HTML.contains(r#"<input id="sqlTemplateSearch" list="sqlTemplateOptions""#));
        assert!(INDEX_HTML.contains(r#"<input id="sqlTemplate" type="hidden">"#));
        assert!(INDEX_HTML.contains(r#"<select id="sqlSnippetSelect" aria-label="Snippet">"#));
        assert!(INDEX_HTML.contains(r#"<pre id="sqlInputLines" class="sql-line-numbers""#));
        assert!(INDEX_HTML.contains(r#"<pre id="sqlInputHighlight" class="sql-highlight""#));
        assert!(INDEX_HTML.contains(r#"id="sqlInputCompletions" class="sql-completion-list""#));
        assert!(INDEX_HTML.contains(r#"<input id="sqlFile" type="file""#));
        assert!(INDEX_HTML.contains(r#"<textarea id="sqlInput""#));
        assert!(INDEX_HTML.contains(r#"<button id="downloadSql">SQLファイル保存</button>"#));
        assert!(INDEX_HTML.contains("$('sqlInput').addEventListener('keydown', handleSqlEditorKeydown)"));
        assert!(INDEX_HTML.contains("function updateSqlInputMeta()"));
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
