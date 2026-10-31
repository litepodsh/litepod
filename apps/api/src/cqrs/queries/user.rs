use crate::dtos::response::AppError;
use crate::dtos::user::{UserDto, UserOrganizationDto, UserWithOrgsDto};
use crate::git::repositories::{organization, organization_member, user};
use crate::state::state::SharedState;

pub async fn list_users(state: &SharedState) -> Result<Vec<UserWithOrgsDto>, AppError> {
    let rows = user::list_all_users(state)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;

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
        users.push(UserWithOrgsDto {
            id: row.id,
            email: row.email,
            name: row.name,
            created_at: row.created_at,
            disabled: row.status_id == 4,
            is_sysadmin,
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
