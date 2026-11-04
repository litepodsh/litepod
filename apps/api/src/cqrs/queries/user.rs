use crate::dtos::response::AppError;
use crate::dtos::user::{UserDto, UserOrganizationDto, UserWithOrgsDto};
use crate::git::repositories::{organization, organization_member, user};
use crate::state::state::SharedState;

pub async fn list_users(state: &SharedState) -> Result<Vec<UserWithOrgsDto>, AppError> {
    let rows = user::list_all_users(state)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;
    let mut usage = user::list_usage(state)
        .await
        .map_err(AppError::DatabaseError)?;
    let mut running = user::list_running_work(state)
        .await
        .map_err(AppError::DatabaseError)?;

    let mut users = Vec::new();
    for row in rows {
        let organizations = organization::list_organizations_for_user(state, &row.id)
            .await
            .map_err(AppError::DatabaseError)?;
        let memberships = organization_member::list_memberships_for_user(state, &row.id)
            .await
            .map_err(AppError::DatabaseError)?;
        let mut by_org: std::collections::HashMap<String, (i64, String, String)> =
            std::collections::HashMap::new();
        for m in memberships {
            by_org.insert(m.organization_id, (m.role_id, m.role_name, m.joined_at));
        }
        let orgs: Vec<UserOrganizationDto> = organizations
            .into_iter()
            .map(|o| {
                let (role_id, role_name, joined_at) =
                    by_org
                        .get(&o.id)
                        .cloned()
                        .unwrap_or((0, String::new(), String::new()));
                UserOrganizationDto {
                    id: o.id,
                    name: o.name,
                    slug: o.slug,
                    is_root: o.is_root,
                    role_id,
                    role_name,
                    joined_at,
                }
            })
            .collect();
        let is_sysadmin = user::is_sysadmin(state, &row.id).await;
        let disabled = row.status_id == 4;
        let activity = running
            .remove(&row.id)
            .map(|(action, target)| (action, Some(target)))
            .or_else(|| {
                crate::users::presence::status(&row.id).map(|recent| (recent.action, recent.target))
            })
            .filter(|_| !disabled);
        let online = activity.is_some();
        let (recent_action, recent_action_target) = activity.unzip();
        let recent_action_target = recent_action_target.flatten();
        let usage = usage.remove(&row.id).unwrap_or_default();
        users.push(UserWithOrgsDto {
            id: row.id,
            email: row.email,
            name: row.name,
            created_at: row.created_at,
            disabled,
            is_sysadmin,
            online,
            recent_action,
            recent_action_target,
            last_login_at: usage.last_login_at,
            remote_servers: usage.remote_servers,
            remote_servers_ready: usage.remote_servers_ready,
            applications: usage.applications,
            applications_running: usage.applications_running,
            organizations: orgs,
        });
    }
    Ok(users)
}

pub async fn get_user(state: &SharedState, user_id: &str) -> Result<UserDto, AppError> {
    let row = user::get_user_by_id(state, user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    let is_sysadmin = user::is_sysadmin(state, &row.id).await;
    Ok(UserDto {
        id: row.id,
        email: row.email,
        name: row.name,
        is_sysadmin,
        created_at: row.created_at,
    })
}
