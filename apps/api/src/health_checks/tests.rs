use crate::{
    dtos::health_check::UpdateHealthCheckRequest,
    testing::{cleanup_database, seed_application, test_state},
};

fn valid_request() -> UpdateHealthCheckRequest {
    UpdateHealthCheckRequest {
        enabled: true,
        protocol: "http".into(),
        path: "/healthz".into(),
        port: Some(3000),
        interval_seconds: 30,
        timeout_seconds: 5,
        unhealthy_threshold: 3,
        restart_on_unhealthy: true,
    }
}

#[tokio::test]
async fn get_defaults_to_disabled_and_unknown_for_an_unconfigured_application() {
    let (state, path) = test_state().await;
    let (_, _, application_id) = seed_application(&state, "organization-a").await;

    let dto = super::get(&state, &application_id).await.unwrap();
    assert!(!dto.enabled);
    assert_eq!(dto.last_status, "unknown");
    assert_eq!(dto.consecutive_failures, 0);

    cleanup_database(&path);
}

#[tokio::test]
async fn upsert_then_get_round_trips_the_configuration() {
    let (state, path) = test_state().await;
    let (_, _, application_id) = seed_application(&state, "organization-a").await;

    let saved = super::upsert(&state, &application_id, &valid_request())
        .await
        .unwrap();
    assert!(saved.enabled);
    assert_eq!(saved.protocol, "http");
    assert_eq!(saved.path, "/healthz");
    assert_eq!(saved.port, Some(3000));
    assert!(saved.restart_on_unhealthy);

    let reloaded = super::get(&state, &application_id).await.unwrap();
    assert_eq!(reloaded.path, "/healthz");

    cleanup_database(&path);
}

#[tokio::test]
async fn upsert_rejects_an_unknown_protocol() {
    let (state, path) = test_state().await;
    let (_, _, application_id) = seed_application(&state, "organization-a").await;

    let mut req = valid_request();
    req.protocol = "udp".into();
    let error = super::upsert(&state, &application_id, &req)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Protocol must be http or tcp"));

    cleanup_database(&path);
}

#[tokio::test]
async fn upsert_rejects_an_out_of_range_interval() {
    let (state, path) = test_state().await;
    let (_, _, application_id) = seed_application(&state, "organization-a").await;

    let mut req = valid_request();
    req.interval_seconds = 2;
    let error = super::upsert(&state, &application_id, &req)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Interval must be"));

    cleanup_database(&path);
}

#[tokio::test]
async fn upsert_blanks_an_empty_path_to_root() {
    let (state, path) = test_state().await;
    let (_, _, application_id) = seed_application(&state, "organization-a").await;

    let mut req = valid_request();
    req.path = "   ".into();
    let saved = super::upsert(&state, &application_id, &req).await.unwrap();
    assert_eq!(saved.path, "/");

    cleanup_database(&path);
}
