use crate::dtos::auth::MeResponse;
use crate::dtos::response::AppError;
use crate::git::repositories::user;
use crate::state::state::SharedState;

pub async fn me(state: &SharedState, user_id: &str) -> Result<MeResponse, AppError> {
    let user_row = user::get_user_by_id(state, user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    let is_sysadmin = user::is_sysadmin(state, &user_row.id).await;

    Ok(MeResponse {
        user_id: user_row.id,
        email: user_row.email,
        name: user_row.name,
        is_sysadmin,
        created_at: user_row.created_at,
    })
}
