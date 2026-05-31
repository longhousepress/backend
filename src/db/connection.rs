use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

pub async fn load_db(db_path: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(3));

    let db = SqlitePool::connect_with(opts).await?;

    sqlx::migrate!().run(&db).await?;

    Ok(db)
}
