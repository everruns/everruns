// HTTP client wrapper for Everruns API

use reqwest::StatusCode;
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Not found")]
    NotFound,
}

pub struct Client {
    base_url: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http.get(&url).send().await?;
        self.handle_response(response).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http.post(&url).json(body).send().await?;
        self.handle_response(response).await
    }

    #[allow(dead_code)]
    pub async fn patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http.patch(&url).json(body).send().await?;
        self.handle_response(response).await
    }

    pub async fn delete(&self, path: &str) -> Result<(), ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.http.delete(&url).send().await?;

        if response.status() == StatusCode::NO_CONTENT || response.status() == StatusCode::OK {
            return Ok(());
        }

        if response.status() == StatusCode::NOT_FOUND {
            return Err(ClientError::NotFound);
        }

        let status = response.status().as_u16();
        let message = response.text().await.unwrap_or_default();
        Err(ClientError::Api { status, message })
    }

    async fn handle_response<T: DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            return Err(ClientError::NotFound);
        }

        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(ClientError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let body = response.json().await?;
        Ok(body)
    }
}
