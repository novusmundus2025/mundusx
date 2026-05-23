use native_tls::TlsConnector;
use postgres::{Client, Config};
use postgres_native_tls::MakeTlsConnector;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct MigrationFile {
    pub version: String,
    pub name: String,
    pub sql: String,
    pub path: PathBuf,
}

pub fn migration_files() -> Result<Vec<MigrationFile>, String> {
    let root = repo_root()?;
    let mut files = Vec::new();

    let schema_path = root.join("supabase/schema.sql");
    if schema_path.exists() {
        files.push(read_migration_file(&schema_path, "0000", "schema")?);
    }

    let migrations_dir = root.join("supabase/migrations");
    if migrations_dir.exists() {
        let mut entries = fs::read_dir(&migrations_dir)
            .map_err(|error| format!("failed to read migrations dir: {error}"))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sql"))
            .collect::<Vec<_>>();
        entries.sort();

        for path in entries {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("invalid migration filename: {}", path.display()))?;
            let version = file_name
                .split_once('_')
                .map(|(version, _)| version.to_string())
                .unwrap_or_else(|| file_name.trim_end_matches(".sql").to_string());
            let name = file_name
                .trim_end_matches(".sql")
                .split_once('_')
                .map(|(_, name)| name.to_string())
                .unwrap_or_else(|| "migration".to_string());
            files.push(read_migration_file(&path, &version, &name)?);
        }
    }

    files.sort_by(|a, b| a.version.cmp(&b.version).then_with(|| a.name.cmp(&b.name)));
    Ok(files)
}

pub fn apply_migrations(database_url: &str) -> Result<Vec<MigrationFile>, String> {
    let mut client = connect_client(database_url)?;

    client
        .batch_execute(
            r#"
            create table if not exists public.schema_migrations (
                version text primary key,
                name text not null,
                checksum text not null,
                applied_at timestamptz not null default now()
            );
        "#,
        )
        .map_err(|error| format!("failed to create schema_migrations: {error}"))?;

    let applied = applied_versions(&mut client)?;
    let migrations = migration_files()?;
    let mut applied_files = Vec::new();

    for migration in migrations {
        if applied.contains(&migration.version) {
            continue;
        }

        client.batch_execute(&migration.sql).map_err(|error| {
            format!(
                "failed to apply migration {}: {}",
                migration.version,
                db_error_message(&error)
            )
        })?;

        let checksum = checksum(&migration.sql);
        client
            .execute(
                "insert into public.schema_migrations (version, name, checksum) values ($1, $2, $3) on conflict (version) do update set name = excluded.name, checksum = excluded.checksum, applied_at = now()",
                &[&migration.version, &migration.name, &checksum],
            )
            .map_err(|error| {
                format!(
                    "failed to record migration {}: {}",
                    migration.version,
                    db_error_message(&error)
                )
            })?;

        applied_files.push(migration);
    }

    Ok(applied_files)
}

pub fn applied_migrations(database_url: &str) -> Result<Vec<String>, String> {
    let mut client = connect_client(database_url)?;
    applied_versions(&mut client)
}

fn connect_client(database_url: &str) -> Result<Client, String> {
    let mut config = Config::from_str(database_url)
        .map_err(|error| format!("failed to parse database url: {error}"))?;
    config.connect_timeout(Duration::from_secs(10));

    let strict_connector =
        postgres_connector(false).map_err(|error| format!("failed to build TLS: {error}"))?;
    match config.connect(strict_connector) {
        Ok(client) => Ok(client),
        Err(strict_error) => {
            eprintln!(
                "strict database TLS failed, retrying with relaxed certificate checks: {strict_error}"
            );
            let relaxed_connector = postgres_connector(true)
                .map_err(|error| format!("failed to build TLS: {error}"))?;
            config
                .connect(relaxed_connector)
                .map_err(|error| format!("failed to connect to database: {error}"))
        }
    }
}

fn applied_versions(client: &mut Client) -> Result<Vec<String>, String> {
    let rows = client
        .query("select version from public.schema_migrations order by version asc", &[])
        .map_err(|error| format!("failed to query schema_migrations: {error}"))?;
    Ok(rows.into_iter().map(|row| row.get::<_, String>(0)).collect())
}

fn read_migration_file(path: &Path, version: &str, name: &str) -> Result<MigrationFile, String> {
    let sql = fs::read_to_string(path)
        .map_err(|error| format!("failed to read migration {}: {error}", path.display()))?;
    Ok(MigrationFile {
        version: version.to_string(),
        name: name.to_string(),
        sql,
        path: path.to_path_buf(),
    })
}

fn repo_root() -> Result<PathBuf, String> {
    let mut current = env::current_dir().map_err(|error| format!("failed to read cwd: {error}"))?;
    loop {
        if current.join("Cargo.toml").exists() && current.join("apps").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err("failed to locate repository root".to_string());
        }
    }
}

fn checksum(input: &str) -> String {
    let mut state: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x100000001b3);
    }
    format!("{state:016x}")
}

fn postgres_connector(relaxed: bool) -> Result<MakeTlsConnector, native_tls::Error> {
    let mut builder = TlsConnector::builder();
    if relaxed {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    builder.build().map(MakeTlsConnector::new)
}

fn db_error_message(error: &postgres::Error) -> String {
    if let Some(db_error) = error.as_db_error() {
        let mut message = db_error.message().to_string();
        if let Some(detail) = db_error.detail() {
            message.push_str(&format!(" | detail: {detail}"));
        }
        if let Some(hint) = db_error.hint() {
            message.push_str(&format!(" | hint: {hint}"));
        }
        message.push_str(&format!(" | code: {}", db_error.code().code()));
        message
    } else {
        error.to_string()
    }
}
