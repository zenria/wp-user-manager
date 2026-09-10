//! Thin client over the WordPress REST API (`/wp-json/wp/v2`).
//!
//! Authentication uses *application passwords*, which WordPress accepts as
//! HTTP Basic credentials (user login + application password).

use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const PER_PAGE: u32 = 100;

#[derive(Debug, Error)]
pub enum WpError {
    #[error("HTTP request to WordPress failed")]
    Transport(#[from] reqwest::Error),
    #[error("WordPress API error {status} [{code}]: {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error(
        "authentication rejected by WordPress: check --wp-user and --wp-app-password \
         (an application password, not the account password)"
    )]
    Unauthorized,
    #[error("`{0}` is not a usable WordPress site URL")]
    InvalidUrl(String),
}

/// A WordPress user as returned in the `edit` context.
#[derive(Debug, Clone, Deserialize)]
pub struct WpUser {
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

impl WpUser {
    pub fn roles_display(&self) -> String {
        if self.roles.is_empty() {
            "-".to_string()
        } else {
            self.roles.join(",")
        }
    }

    pub fn is_administrator(&self) -> bool {
        self.roles.iter().any(|role| role == "administrator")
    }
}

/// Payload used to create a user.
#[derive(Debug, Clone, Serialize)]
pub struct NewUser {
    pub username: String,
    pub email: String,
    pub name: String,
    pub password: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

pub struct WpClient {
    http: Client,
    api_base: String,
    user: String,
    app_password: String,
}

impl WpClient {
    pub fn new(
        site_url: &str,
        user: &str,
        app_password: &str,
        timeout: Duration,
    ) -> Result<Self, WpError> {
        let trimmed = site_url.trim().trim_end_matches('/');
        if trimmed.is_empty() || !trimmed.starts_with("http") {
            return Err(WpError::InvalidUrl(site_url.to_string()));
        }
        let api_base = if trimmed.ends_with("/wp-json") {
            format!("{trimmed}/wp/v2")
        } else if trimmed.contains("/wp-json/") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/wp-json/wp/v2")
        };

        let http = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("wp-user-manager/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            http,
            api_base,
            user: user.trim().to_string(),
            // WordPress displays application passwords in groups of four
            // characters; the spaces are not part of the secret.
            app_password: app_password.replace(' ', ""),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.api_base, path.trim_start_matches('/'))
    }

    fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        builder.basic_auth(&self.user, Some(&self.app_password))
    }

    async fn send(&self, builder: RequestBuilder) -> Result<Response, WpError> {
        let response = self.request(builder).send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(WpError::Unauthorized);
        }
        let body = response.text().await.unwrap_or_default();
        let (code, message) = serde_json::from_str::<ApiError>(&body)
            .map(|e| (e.code, e.message))
            .unwrap_or_else(|_| (String::from("unknown"), truncate(&body, 300)));
        Err(WpError::Api {
            status: status.as_u16(),
            code,
            message,
        })
    }

    /// The user the application password belongs to.
    pub async fn current_user(&self) -> Result<WpUser, WpError> {
        let response = self
            .send(
                self.http
                    .get(self.url("users/me"))
                    .query(&[("context", "edit")]),
            )
            .await?;
        Ok(response.json().await?)
    }

    /// Every user of the site, all pages, with their e-mail addresses.
    pub async fn list_users(&self) -> Result<Vec<WpUser>, WpError> {
        let mut users = Vec::new();
        let mut page = 1u32;
        loop {
            let response = self
                .send(self.http.get(self.url("users")).query(&[
                    ("context", "edit"),
                    ("per_page", &PER_PAGE.to_string()),
                    ("page", &page.to_string()),
                    ("orderby", "id"),
                    ("order", "asc"),
                ]))
                .await?;
            let total_pages = header_number(&response, "x-wp-totalpages").unwrap_or(1);
            let batch: Vec<WpUser> = response.json().await?;
            let batch_len = batch.len() as u32;
            users.extend(batch);
            if page >= total_pages || batch_len < PER_PAGE {
                break;
            }
            page += 1;
        }
        Ok(users)
    }

    pub async fn create_user(&self, new_user: &NewUser) -> Result<WpUser, WpError> {
        let response = self
            .send(self.http.post(self.url("users")).json(new_user))
            .await?;
        Ok(response.json().await?)
    }

    /// Permanently deletes a user. `reassign` designates the user inheriting
    /// their content; when `None`, the content is deleted along with the user.
    pub async fn delete_user(&self, id: u64, reassign: Option<u64>) -> Result<(), WpError> {
        let reassign = reassign.map_or_else(|| "false".to_string(), |id| id.to_string());
        self.send(
            self.http
                .delete(self.url(&format!("users/{id}")))
                .query(&[("force", "true"), ("reassign", &reassign)]),
        )
        .await?;
        Ok(())
    }
}

fn header_number(response: &Response, name: &str) -> Option<u32> {
    response
        .headers()
        .get(name)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn truncate(value: &str, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max {
        return value.to_string();
    }
    let head: String = value.chars().take(max).collect();
    format!("{head}…")
}
