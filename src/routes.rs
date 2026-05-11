use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::auth;
use crate::captcha;
use crate::db;
use crate::session;
use crate::slug::slugify;
use crate::users;
use crate::AppState;

const INDEX_TEMPLATE: &str = include_str!("../templates/index.html");
const POT_TEMPLATE: &str = include_str!("../templates/pot.html");
const ADMIN_TEMPLATE: &str = include_str!("../templates/admin.html");
const DASHBOARD_TEMPLATE: &str = include_str!("../templates/dashboard.html");
const ADMIN_DASHBOARD_TEMPLATE: &str = include_str!("../templates/admin-dashboard.html");
const LOGIN_TEMPLATE: &str = include_str!("../templates/login.html");
const SIGNUP_TEMPLATE: &str = include_str!("../templates/signup.html");
const PROFILE_TEMPLATE: &str = include_str!("../templates/profile.html");

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/login", get(login_page))
        .route("/signup", get(signup_page))
        .route("/dashboard", get(dashboard_page))
        .route("/me", get(profile_page))
        .route("/p/{slug}", get(pot_page))
        .route("/admin/dashboard", get(admin_dashboard_page))
        .route("/admin/{id}", get(admin_page))
        .route("/health", get(health))
        .route("/api/captcha", get(api_captcha))
        .route("/api/signup", post(api_signup))
        .route("/api/login", post(api_login))
        .route("/api/logout", post(api_logout))
        .route("/api/me", put(api_update_me))
        .route("/api/me/password", put(api_update_password))
        .route("/api/users/{id}/promote", post(api_promote_user))
        .route("/api/users/{id}/demote", post(api_demote_user))
        .route("/api/pots", post(create_pot))
        .route("/api/pots/{id}", put(update_pot_handler).delete(delete_pot_handler))
        .route("/api/pots/{id}/close", post(close_pot))
        .route("/api/pots/{id}/contribute", post(contribute))
        .route("/api/pots/{id}/export.csv", get(export_csv))
        .route("/api/contributions/{id}", delete(delete_contribution_handler))
        .route("/api/contributions/{id}/confirm", put(confirm_contribution_handler))
        .with_state(state)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn format_euros(cents: i64) -> String {
    if cents % 100 == 0 {
        format!("{}\u{a0}€", cents / 100)
    } else {
        format!("{},{:02}\u{a0}€", cents / 100, (cents % 100).abs())
    }
}

fn internal(e: anyhow::Error) -> (StatusCode, String) {
    tracing::error!("{e:#}");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
}

fn not_found() -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, "not found".into())
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

fn is_closed(pot: &db::Pot) -> bool {
    if pot.closed {
        return true;
    }
    if let Some(ref deadline) = pot.deadline {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        return deadline.as_str() < today.as_str();
    }
    false
}

fn days_remaining(deadline: &str) -> i64 {
    let today = chrono::Utc::now().date_naive();
    if let Ok(dl) = chrono::NaiveDate::parse_from_str(deadline, "%Y-%m-%d") {
        (dl - today).num_days()
    } else {
        0
    }
}

async fn is_admin_session(pool: &PgPool, state: &AppState, headers: &HeaderMap) -> bool {
    let Some(uid) = auth::extract_user_id(headers, &state.jwt_secret) else {
        return false;
    };
    users::is_admin(pool, &uid).await.unwrap_or(false)
}

async fn require_admin(
    pool: &PgPool,
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, String)> {
    if is_admin_session(pool, state, headers).await {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "admin required".into()))
    }
}

async fn check_pot_access(
    pool: &PgPool,
    state: &AppState,
    headers: &HeaderMap,
    pot: &db::Pot,
) -> Result<(), (StatusCode, String)> {
    let user_id = auth::extract_user_id(headers, &state.jwt_secret)
        .ok_or((StatusCode::UNAUTHORIZED, "login required".into()))?;
    if users::is_admin(pool, &user_id).await.unwrap_or(false) {
        return Ok(());
    }
    if pot.owner_id.as_deref() == Some(user_id.as_str()) {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "forbidden".into()))
    }
}

