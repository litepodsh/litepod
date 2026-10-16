use sqlx::Row;

use crate::state::state::SharedState;

pub struct RoleRow {
    pub id: i64,
    #[allow(dead_code)]
    pub name: String,
}

pub async fn get_organization_member_role(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
) -> Option<RoleRow> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT r.id, r.name FROM organization_members tm JOIN roles r ON r.id = tm.role_id WHERE tm.organization_id = ?1 AND tm.user_id = ?2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&conn)
    .await
    .unwrap();

    row.map(|row| RoleRow {
        id: row.try_get(0).unwrap(),
        name: row.try_get(1).unwrap(),
    })
}
