use anyhow::Context;
use rusqlite::Connection;

pub struct Pot {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub goal_cents: i64,
    pub currency: String,
    pub payment_info: String,
    pub organizer: String,
    pub deadline: Option<String>,
    pub allow_anonymous: bool,
    pub show_amounts: bool,
    pub show_names: bool,
    pub closed: bool,
    pub created_at: String,
    pub owner_id: Option<String>,
}

pub struct Contribution {
    pub id: String,
    pub pot_id: String,
    pub name: Option<String>,
    pub amount_cents: i64,
    pub message: Option<String>,
    pub confirmed: bool,
    pub created_at: String,
    pub user_id: Option<String>,
}

pub struct PotSummary {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub goal_cents: i64,
    pub total_cents: i64,
    pub contributor_count: i64,
    pub closed: bool,
    pub deadline: Option<String>,
    pub owner_id: Option<String>,
}

pub struct ContributionWithPot {
    pub id: String,
    pub pot_id: String,
    pub pot_slug: String,
    pub pot_title: String,
    pub amount_cents: i64,
    pub message: Option<String>,
    pub confirmed: bool,
    pub created_at: String,
}

pub fn init(path: &str) -> anyhow::Result<Connection> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).context("creating db directory")?;
    }
    let conn = Connection::open(path).context("opening database")?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=5000;
         PRAGMA foreign_keys=ON;",
    )
    .context("setting pragmas")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS users (
            id            TEXT PRIMARY KEY,
            email         TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            name          TEXT NOT NULL DEFAULT '',
            is_admin      INTEGER NOT NULL DEFAULT 0,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS pots (
            id              TEXT PRIMARY KEY,
            slug            TEXT NOT NULL UNIQUE,
            title           TEXT NOT NULL,
            description     TEXT NOT NULL DEFAULT '',
            goal_cents      INTEGER NOT NULL,
            currency        TEXT NOT NULL DEFAULT 'EUR',
            payment_info    TEXT NOT NULL DEFAULT '',
            organizer       TEXT NOT NULL DEFAULT '',
            deadline        TEXT,
            allow_anonymous INTEGER NOT NULL DEFAULT 1,
            show_amounts    INTEGER NOT NULL DEFAULT 1,
            show_names      INTEGER NOT NULL DEFAULT 1,
            closed          INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            owner_id        TEXT REFERENCES users(id) ON DELETE SET NULL
        );
        CREATE TABLE IF NOT EXISTS contributions (
            id           TEXT PRIMARY KEY,
            pot_id       TEXT NOT NULL REFERENCES pots(id) ON DELETE CASCADE,
            name         TEXT,
            amount_cents INTEGER NOT NULL,
            message      TEXT,
            confirmed    INTEGER NOT NULL DEFAULT 0,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            user_id      TEXT REFERENCES users(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_contributions_pot ON contributions(pot_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_pots_owner ON pots(owner_id);
        CREATE INDEX IF NOT EXISTS idx_contributions_user ON contributions(user_id);",
    )
    .context("creating tables")?;
    let _ = conn.execute(
        "ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(conn)
}

pub fn create_pot(
    conn: &Connection,
    id: &str,
    slug: &str,
    title: &str,
    description: &str,
    goal_cents: i64,
    currency: &str,
    payment_info: &str,
    organizer: &str,
    deadline: Option<&str>,
    allow_anonymous: bool,
    show_amounts: bool,
    show_names: bool,
    owner_id: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pots (id, slug, title, description, goal_cents, currency, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, owner_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![id, slug, title, description, goal_cents, currency, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, owner_id],
    ).context("inserting pot")?;
    Ok(())
}

const POT_COLS: &str = "id, slug, title, description, goal_cents, currency, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, closed, created_at, owner_id";

pub fn get_pot_by_slug(conn: &Connection, slug: &str) -> anyhow::Result<Option<Pot>> {
    let sql = format!("SELECT {POT_COLS} FROM pots WHERE slug = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map([slug], row_to_pot)?;
    Ok(rows.next().transpose()?)
}

pub fn get_pot_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<Pot>> {
    let sql = format!("SELECT {POT_COLS} FROM pots WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map([id], row_to_pot)?;
    Ok(rows.next().transpose()?)
}

pub fn list_pots(conn: &Connection) -> anyhow::Result<Vec<PotSummary>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.slug, p.title, p.goal_cents, p.closed, p.deadline, p.owner_id,
                COALESCE(SUM(c.amount_cents), 0) AS total_cents,
                COUNT(c.id) AS contributor_count
         FROM pots p LEFT JOIN contributions c ON c.pot_id = p.id
         GROUP BY p.id
         ORDER BY p.closed ASC, p.created_at DESC",
    )?;
    let rows = stmt.query_map([], pot_summary_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn list_pots_by_owner(conn: &Connection, owner_id: &str) -> anyhow::Result<Vec<PotSummary>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.slug, p.title, p.goal_cents, p.closed, p.deadline, p.owner_id,
                COALESCE(SUM(c.amount_cents), 0) AS total_cents,
                COUNT(c.id) AS contributor_count
         FROM pots p LEFT JOIN contributions c ON c.pot_id = p.id
         WHERE p.owner_id = ?1
         GROUP BY p.id
         ORDER BY p.closed ASC, p.created_at DESC",
    )?;
    let rows = stmt.query_map([owner_id], pot_summary_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn pot_summary_row(row: &rusqlite::Row) -> rusqlite::Result<PotSummary> {
    Ok(PotSummary {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        goal_cents: row.get(3)?,
        closed: row.get::<_, i64>(4)? != 0,
        deadline: row.get(5)?,
        owner_id: row.get(6)?,
        total_cents: row.get(7)?,
        contributor_count: row.get(8)?,
    })
}

pub fn list_contributions_by_user(
    conn: &Connection,
    user_id: &str,
) -> anyhow::Result<Vec<ContributionWithPot>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.pot_id, p.slug, p.title, c.amount_cents, c.message, c.confirmed, c.created_at
         FROM contributions c JOIN pots p ON p.id = c.pot_id
         WHERE c.user_id = ?1
         ORDER BY c.created_at DESC",
    )?;
    let rows = stmt.query_map([user_id], |row| {
        Ok(ContributionWithPot {
            id: row.get(0)?,
            pot_id: row.get(1)?,
            pot_slug: row.get(2)?,
            pot_title: row.get(3)?,
            amount_cents: row.get(4)?,
            message: row.get(5)?,
            confirmed: row.get::<_, i64>(6)? != 0,
            created_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn update_pot(
    conn: &Connection,
    id: &str,
    title: &str,
    description: &str,
    goal_cents: i64,
    payment_info: &str,
    organizer: &str,
    deadline: Option<&str>,
    allow_anonymous: bool,
    show_amounts: bool,
    show_names: bool,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE pots SET title=?1, description=?2, goal_cents=?3, payment_info=?4, organizer=?5, deadline=?6, allow_anonymous=?7, show_amounts=?8, show_names=?9 WHERE id=?10",
        rusqlite::params![title, description, goal_cents, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, id],
    ).context("updating pot")?;
    Ok(())
}

pub fn delete_pot(conn: &Connection, id: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM pots WHERE id = ?1", [id])
        .context("deleting pot")?;
    Ok(())
}

pub fn toggle_close(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    conn.execute(
        "UPDATE pots SET closed = CASE WHEN closed = 0 THEN 1 ELSE 0 END WHERE id = ?1",
        [id],
    )
    .context("toggling close")?;
    let closed: bool = conn.query_row("SELECT closed FROM pots WHERE id = ?1", [id], |r| {
        Ok(r.get::<_, i64>(0)? != 0)
    })?;
    Ok(closed)
}

pub fn slug_exists(conn: &Connection, slug: &str) -> anyhow::Result<bool> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM pots WHERE slug = ?1", [slug], |r| r.get(0))?;
    Ok(count > 0)
}

pub fn get_contribution_pot_id(conn: &Connection, id: &str) -> anyhow::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT pot_id FROM contributions WHERE id = ?1")?;
    let mut rows = stmt.query_map([id], |row| row.get::<_, String>(0))?;
    Ok(rows.next().transpose()?)
}

pub fn create_contribution(
    conn: &Connection,
    id: &str,
    pot_id: &str,
    name: Option<&str>,
    amount_cents: i64,
    message: Option<&str>,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO contributions (id, pot_id, name, amount_cents, message, user_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, pot_id, name, amount_cents, message, user_id],
    ).context("inserting contribution")?;
    Ok(())
}

pub fn get_contributions(conn: &Connection, pot_id: &str) -> anyhow::Result<Vec<Contribution>> {
    let mut stmt = conn.prepare(
        "SELECT id, pot_id, name, amount_cents, message, confirmed, created_at, user_id
         FROM contributions WHERE pot_id = ?1 ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([pot_id], |row| {
        Ok(Contribution {
            id: row.get(0)?,
            pot_id: row.get(1)?,
            name: row.get(2)?,
            amount_cents: row.get(3)?,
            message: row.get(4)?,
            confirmed: row.get::<_, i64>(5)? != 0,
            created_at: row.get(6)?,
            user_id: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn delete_contribution(conn: &Connection, id: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM contributions WHERE id = ?1", [id])
        .context("deleting contribution")?;
    Ok(())
}

pub fn confirm_contribution(conn: &Connection, id: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE contributions SET confirmed = 1 WHERE id = ?1",
        [id],
    )
    .context("confirming contribution")?;
    Ok(())
}

fn row_to_pot(row: &rusqlite::Row) -> rusqlite::Result<Pot> {
    Ok(Pot {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        goal_cents: row.get(4)?,
        currency: row.get(5)?,
        payment_info: row.get(6)?,
        organizer: row.get(7)?,
        deadline: row.get(8)?,
        allow_anonymous: row.get::<_, i64>(9)? != 0,
        show_amounts: row.get::<_, i64>(10)? != 0,
        show_names: row.get::<_, i64>(11)? != 0,
        closed: row.get::<_, i64>(12)? != 0,
        created_at: row.get(13)?,
        owner_id: row.get(14)?,
    })
}