async fn nav_html(state: &AppState, headers: &HeaderMap) -> String {
    let user = if let Some(uid) = auth::extract_user_id(headers, &state.jwt_secret) {
        users::get_user_by_id(&state.pool, &uid).await.ok().flatten()
    } else {
        None
    };
    let admin = user.as_ref().map_or(false, |u| u.is_admin);
    let mut links = Vec::new();
    links.push(r#"<a href="/" class="nav__link">Accueil</a>"#.to_string());
    if user.is_some() {
        links.push(r#"<a href="/dashboard" class="nav__link">Dashboard</a>"#.to_string());
        links.push(r#"<a href="/me" class="nav__link">Mon compte</a>"#.to_string());
    }
    if admin {
        links.push(r#"<a href="/admin/dashboard" class="nav__link">Admin</a>"#.to_string());
    }
    if let Some(u) = user {
        links.push(format!(
            r#"<span class="nav__user">{}</span> <button class="nav__logout" onclick="logout()">Logout</button>"#,
            html_escape(if !u.name.is_empty() { &u.name } else { &u.email })
        ));
    } else {
        links.push(r#"<a href="/login" class="nav__link">Login</a>"#.to_string());
        links.push(r#"<a href="/signup" class="nav__link nav__link--primary">Signup</a>"#.to_string());
    }
    let logout_js = r#"<script>function logout(){fetch('/api/logout',{method:'POST'}).then(function(){location.href='/'})}</script>"#;
    format!(r#"<nav class="nav">{}</nav>{}"#, links.join(" "), logout_js)
}

fn set_session_cookie(token: &str) -> String {
    format!("session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000", token)
}

fn clear_session_cookie() -> &'static str {
    "session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
}

// ── pages ──

async fn health() -> &'static str {
    "ok"
}

async fn index_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Html<String>, (StatusCode, String)> {
    let pots = db::list_pots(&state.pool).await.map_err(internal)?;
    let nav = nav_html(&state, &headers).await;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut cards = String::new();
    if pots.is_empty() {
        cards.push_str("<p class=\"empty\">Aucune cagnotte pour le moment.</p>");
    }
    for pot in &pots {
        let pct = if pot.goal_cents > 0 { (pot.total_cents * 100 / pot.goal_cents).min(100) } else { 0 };
        let color = if pot.total_cents >= pot.goal_cents { "var(--mustard)" } else { "var(--teal)" };
        let deadline_closed = pot.deadline.as_deref().map_or(false, |d| d < today.as_str());
        let effectively_closed = pot.closed || deadline_closed;
        let closed_class = if effectively_closed { " card--closed" } else { "" };

        cards.push_str(&format!(
            r#"<a href="/p/{slug}" class="card{closed_class}">
  <h2 class="card__title">{title}</h2>
  <div class="card__bar"><div class="card__fill" style="width:{pct}%;background:{color}"></div></div>
  <div class="card__info"><span class="card__amount">{current} / {goal}</span><span class="card__count">{count} contributeur{pl}</span></div>
</a>"#,
            slug = html_escape(&pot.slug),
            title = html_escape(&pot.title),
            pct = pct,
            color = color,
            current = format_euros(pot.total_cents),
            goal = format_euros(pot.goal_cents),
            count = pot.contributor_count,
            pl = if pot.contributor_count > 1 { "s" } else { "" },
            closed_class = closed_class,
        ));
    }

    let html = INDEX_TEMPLATE
        .replace("{{nav}}", &nav)
        .replace("{{pot_cards}}", &cards);
    Ok(Html(html))
}

async fn login_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Html<String> {
    let nav = nav_html(&state, &headers).await;
    Html(LOGIN_TEMPLATE.replace("{{nav}}", &nav))
}

async fn signup_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Html<String> {
    let nav = nav_html(&state, &headers).await;
    Html(SIGNUP_TEMPLATE.replace("{{nav}}", &nav))
}

async fn profile_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let Some(user_id) = auth::extract_user_id(&headers, &state.jwt_secret) else {
        return Ok(redirect_to("/login"));
    };
    let user = users::get_user_by_id(&state.pool, &user_id).await.map_err(internal)?.ok_or(not_found())?;
    let nav = nav_html(&state, &headers).await;

    let html = PROFILE_TEMPLATE
        .replace("{{nav}}", &nav)
        .replace("{{name}}", &html_escape(&user.name))
        .replace("{{email}}", &html_escape(&user.email));

    let mut resp = axum::response::Response::new(html.into());
    resp.headers_mut().insert("content-type", "text/html; charset=utf-8".parse().unwrap());
    Ok(resp)
}

fn redirect_to(loc: &str) -> axum::response::Response {
    let mut resp = axum::response::Response::new(axum::body::Body::empty());
    *resp.status_mut() = StatusCode::SEE_OTHER;
    resp.headers_mut().insert("location", loc.parse().unwrap());
    resp
}

async fn dashboard_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let Some(user_id) = auth::extract_user_id(&headers, &state.jwt_secret) else {
        return Ok(redirect_to("/login"));
    };
    let user = users::get_user_by_id(&state.pool, &user_id).await.map_err(internal)?.ok_or(not_found())?;
    let pots = db::list_pots_by_owner(&state.pool, &user_id).await.map_err(internal)?;
    let contribs = db::list_contributions_by_user(&state.pool, &user_id).await.map_err(internal)?;
    let nav = nav_html(&state, &headers).await;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut pot_rows = String::new();
    if pots.is_empty() {
        pot_rows.push_str(r#"<tr><td colspan="7" class="empty">Aucune cagnotte. Crée-en une ci-dessous.</td></tr>"#);
    }
    for p in &pots {
        let pct = if p.goal_cents > 0 { (p.total_cents * 100 / p.goal_cents).min(999) } else { 0 };
        let dl_closed = p.deadline.as_deref().map_or(false, |d| d < today.as_str());
        let closed = p.closed || dl_closed;
        let status = if closed { "Fermée" } else { "Ouverte" };
        let status_class = if closed { "status--closed" } else { "status--open" };
        pot_rows.push_str(&format!(
            r#"<tr>
  <td class="row__title"><a href="/p/{slug}">{title}</a></td>
  <td class="row__amount">{cur} / {goal}</td>
  <td class="row__pct">{pct}%</td>
  <td>{count}</td>
  <td>{deadline}</td>
  <td><span class="status {status_class}">{status}</span></td>
  <td class="row__actions">
    <a class="btn btn--sm btn--teal" href="/admin/{id}">Gérer</a>
    <a class="btn btn--sm btn--outline" href="/p/{slug}" target="_blank">Voir</a>
  </td>
</tr>"#,
            slug = html_escape(&p.slug),
            title = html_escape(&p.title),
            cur = format_euros(p.total_cents),
            goal = format_euros(p.goal_cents),
            pct = pct,
            count = p.contributor_count,
            deadline = html_escape(p.deadline.as_deref().unwrap_or("—")),
            status_class = status_class,
            status = status,
            id = html_escape(&p.id),
        ));
    }

    let mut contrib_rows = String::new();
    if contribs.is_empty() {
        contrib_rows.push_str(r#"<tr><td colspan="4" class="empty">Aucune contribution.</td></tr>"#);
    }
    for c in &contribs {
        let confirmed = if c.confirmed { r#"<span class="badge">✓</span>"# } else { "" };
        contrib_rows.push_str(&format!(
            r#"<tr>
  <td><a href="/p/{slug}">{title}</a></td>
  <td>{amount}</td>
  <td>{date}</td>
  <td>{confirmed}</td>
</tr>"#,
            slug = html_escape(&c.pot_slug),
            title = html_escape(&c.pot_title),
            amount = format_euros(c.amount_cents),
            date = html_escape(&c.created_at),
            confirmed = confirmed,
        ));
    }

    let html = DASHBOARD_TEMPLATE
        .replace("{{nav}}", &nav)
        .replace("{{user_name}}", &html_escape(if !user.name.is_empty() { &user.name } else { &user.email }))
        .replace("{{pot_count}}", &format!("{}", pots.len()))
        .replace("{{contrib_count}}", &format!("{}", contribs.len()))
        .replace("{{pot_rows}}", &pot_rows)
        .replace("{{contrib_rows}}", &contrib_rows);

    let mut resp = axum::response::Response::new(html.into());
    resp.headers_mut().insert("content-type", "text/html; charset=utf-8".parse().unwrap());
    Ok(resp)
}

async fn pot_page(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>, (StatusCode, String)> {
    let pot = db::get_pot_by_slug(&state.pool, &slug).await.map_err(internal)?.ok_or_else(not_found)?;
    let contributions = db::get_contributions(&state.pool, &pot.id).await.map_err(internal)?;
    let nav = nav_html(&state, &headers).await;

    let total_cents: i64 = contributions.iter().map(|c| c.amount_cents).sum();
    let pct = if pot.goal_cents > 0 { total_cents * 100 / pot.goal_cents } else { 0 };
    let pct_display = pct.min(999);
    let pct_width = pct.min(100);
    let exceeded = total_cents >= pot.goal_cents && pot.goal_cents > 0;
    let progress_color = if exceeded { "var(--mustard)" } else { "var(--teal)" };
    let progress_extra = if exceeded { "<span class=\"exceeded\">Objectif dépassé !</span>" } else { "" };

    let closed = is_closed(&pot);
    let time_info = if closed {
        "Terminée".to_string()
    } else if let Some(ref deadline) = pot.deadline {
        let days = days_remaining(deadline);
        if days <= 0 {
            "Terminée".to_string()
        } else if days == 1 {
            "reste 1 jour".to_string()
        } else {
            format!("reste {} jours", days)
        }
    } else {
        String::new()
    };

    let payment_section = if closed {
        String::new()
    } else {
        format!(
            r#"<section class="section">
  <h3 class="section__title">Comment participer</h3>
  <div class="payment-info">{}</div>
  <p class="payment-hint">Après ton paiement, confirme ci-dessous :</p>
</section>"#,
            html_escape(&pot.payment_info).replace('\n', "<br>")
        )
    };

    let logged_in = auth::extract_user_id(&headers, &state.jwt_secret).is_some();
    let logged_in_notice = if logged_in {
        r#"<p class="logged-in-notice">Connecté : la contribution sera liée à ton compte.</p>"#
    } else {
        r#"<p class="logged-in-notice"><a href="/login">Connecte-toi</a> pour suivre tes contributions.</p>"#
    };

    let form_section = if closed {
        r#"<section class="section"><p class="closed-msg">Cette cagnotte est terminée.</p></section>"#.to_string()
    } else {
        let name_required = if pot.allow_anonymous { "" } else { " required" };
        let name_placeholder = if pot.allow_anonymous { "Ton prénom (optionnel)" } else { "Ton prénom" };
        format!(
            r#"<section class="section">
  {logged_in_notice}
  <form id="contribute-form" class="form">
    <div class="form__field">
      <label for="name">{name_placeholder}</label>
      <input type="text" id="name" name="name" maxlength="100" placeholder="{name_placeholder}"{name_required}>
    </div>
    <div class="form__field">
      <label for="amount">Montant (€)</label>
      <input type="number" id="amount" name="amount" min="1" step="0.01" required placeholder="25">
    </div>
    <div class="form__field">
      <label for="message">Message (optionnel)</label>
      <textarea id="message" name="message" maxlength="200" rows="2" placeholder="Un petit mot..."></textarea>
    </div>
    <button type="submit" class="btn">Je confirme ma participation</button>
    <div id="form-feedback" class="form__feedback"></div>
  </form>
</section>"#,
            logged_in_notice = logged_in_notice,
            name_placeholder = name_placeholder,
            name_required = name_required,
        )
    };

    let mut contrib_list = String::new();
    for c in &contributions {
        let name_part = if pot.show_names {
            html_escape(c.name.as_deref().unwrap_or("Anonyme"))
        } else {
            String::new()
        };
        let amount_part = if pot.show_amounts { format_euros(c.amount_cents) } else { String::new() };
        let msg_part = match &c.message {
            Some(m) if !m.is_empty() => format!(r#" <span class="contrib__msg">"&thinsp;{}&thinsp;"</span>"#, html_escape(m)),
            _ => String::new(),
        };
        let confirmed_badge = if c.confirmed { r#" <span class="badge" title="Confirmé">✓</span>"# } else { "" };
        let sep = if pot.show_names && pot.show_amounts { " · " } else { "" };
        contrib_list.push_str(&format!(
            r#"<li class="contrib">{name_part}{sep}{amount_part}{msg_part}{confirmed_badge}</li>"#,
        ));
    }

    let contributor_count = contributions.len();
    let count_text = format!("{} contributeur{}", contributor_count, if contributor_count > 1 { "s" } else { "" });
    let time_sep = if !time_info.is_empty() && contributor_count > 0 { " · " } else { "" };

    let html = POT_TEMPLATE
        .replace("{{nav}}", &nav)
        .replace("{{title}}", &html_escape(&pot.title))
        .replace("{{organizer}}", &html_escape(&pot.organizer))
        .replace("{{description}}", &html_escape(&pot.description).replace('\n', "<br>"))
        .replace("{{current_amount}}", &format_euros(total_cents))
        .replace("{{goal_amount}}", &format_euros(pot.goal_cents))
        .replace("{{percentage}}", &format!("{}%", pct_display))
        .replace("{{progress_width}}", &format!("{}", pct_width))
        .replace("{{progress_color}}", progress_color)
        .replace("{{progress_extra}}", progress_extra)
        .replace("{{count_text}}", &count_text)
        .replace("{{time_sep}}", time_sep)
        .replace("{{time_info}}", &time_info)
        .replace("{{payment_section}}", &payment_section)
        .replace("{{form_section}}", &form_section)
        .replace("{{contributions_list}}", &contrib_list)
        .replace("{{slug}}", &html_escape(&pot.slug))
        .replace("{{allow_anonymous}}", if pot.allow_anonymous { "true" } else { "false" });

    Ok(Html(html))
}

async fn admin_page(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let pot = db::get_pot_by_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    let contributions = db::get_contributions(&state.pool, &pot.id).await.map_err(internal)?;
    let nav = nav_html(&state, &headers).await;

    let total_cents: i64 = contributions.iter().map(|c| c.amount_cents).sum();
    let pct = if pot.goal_cents > 0 { total_cents * 100 / pot.goal_cents } else { 0 };
    let pct_width = pct.min(100);
    let exceeded = total_cents >= pot.goal_cents && pot.goal_cents > 0;
    let progress_color = if exceeded { "var(--mustard)" } else { "var(--teal)" };
    let closed = is_closed(&pot);

    let mut contrib_rows = String::new();
    for c in &contributions {
        let name = html_escape(c.name.as_deref().unwrap_or("Anonyme"));
        let amount = format_euros(c.amount_cents);
        let msg = html_escape(c.message.as_deref().unwrap_or(""));
        let confirmed_class = if c.confirmed { " confirmed" } else { "" };
        let confirm_btn = if c.confirmed {
            r#"<span class="badge">✓</span>"#.to_string()
        } else {
            format!(r#"<button class="btn btn--sm btn--teal" onclick="confirmContrib('{}')">Confirmer</button>"#, html_escape(&c.id))
        };
        contrib_rows.push_str(&format!(
            r#"<tr class="contrib-row{confirmed_class}" id="contrib-{cid}">
  <td>{name}</td><td>{amount}</td><td>{msg}</td><td>{date}</td>
  <td>{confirm_btn} <button class="btn btn--sm btn--danger" onclick="deleteContrib('{cid}')">Supprimer</button></td>
</tr>"#,
            confirmed_class = confirmed_class,
            cid = html_escape(&c.id),
            name = name,
            amount = amount,
            msg = msg,
            date = html_escape(&c.created_at),
            confirm_btn = confirm_btn,
        ));
    }

    let close_btn_text = if closed { "Rouvrir" } else { "Fermer" };
    let closed_label = if closed { "Fermée" } else { "Ouverte" };

    let html = ADMIN_TEMPLATE
        .replace("{{nav}}", &nav)
        .replace("{{title}}", &html_escape(&pot.title))
        .replace("{{organizer}}", &html_escape(&pot.organizer))
        .replace("{{description}}", &html_escape(&pot.description))
        .replace("{{current_amount}}", &format_euros(total_cents))
        .replace("{{goal_amount}}", &format_euros(pot.goal_cents))
        .replace("{{percentage}}", &format!("{}%", pct.min(999)))
        .replace("{{progress_width}}", &format!("{}", pct_width))
        .replace("{{progress_color}}", progress_color)
        .replace("{{contributor_count}}", &format!("{}", contributions.len()))
        .replace("{{contrib_rows}}", &contrib_rows)
        .replace("{{close_btn_text}}", close_btn_text)
        .replace("{{closed_label}}", closed_label)
        .replace("{{pot_id}}", &html_escape(&pot.id))
        .replace("{{pot_slug}}", &html_escape(&pot.slug))
        .replace("{{edit_title}}", &html_escape(&pot.title))
        .replace("{{edit_description}}", &html_escape(&pot.description))
        .replace("{{edit_goal}}", &format!("{}", pot.goal_cents))
        .replace("{{edit_payment_info}}", &html_escape(&pot.payment_info))
        .replace("{{edit_organizer}}", &html_escape(&pot.organizer))
        .replace("{{edit_deadline}}", pot.deadline.as_deref().unwrap_or(""))
        .replace("{{edit_allow_anonymous}}", if pot.allow_anonymous { "checked" } else { "" })
        .replace("{{edit_show_amounts}}", if pot.show_amounts { "checked" } else { "" })
        .replace("{{edit_show_names}}", if pot.show_names { "checked" } else { "" });

    let mut response = axum::response::Response::new(html.into());
    response.headers_mut().insert("content-type", "text/html; charset=utf-8".parse().unwrap());
    Ok(response)
}

async fn admin_dashboard_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    require_admin(&state.pool, &state, &headers).await?;
    let pots = db::list_pots(&state.pool).await.map_err(internal)?;
    let all_users = users::list_users(&state.pool).await.map_err(internal)?;
    let nav = nav_html(&state, &headers).await;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut rows = String::new();
    if pots.is_empty() {
        rows.push_str(r#"<tr><td colspan="7" class="empty">Aucune cagnotte.</td></tr>"#);
    }
    for pot in &pots {
        let pct = if pot.goal_cents > 0 { (pot.total_cents * 100 / pot.goal_cents).min(999) } else { 0 };
        let deadline_closed = pot.deadline.as_deref().map_or(false, |d| d < today.as_str());
        let closed = pot.closed || deadline_closed;
        let status = if closed { "Fermée" } else { "Ouverte" };
        let status_class = if closed { "status--closed" } else { "status--open" };
        let owner = pot.owner_id.as_deref().unwrap_or("—");
        rows.push_str(&format!(
            r#"<tr class="row">
  <td class="row__title"><a href="/p/{slug}">{title}</a></td>
  <td class="row__amount">{current} / {goal}</td>
  <td class="row__pct">{pct}%</td>
  <td>{count}</td>
  <td>{owner}</td>
  <td><span class="status {status_class}">{status}</span></td>
  <td class="row__actions">
    <a class="btn btn--sm btn--teal" href="/admin/{id}">Gérer</a>
    <a class="btn btn--sm btn--outline" href="/p/{slug}" target="_blank">Voir</a>
  </td>
</tr>"#,
            slug = html_escape(&pot.slug),
            title = html_escape(&pot.title),
            current = format_euros(pot.total_cents),
            goal = format_euros(pot.goal_cents),
            pct = pct,
            count = pot.contributor_count,
            owner = html_escape(owner),
            status_class = status_class,
            status = status,
            id = html_escape(&pot.id),
        ));
    }

    let mut user_rows = String::new();
    if all_users.is_empty() {
        user_rows.push_str(r#"<tr><td colspan="5" class="empty">Aucun utilisateur.</td></tr>"#);
    }
    for u in &all_users {
        let role_badge = if u.is_admin {
            r#"<span class="badge badge--admin">ADMIN</span>"#
        } else {
            r#"<span class="badge badge--user">user</span>"#
        };
        let action_btn = if u.is_admin {
            format!(r#"<button class="btn btn--sm btn--danger" onclick="demoteUser('{id}')">Rétrograder</button>"#, id = html_escape(&u.id))
        } else {
            format!(r#"<button class="btn btn--sm btn--teal" onclick="promoteUser('{id}')">Promouvoir</button>"#, id = html_escape(&u.id))
        };
        user_rows.push_str(&format!(
            r#"<tr id="user-{id}">
  <td>{name}</td>
  <td>{email}</td>
  <td>{role}</td>
  <td>{created}</td>
  <td>{action}</td>
</tr>"#,
            id = html_escape(&u.id),
            name = html_escape(if u.name.is_empty() { "—" } else { &u.name }),
            email = html_escape(&u.email),
            role = role_badge,
            created = html_escape(&u.created_at),
            action = action_btn,
        ));
    }

    let html = ADMIN_DASHBOARD_TEMPLATE
        .replace("{{nav}}", &nav)
        .replace("{{rows}}", &rows)
        .replace("{{pot_count}}", &format!("{}", pots.len()))
        .replace("{{user_count}}", &format!("{}", all_users.len()))
        .replace("{{user_rows}}", &user_rows);

    let mut response = axum::response::Response::new(html.into());
    response.headers_mut().insert("content-type", "text/html; charset=utf-8".parse().unwrap());
    Ok(response)
}

// ── auth API ──

async fn api_captcha(State(state): State<Arc<AppState>>) -> Json<Value> {
    let c = captcha::issue(&state.jwt_secret);
    Json(json!({
        "algorithm": c.algorithm,
        "challenge": c.challenge,
        "maxnumber": c.maxnumber,
        "salt": c.salt,
        "signature": c.signature,
    }))
}

#[derive(Deserialize)]
struct SignupReq {
    email: String,
    password: String,
    name: Option<String>,
    captcha: String,
}

async fn api_signup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SignupReq>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !captcha::verify(&state.jwt_secret, &body.captcha) {
        return Err((StatusCode::BAD_REQUEST, "invalid captcha".into()));
    }
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err((StatusCode::BAD_REQUEST, "invalid email".into()));
    }
    if body.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "password must be at least 8 characters".into()));
    }
    let name = body.name.unwrap_or_default().trim().to_string();

    let hash = users::hash_password(&body.password).map_err(internal)?;
    let user_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    if users::email_exists(&state.pool, &email).await.map_err(internal)? {
        return Err((StatusCode::CONFLICT, "email already registered".into()));
    }
    let is_first = users::count_users(&state.pool).await.map_err(internal)? == 0;
    users::create_user(&state.pool, &user_id, &email, &hash, &name).await.map_err(internal)?;
    if is_first {
        users::set_admin(&state.pool, &user_id, true).await.map_err(internal)?;
        tracing::info!("first user {} promoted to admin", email);
    }

    let token = session::issue_token(&state.jwt_secret, &user_id).map_err(internal)?;
    let mut resp = axum::response::Response::new(json!({"ok": true, "id": user_id}).to_string().into());
    resp.headers_mut().insert("content-type", "application/json".parse().unwrap());
    resp.headers_mut().insert("set-cookie", set_session_cookie(&token).parse().unwrap());
    Ok(resp)
}

#[derive(Deserialize)]
struct LoginReq {
    email: String,
    password: String,
    captcha: String,
}

async fn api_login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginReq>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !captcha::verify(&state.jwt_secret, &body.captcha) {
        return Err((StatusCode::BAD_REQUEST, "invalid captcha".into()));
    }
    let email = body.email.trim().to_lowercase();
    let pair = users::get_user_with_hash_by_email(&state.pool, &email).await.map_err(internal)?;
    let (user, stored_hash) = pair.ok_or((StatusCode::UNAUTHORIZED, "invalid credentials".into()))?;
    if !users::verify_password(&body.password, &stored_hash) {
        return Err((StatusCode::UNAUTHORIZED, "invalid credentials".into()));
    }
    let token = session::issue_token(&state.jwt_secret, &user.id).map_err(internal)?;
    let mut resp = axum::response::Response::new(json!({"ok": true, "id": user.id}).to_string().into());
    resp.headers_mut().insert("content-type", "application/json".parse().unwrap());
    resp.headers_mut().insert("set-cookie", set_session_cookie(&token).parse().unwrap());
    Ok(resp)
}

async fn api_logout() -> axum::response::Response {
    let mut resp = axum::response::Response::new(json!({"ok": true}).to_string().into());
    resp.headers_mut().insert("content-type", "application/json".parse().unwrap());
    resp.headers_mut().append("set-cookie", clear_session_cookie().parse().unwrap());
    resp.headers_mut().append("set-cookie", "admin_token=; Path=/; SameSite=Strict; Max-Age=0".parse().unwrap());
    resp
}

#[derive(Deserialize)]
struct UpdateMeReq {
    email: String,
    name: String,
}

async fn api_update_me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<UpdateMeReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let user_id = auth::require_user(&headers, &state.jwt_secret)?;
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err((StatusCode::BAD_REQUEST, "invalid email".into()));
    }
    users::update_user(&state.pool, &user_id, &email, body.name.trim()).await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct UpdatePwReq {
    current_password: String,
    new_password: String,
}

async fn api_update_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<UpdatePwReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let user_id = auth::require_user(&headers, &state.jwt_secret)?;
    if body.new_password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "password must be at least 8 characters".into()));
    }
    let stored = users::get_password_hash(&state.pool, &user_id).await.map_err(internal)?;
    let stored = stored.ok_or((StatusCode::UNAUTHORIZED, "user not found".into()))?;
    if !users::verify_password(&body.current_password, &stored) {
        return Err((StatusCode::UNAUTHORIZED, "current password incorrect".into()));
    }
    let new_hash = users::hash_password(&body.new_password).map_err(internal)?;
    users::update_password(&state.pool, &user_id, &new_hash).await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn api_promote_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_admin(&state.pool, &state, &headers).await?;
    users::set_admin(&state.pool, &id, true).await.map_err(internal)?;
    Ok(Json(json!({"ok": true, "is_admin": true})))
}

async fn api_demote_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_admin(&state.pool, &state, &headers).await?;
    users::set_admin(&state.pool, &id, false).await.map_err(internal)?;
    Ok(Json(json!({"ok": true, "is_admin": false})))
}

// ── pot API ──

#[derive(Deserialize)]
struct CreatePotRequest {
    title: String,
    description: Option<String>,
    goal_cents: i64,
    currency: Option<String>,
    payment_info: Option<String>,
    organizer: Option<String>,
    deadline: Option<String>,
    allow_anonymous: Option<bool>,
    show_amounts: Option<bool>,
    show_names: Option<bool>,
}

async fn create_pot(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreatePotRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let user_id = auth::require_user(&headers, &state.jwt_secret)?;

    if body.title.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "title is required".into()));
    }
    if body.goal_cents <= 0 {
        return Err((StatusCode::BAD_REQUEST, "goal_cents must be positive".into()));
    }

    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let base_slug = slugify(&body.title);
    if base_slug.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "title produces empty slug".into()));
    }

    let owner_id = Some(user_id.as_str());

    let mut slug = base_slug.clone();
    let mut suffix = 2;
    while db::slug_exists(&state.pool, &slug).await.map_err(internal)? {
        slug = format!("{}-{}", &base_slug.chars().take(46).collect::<String>(), suffix);
        suffix += 1;
    }

    db::create_pot(
        &state.pool,
        &id,
        &slug,
        body.title.trim(),
        body.description.as_deref().unwrap_or(""),
        body.goal_cents,
        body.currency.as_deref().unwrap_or("EUR"),
        body.payment_info.as_deref().unwrap_or(""),
        body.organizer.as_deref().unwrap_or(""),
        body.deadline.as_deref(),
        body.allow_anonymous.unwrap_or(true),
        body.show_amounts.unwrap_or(true),
        body.show_names.unwrap_or(true),
        owner_id,
    )
    .await
    .map_err(internal)?;

    Ok(Json(json!({
        "id": id,
        "slug": slug,
        "public_url": format!("/p/{}", slug),
        "admin_url": format!("/admin/{}", id),
    })))
}

