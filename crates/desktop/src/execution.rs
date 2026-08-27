use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

use crate::filesystem::workspace_base_directory;
use directories::ProjectDirs;
use probe_core::{HttpRequest, RequestKey};
use probe_http::{
    ExecutionOptions, HttpEngine, HttpError, HttpResponse, ResponseBodyFile, ResponseCache,
};
use tokio::sync::oneshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResponseState {
    Running { started_at: Instant },
    Complete(HttpResponse),
    Failed(String),
    Cancelled,
}

impl ResponseState {
    pub(crate) const fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub(crate) fn elapsed(&self) -> Option<Duration> {
        match self {
            Self::Running { started_at } => Some(started_at.elapsed()),
            Self::Complete(response) => Some(response.duration),
            Self::Failed(_) | Self::Cancelled => None,
        }
    }
}

pub(crate) struct ActiveRequest {
    pub(crate) generation: u64,
    pub(crate) cancellation: oneshot::Sender<()>,
}

#[derive(Default)]
pub(crate) struct ExecutionState {
    next_generation: u64,
    responses: BTreeMap<RequestKey, ResponseState>,
    active: BTreeMap<RequestKey, ActiveRequest>,
    key_aliases: BTreeMap<RequestKey, RequestKey>,
}

impl ExecutionState {
    pub(crate) fn begin(&mut self, key: RequestKey, cancellation: oneshot::Sender<()>) -> u64 {
        self.cancel(key);
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        self.active.insert(
            key,
            ActiveRequest {
                generation,
                cancellation,
            },
        );
        self.responses.insert(
            key,
            ResponseState::Running {
                started_at: Instant::now(),
            },
        );
        generation
    }

    pub(crate) fn finish(
        &mut self,
        key: RequestKey,
        generation: u64,
        result: Result<HttpResponse, HttpError>,
    ) {
        let key = self.resolve_key(key);
        if self
            .active
            .get(&key)
            .is_none_or(|active| active.generation != generation)
        {
            return;
        }
        self.active.remove(&key);
        self.key_aliases.retain(|_, target| *target != key);
        let response = match result {
            Ok(response) => ResponseState::Complete(response),
            Err(HttpError::Cancelled) => ResponseState::Cancelled,
            Err(error) => ResponseState::Failed(error.to_string()),
        };
        self.responses.insert(key, response);
    }

    pub(crate) fn fail(&mut self, key: RequestKey, message: String) {
        let key = self.resolve_key(key);
        self.cancel(key);
        self.responses.insert(key, ResponseState::Failed(message));
    }

