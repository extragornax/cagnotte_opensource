use anyhow::Context;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand_core::OsRng;
use rusqlite::Connection;

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

pub fn create_user(
    conn: &Connection,
    id: &str,
    email: &str,
    password_hash: &str,
    name: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO users (id, email, password_hash, name) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, email, password_hash, name],
    )
    .context("inserting user")?;
    Ok(())
}

pub fn get_user_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, name, is_admin, created_at FROM users WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map([id], row_to_user)?;
    Ok(rows.next().transpose()?)
}

pub fn get_user_with_hash_by_email(
    conn: &Connection,
    email: &str,
) -> anyhow::Result<Option<(User, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, name, is_admin, created_at, password_hash FROM users WHERE email = ?1",
    )?;
    let mut rows = stmt.query_map([email], |row| {
        Ok((
            User {
                id: row.get(0)?,
                email: row.get(1)?,
                name: row.get(2)?,
                is_admin: row.get::<_, i64>(3)? != 0,
                created_at: row.get(4)?,
            },
            row.get::<_, String>(5)?,
        ))
    })?;
    Ok(rows.next().transpose()?)
}

pub fn list_users(conn: &Connection) -> anyhow::Result<Vec<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, name, is_admin, created_at FROM users ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], row_to_user)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn is_admin(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT is_admin FROM users WHERE id = ?1",
        [id],
        |r| r.get(0),
    ).unwrap_or(0);
    Ok(count != 0)
}

pub fn set_admin(conn: &Connection, id: &str, is_admin: bool) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE users SET is_admin = ?1 WHERE id = ?2",
        rusqlite::params![is_admin as i64, id],
    )
    .context("updating admin flag")?;
    Ok(())
}

fn row_to_user(row: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        email: row.get(1)?,
        name: row.get(2)?,
        is_admin: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
    })
}

pub fn count_users(conn: &Connection) -> anyhow::Result<i64> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    Ok(n)
}

pub fn email_exists(conn: &Connection, email: &str) -> anyhow::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM users WHERE email = ?1",
        [email],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

pub fn update_user(conn: &Connection, id: &str, email: &str, name: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE users SET email = ?1, name = ?2 WHERE id = ?3",
        rusqlite::params![email, name, id],
    )
    .context("updating user")?;
    Ok(())
}

pub fn update_password(conn: &Connection, id: &str, password_hash: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE users SET password_hash = ?1 WHERE id = ?2",
        rusqlite::params![password_hash, id],
    )
    .context("updating password")?;
    Ok(())
}
