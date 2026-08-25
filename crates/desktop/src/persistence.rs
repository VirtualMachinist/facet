use std::collections::{BTreeMap, VecDeque};

use probe_core::{HttpRequest, RequestKey, RequestUpdate};

#[derive(Debug, Default)]
pub(crate) struct PersistenceState {
    saved: BTreeMap<RequestKey, HttpRequest>,
    revisions: BTreeMap<RequestKey, u64>,
    saving: BTreeMap<RequestKey, u64>,
    queue: VecDeque<RequestKey>,
}

impl PersistenceState {
    pub(crate) fn reset(&mut self, requests: impl IntoIterator<Item = (RequestKey, HttpRequest)>) {
        self.saved = requests.into_iter().collect();
        self.revisions = self.saved.keys().map(|key| (*key, 0)).collect();
        self.saving.clear();
        self.queue.clear();
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn saved_request(&self, key: RequestKey) -> Option<&HttpRequest> {
        self.saved.get(&key)
    }

    pub(crate) fn edited(&mut self, key: RequestKey) {
        *self.revisions.entry(key).or_default() += 1;
    }

    pub(crate) fn is_dirty(&self, key: RequestKey, request: &HttpRequest) -> bool {
        self.saved.get(&key) != Some(request)
            || self.saving.get(&key).is_some_and(|revision| {
                self.revisions
                    .get(&key)
                    .is_some_and(|current| current > revision)
            })
    }

    pub(crate) fn dirty_keys<'a>(
        &'a self,
        requests: impl IntoIterator<Item = (RequestKey, &'a HttpRequest)> + 'a,
    ) -> Vec<RequestKey> {
        requests
            .into_iter()
            .filter_map(|(key, request)| self.is_dirty(key, request).then_some(key))
            .collect()
    }

    pub(crate) fn enqueue(&mut self, keys: impl IntoIterator<Item = RequestKey>) {
        for key in keys {
            if !self.queue.contains(&key) {
                self.queue.push_back(key);
            }
        }
    }

    pub(crate) fn next(&mut self) -> Option<RequestKey> {
        let key = self.queue.pop_front()?;
        self.saving
            .insert(key, self.revisions.get(&key).copied().unwrap_or_default());
        Some(key)
    }

    pub(crate) fn has_outstanding_saves(&self) -> bool {
        !self.queue.is_empty() || !self.saving.is_empty()
    }

    pub(crate) fn begin(
        &self,
        key: RequestKey,
        request: &HttpRequest,
    ) -> (u64, HttpRequest, RequestUpdate) {
        let snapshot = request.clone();
        let baseline = self.saved.get(&key);
        let update = RequestUpdate {
            name: (baseline.and_then(|request| request.metadata.name.as_ref())
                != snapshot.metadata.name.as_ref())
            .then(|| snapshot.metadata.name.clone())
            .flatten(),
            method: (baseline.and_then(|request| request.method.as_ref())
                != snapshot.method.as_ref())
            .then(|| snapshot.method.clone())
            .flatten(),
            url: (baseline.and_then(|request| request.url.as_ref()) != snapshot.url.as_ref())
                .then(|| snapshot.url.clone())
                .flatten(),
            headers: (baseline.map(|request| &request.headers) != Some(&snapshot.headers))
                .then(|| snapshot.headers.clone()),
            query_parameters: (baseline.map(|request| &request.query_parameters)
                != Some(&snapshot.query_parameters))
            .then(|| snapshot.query_parameters.clone()),
            path_parameters: (baseline.map(|request| &request.path_parameters)
                != Some(&snapshot.path_parameters))
            .then(|| snapshot.path_parameters.clone()),
            body: (baseline.and_then(|request| request.body.as_ref()) != snapshot.body.as_ref())
                .then(|| snapshot.body.clone()),
            authentication: (baseline.and_then(|request| request.authentication.as_ref())
                != snapshot.authentication.as_ref())
            .then(|| snapshot.authentication.clone()),
        };
        (
            self.revisions.get(&key).copied().unwrap_or_default(),
            snapshot,
            update,
        )
    }

    pub(crate) fn complete(&mut self, key: RequestKey, snapshot: HttpRequest) {
        self.saving.remove(&key);
        self.saved.insert(key, snapshot);
    }

    pub(crate) fn fail(&mut self, key: RequestKey) {
        self.saving.remove(&key);
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use probe_core::{Collection, CollectionItem, HttpRequest, Workspace, WorkspaceItemRef};

    use super::PersistenceState;

    fn request_key() -> probe_core::RequestKey {
        let workspace = Workspace::from_collection(Collection {
            items: vec![CollectionItem::HttpRequest(HttpRequest::default())],
            ..Collection::default()
        });
        let [WorkspaceItemRef::Request(key)] = workspace.root_items() else {
            unreachable!()
        };
        *key
    }

    #[test]
    fn completion_tracks_the_saved_snapshot_not_newer_edits() {
        let key = request_key();
        let original = HttpRequest::default();
        let mut state = PersistenceState::default();
        state.reset([(key, original.clone())]);

        let mut request = original;
        request.url = Some("https://saved.example".to_owned());
        state.edited(key);
        let (_, saved_snapshot, _) = state.begin(key, &request);
        request.url = Some("https://newer.example".to_owned());
        state.edited(key);
        state.complete(key, saved_snapshot);

        assert!(state.is_dirty(key, &request));
    }

    #[test]
    fn save_update_contains_only_fields_changed_since_the_baseline() {
        let key = request_key();
        let mut original = HttpRequest {
            method: Some("GET".to_owned()),
            url: Some("https://old.example".to_owned()),
            ..HttpRequest::default()
        };
        let mut state = PersistenceState::default();
        state.reset([(key, original.clone())]);
        original.url = Some("https://new.example".to_owned());

        let (_, _, update) = state.begin(key, &original);

        assert_eq!(update.url.as_deref(), Some("https://new.example"));
        assert!(update.method.is_none());
        assert!(update.headers.is_none());
        assert!(update.body.is_none());
    }

    #[test]
    fn enqueue_keeps_a_follow_up_save_for_a_request_that_is_already_saving() {
        let key = request_key();
        let mut state = PersistenceState::default();
        state.reset([(key, HttpRequest::default())]);

        state.enqueue([key]);
        assert_eq!(state.next(), Some(key));
        state.enqueue([key]);
        state.complete(key, HttpRequest::default());

        assert_eq!(state.next(), Some(key));
    }
}