    pub(crate) fn cancel(&mut self, key: RequestKey) {
        let key = self.resolve_key(key);
        if let Some(active) = self.active.remove(&key) {
            let _ = active.cancellation.send(());
            self.responses.insert(key, ResponseState::Cancelled);
            self.key_aliases.retain(|_, target| *target != key);
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for (_, active) in std::mem::take(&mut self.active) {
            let _ = active.cancellation.send(());
        }
        self.key_aliases.clear();
        for response in self.responses.values_mut() {
            if response.is_running() {
                *response = ResponseState::Cancelled;
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.cancel_all();
        self.responses.clear();
        self.key_aliases.clear();
    }

    pub(crate) fn response(&self, key: RequestKey) -> Option<&ResponseState> {
        self.responses.get(&self.resolve_key(key))
    }

    pub(crate) fn remap_requests(&mut self, key_remaps: &BTreeMap<RequestKey, RequestKey>) {
        if key_remaps.is_empty() {
            self.cancel_all();
            self.responses.clear();
            return;
        }

        self.key_aliases = self
            .key_aliases
            .iter()
            .filter_map(|(alias, target)| key_remaps.get(target).map(|new| (*alias, *new)))
            .collect();

        self.responses = std::mem::take(&mut self.responses)
            .into_iter()
            .filter_map(|(key, response)| key_remaps.get(&key).map(|new| (*new, response)))
            .collect();

        let mut active = BTreeMap::new();
        for (key, request) in std::mem::take(&mut self.active) {
            if let Some(new_key) = key_remaps.get(&key).copied() {
                self.key_aliases.insert(key, new_key);
                active.insert(new_key, request);
            } else {
                let _ = request.cancellation.send(());
            }
        }
        self.active = active;
    }

    fn resolve_key(&self, key: RequestKey) -> RequestKey {
        let mut key = key;
        while let Some(next) = self.key_aliases.get(&key).copied() {
            if next == key {
                break;
            }
            key = next;
        }
        key
    }
}

pub(crate) fn body_file_path_for_storage(selected: &Path, workspace_path: Option<&Path>) -> String {
    if let Some(workspace_path) = workspace_path
        && let Some(base) = workspace_base_directory(workspace_path)
        && let Ok(relative) = selected.strip_prefix(&base)
    {
        let relative = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if relative.is_empty() {
            return selected.display().to_string();
        }
        if relative.contains('/') {
            return relative;
        }
        return format!("./{relative}");
    }
    selected.display().to_string()
}

pub(crate) fn execute_http_request(
    request: HttpRequest,
    options: ExecutionOptions,
    cancellation: oneshot::Receiver<()>,
) -> Result<HttpResponse, HttpError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| HttpError::ClientConfiguration(error.to_string()))?;
    runtime.block_on(async move {
        let engine = HttpEngine::new()?;
        engine
            .execute_cancellable(&request, &options, async move {
                let _ = cancellation.await;
            })
            .await
    })
}

pub(crate) const RESPONSE_CACHE_QUOTA_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) fn response_cache() -> ResponseCache {
    let directory = ProjectDirs::from("dev", "Probe", "Probe")
        .map(|directories| directories.cache_dir().join("responses"))
        .unwrap_or_else(|| std::env::temp_dir().join("probe-responses"));
    ResponseCache::new(directory, RESPONSE_CACHE_QUOTA_BYTES)
}

pub(crate) fn read_response_page(
    file: &ResponseBodyFile,
    offset: usize,
    length: usize,
) -> Result<Vec<u8>, String> {
    let mut source = std::fs::File::open(file.path()).map_err(|error| error.to_string())?;
    source
        .seek(SeekFrom::Start(offset as u64))
        .map_err(|error| error.to_string())?;
    let mut body = vec![0; length];
    source
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    Ok(body)
}

pub(crate) fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.2} s", duration.as_secs_f64())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

