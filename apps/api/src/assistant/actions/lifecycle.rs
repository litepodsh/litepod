use sqlx::Row;
use uuid::Uuid;

use crate::{
    deployment::compose::store::ComposeFileStore, dtos::response::AppError,
    security::middleware::auth::AuthUser, state::state::SharedState,
};

use super::apply::*;
use super::types::*;

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The row every write tool writes. Kept in one place so a new tool cannot
/// forget the expiry, the owner or the route.
#[allow(clippy::too_many_arguments)]
pub(super) async fn store_action(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    tool: &str,
    arguments: &str,
    title: String,
    summary: String,
) -> Result<ActionDto, AppError> {
    let id = Uuid::now_v7().to_string();
    let created_at = now_ms();
    let expires_at = created_at + TTL_MS;
    let route = target.route(tool);
    // NULL rather than an empty string: the column is a foreign key, and a
    // project-scoped action has no application to point at.
    let application_id = (!target.application_id.is_empty()).then_some(&target.application_id);

    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO assistant_actions (id, conversation_id, organization_id, project_id, environment_id, application_id, tool, arguments, title, summary, route, status, proposed_by, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12, ?13, ?14)",
    )
    .bind(&id)
    .bind(conversation_id)
    .bind(&target.organization_id)
    .bind(&target.project_id)
    .bind(&target.environment_id)
    .bind(application_id)
    .bind(tool)
    .bind(arguments)
    .bind(&title)
    .bind(&summary)
    .bind(&route)
    .bind(&actor.user_id)
    .bind(created_at)
    .bind(expires_at)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    audit(
        state,
        actor,
        &Audited {
            action_id: &id,
            conversation_id,
            organization_id: &target.organization_id,
            project_id: &target.project_id,
            application_id: &target.application_id,
            tool,
            title: &title,
        },
        "assistant.action_proposed",
        None,
    )
    .await;

    Ok(ActionDto {
        id,
        tool: tool.to_owned(),
        title,
        summary,
        status: "pending".into(),
        route,
        application_id: target.application_id.clone(),
        added: None,
        replaces: None,
        removed: None,
        diff: None,
        expires_at,
        failure: None,
    })
}

pub(super) struct StoredAction {
    pub(super) id: String,
    pub(super) conversation_id: String,
    pub(super) organization_id: String,
    pub(super) project_id: String,
    pub(super) environment_id: String,
    pub(super) application_id: String,
    pub(super) tool: String,
    pub(super) arguments: String,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) route: String,
    pub(super) status: String,
    pub(super) expires_at: i64,
    pub(super) failure: Option<String>,
}

pub(super) const COLUMNS: &str = "id, conversation_id, organization_id, project_id, environment_id, application_id, tool, arguments, title, summary, route, status, expires_at, failure";

pub(super) fn read_action(row: &sqlx::sqlite::SqliteRow) -> Result<StoredAction, AppError> {
    let text = |index: usize| -> Result<String, AppError> {
        row.try_get(index)
            .map_err(|error| AppError::DatabaseError(error.to_string()))
    };
    Ok(StoredAction {
        id: text(0)?,
        conversation_id: text(1)?,
        organization_id: text(2)?,
        project_id: text(3)?,
        environment_id: text(4)?,
        // NULL for the project-scoped tools; the empty string is what the rest
        // of this module reads as "no application".
        application_id: row
            .try_get::<Option<String>, _>(5)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .unwrap_or_default(),
        tool: text(6)?,
        arguments: text(7)?,
        title: text(8)?,
        summary: text(9)?,
        route: text(10)?,
        status: text(11)?,
        expires_at: row
            .try_get(12)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        failure: row
            .try_get(13)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

pub(super) fn dto(stored: &StoredAction, status: &str, added: Option<String>) -> ActionDto {
    ActionDto {
        id: stored.id.clone(),
        tool: stored.tool.clone(),
        title: stored.title.clone(),
        summary: stored.summary.clone(),
        status: status.into(),
        route: stored.route.clone(),
        application_id: stored.application_id.clone(),
        added,
        replaces: None,
        removed: None,
        diff: None,
        expires_at: stored.expires_at,
        failure: stored.failure.clone(),
    }
}

/// Diff helper used only on the read path for pending compose actions. The
/// `state` is needed to reach the on-disk compose; `arguments_json` is the
/// raw row value already in hand so we do not parse twice.
pub(super) async fn compose_diff_for(
    state: &SharedState,
    application_id: &str,
    arguments_json: &str,
) -> Option<String> {
    let arguments: SetComposeFile = serde_json::from_str(arguments_json).ok()?;
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let current = store.read_content().await.ok()?;
    if current == arguments.compose {
        return None;
    }
    Some(compose_diff(&current, &arguments.compose))
}

/// Every action in a conversation, for the panel to render on reload.
pub async fn list_for_conversation(
    state: &SharedState,
    conversation_id: &str,
) -> Result<Vec<ActionDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM assistant_actions WHERE conversation_id = ?1 ORDER BY created_at"
    ))
    .bind(conversation_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let stored = read_action(row)?;
        let status = stored.status.clone();
        let mut action = dto(&stored, &status, None);
        if status == "pending" && stored.tool == SET_COMPOSE_FILE {
            action.diff = compose_diff_for(state, &stored.application_id, &stored.arguments).await;
        }
        out.push(action);
    }
    Ok(out)
}

