use crate::{dtos::response::AppError, state::state::SharedState};

use super::{dragonflydb, keydb, mariadb, mongodb, postgres, sqlserver};

/// The in-network connection string of an internal database resource, whatever
/// its kind. It carries the generated password, so it is for server-side use
/// (an environment variable), never for a reply a model reads.
pub async fn private_connection_string(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    template_kind: &str,
) -> Result<String, AppError> {
    let args = (
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
        "",
    );
    let connection = match template_kind {
        "postgres" => {
            postgres::resource_detail(args.0, args.1, args.2, args.3, args.4, args.5)
                .await?
                .private_connection_string
        }
        "mariadb" => {
            mariadb::resource_detail(args.0, args.1, args.2, args.3, args.4, args.5)
                .await?
                .private_connection_string
        }
        "mongodb" => {
            mongodb::resource_detail(args.0, args.1, args.2, args.3, args.4, args.5)
                .await?
                .private_connection_string
        }
        "keydb" => {
            keydb::resource_detail(args.0, args.1, args.2, args.3, args.4, args.5)
                .await?
                .private_connection_string
        }
        "dragonflydb" => {
            dragonflydb::resource_detail(args.0, args.1, args.2, args.3, args.4, args.5)
                .await?
                .private_connection_string
        }
        "sqlserver" => {
            sqlserver::resource_detail(args.0, args.1, args.2, args.3, args.4, args.5)
                .await?
                .private_connection_string
        }
        other => {
            return Err(AppError::ValidationError(format!(
                "`{other}` is not an internal database"
            )));
        }
    };
    connection.ok_or_else(|| {
        AppError::ValidationError("That database has no private connection string".into())
    })
}