pub(crate) fn format_size(size: usize) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use probe_core::HttpRequest;
    use probe_http::ExecutionOptions;
    use probe_http::{HttpError, HttpResponse};
    use tokio::sync::oneshot;

    use super::{
        ExecutionState, ResponseState, body_file_path_for_storage, execute_http_request,
        format_duration, format_size,
    };

    fn key() -> probe_core::RequestKey {
        let workspace = probe_core::Workspace::from_collection(probe_core::Collection {
            items: vec![probe_core::CollectionItem::HttpRequest(
                probe_core::HttpRequest::default(),
            )],
            ..probe_core::Collection::default()
        });
        let probe_core::WorkspaceItemRef::Request(key) = workspace.root_items()[0] else {
            panic!("expected request key");
        };
        key
    }

    fn two_keys() -> (probe_core::RequestKey, probe_core::RequestKey) {
        let workspace = probe_core::Workspace::from_collection(probe_core::Collection {
            items: vec![
                probe_core::CollectionItem::HttpRequest(probe_core::HttpRequest::default()),
                probe_core::CollectionItem::HttpRequest(probe_core::HttpRequest::default()),
            ],
            ..probe_core::Collection::default()
        });
        let probe_core::WorkspaceItemRef::Request(first) = workspace.root_items()[0] else {
            panic!("expected first request key");
        };
        let probe_core::WorkspaceItemRef::Request(second) = workspace.root_items()[1] else {
            panic!("expected second request key");
        };
        (first, second)
    }

    #[test]
    fn stale_completion_cannot_replace_a_newer_execution() {
        let key = key();
        let mut state = ExecutionState::default();
        let (first, _) = oneshot::channel();
        let first_generation = state.begin(key, first);
        let (second, _) = oneshot::channel();
        let second_generation = state.begin(key, second);

        state.finish(key, first_generation, Ok(response(201)));
        assert!(matches!(
            state.response(key),
            Some(ResponseState::Running { .. })
        ));
        state.finish(key, second_generation, Ok(response(204)));
        assert!(matches!(
            state.response(key),
            Some(ResponseState::Complete(response)) if response.status == 204
        ));
    }

    #[test]
    fn cancellation_is_visible_and_normalized() {
        let key = key();
        let mut state = ExecutionState::default();
        let (sender, mut receiver) = oneshot::channel();
        let generation = state.begin(key, sender);
        state.cancel(key);
        assert!(receiver.try_recv().is_ok());
        assert_eq!(state.response(key), Some(&ResponseState::Cancelled));
        state.finish(key, generation, Err(HttpError::Cancelled));
        assert_eq!(state.response(key), Some(&ResponseState::Cancelled));
    }

    #[test]
    fn reloaded_workspace_keeps_in_flight_execution_completable() {
        let (old_key, new_key) = two_keys();
        let mut state = ExecutionState::default();
        let (sender, mut receiver) = oneshot::channel();
        let generation = state.begin(old_key, sender);

        state.remap_requests(&BTreeMap::from([(old_key, new_key)]));

        assert!(receiver.try_recv().is_err());
        assert!(matches!(
            state.response(new_key),
            Some(ResponseState::Running { .. })
        ));
        assert!(matches!(
            state.response(old_key),
            Some(ResponseState::Running { .. })
        ));

        state.finish(old_key, generation, Ok(response(202)));

        assert!(matches!(
            state.response(new_key),
            Some(ResponseState::Complete(response)) if response.status == 202
        ));
        assert!(state.response(old_key).is_none());
    }

    #[test]
    fn metadata_units_are_readable() {
        assert_eq!(format_duration(Duration::from_millis(83)), "83 ms");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1.25 s");
        assert_eq!(format_size(812), "812 B");
        assert_eq!(format_size(2048), "2.0 KB");
    }

    #[test]
    fn running_response_tracks_elapsed_time() {
        let key = key();
        let mut state = ExecutionState::default();
        let (sender, _) = oneshot::channel();

        state.begin(key, sender);

        assert!(matches!(
            state.response(key).and_then(ResponseState::elapsed),
            Some(elapsed) if elapsed >= Duration::ZERO
        ));
    }

    #[test]
    fn desktop_adapter_forwards_cancellation_to_the_shared_http_engine() {
        let (cancel, cancellation) = oneshot::channel();
        cancel.send(()).expect("cancellation should be delivered");
        let result = execute_http_request(
            HttpRequest {
                method: Some("GET".to_owned()),
                url: Some("http://127.0.0.1:1/phase-12".to_owned()),
                ..HttpRequest::default()
            },
            ExecutionOptions::default(),
            cancellation,
        );
        assert_eq!(result, Err(HttpError::Cancelled));
    }

    fn response(status: u16) -> HttpResponse {
        HttpResponse {
            status,
            reason: String::new(),
            url: String::new(),
            duration: Duration::ZERO,
            size: 0,
            headers: Vec::new(),
            body: Vec::new(),
            body_complete: true,
            body_file: None,
            body_retention_error: None,
        }
    }

    #[test]
    fn body_file_paths_prefer_workspace_relative_storage() {
        let workspace = std::path::Path::new("/tmp/collection/opencollection.yml");
        assert_eq!(
            body_file_path_for_storage(
                std::path::Path::new("/tmp/collection/archive.zip"),
                Some(workspace),
            ),
            "./archive.zip"
        );
        assert_eq!(
            body_file_path_for_storage(
                std::path::Path::new("/tmp/collection/assets/data.json"),
                Some(workspace),
            ),
            "assets/data.json"
        );
    }

    #[test]
    fn body_file_paths_fall_back_to_absolute_storage() {
        let workspace = std::path::Path::new("/tmp/collection/opencollection.yml");
        assert_eq!(
            body_file_path_for_storage(std::path::Path::new("/etc/hosts"), Some(workspace),),
            "/etc/hosts"
        );
    }
}
