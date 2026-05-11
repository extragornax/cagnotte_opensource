use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

#[derive(sqlx::FromRow)]
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
    pub is_public: bool,
}

#[derive(sqlx::FromRow)]
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
    pub is_public: bool,
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

pub fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub async fn connect(url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await
        .context("connecting to postgres")?;
    init(&pool).await?;
    Ok(pool)
}

pub async fn init(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id            TEXT PRIMARY KEY,
            email         TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            name          TEXT NOT NULL DEFAULT '',
            is_admin      BOOLEAN NOT NULL DEFAULT FALSE,
            created_at    TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .context("create users")?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pots (
            id              TEXT PRIMARY KEY,
            slug            TEXT NOT NULL UNIQUE,
            title           TEXT NOT NULL,
            description     TEXT NOT NULL DEFAULT '',
            goal_cents      BIGINT NOT NULL,
            currency        TEXT NOT NULL DEFAULT 'EUR',
            payment_info    TEXT NOT NULL DEFAULT '',
            organizer       TEXT NOT NULL DEFAULT '',
            deadline        TEXT,
            allow_anonymous BOOLEAN NOT NULL DEFAULT TRUE,
            show_amounts    BOOLEAN NOT NULL DEFAULT TRUE,
            show_names      BOOLEAN NOT NULL DEFAULT TRUE,
            closed          BOOLEAN NOT NULL DEFAULT FALSE,
            created_at      TEXT NOT NULL,
            owner_id        TEXT REFERENCES users(id) ON DELETE SET NULL,
            is_public       BOOLEAN NOT NULL DEFAULT FALSE
        )",
    )
    .execute(pool)
    .await
    .context("create pots")?;

    sqlx::query("ALTER TABLE pots ADD COLUMN IF NOT EXISTS is_public BOOLEAN NOT NULL DEFAULT FALSE")
        .execute(pool)
        .await
        .ok();

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS contributions (
            id           TEXT PRIMARY KEY,
            pot_id       TEXT NOT NULL REFERENCES pots(id) ON DELETE CASCADE,
            name         TEXT,
            amount_cents BIGINT NOT NULL,
            message      TEXT,
            confirmed    BOOLEAN NOT NULL DEFAULT FALSE,
            created_at   TEXT NOT NULL,
            user_id      TEXT REFERENCES users(id) ON DELETE SET NULL
        )",
    )
    .execute(pool)
    .await
    .context("create contributions")?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_contributions_pot ON contributions(pot_id, created_at)").execute(pool).await.ok();
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_pots_owner ON pots(owner_id)").execute(pool).await.ok();
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_contributions_user ON contributions(user_id)").execute(pool).await.ok();

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_pot(
    pool: &PgPool,
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
    sqlx::query(
        "INSERT INTO pots (id, slug, title, description, goal_cents, currency, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, created_at, owner_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(id)
    .bind(slug)
    .bind(title)
    .bind(description)
    .bind(goal_cents)
    .bind(currency)
    .bind(payment_info)
    .bind(organizer)
    .bind(deadline)
    .bind(allow_anonymous)
    .bind(show_amounts)
    .bind(show_names)
    .bind(now_str())
    .bind(owner_id)
    .execute(pool)
    .await
    .context("inserting pot")?;
    Ok(())
}

pub async fn get_pot_by_slug(pool: &PgPool, slug: &str) -> anyhow::Result<Option<Pot>> {
    let pot = sqlx::query_as::<_, Pot>(
        "SELECT id, slug, title, description, goal_cents, currency, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, closed, created_at, owner_id, is_public
         FROM pots WHERE slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;
    Ok(pot)
}

pub async fn get_pot_by_id(pool: &PgPool, id: &str) -> anyhow::Result<Option<Pot>> {
    let pot = sqlx::query_as::<_, Pot>(
        "SELECT id, slug, title, description, goal_cents, currency, payment_info, organizer, deadline, allow_anonymous, show_amounts, show_names, closed, created_at, owner_id, is_public
         FROM pots WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(pot)
}

pub enum PotFilter<'a> {
    All,
    PublicOnly,
    Owner(&'a str),
}

async fn pot_summaries(pool: &PgPool, filter: PotFilter<'_>) -> anyhow::Result<Vec<PotSummary>> {
    let (sql, owner_bind): (&str, Option<&str>) = match filter {
        PotFilter::Owner(o) => (
            "SELECT p.id, p.slug, p.title, p.goal_cents, p.closed, p.deadline, p.owner_id, p.is_public,
                    COALESCE(SUM(c.amount_cents)::BIGINT, 0) AS total_cents,
                    COUNT(c.id)::BIGINT AS contributor_count
             FROM pots p LEFT JOIN contributions c ON c.pot_id = p.id
             WHERE p.owner_id = $1
             GROUP BY p.id
             ORDER BY p.closed ASC, p.created_at DESC",
            Some(o),
        ),
        PotFilter::PublicOnly => (
            "SELECT p.id, p.slug, p.title, p.goal_cents, p.closed, p.deadline, p.owner_id, p.is_public,
                    COALESCE(SUM(c.amount_cents)::BIGINT, 0) AS total_cents,
                    COUNT(c.id)::BIGINT AS contributor_count
             FROM pots p LEFT JOIN contributions c ON c.pot_id = p.id
             WHERE p.is_public = TRUE
             GROUP BY p.id
             ORDER BY p.closed ASC, p.created_at DESC",
            None,
        ),
        PotFilter::All => (
            "SELECT p.id, p.slug, p.title, p.goal_cents, p.closed, p.deadline, p.owner_id, p.is_public,
                    COALESCE(SUM(c.amount_cents)::BIGINT, 0) AS total_cents,
                    COUNT(c.id)::BIGINT AS contributor_count
             FROM pots p LEFT JOIN contributions c ON c.pot_id = p.id
             GROUP BY p.id
             ORDER BY p.closed ASC, p.created_at DESC",
            None,
        ),
    };
    let mut q = sqlx::query(sql);
    if let Some(o) = owner_bind {
        q = q.bind(o);
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| PotSummary {
            id: r.get("id"),
            slug: r.get("slug"),
            title: r.get("title"),
            goal_cents: r.get("goal_cents"),
            closed: r.get("closed"),
            deadline: r.get("deadline"),
            owner_id: r.get("owner_id"),
            is_public: r.get("is_public"),
            total_cents: r.get("total_cents"),
            contributor_count: r.get("contributor_count"),
        })
        .collect())
}

pub async fn list_public_pots(pool: &PgPool) -> anyhow::Result<Vec<PotSummary>> {
    pot_summaries(pool, PotFilter::PublicOnly).await
}

pub async fn list_all_pots(pool: &PgPool) -> anyhow::Result<Vec<PotSummary>> {
    pot_summaries(pool, PotFilter::All).await
}

pub async fn list_pots_by_owner(pool: &PgPool, owner_id: &str) -> anyhow::Result<Vec<PotSummary>> {
    pot_summaries(pool, PotFilter::Owner(owner_id)).await
}

pub async fn list_contributions_by_user(
    pool: &PgPool,
    user_id: &str,
) -> anyhow::Result<Vec<ContributionWithPot>> {
    let rows = sqlx::query(
        "SELECT c.id, c.pot_id, p.slug AS pot_slug, p.title AS pot_title, c.amount_cents, c.message, c.confirmed, c.created_at
         FROM contributions c JOIN pots p ON p.id = c.pot_id
         WHERE c.user_id = $1
         ORDER BY c.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ContributionWithPot {
            id: r.get("id"),
            pot_id: r.get("pot_id"),
            pot_slug: r.get("pot_slug"),
            pot_title: r.get("pot_title"),
            amount_cents: r.get("amount_cents"),
            message: r.get("message"),
            confirmed: r.get("confirmed"),
            created_at: r.get("created_at"),
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_pot(
    pool: &PgPool,
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
    sqlx::query(
        "UPDATE pots SET title=$1, description=$2, goal_cents=$3, payment_info=$4, organizer=$5, deadline=$6, allow_anonymous=$7, show_amounts=$8, show_names=$9 WHERE id=$10",
    )
    .bind(title)
    .bind(description)
    .bind(goal_cents)
    .bind(payment_info)
    .bind(organizer)
    .bind(deadline)
    .bind(allow_anonymous)
    .bind(show_amounts)
    .bind(show_names)
    .bind(id)
    .execute(pool)
    .await
    .context("updating pot")?;
    Ok(())
}

pub async fn delete_pot(pool: &PgPool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM pots WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("deleting pot")?;
    Ok(())
}

pub async fn toggle_close(pool: &PgPool, id: &str) -> anyhow::Result<bool> {
    let row = sqlx::query("UPDATE pots SET closed = NOT closed WHERE id = $1 RETURNING closed")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("toggling close")?;
    Ok(row.get("closed"))
}

pub async fn toggle_public(pool: &PgPool, id: &str) -> anyhow::Result<bool> {
    let row = sqlx::query("UPDATE pots SET is_public = NOT is_public WHERE id = $1 RETURNING is_public")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("toggling public")?;
    Ok(row.get("is_public"))
}

pub async fn slug_exists(pool: &PgPool, slug: &str) -> anyhow::Result<bool> {
    let row = sqlx::query("SELECT COUNT(*)::BIGINT AS c FROM pots WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await?;
    let n: i64 = row.get("c");
    Ok(n > 0)
}

pub async fn get_contribution_pot_id(pool: &PgPool, id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query("SELECT pot_id FROM contributions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("pot_id")))
}

pub async fn create_contribution(
    pool: &PgPool,
    id: &str,
    pot_id: &str,
    name: Option<&str>,
    amount_cents: i64,
    message: Option<&str>,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO contributions (id, pot_id, name, amount_cents, message, created_at, user_id) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(pot_id)
    .bind(name)
    .bind(amount_cents)
    .bind(message)
    .bind(now_str())
    .bind(user_id)
    .execute(pool)
    .await
    .context("inserting contribution")?;
    Ok(())
}

pub async fn get_contributions(pool: &PgPool, pot_id: &str) -> anyhow::Result<Vec<Contribution>> {
    let rows = sqlx::query_as::<_, Contribution>(
        "SELECT id, pot_id, name, amount_cents, message, confirmed, created_at, user_id
         FROM contributions WHERE pot_id = $1 ORDER BY created_at DESC",
    )
    .bind(pot_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_contribution(pool: &PgPool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM contributions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("deleting contribution")?;
    Ok(())
}

pub async fn confirm_contribution(pool: &PgPool, id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE contributions SET confirmed = TRUE WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("confirming contribution")?;
    Ok(())
}