#[derive(Deserialize)]
struct UpdatePotRequest {
    title: String,
    description: Option<String>,
    goal_cents: i64,
    payment_info: Option<String>,
    organizer: Option<String>,
    deadline: Option<String>,
    allow_anonymous: Option<bool>,
    show_amounts: Option<bool>,
    show_names: Option<bool>,
}

async fn update_pot_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdatePotRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pot = db::get_pot_by_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    db::update_pot(
        &state.pool,
        &id,
        body.title.trim(),
        body.description.as_deref().unwrap_or(""),
        body.goal_cents,
        body.payment_info.as_deref().unwrap_or(""),
        body.organizer.as_deref().unwrap_or(""),
        body.deadline.as_deref(),
        body.allow_anonymous.unwrap_or(true),
        body.show_amounts.unwrap_or(true),
        body.show_names.unwrap_or(true),
    )
    .await
    .map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn delete_pot_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pot = db::get_pot_by_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    db::delete_pot(&state.pool, &id).await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn close_pot(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pot = db::get_pot_by_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    let closed = db::toggle_close(&state.pool, &id).await.map_err(internal)?;
    Ok(Json(json!({"ok": true, "closed": closed})))
}

#[derive(Deserialize)]
struct ContributeRequest {
    name: Option<String>,
    amount_cents: i64,
    message: Option<String>,
}

