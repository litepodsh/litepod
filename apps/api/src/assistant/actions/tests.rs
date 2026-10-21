use uuid::Uuid;

use crate::{
    deployment::{self, compose::store::ComposeFileStore},
    state::state::SharedState,
};

use super::{apply::*, lifecycle::*, propose::*, types::*};
use crate::testing::{auth_user, cleanup_database, seed_application, test_state};

/// `test_state` already seeds `organization-a` and `organization-b`.
async fn target(state: &SharedState, organization_id: &str) -> ActionTarget {
    let (project_id, environment_id, application_id) =
        seed_application(state, organization_id).await;
    ActionTarget {
        organization_id: organization_id.into(),
        project_id,
        project_name: "project".into(),
        environment_id,
        application_name: application_id.clone(),
        application_id,
    }
}

fn arguments(name: &str, value: &str) -> SetEnvironmentVariable {
    SetEnvironmentVariable {
        name: name.into(),
        value: value.into(),
        scope: "runtime".into(),
        reason: None,
    }
}

#[test]
fn compose_diff_marks_additions_and_removals() {
    let out = compose_diff("a\nb\nc\n", "a\nB\nc\nd\n");
    // Same line stays, "b"→"B" shows a removal + insertion, "d" added.
    assert!(out.contains(" a"));
    assert!(out.lines().any(|line| line.starts_with("-b")));
    assert!(out.lines().any(|line| line.starts_with("+B")));
    assert!(out.lines().any(|line| line.starts_with("+d")));
}

#[test]
fn compose_diff_truncates_with_an_explicit_marker() {
    // More than 4 KB of churning content so the output must be cut.
    let before = "x\n".repeat(10_000);
    let after = "y\n".repeat(10_000);
    let out = compose_diff(&before, &after);
    assert!(
        out.len() <= DIFF_PREVIEW_BYTES + 64,
        "diff blew past the cap: {} bytes",
        out.len()
    );
    assert!(
        out.contains("diff truncated"),
        "the cap must say so instead of silently dropping lines"
    );
}

async fn conversation(state: &SharedState, organization_id: &str) -> String {
    let id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO assistant_conversations (id, organization_id, user_id, title, created_at, updated_at) VALUES (?1, ?2, 'editor', 'test', 0, 0)",
    )
    .bind(&id)
    .bind(organization_id)
    .execute(&state.db)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn proposing_writes_nothing_until_the_change_is_confirmed() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("SENTRY_DSN", "https://key@sentry.example/1"),
    )
    .await
    .unwrap();
    assert_eq!(action.status, "pending");

    let before = deployment::environment(&state, &target.application_id, "runtime", false)
        .await
        .unwrap();
    assert!(
        before.is_empty(),
        "proposing must not touch the application"
    );

    let applied = confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();
    assert_eq!(applied.status, "applied");

    let after = deployment::environment(&state, &target.application_id, "runtime", true)
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].name, "SENTRY_DSN");
    assert_eq!(after[0].value, "https://key@sentry.example/1");

    cleanup_database(&path);
}

#[tokio::test]
async fn the_card_never_carries_the_secret() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;

    let action = propose_environment_variable(
        &state,
        &chat,
        &auth_user("editor"),
        &target,
        arguments("API_TOKEN", "sk-live-9f2b41"),
    )
    .await
    .unwrap();

    let added = action.added.unwrap();
    assert!(added.starts_with("API_TOKEN=sk-l"), "got {added}");
    assert!(
        !added.contains("9f2b41"),
        "the card leaked the value: {added}"
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn an_action_can_only_be_settled_once() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    assert!(
        confirm(&state, "organization-a", &actor, &action.id)
            .await
            .is_err(),
        "a confirmed change must not be replayable"
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_change_proposed_in_one_persons_thread_is_not_another_persons_to_settle() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let owner = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &owner,
        &target,
        arguments("SENTRY_DSN", "https://key@sentry.example/1"),
    )
    .await
    .unwrap();

    // A colleague with the same role in the same organization. The route's
    // extractor lets them in; the conversation the change came out of is
    // still not theirs to read or to act on.
    let colleague = auth_user("other-editor");
    assert!(
        confirm(&state, "organization-a", &colleague, &action.id)
            .await
            .is_err(),
        "another member must not apply a change from a private thread"
    );
    assert!(
        reject(&state, "organization-a", &colleague, &action.id)
            .await
            .is_err(),
        "nor settle it as rejected"
    );

    // Untouched, and still the owner's to decide.
    let applied = confirm(&state, "organization-a", &owner, &action.id)
        .await
        .unwrap();
    assert_eq!(applied.status, "applied");

    cleanup_database(&path);
}

