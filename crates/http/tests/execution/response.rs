use std::time::Duration;

use probe_core::RequestSettings;
use probe_http::{
    ExecutionOptions, HttpEngine, HttpError, MAX_IN_MEMORY_RESPONSE_BYTES, ResponseCache,
};

use super::support::{delayed_server, request, serve_once, temporary_path};

#[tokio::test]
async fn reports_timeout_and_cancellation_separately() {
    let (timeout_url, timeout_server) = delayed_server().await;
    let mut timed = request("GET", timeout_url);
    timed.settings = RequestSettings {
        timeout: Some(Duration::from_millis(20)),
        ..RequestSettings::default()
    };
    let error = HttpEngine::new()
        .unwrap()
        .execute(&timed, &ExecutionOptions::default())
        .await
        .unwrap_err();
    assert_eq!(error, HttpError::Timeout);
    timeout_server.abort();

    let (cancel_url, cancel_server) = delayed_server().await;
    let error = HttpEngine::new()
        .unwrap()
        .execute_cancellable(
            &request("GET", cancel_url),
            &ExecutionOptions::default(),
            tokio::time::sleep(Duration::from_millis(20)),
        )
        .await
        .unwrap_err();
    assert_eq!(error, HttpError::Cancelled);
    cancel_server.abort();
}

#[tokio::test]
async fn bounds_in_memory_responses_and_streams_file_output() {
    let body = vec![b'x'; MAX_IN_MEMORY_RESPONSE_BYTES + 32 * 1024];
    let spool_directory = temporary_path("response-spool");
    let response_cache = ResponseCache::new(
        spool_directory.clone(),
        MAX_IN_MEMORY_RESPONSE_BYTES as u64 * 2,
    );
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let response = HttpEngine::new()
        .unwrap()
        .execute(
            &request("GET", format!("{base_url}/bounded")),
            &ExecutionOptions {
                response_cache: Some(response_cache),
                ..ExecutionOptions::default()
            },
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert_eq!(response.size, body.len());
    assert_eq!(response.body, body[..MAX_IN_MEMORY_RESPONSE_BYTES]);
    assert!(!response.body_complete);
    assert!(response.body_retention_error.is_none());
    let body_file = response
        .body_file
        .as_ref()
        .expect("large body should spool");
    let spool_path = body_file.path().to_owned();
    assert_eq!(std::fs::read(&spool_path).unwrap(), body);
    let response_clone = response.clone();
    drop(response);
    assert!(spool_path.exists(), "a clone must retain the spool file");
    drop(response_clone);
    assert!(
        !spool_path.exists(),
        "the final owner must remove the spool file"
    );
    std::fs::remove_dir_all(spool_directory).unwrap();

    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let response = HttpEngine::new()
        .unwrap()
        .execute(
            &request("GET", format!("{base_url}/bounded-without-retention")),
            &ExecutionOptions::default(),
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert_eq!(response.size, body.len());
    assert_eq!(response.body, body[..MAX_IN_MEMORY_RESPONSE_BYTES]);
    assert!(!response.body_complete);
    assert!(response.body_file.is_none());
    assert!(response.body_retention_error.is_none());

    let output = temporary_path("streamed-response.bin");
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let response = HttpEngine::new()
        .unwrap()
        .execute_to_file(
            &request("GET", format!("{base_url}/file")),
            &ExecutionOptions::default(),
            &output,
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert_eq!(response.size, body.len());
    assert!(response.body.is_empty());
    assert!(!response.body_complete);
    assert!(response.body_file.is_none());
    assert!(response.body_retention_error.is_none());
    assert_eq!(std::fs::read(&output).unwrap(), body);
    std::fs::remove_file(output).unwrap();
}

#[tokio::test]
async fn response_cache_enforces_the_global_quota_and_recovers_orphaned_sessions() {
    let body = vec![b'x'; MAX_IN_MEMORY_RESPONSE_BYTES + 32 * 1024];
    let cache_directory = temporary_path("quota-response-cache");
    let orphan = cache_directory.join("session-crashed");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("session.lock"), []).unwrap();
    std::fs::write(orphan.join("orphan.body"), vec![0; 4096]).unwrap();

    let quota = (body.len() as u64 * 2) - 1;
    let first_cache = ResponseCache::new(cache_directory.clone(), quota);
    first_cache.initialize().unwrap();
    assert!(
        !orphan.exists(),
        "startup should remove an orphaned session"
    );
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let first = HttpEngine::new()
        .unwrap()
        .execute(
            &request("GET", format!("{base_url}/first-retained")),
            &ExecutionOptions {
                response_cache: Some(first_cache.clone()),
                ..ExecutionOptions::default()
            },
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    let first_path = first.body_file.as_ref().unwrap().path().to_owned();
    assert!(first_path.exists());

    let second_cache = ResponseCache::new(cache_directory.clone(), quota);
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let second = HttpEngine::new()
        .unwrap()
        .execute(
            &request("GET", format!("{base_url}/quota-exceeded")),
            &ExecutionOptions {
                response_cache: Some(second_cache.clone()),
                ..ExecutionOptions::default()
            },
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert!(
        first_path.exists(),
        "an active session must not be recovered"
    );
    assert!(second.body_file.is_none());
    assert!(
        second
            .body_retention_error
            .as_deref()
            .is_some_and(|message| message.contains("quota"))
    );
    assert_eq!(second.body, body[..MAX_IN_MEMORY_RESPONSE_BYTES]);

    drop(first);
    drop(first_cache);
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let third = HttpEngine::new()
        .unwrap()
        .execute(
            &request("GET", format!("{base_url}/space-reclaimed")),
            &ExecutionOptions {
                response_cache: Some(second_cache.clone()),
                ..ExecutionOptions::default()
            },
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert!(third.body_file.is_some());
    assert!(third.body_retention_error.is_none());

    drop(third);
    drop(second_cache);
    std::fs::remove_dir_all(cache_directory).unwrap();
}