async fn contribute(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ContributeRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.amount_cents < 100 {
        return Err((StatusCode::BAD_REQUEST, "amount must be at least 1 €".into()));
    }
    if body.message.as_ref().is_some_and(|m| m.len() > 200) {
        return Err((StatusCode::BAD_REQUEST, "message too long".into()));
    }

    let ip = client_ip(&headers);
    {
        let mut limiter = state.rate_limiter.lock().unwrap();
        if let Some(last) = limiter.get(&ip) {
            if last.elapsed() < Duration::from_secs(60) {
                return Err((StatusCode::TOO_MANY_REQUESTS, "Attends une minute avant de contribuer à nouveau.".into()));
            }
        }
        limiter.insert(ip, Instant::now());
    }

    let user_id = auth::extract_user_id(&headers, &state.jwt_secret);

    let pot = db::get_pot_by_slug(&state.pool, &slug).await.map_err(internal)?.ok_or_else(not_found)?;

    if is_closed(&pot) {
        return Err((StatusCode::BAD_REQUEST, "Cette cagnotte est terminée.".into()));
    }

    if !pot.allow_anonymous && body.name.as_ref().map_or(true, |n| n.trim().is_empty()) {
        return Err((StatusCode::BAD_REQUEST, "name is required".into()));
    }

    let contrib_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let name = body.name.as_deref().map(|n| n.trim()).filter(|n| !n.is_empty());
    let message = body.message.as_deref().map(|m| m.trim()).filter(|m| !m.is_empty());

    db::create_contribution(&state.pool, &contrib_id, &pot.id, name, body.amount_cents, message, user_id.as_deref())
        .await
        .map_err(internal)?;

    Ok(Json(json!({"ok": true, "id": contrib_id})))
}