#[tokio::test]
async fn rejecting_leaves_the_application_alone() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();
    assert_eq!(
        reject(&state, "organization-a", &actor, &action.id)
            .await
            .unwrap()
            .status,
        "rejected"
    );
    assert!(
        deployment::environment(&state, &target.application_id, "runtime", false)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        confirm(&state, "organization-a", &actor, &action.id)
            .await
            .is_err()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_proposal_is_audited_even_when_nobody_applies_it() {
    let (state, path) = test_state().await;
    sqlx::query("UPDATE organizations SET audit_logs_enabled = 1")
        .execute(&state.db)
        .await
        .unwrap();
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();
    reject(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let logged: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, metadata FROM audit_logs WHERE organization_id = 'organization-a' AND action LIKE 'assistant.%' ORDER BY rowid",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    let actions: Vec<&str> = logged.iter().map(|(action, _)| action.as_str()).collect();
    assert_eq!(
        actions,
        ["assistant.action_proposed", "assistant.action_rejected"]
    );
    assert!(logged[0].1.contains(&action.id));

    cleanup_database(&path);
}

#[tokio::test]
async fn an_action_from_another_organization_is_not_reachable() {
    let (state, path) = test_state().await;
    let theirs = target(&state, "organization-b").await;
    let chat = conversation(&state, "organization-b").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &theirs,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();

    // The extractor proved the caller may edit organization-a; that must not
    // carry over to an action id belonging to organization-b.
    assert!(
        confirm(&state, "organization-a", &actor, &action.id)
            .await
            .is_err()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn an_expired_action_is_refused_rather_than_applied() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE assistant_actions SET expires_at = 1 WHERE id = ?1")
        .bind(&action.id)
        .execute(&state.db)
        .await
        .unwrap();

    assert!(
        confirm(&state, "organization-a", &actor, &action.id)
            .await
            .is_err()
    );
    assert!(
        deployment::environment(&state, &target.application_id, "runtime", false)
            .await
            .unwrap()
            .is_empty()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_name_a_container_would_reject_never_becomes_an_action() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    for name in ["lower_case", "9LEADING", "HAS-DASH", ""] {
        assert!(
            propose_environment_variable(&state, &chat, &actor, &target, arguments(name, "x"))
                .await
                .is_err(),
            "`{name}` should not be proposable"
        );
    }

    cleanup_database(&path);
}

#[tokio::test]
async fn removing_a_variable_waits_for_the_confirm_too() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let added = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();
    confirm(&state, "organization-a", &actor, &added.id)
        .await
        .unwrap();

    let removal = propose_delete_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", ""),
    )
    .await
    .unwrap();
    assert_eq!(removal.removed.as_deref(), Some("LOG_LEVEL"));

    // Still there: proposing is not removing.
    assert_eq!(
        deployment::environment(&state, &target.application_id, "runtime", false)
            .await
            .unwrap()
            .len(),
        1
    );

    confirm(&state, "organization-a", &actor, &removal.id)
        .await
        .unwrap();
    assert!(
        deployment::environment(&state, &target.application_id, "runtime", false)
            .await
            .unwrap()
            .is_empty()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn removing_something_that_is_not_there_never_becomes_a_card() {
    // A card for a variable the application does not set is a card the user
    // cannot make sense of.
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;

    assert!(
        propose_delete_environment_variable(
            &state,
            &chat,
            &auth_user("editor"),
            &target,
            arguments("NOT_SET", ""),
        )
        .await
        .is_err()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_repository_with_no_branch_gets_main() {
    // Left to the deploy step, an unset branch is whatever it guesses. The
    // card has to state it, so it is filled in at proposal time.
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;

    let action = propose_source(
        &state,
        &chat,
        &auth_user("editor"),
        &target,
        SetSource {
            git_url: Some("https://example.com/team/app.git".into()),
            railpack_project_path: Some(".".into()),
            container_port: Some(3000),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(
        action.added.as_deref().unwrap().contains("branch = main"),
        "{:?}",
        action.added
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_source_change_leaves_the_fields_it_did_not_mention_alone() {
    // `source::save` takes the whole document. A proposal about the branch
    // must not reset the build method on the way through.
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    // A freshly seeded application defaults to `git` but has no repository
    // configured yet; `SetSource` has no `dockerfile_path`/`container_port`
    // fields at all (the assistant only ever tweaks an already-configured
    // git app), so establish a real git baseline the way the source tab
    // would, directly through `source::save`, before proposing the
    // branch-only tweak below.
    deployment::source::save(
        &state,
        &target.application_id,
        crate::dtos::deployment::UpdateSourceRequest {
            deploy_type: "git".into(),
            build_method: "docker".into(),
            git_provider_id: None,
            git_url: Some("https://example.com/team/app.git".into()),
            git_branch: Some("main".into()),
            auto_deploy: None,
            watch_paths: None,
            dockerfile_path: Some("./Dockerfile".into()),
            compose_file_path: None,
            compose_public_service: None,
            container_port: Some(3000),
            railpack_project_path: None,
            railpack_start_command: None,
            build_cpu_cores: None,
            build_memory_mib: None,
            build_swap_mib: None,
            image_url: None,
            registry_username: None,
            registry_password: None,
            restart_policy: None,
        },
    )
    .await
    .unwrap();

    let before = deployment::source::get(&state, &target.application_id)
        .await
        .unwrap();

    let action = propose_source(
        &state,
        &chat,
        &actor,
        &target,
        SetSource {
            git_branch: Some("release".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let after = deployment::source::get(&state, &target.application_id)
        .await
        .unwrap();
    assert_eq!(after.git_branch.as_deref(), Some("release"));
    assert_eq!(after.deploy_type, before.deploy_type);
    assert_eq!(after.build_method, before.build_method);
    assert_eq!(after.railpack_project_path, before.railpack_project_path);

    cleanup_database(&path);
}

#[tokio::test]
async fn a_change_that_changes_nothing_never_becomes_a_card() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;

    assert!(
        propose_update_application(
            &state,
            &chat,
            &auth_user("editor"),
            &target,
            UpdateApplication {
                name: None,
                description: None,
                reason: None,
            },
        )
        .await
        .is_err()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn renaming_waits_for_the_confirm() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_update_application(
        &state,
        &chat,
        &actor,
        &target,
        UpdateApplication {
            name: Some("renamed".into()),
            description: None,
            reason: None,
        },
    )
    .await
    .unwrap();

    let unchanged: String = sqlx::query_scalar("SELECT name FROM applications WHERE id = ?1")
        .bind(&target.application_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_ne!(unchanged, "renamed", "proposing must not rename");

    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let renamed: String = sqlx::query_scalar("SELECT name FROM applications WHERE id = ?1")
        .bind(&target.application_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(renamed, "renamed");

    cleanup_database(&path);
}

#[tokio::test]
async fn the_deploy_trigger_is_one_the_schema_accepts() {
    // `triggered_by` is a CHECK list. A value the schema does not know kills
    // a confirmed redeploy with a constraint error, which is how this first
    // shipped broken.
    let (state, path) = test_state().await;
    let (_, _, application_id) = seed_application(&state, "organization-a").await;

    let inserted = sqlx::query(
        "INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by) VALUES ('run-trigger', ?1, 'git', 'railpack', 'building', 0, ?2)",
    )
    .bind(&application_id)
    .bind(ASSISTANT_TRIGGER)
    .execute(&state.db)
    .await;
    assert!(
        inserted.is_ok(),
        "`{ASSISTANT_TRIGGER}` is not in the triggered_by CHECK list: {inserted:?}"
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn settling_tells_the_model_without_telling_the_reader() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_environment_variable(
        &state,
        &chat,
        &actor,
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let notes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM assistant_messages WHERE conversation_id = ?1 AND role = 'note'",
    )
    .bind(&chat)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(notes, 1, "the model was not told what the user decided");

    // The note is replay material: it must not appear in the transcript.
    let transcript = crate::assistant::conversation::read(&state, "organization-a", &actor, &chat)
        .await
        .unwrap();
    assert!(
        transcript
            .messages
            .iter()
            .all(|message| message.role != "note"),
        "a system note leaked into the transcript"
    );

    cleanup_database(&path);
}

#[test]
fn route_is_a_ui_path_not_an_api_path() {
    // The API path for the same application starts `/api/v1/organizations/…`.
    // Handing that to the browser is a 404, which is exactly what happened
    // the first time this shipped.
    let target = ActionTarget {
        organization_id: "org".into(),
        project_id: "proj".into(),
        project_name: "Project".into(),
        environment_id: "env".into(),
        application_id: "app".into(),
        application_name: "static".into(),
    };
    assert_eq!(
        target.route(SET_ENVIRONMENT_VARIABLE),
        "/projects/proj/environments/env/applications/app?tab=environment"
    );
    // A domain change happens on another tab, and a project change is not
    // on an application page at all.
    assert_eq!(
        target.route(ADD_DOMAIN),
        "/projects/proj/environments/env/applications/app?tab=domains"
    );
    // The project tools land on the list: the panel navigates before it
    // applies, so a create has no page yet and a delete's is about to go.
    assert_eq!(target.route(UPDATE_PROJECT), "/projects");
    assert_eq!(target.route(DELETE_PROJECT), "/projects");
    assert_eq!(target.route(CREATE_PROJECT), "/projects");
}

#[tokio::test]
async fn the_route_points_at_the_application_the_change_touches() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;

    let action = propose_environment_variable(
        &state,
        &chat,
        &auth_user("editor"),
        &target,
        arguments("LOG_LEVEL", "debug"),
    )
    .await
    .unwrap();

    assert!(action.route.starts_with("/projects/"));
    assert!(action.route.contains(&target.application_id));
    assert!(action.route.ends_with("?tab=environment"));

    cleanup_database(&path);
}

#[tokio::test]
async fn a_project_action_stores_no_application_and_still_applies() {
    // The whole point of the nullable `application_id`: creating a project
    // has no application to reference, and the foreign key would refuse an
    // empty string.
    let (state, path) = test_state().await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let target = ActionTarget::project("organization-a", "", "");

    let action = propose_create_project(
        &state,
        &chat,
        &actor,
        &target,
        ProjectFields {
            name: Some("  Billing  ".into()),
            description: None,
            reason: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(action.route, "/projects");
    assert!(action.application_id.is_empty());

    let before = crate::cqrs::queries::project::list_projects(&state, "organization-a")
        .await
        .unwrap();
    assert!(
        !before.iter().any(|project| project.name == "Billing"),
        "proposing must not create the project"
    );

    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let after = crate::cqrs::queries::project::list_projects(&state, "organization-a")
        .await
        .unwrap();
    assert!(
        after.iter().any(|project| project.name == "Billing"),
        "the name is trimmed, not stored with its spaces"
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn deleting_a_project_takes_its_applications_with_it() {
    let (state, path) = test_state().await;
    let seeded = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let project = ActionTarget::project("organization-a", &seeded.project_id, "seeded");

    let action = propose_delete_project(&state, &chat, &actor, &project, ProjectFields::default())
        .await
        .unwrap();
    assert!(
        action.summary.contains("cannot be undone"),
        "the card has to say so: {}",
        action.summary
    );
    assert!(
        crate::cqrs::queries::project::get_project(&state, "organization-a", &seeded.project_id)
            .await
            .is_ok(),
        "proposing must not delete anything"
    );

    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();
    assert!(
        crate::cqrs::queries::project::get_project(&state, "organization-a", &seeded.project_id)
            .await
            .is_err()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_domain_is_proposed_then_added_then_removed() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let add = propose_add_domain(
        &state,
        &chat,
        &actor,
        &target,
        DomainFields {
            host: Some("LOCALHOST".into()),
            port: Some(18_432),
            ..DomainFields::default()
        },
    )
    .await
    .unwrap();
    assert!(add.route.ends_with("?tab=domains"));
    assert!(
        deployment::domains::list(&state, &target.application_id)
            .await
            .unwrap()
            .is_empty(),
        "proposing must not route anything"
    );

    confirm(&state, "organization-a", &actor, &add.id)
        .await
        .unwrap();
    let domains = deployment::domains::list(&state, &target.application_id)
        .await
        .unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].host, "localhost");
    assert!(domains[0].is_primary, "the first domain is the primary one");

    // Named by host, not by id: that is what the model has after `get domains`.
    let remove = propose_delete_domain(
        &state,
        &chat,
        &actor,
        &target,
        DomainFields {
            domain_id: Some("localhost".into()),
            ..DomainFields::default()
        },
    )
    .await
    .unwrap();
    confirm(&state, "organization-a", &actor, &remove.id)
        .await
        .unwrap();
    assert!(
        deployment::domains::list(&state, &target.application_id)
            .await
            .unwrap()
            .is_empty()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_compose_application_is_created_whole_or_not_at_all() {
    let (state, path) = test_state().await;
    let seeded = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let mut into = ActionTarget::project("organization-a", &seeded.project_id, "seeded");
    into.environment_id = seeded.environment_id.clone();

    let compose = "services:\n  uptime-kuma:\n    image: louislam/uptime-kuma:1\n    ports:\n      - \"3001\"\n";
    let arguments = CreateApplication {
        name: "uptime-kuma".into(),
        server_id: "local".into(),
        compose: Some(compose.into()),
        port: Some(3001),
        ..CreateApplication::default()
    };

    // A port no service publishes is refused while it is still a proposal.
    assert!(
        propose_create_application(
            &state,
            &chat,
            &actor,
            &into,
            CreateApplication {
                port: Some(9999),
                ..arguments.clone()
            },
        )
        .await
        .is_err()
    );

    let action = propose_create_application(&state, &chat, &actor, &into, arguments)
        .await
        .unwrap();
    assert!(action.application_id.is_empty(), "it does not exist yet");

    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let detail =
        crate::cqrs::queries::project::get_project(&state, "organization-a", &seeded.project_id)
            .await
            .unwrap();
    let created = detail
        .environments
        .iter()
        .flat_map(|environment| environment.applications.iter())
        .find(|application| application.name == "uptime-kuma")
        .expect("the application was created");

    // Created *and* wired up: an application with no compose file and no
    // source is the half-created thing this action exists to avoid.
    let source = deployment::source::get(&state, &created.id).await.unwrap();
    assert_eq!(source.deploy_type, "compose");
    assert_eq!(
        source.compose_public_service.as_deref(),
        Some("uptime-kuma")
    );
    assert!(
        ComposeFileStore::new(&state.data_dir, &created.id)
            .read_content()
            .await
            .unwrap()
            .contains("louislam/uptime-kuma")
    );

    // An edit cannot bring a host path in either: rootless Podman cannot
    // write to a directory it does not own.
    let mut created_target = seeded.clone();
    created_target.application_id = created.id.clone();
    created_target.application_name = created.name.clone();
    let error = propose_compose_file(
        &state,
        &chat,
        &actor,
        &created_target,
        SetComposeFile {
            compose: format!("{compose}    volumes:\n      - ./data:/app/data\n"),
            reason: None,
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("named volumes"), "{error}");

    cleanup_database(&path);
}

#[tokio::test]
async fn a_static_site_waits_for_its_upload_then_comes_up_whole() {
    let (state, path) = test_state().await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let target = ActionTarget::project("organization-a", "", "");

    let (action, token) = propose_static_site(
        &state,
        &chat,
        &actor,
        &target,
        StaticSite {
            project_name: "landing".into(),
            name: "site".into(),
            server_id: "local".into(),
            host: Some("site.example.com".into()),
            ..StaticSite::default()
        },
    )
    .await
    .unwrap();

    // Confirming before the upload creates nothing and leaves it pending.
    assert!(confirm(&state, "organization-a", &actor, &action.id).await.is_err());
    sqlx::query("UPDATE assistant_actions SET status = 'pending', failure = NULL WHERE id = ?1")
        .bind(&action.id)
        .execute(&state.db)
        .await
        .unwrap();
    let projects = crate::cqrs::queries::project::list_projects(&state, "organization-a")
        .await
        .unwrap();
    assert!(!projects.iter().any(|project| project.name == "landing"));

    let html = b"<!DOCTYPE html><html><body>hi</body></html>";
    assert!(
        super::stage_upload(&state, &action.id, "wrong-token", html)
            .await
            .is_err()
    );
    super::stage_upload(&state, &action.id, &token, html)
        .await
        .unwrap();

    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let project = crate::cqrs::queries::project::list_projects(&state, "organization-a")
        .await
        .unwrap()
        .into_iter()
        .find(|project| project.name == "landing")
        .expect("the project was created");
    let detail = crate::cqrs::queries::project::get_project(&state, "organization-a", &project.id)
        .await
        .unwrap();
    let site = detail
        .environments
        .iter()
        .flat_map(|environment| environment.applications.iter())
        .find(|application| application.name == "site")
        .expect("the application was created");
    assert_eq!(
        deployment::source::get(&state, &site.id).await.unwrap().deploy_type,
        "static"
    );
    let domains = deployment::domains::list(&state, &site.id).await.unwrap();
    assert_eq!(domains[0].host, "site.example.com");
    assert_eq!(domains[0].port, STATIC_PORT);
    // The token dies with the proposal.
    assert!(
        super::stage_upload(&state, &action.id, &token, html)
            .await
            .is_err()
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_static_site_goes_into_an_empty_application_of_that_name() {
    let (state, path) = test_state().await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let app = target(&state, "organization-a").await;
    // The seeded ssh server is not trusted yet; the local one deploys.
    sqlx::query("UPDATE applications SET server_id = 'local' WHERE id = ?1")
        .bind(&app.application_id)
        .execute(&state.db)
        .await
        .unwrap();
    let project = ActionTarget {
        application_id: String::new(),
        application_name: String::new(),
        ..app.clone()
    };
    let site = || StaticSite {
        project_name: app.project_id.clone(),
        name: app.application_name.clone(),
        server_id: "local".into(),
        host: Some("site.example.com".into()),
        ..StaticSite::default()
    };

    // Never deployed and no repository: it is reused, not refused by name.
    let (action, token) = propose_static_site(&state, &chat, &actor, &project, site())
        .await
        .unwrap();
    super::stage_upload(&state, &action.id, &token, b"<!DOCTYPE html><html></html>")
        .await
        .unwrap();
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    assert_eq!(
        deployment::source::get(&state, &app.application_id)
            .await
            .unwrap()
            .deploy_type,
        "static"
    );
    let domains = deployment::domains::list(&state, &app.application_id)
        .await
        .unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].port, STATIC_PORT);

    let (filename, bytes, _) = deployment::download_artifact(&state, &app.application_id)
        .await
        .unwrap();
    assert!(filename.ends_with(".html"));
    assert_eq!(bytes, b"<!DOCTYPE html><html></html>");

    // Asking again replaces the site and keeps the one domain.
    let (again, token) = propose_static_site(&state, &chat, &actor, &project, site())
        .await
        .unwrap();
    super::stage_upload(&state, &again.id, &token, b"<!DOCTYPE html><html>2</html>")
        .await
        .unwrap();
    confirm(&state, "organization-a", &actor, &again.id)
        .await
        .unwrap();
    assert_eq!(
        deployment::domains::list(&state, &app.application_id)
            .await
            .unwrap()
            .len(),
        1
    );

    cleanup_database(&path);
}

#[tokio::test]
async fn a_static_site_never_overwrites_a_configured_application() {
    let (state, path) = test_state().await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let app = target(&state, "organization-a").await;
    let current = deployment::source::get(&state, &app.application_id)
        .await
        .unwrap();
    deployment::source::save(
        &state,
        &app.application_id,
        crate::dtos::deployment::UpdateSourceRequest {
            deploy_type: "git".into(),
            build_method: current.build_method,
            git_provider_id: None,
            git_url: Some("https://github.com/example/site.git".into()),
            git_branch: Some("main".into()),
            auto_deploy: Some(false),
            watch_paths: None,
            dockerfile_path: Some("Dockerfile".into()),
            compose_file_path: None,
            compose_public_service: None,
            container_port: Some(80),
            railpack_project_path: Some(".".into()),
            railpack_start_command: None,
            build_cpu_cores: None,
            build_memory_mib: None,
            build_swap_mib: None,
            image_url: None,
            registry_username: None,
            registry_password: None,
            restart_policy: None,
        },
    )
    .await
    .unwrap();
    let project = ActionTarget {
        application_id: String::new(),
        application_name: String::new(),
        ..app.clone()
    };

    let refused = propose_static_site(
        &state,
        &chat,
        &actor,
        &project,
        StaticSite {
            project_name: app.project_id.clone(),
            name: app.application_name.clone(),
            server_id: "local".into(),
            ..StaticSite::default()
        },
    )
    .await;
    assert!(refused.is_err());

    // A compose file on a git application would be stored and never used.
    let compose = propose_compose_file(
        &state,
        &chat,
        &actor,
        &app,
        SetComposeFile {
            compose: "services:\n  web:\n    image: nginx\n".into(),
            reason: None,
        },
    )
    .await;
    assert!(compose.unwrap_err().to_string().contains("never use a compose file"));

    cleanup_database(&path);
}

#[tokio::test]
async fn a_database_template_lands_in_a_new_project_with_one_confirmation() {
    let (state, path) = test_state().await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let target = ActionTarget::project("organization-a", "", "");
    let arguments = FromTemplate {
        project_name: "data".into(),
        template: "postgres".into(),
        name: Some("main-db".into()),
        server_id: "local".into(),
        ..FromTemplate::default()
    };

    // A database generates its own credentials: values are refused, not ignored.
    let mut with_values = arguments.clone();
    with_values.values.insert("POSTGRES_PASSWORD".into(), "x".into());
    assert!(
        propose_from_template(&state, &chat, &actor, &target, with_values)
            .await
            .is_err()
    );

    let (action, unset) = propose_from_template(&state, &chat, &actor, &target, arguments)
        .await
        .unwrap();
    assert!(unset.is_empty());
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let project = crate::cqrs::queries::project::list_projects(&state, "organization-a")
        .await
        .unwrap()
        .into_iter()
        .find(|project| project.name == "data")
        .expect("the project was created");
    let detail = crate::cqrs::queries::project::get_project(&state, "organization-a", &project.id)
        .await
        .unwrap();
    let database = detail
        .environments
        .iter()
        .flat_map(|environment| environment.applications.iter())
        .find(|application| application.name == "main-db")
        .expect("the database was created");
    assert_eq!(database.template_kind.as_deref(), Some("postgres"));

    cleanup_database(&path);
}

#[tokio::test]
async fn a_compose_app_is_created_like_a_catalog_template() {
    let (state, path) = test_state().await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let target = ActionTarget::project("organization-a", "", "");
    let compose = "services:\n  kuma:\n    image: docker.io/louislam/uptime-kuma:1\n    restart: unless-stopped\n    expose:\n      - \"3001\"\n    environment:\n      ADMIN_PASSWORD: ${ADMIN_PASSWORD}\n      PUBLIC_URL: ${PUBLIC_URL}\n    volumes:\n      - kuma_data:/app/data\n\nvolumes:\n  kuma_data:\n";
    let arguments = ComposeApp {
        project_name: "monitoring".into(),
        name: "kuma".into(),
        server_id: "local".into(),
        compose: compose.into(),
        env: "# login\nADMIN_PASSWORD={{password_24}}\nPUBLIC_URL={{url}}\n".into(),
        port: Some(3001),
        ..ComposeApp::default()
    };

    // A host path is refused while it is still a proposal.
    let bound = compose.replace("kuma_data:/app/data", "./data:/app/data");
    let error = propose_deploy_compose(
        &state,
        &chat,
        &actor,
        &target,
        ComposeApp {
            compose: bound,
            ..arguments.clone()
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("named volumes"), "{error}");

    let action = propose_deploy_compose(&state, &chat, &actor, &target, arguments)
        .await
        .unwrap();
    assert!(
        !action.added.as_deref().unwrap_or_default().contains("{{"),
        "the card names variables, never values"
    );
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let project = crate::cqrs::queries::project::list_projects(&state, "organization-a")
        .await
        .unwrap()
        .into_iter()
        .find(|project| project.name == "monitoring")
        .expect("the project was created");
    let detail = crate::cqrs::queries::project::get_project(&state, "organization-a", &project.id)
        .await
        .unwrap();
    let created = detail
        .environments
        .iter()
        .flat_map(|environment| environment.applications.iter())
        .find(|application| application.name == "kuma")
        .expect("the application keeps the name it was given");

    let source = deployment::source::get(&state, &created.id).await.unwrap();
    assert_eq!(source.deploy_type, "compose");
    assert_eq!(source.compose_public_service.as_deref(), Some("kuma"));
    let env = deployment::decrypted_environment(&state, &created.id, "runtime")
        .await
        .unwrap();
    let password = &env.iter().find(|(name, _)| name == "ADMIN_PASSWORD").unwrap().1;
    assert_eq!(password.len(), 24, "the secret marker was generated");
    // A domain marker is kept as written and resolved on each deploy.
    assert!(env.iter().any(|(name, value)| name == "PUBLIC_URL" && value == "{{url}}"));
    let ports = crate::git::repositories::project::get_catalog_ports(&state, &created.id)
        .await
        .unwrap();
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].container, 3001);

    cleanup_database(&path);
}

#[test]
fn every_stored_proposal_is_an_allowed_action() {
    for tool in [
        SET_ENVIRONMENT_VARIABLE,
        DELETE_ENVIRONMENT_VARIABLE,
        SET_COMPOSE_FILE,
        UPDATE_APPLICATION,
        SET_SOURCE,
        CREATE_APPLICATION,
        CREATE_PROJECT,
        UPDATE_PROJECT,
        DELETE_PROJECT,
        ADD_DOMAIN,
        UPDATE_DOMAIN,
        DELETE_DOMAIN,
        DEPLOY_STATIC_SITE,
        CREATE_FROM_TEMPLATE,
        DEPLOY_COMPOSE,
        REDEPLOY_APPLICATION,
        START_APPLICATION,
        STOP_APPLICATION,
    ] {
        assert!(
            crate::assistant::capabilities::action_allowed(exec_action(tool)),
            "{tool} is not in the assistant's allowlist"
        );
    }
}

#[tokio::test]
async fn a_stop_is_stored_and_nothing_stops() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let conversation = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_lifecycle(
        &state,
        &conversation,
        &actor,
        &target,
        STOP_APPLICATION,
        Some("it is idle".into()),
    )
    .await
    .unwrap();

    assert_eq!(action.status, "pending");
    assert_eq!(action.tool, STOP_APPLICATION);
    assert!(action.title.starts_with("Stop "), "{}", action.title);
    assert!(action.summary.contains("it is idle"), "{}", action.summary);
    cleanup_database(&path);
}

#[tokio::test]
async fn only_start_and_stop_are_lifecycle_proposals() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let conversation = conversation(&state, "organization-a").await;
    let refused = propose_lifecycle(
        &state,
        &conversation,
        &auth_user("editor"),
        &target,
        DELETE_PROJECT,
        None,
    )
    .await;
    assert!(refused.is_err());
    cleanup_database(&path);
}

#[test]
fn an_html_fragment_is_refused_with_its_opening_tag() {
    let error = crate::deployment::file_kind(
        "loader.html",
        b"<div class=\"loader\">\n  <svg width=\"100\"></svg>\n</div>",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("loader.html starts with `<div class=\"loader\">`"));
    assert!(error.contains("<!DOCTYPE html><html><body>"));
    assert_eq!(
        crate::deployment::file_kind("toggle.html", b"<!-- from uiverse -->\n<html></html>").unwrap(),
        "html"
    );
}

#[test]
fn an_inline_site_is_zipped_and_checked() {
    let files = serde_json::json!({ "files": { "index.html": "<!doctype html><p>hi</p>", "css/a.css": "p{}" } });
    let bytes = super::inline_site(&files).unwrap().unwrap();
    assert_eq!(crate::deployment::file_kind("site", &bytes).unwrap(), "zip");

    assert!(super::inline_site(&serde_json::json!({})).unwrap().is_none());
    assert!(super::inline_site(&serde_json::json!({ "files": { "a.html": "x" } })).is_err());
    assert!(
        super::inline_site(&serde_json::json!({ "files": { "index.html": "x", "../up.html": "x" } }))
            .is_err()
    );
    assert!(super::inline_site(&serde_json::json!({ "zip_base64": "not base64!" })).is_err());
}

#[tokio::test]
async fn a_new_repository_asks_where_it_builds_from() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");
    let repository = || SetSource {
        git_url: Some("https://github.com/team/app.git".into()),
        container_port: Some(3000),
        ..Default::default()
    };
    let refused = |arguments| async {
        propose_source(&state, &chat, &actor, &target, arguments)
            .await
            .unwrap_err()
            .to_string()
    };

    // No build method means Railpack, and Railpack needs its folder.
    assert!(refused(repository()).await.contains("railpack_project_path"));
    assert!(
        refused(SetSource {
            build_method: Some("docker".into()),
            ..repository()
        })
        .await
        .contains("dockerfile_path")
    );
    assert!(
        refused(SetSource {
            build_method: Some("compose".into()),
            compose_file_path: Some("./compose.yml".into()),
            ..repository()
        })
        .await
        .contains("compose_public_service")
    );
    assert!(
        refused(SetSource {
            railpack_project_path: Some("../outside".into()),
            ..repository()
        })
        .await
        .contains("inside the repository")
    );

    let action = propose_source(
        &state,
        &chat,
        &actor,
        &target,
        SetSource {
            build_method: Some("compose".into()),
            compose_file_path: Some("deploy/compose.yml".into()),
            compose_public_service: Some("web".into()),
            ..repository()
        },
    )
    .await
    .unwrap();
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();
    let source = deployment::source::get(&state, &target.application_id)
        .await
        .unwrap();
    assert_eq!(source.build_method, "compose");
    assert_eq!(source.compose_file_path.as_deref(), Some("deploy/compose.yml"));
    assert_eq!(source.compose_public_service.as_deref(), Some("web"));

    cleanup_database(&path);
}

#[tokio::test]
async fn a_public_repository_becomes_a_railpack_git_deploy() {
    let (state, path) = test_state().await;
    let target = target(&state, "organization-a").await;
    let chat = conversation(&state, "organization-a").await;
    let actor = auth_user("editor");

    let action = propose_source(
        &state,
        &chat,
        &actor,
        &target,
        SetSource {
            git_url: Some("https://github.com/team/app.git".into()),
            build_method: Some("railpack".into()),
            railpack_project_path: Some("apps/web".into()),
            container_port: Some(3000),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let added = action.added.as_deref().unwrap();
    assert!(added.contains("build = railpack"), "{added}");
    assert!(added.contains("port = 3000"), "{added}");
    confirm(&state, "organization-a", &actor, &action.id)
        .await
        .unwrap();

    let source = deployment::source::get(&state, &target.application_id)
        .await
        .unwrap();
    assert_eq!(source.deploy_type, "git");
    assert_eq!(source.build_method, "railpack");
    assert_eq!(source.container_port, Some(3000));
    assert_eq!(source.git_branch.as_deref(), Some("main"));
    assert_eq!(source.railpack_project_path, "apps/web");

    assert!(
        propose_source(
            &state,
            &chat,
            &actor,
            &target,
            SetSource {
                build_method: Some("nixpacks".into()),
                ..Default::default()
            },
        )
        .await
        .is_err()
    );

    cleanup_database(&path);
}
