# Cagnotte

Self-hosted shared money pot. Track collective gifts (leaving present, group dinner, refund split) without payment processing. Contributors pay out-of-band (Lydia, IBAN, cash, whatever you put in the pot's payment info) and declare the amount on the pot page.

No fees. No payment provider. No KYC. The app records intent; the money moves on rails you already trust.

## Stack

- **Rust** + **Axum 0.8** — HTTP server
- **Postgres 17** via **sqlx** — storage
- **argon2** — password hashing
- **JWT** (`jsonwebtoken`) — session cookies, 30-day expiry, HttpOnly
- **Altcha-style proof-of-work captcha** — self-hosted SHA-256/HMAC, no third-party
- Vanilla JS + inline CSS in HTML templates. No frontend bundler.
- Docker Compose for deployment

## Features

- User accounts (signup, login, profile, password change)
- First registered user is auto-promoted to admin
- Per-user admin role, stored in DB, promote/demote via admin dashboard
- Captcha on signup and login (PoW, falls back to pure-JS SHA-256 when `crypto.subtle` is unavailable on plain-HTTP origins)
- Create / edit / close / delete pots
- Public pot page at `/p/{slug}` with progress bar, payment info, contribution form, contributor list
- **Pots are private by default** — only reachable by direct link until the owner publishes them
- Homepage `/` lists only published pots
- User dashboard `/dashboard` — own pots + contributions, create form
- Admin dashboard `/admin/dashboard` — all pots (any visibility, any owner) + user management
- Pot management page `/admin/{id}` — accessible to owner or any admin
- CSV export of contributions
- Anonymous or named contributions (configurable per pot)
- Optional deadline; pots auto-close past their deadline
- Rate-limited contributions (1/min/IP)
- Risograph/editorial UI: Bricolage Grotesque + Fraunces + Space Mono, paper/ink/teal/vermilion palette, SVG grain overlay

## Quick start (Docker Compose)

```bash
cp .env.example .env
# edit .env: set POSTGRES_PASSWORD and JWT_SECRET (long random strings)
# openssl rand -hex 64  → JWT_SECRET
# openssl rand -hex 32  → POSTGRES_PASSWORD
docker compose up -d
```

App on `http://localhost:9023`. Visit `/signup` — the first account created becomes admin automatically.

## Quick start (local dev)

```bash
# 1. Run Postgres (any way you like)
docker run -d --rm --name pg -e POSTGRES_PASSWORD=dev -e POSTGRES_USER=cagnotte -e POSTGRES_DB=cagnotte -p 5432:5432 postgres:17-alpine

# 2. Run the app
export DATABASE_URL=postgres://cagnotte:dev@localhost:5432/cagnotte
export JWT_SECRET=$(openssl rand -hex 64)
cargo run
```

App on `http://localhost:3000`.

## Configuration

All via environment variables.

| Var | Required | Default | Notes |
|-----|----------|---------|-------|
| `DATABASE_URL` | yes | — | Postgres connection string, e.g. `postgres://user:pass@host:5432/db` |
| `JWT_SECRET` | recommended | random per run | Long random string. Signs session cookies and captcha challenges. If unset, sessions invalidate on every restart. |
| `PORT` | no | `3000` (compose: `9021` inside container) | TCP port the app listens on |
| `RUST_LOG` | no | `info,cagnotte=info` | Tracing filter |

Docker Compose binds container port 9021 to host port 9023. Change the `ports:` line if you want a different host port.

## Architecture

```
src/
├── main.rs       — entry point: env vars, pool, axum server, graceful shutdown
├── db.rs         — sqlx queries, schema init, idempotent migrations (ADD COLUMN IF NOT EXISTS)
├── users.rs      — user CRUD, argon2 hashing
├── auth.rs       — JWT extraction from session cookie
├── session.rs    — JWT issue/verify
├── captcha.rs    — Altcha-style PoW: HMAC-signed SHA-256 challenge
├── slug.rs       — title → URL-safe slug, French accent handling
└── routes.rs     — all HTTP handlers, HTML rendering via include_str! + str::replace

templates/
├── index.html              — homepage (published pots only)
├── pot.html                — public pot page with contribution form
├── admin.html              — single-pot management (owner or admin)
├── dashboard.html          — user dashboard
├── admin-dashboard.html    — site-wide admin view
├── login.html, signup.html, profile.html
```

### Auth model

- All admin actions require a logged-in user with `users.is_admin = TRUE`.
- There is **no admin token** anymore. Session cookies only. Bootstrapping is handled by auto-promoting the first signup.
- Pot mutations (`PUT /api/pots/{id}`, `DELETE`, `/close`, `/publish`) allowed for the pot's owner or any admin.
- Contributions can be made by anyone (rate-limited per IP). If the contributor is logged in, the contribution is linked to their user_id and shows on their dashboard.

### Captcha

- Server issues a challenge: `{algorithm, challenge: sha256(salt+number), salt, maxnumber, signature: HMAC(jwt_secret, challenge)}`.
- Browser brute-forces the number (≤ 50 000 iterations, ~1 s) and submits the solved payload.
- Server recomputes the SHA-256, verifies the HMAC signature, accepts.
- Stateless. No CAPTCHA images, no third-party, GDPR-clean. Replay caveat: payloads are not single-use; PoW cost is the only abuse brake. Add an in-memory used-nonce set if you need stricter.
- Browser path uses `crypto.subtle.digest` when available, falls back to bundled pure-JS SHA-256 on insecure-context origins (plain HTTP on LAN).

### Pot visibility

`pots.is_public BOOLEAN NOT NULL DEFAULT FALSE`. New pots and any pre-existing pots after migration are private — accessible only by knowing `/p/{slug}`. Owner or admin can toggle via the **Rendre publique** button on `/admin/{id}`.

## API

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| `GET`  | `/api/captcha` | none | Issue PoW challenge |
| `POST` | `/api/signup` | captcha | Create account, set session cookie |
| `POST` | `/api/login` | captcha + password | Set session cookie |
| `POST` | `/api/logout` | none | Clear session cookie |
| `PUT`  | `/api/me` | session | Update email + name |
| `PUT`  | `/api/me/password` | session + current password | Change password |
| `POST` | `/api/users/{id}/promote` | admin | Grant admin |
| `POST` | `/api/users/{id}/demote` | admin | Revoke admin |
| `POST` | `/api/pots` | session | Create pot (caller becomes owner) |
| `PUT`  | `/api/pots/{id}` | owner or admin | Edit pot |
| `DELETE` | `/api/pots/{id}` | owner or admin | Delete pot + contributions |
| `POST` | `/api/pots/{id}/close` | owner or admin | Toggle closed |
| `POST` | `/api/pots/{id}/publish` | owner or admin | Toggle public |
| `POST` | `/api/pots/{id}/contribute` | none, rate-limited | Add contribution to pot by slug |
| `GET`  | `/api/pots/{id}/export.csv` | owner or admin | CSV of all contributions |
| `DELETE` | `/api/contributions/{id}` | pot owner or admin | Delete contribution |
| `PUT`  | `/api/contributions/{id}/confirm` | pot owner or admin | Mark confirmed |

Note: `POST /api/pots/{id}/contribute` takes the pot **slug** in the path despite the `{id}` placeholder — historical. Other admin endpoints take the pot **id**.

## Schema

```sql
users(id PK, email UNIQUE, password_hash, name, is_admin, created_at)
pots(id PK, slug UNIQUE, title, description, goal_cents, currency, payment_info,
     organizer, deadline, allow_anonymous, show_amounts, show_names, closed,
     created_at, owner_id → users.id, is_public)
contributions(id PK, pot_id → pots.id, name, amount_cents, message, confirmed,
              created_at, user_id → users.id)
```

All amounts in cents (i64). Booleans native. Strings for dates (`YYYY-MM-DD` for deadlines, `YYYY-MM-DD HH:MM:SS` UTC for `created_at`).

## Production notes

- **Put TLS in front.** Web Crypto API (`crypto.subtle`) is only available in secure contexts. The pure-JS SHA-256 fallback works on plain HTTP but is slower. Use Caddy / nginx / Cloudflare Tunnel for HTTPS.
- **Rotate `JWT_SECRET`.** Changing it invalidates every active session.
- **Postgres backups.** Volume `cagnotte-pgdata` is your data. Back it up. `pg_dump cagnotte` from inside the container is the easy path.
- **Rate limiting is in-memory.** Restarts reset it. Single-instance only. For multi-instance, swap to Redis-backed limiter.
- **No email.** Password reset and email verification are not implemented. Forgotten password = admin manually issues new hash, or user creates a new account and gets a new pot ownership story.

## License

MIT.