async fn delete_contribution_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pot_id = db::get_contribution_pot_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    let pot = db::get_pot_by_id(&state.pool, &pot_id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    db::delete_contribution(&state.pool, &id).await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn confirm_contribution_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let pot_id = db::get_contribution_pot_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    let pot = db::get_pot_by_id(&state.pool, &pot_id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    db::confirm_contribution(&state.pool, &id).await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

async fn export_csv(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let pot = db::get_pot_by_id(&state.pool, &id).await.map_err(internal)?.ok_or_else(not_found)?;
    check_pot_access(&state.pool, &state, &headers, &pot).await?;
    let contributions = db::get_contributions(&state.pool, &id).await.map_err(internal)?;

    let mut csv = String::from("nom,montant,message,date,confirmé\n");
    for c in &contributions {
        let name = c.name.as_deref().unwrap_or("Anonyme");
        let amount = format!("{:.2}", c.amount_cents as f64 / 100.0);
        let msg = c.message.as_deref().unwrap_or("");
        let confirmed = if c.confirmed { "oui" } else { "non" };
        csv.push_str(&format!(
            "\"{}\",{},\"{}\",{},{}\n",
            name.replace('"', "\"\""),
            amount,
            msg.replace('"', "\"\""),
            c.created_at,
            confirmed,
        ));
    }

    let mut response = axum::response::Response::new(csv.into());
    response.headers_mut().insert("content-type", "text/csv; charset=utf-8".parse().unwrap());
    response.headers_mut().insert(
        "content-disposition",
        format!("attachment; filename=\"cagnotte-{}.csv\"", id).parse().unwrap(),
    );
    Ok(response)
}