pub(super) async fn load(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
    action_id: &str,
) -> Result<StoredAction, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    // Scoped by organization as well as id: the extractor proved the caller may
    // edit *this* organization, and an id from another one must not ride in on
    // that.
    //
    // Scoped by proposer too, which matches the conversation it came from: an
    // assistant conversation is private to one user, so the change it proposed
    // is settled by that user and nobody else. Without this an editor who
    // learned an id could apply a change out of a colleague's private thread.
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM assistant_actions \
         WHERE id = ?1 AND organization_id = ?2 AND proposed_by = ?3"
    ))
    .bind(action_id)
    .bind(organization_id)
    .bind(&actor.user_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    read_action(&row)
}

pub(super) async fn settle(
    state: &SharedState,
    actor: &AuthUser,
    action_id: &str,
    status: &str,
    failure: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE assistant_actions SET status = ?1, settled_by = ?2, settled_at = ?3, failure = ?4 WHERE id = ?5",
    )
    .bind(status)
    .bind(&actor.user_id)
    .bind(now_ms())
    .bind(failure)
    .bind(action_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// The `exec` action a stored proposal came from: the write it performs, which
/// is the action's own name except for a redeploy.
pub(super) fn exec_action(tool: &str) -> &str {
    match tool {
        REDEPLOY_APPLICATION => "redeploy",
        START_APPLICATION => "start",
        STOP_APPLICATION => "stop",
        RESTART_APPLICATION => "restart",
        ROLLBACK_APPLICATION => "rollback",
        other => other,
    }
}

/// Applies a pending action.
///
/// The caller has been checked as an editor of the organization by the route's
/// extractor; `ensure_editor` is what proves they may edit this particular
/// application, because an organization can hold projects a member cannot touch.
pub async fn confirm(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
    action_id: &str,
) -> Result<ActionDto, AppError> {
    let stored = load(state, organization_id, actor, action_id).await?;
    if stored.status != "pending" {
        return Err(AppError::ValidationError(format!(
            "This change was already {}",
            stored.status
        )));
    }
    if now_ms() > stored.expires_at {
        settle(state, actor, action_id, "expired", None).await?;
        super::upload::discard(state, action_id).await;
        audit(
            state,
            actor,
            &stored.audited(),
            "assistant.action_expired",
            None,
        )
        .await;
        return Err(AppError::ValidationError(
            "This change expired. Ask again so the assistant can re-read the environment.".into(),
        ));
    }
    // Proposed while it was allowed, confirmed after a release took it away:
    // the allowlist is checked here too, not only when the model calls `exec`.
    if !crate::assistant::capabilities::action_allowed(exec_action(&stored.tool)) {
        let message = "The assistant is no longer allowed to make this change";
        settle(state, actor, action_id, "failed", Some(message)).await?;
        audit(
            state,
            actor,
            &stored.audited(),
            "assistant.action_failed",
            Some(message),
        )
        .await;
        return Err(AppError::ValidationError(message.into()));
    }
    // Each write tool applies itself; everything around it - the gate, the
    // audit line, the note back to the model - is the same either way.
    let applied = match stored.tool.as_str() {
        SET_ENVIRONMENT_VARIABLE => apply_environment_variable(state, &stored).await,
        REDEPLOY_APPLICATION => apply_redeploy(state, actor, &stored).await,
        START_APPLICATION | STOP_APPLICATION => apply_lifecycle(state, actor, &stored).await,
        RESTART_APPLICATION => apply_restart(state, actor, &stored).await,
        ROLLBACK_APPLICATION => apply_rollback(state, actor, &stored).await,
        SET_COMPOSE_FILE => apply_compose_file(state, &stored).await,
        DELETE_ENVIRONMENT_VARIABLE => apply_delete_environment_variable(state, &stored).await,
        UPDATE_APPLICATION => apply_update_application(state, &stored).await,
        SET_SOURCE => apply_source(state, &stored).await,
        LINK_DATABASE => apply_link_database(state, &stored).await,
        CREATE_APPLICATION => apply_create_application(state, &stored).await,
        CREATE_PROJECT => apply_create_project(state, &stored).await,
        UPDATE_PROJECT => apply_update_project(state, &stored).await,
        DELETE_PROJECT => apply_delete_project(state, &stored).await,
        ADD_DOMAIN => apply_add_domain(state, &stored).await,
        UPDATE_DOMAIN => apply_update_domain(state, &stored).await,
        DELETE_DOMAIN => apply_delete_domain(state, &stored).await,
        DEPLOY_STATIC_SITE => apply_static_site(state, actor, &stored).await,
        CREATE_FROM_TEMPLATE => apply_from_template(state, actor, &stored).await,
        DEPLOY_COMPOSE => apply_deploy_compose(state, actor, &stored).await,
        DEPLOY_ZIP | DEPLOY_REPOSITORY => apply_source_app(state, actor, &stored).await,
        other => {
            return Err(AppError::ValidationError(format!(
                "`{other}` is not a tool this build can apply"
            )));
        }
    };

    let applied = match applied {
        Ok(applied) => applied,
        Err(error) => {
            let message = error.to_string();
            settle(state, actor, action_id, "failed", Some(&message)).await?;
            // The next turn reads this, so the assistant can say why and fix it
            // instead of the user having to paste the error back.
            note(state, &stored, &format!("confirmed but failed: {message}")).await;
            audit(
                state,
                actor,
                &stored.audited(),
                "assistant.action_failed",
                Some(&message),
            )
            .await;
            return Err(error);
        }
    };

    settle(state, actor, action_id, "applied", None).await?;
    note(state, &stored, "confirmed and applied").await;

    // An MCP proposal lives in its token's conversation (`mcp::session`), so the
    // title names the agent that asked; the person confirming is still the actor.
    audit(state, actor, &stored.audited(), applied.audit_action, None).await;

    Ok(ActionDto {
        removed: applied.removed,
        ..dto(&stored, "applied", applied.added)
    })
}

/// What an audit line about a proposal names, whether it was just stored or loaded back.
pub(super) struct Audited<'a> {
    pub(super) action_id: &'a str,
    pub(super) conversation_id: &'a str,
    pub(super) organization_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) application_id: &'a str,
    pub(super) tool: &'a str,
    pub(super) title: &'a str,
}

