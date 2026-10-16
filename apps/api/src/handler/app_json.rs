use axum::{
    Json,
    extract::{FromRequest, Request},
    response::{IntoResponse, Response},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::dtos::response::AppError;

#[allow(dead_code)] // alternative JSON extractor; axum's default `Json` is used by current handlers.
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(AppJson(v)),
            Err(rej) => Err(AppError::ValidationError(format!(
                "invalid request body: {}",
                rej.body_text()
            ))),
        }
    }
}

impl<T: Serialize> IntoResponse for AppJson<T> {
    fn into_response(self) -> Response {
        Json(self.0).into_response()
    }
}
