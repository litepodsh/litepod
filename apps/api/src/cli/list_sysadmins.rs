use sqlx::{Row, SqlitePool};

use crate::cli::OpsError;

pub async fn run(pool: &SqlitePool) -> Result<(), OpsError> {
    let rows = sqlx::query(
        "SELECT u.id, u.email, u.name, u.totp_enabled, u.created_at \
         FROM users u \
         JOIN user_global_roles g ON g.user_id = u.id \
         WHERE g.role_id = 0 \
         ORDER BY u.created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        println!("no sysadmin accounts found");
        return Ok(());
    }

    println!(
        "{:<38}  {:<32}  {:<24}  {:<6}  {}",
        "ID", "EMAIL", "NAME", "2FA", "CREATED"
    );
    for row in &rows {
        let id: String = row.try_get(0)?;
        let email: String = row.try_get(1)?;
        let name: String = row.try_get(2)?;
        let totp: i64 = row.try_get(3)?;
        let created: String = row.try_get(4)?;
        println!(
            "{:<38}  {:<32}  {:<24}  {:<6}  {}",
            id,
            truncate(&email, 32),
            truncate(&name, 24),
            if totp == 1 { "yes" } else { "no" },
            created,
        );
    }
    Ok(())
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