impl StoredAction {
    pub(super) fn audited(&self) -> Audited<'_> {
        Audited {
            action_id: &self.id,
            conversation_id: &self.conversation_id,
            organization_id: &self.organization_id,
            project_id: &self.project_id,
            application_id: &self.application_id,
            tool: &self.tool,
            title: &self.title,
        }
    }
}

/// One audit line per step of a proposal: asked for, applied, turned down, run
/// out, or failed on confirm. A proposal nobody confirms still leaves a trace,
/// so an agent asking over MCP is visible even when nothing changed.
///
/// `failure` marks the line as a failed outcome and carries the reason.
pub(super) async fn audit(
    state: &SharedState,
    actor: &AuthUser,
    action: &Audited<'_>,
    audit_action: &str,
    failure: Option<&str>,
) {
    let proposed_via = mcp_agent(state, action.conversation_id).await;
    let metadata = serde_json::json!({
        "action_id": action.action_id,
        "tool": action.tool,
        "source": if proposed_via.is_some() { "mcp" } else { "assistant" },
        "proposed_via": proposed_via,
    })
    .to_string();
    // A project-scoped action has no application to name, so the audit entry
    // points at the project instead of an empty id.
    let (target_type, target_id) = if action.application_id.is_empty() {
        ("project", action.project_id)
    } else {
        ("application", action.application_id)
    };
    let _ = crate::git::repositories::audit_log::record(
        state,
        action.organization_id,
        actor,
        audit_action,
        Some(target_type),
        Some(target_id),
        Some(action.title),
        Some(&metadata),
        None,
        failure.map(|_| crate::git::repositories::audit_log::AuditOutcome::Failure),
        failure,
    )
    .await;
}

/// The token behind an MCP conversation, e.g. `MCP · Claude`; `None` for an in-app chat.
async fn mcp_agent(state: &SharedState, conversation_id: &str) -> Option<String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let title: String =
        sqlx::query_scalar("SELECT title FROM assistant_conversations WHERE id = ?1")
            .bind(conversation_id)
            .fetch_optional(&conn)
            .await
            .ok()??;
    title
        .strip_prefix(crate::mcp::session::TITLE_PREFIX)
        .map(str::to_owned)
}
