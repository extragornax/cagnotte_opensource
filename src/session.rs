use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

pub fn issue_token(secret: &[u8], user_id: &str) -> anyhow::Result<String> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(30)).timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
    };
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret))?;
    Ok(token)
}

pub fn verify_token(secret: &[u8], token: &str) -> Option<String> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    )
    .ok()?;
    Some(data.claims.sub)
}
