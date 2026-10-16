use super::repository;
use crate::{dtos::response::AppError, state::state::SharedState};

pub async fn bootstrap(state: &SharedState) -> Result<(), AppError> {
    if state.is_cloud {
        // Kept so the SysAdmin can watch the control plane host; nothing may deploy to it.
        repository::assert_no_local_applications(state).await?;
        crate::scheduled_tasks::assert_no_local_scheduled_tasks(state).await?;
    }
    repository::ensure_local(state).await
}
