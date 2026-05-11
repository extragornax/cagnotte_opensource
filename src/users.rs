use anyhow::Context;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand_core::OsRng;
use sqlx::{PgPool, Row};

use crate::db::now_str;

#[derive(sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    pub is_admin: bool,
    pub created_at: String,
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash error: {e}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let parsed = match PasswordHash::new(stored_hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn create_user(
    pool: &PgPool,
    id: &str,
    email: &str,
    password_hash: &str,
    name: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, name, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(email)
    .bind(password_hash)
    .bind(name)
    .bind(now_str())
    .execute(pool)
    .await
    .context("inserting user")?;
    Ok(())
}

pub async fn get_user_by_id(pool: &PgPool, id: &str) -> anyhow::Result<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT id, email, name, is_admin, created_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

pub async fn get_user_with_hash_by_email(
    pool: &PgPool,
    email: &str,
) -> anyhow::Result<Option<(User, String)>> {
    let row = sqlx::query(
        "SELECT id, email, name, is_admin, created_at, password_hash FROM users WHERE email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            User {
                id: r.get("id"),
                email: r.get("email"),
                name: r.get("name"),
                is_admin: r.get("is_admin"),
                created_at: r.get("created_at"),
            },
            r.get::<String, _>("password_hash"),
        )
    }))
}

pub async fn list_users(pool: &PgPool) -> anyhow::Result<Vec<User>> {
    let users = sqlx::query_as::<_, User>(
        "SELECT id, email, name, is_admin, created_at FROM users ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(users)
}

pub async fn count_users(pool: &PgPool) -> anyhow::Result<i64> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS c FROM users")
        .fetch_one(pool)
        .await?;
    Ok(row.get("c"))
}

pub async fn is_admin(pool: &PgPool, id: &str) -> anyhow::Result<bool> {
    let row = sqlx::query("SELECT is_admin FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<bool, _>("is_admin")).unwrap_or(false))
}

pub async fn set_admin(pool: &PgPool, id: &str, flag: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET is_admin = $1 WHERE id = $2")
        .bind(flag)
        .bind(id)
        .execute(pool)
        .await
        .context("setting admin")?;
    Ok(())
}

pub async fn email_exists(pool: &PgPool, email: &str) -> anyhow::Result<bool> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS c FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await?;
    let n: i64 = row.get("c");
    Ok(n > 0)
}

pub async fn update_user(pool: &PgPool, id: &str, email: &str, name: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET email = $1, name = $2 WHERE id = $3")
        .bind(email)
        .bind(name)
        .bind(id)
        .execute(pool)
        .await
        .context("updating user")?;
    Ok(())
}

pub async fn get_password_hash(pool: &PgPool, id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query("SELECT password_hash FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("password_hash")))
}

pub async fn update_password(pool: &PgPool, id: &str, password_hash: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(password_hash)
        .bind(id)
        .execute(pool)
        .await
        .context("updating password")?;
    Ok(())
}
