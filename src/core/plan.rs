pub fn build_plan(
    config: &Config,
    databases: &[Database],
    migrations: &[Migration],
) -> Vec<DatabasePlan> {
    if let Err(error) = validate_database_set(databases) {
        let error = error.to_string();
        return databases
            .iter()
            .map(|database| DatabasePlan {
                database: database.clone(),
                migration_groups: config.migration_groups_for_database(database),
                applied_count: 0,
                pending: migrations.iter().map(MigrationSummary::from).collect(),
                checksum_errors: Vec::new(),
                unknown_applied: Vec::new(),
                error: Some(error.clone()),
            })
            .collect();
    }
    databases
        .iter()
        .map(|database| build_database_plan(config, database, migrations))
        .collect()
}

fn migrations_for_database(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> Vec<Migration> {
    let migration_groups = config.migration_groups_for_database(database);
    let groups = migration_groups
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if groups.is_empty() {
        return migrations.to_vec();
    }
    let mut selected = migrations
        .iter()
        .filter(|migration| groups.contains(migration.group.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|a, b| {
        a.version_number
            .cmp(&b.version_number)
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.group.cmp(&b.group))
    });
    selected.dedup_by(|left, right| left.version == right.version);
    selected
}

pub fn build_database_plan(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> DatabasePlan {
    let migrations = migrations_for_database(config, database, migrations);
    let known_versions = migrations
        .iter()
        .map(|migration| migration.version.as_str())
        .collect::<HashSet<_>>();
    let migration_groups = config.migration_groups_for_database(database);
    if let Err(error) = validate_runtime_config(config) {
        return DatabasePlan {
            database: database.clone(),
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = validate_database_id(&database.id) {
        return DatabasePlan {
            database: database.clone(),
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = validate_migrations(config, &migrations) {
        return DatabasePlan {
            database: database.clone(),
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = config.validate_resolved_path_within_base("DBパス", &database.path) {
        return DatabasePlan {
            database: database.clone(),
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    let database = refresh_database_state(database);
    if let Err(error) = ensure_existing_database_file(&database.path) {
        return DatabasePlan {
            database,
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }

    match open_existing_readonly(&database.path)
        .and_then(|conn| {
            conn.busy_timeout(std::time::Duration::from_millis(
                config.execution.lock_timeout_ms,
            ))?;
            Ok(conn)
        })
        .and_then(|conn| read_applied_migrations(&conn, config.migrations_table()))
    {
        Ok(applied) => {
            let applied_by_version: HashMap<&str, &AppliedMigration> =
                applied.iter().map(|m| (m.version.as_str(), m)).collect();
            let applied_versions: HashSet<&str> = applied_by_version.keys().copied().collect();
            let pending = migrations
                .iter()
                .filter(|migration| !applied_versions.contains(migration.version.as_str()))
                .map(MigrationSummary::from)
                .collect();
            let checksum_errors = migrations
                .iter()
                .filter_map(|migration| {
                    applied_by_version
                        .get(migration.version.as_str())
                        .and_then(|applied| {
                            (applied.checksum != migration.checksum).then(|| ChecksumError {
                                version: migration.version.clone(),
                                expected: migration.checksum.clone(),
                                actual: applied.checksum.clone(),
                            })
                        })
                })
                .collect();
            let unknown_applied = applied
                .iter()
                .filter(|migration| !known_versions.contains(migration.version.as_str()))
                .map(MigrationSummary::from)
                .collect();
            DatabasePlan {
                database,
                migration_groups,
                applied_count: applied.len(),
                pending,
                checksum_errors,
                unknown_applied,
                error: None,
            }
        }
        Err(error) => DatabasePlan {
            database,
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

