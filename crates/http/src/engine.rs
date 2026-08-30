use std::{borrow::Cow, future::Future, future::pending, path::Path, time::Instant};

use probe_core::{HttpRequest, RequestSettings};
use reqwest::{Client, header::HeaderMap, redirect::Policy};

use crate::{
    ExecutionOptions, HttpError, HttpResponse, ResponseHeader,
    request::build_request,
    response::{CollectedBody, collect_bounded, map_reqwest_error, stream_to_file},
};

const DEFAULT_MAX_REDIRECTS: usize = 10;

/// Reusable asynchronous HTTP engine shared by every interface.
#[derive(Clone, Debug)]
pub struct HttpEngine {
    default_client: Client,
}

impl HttpEngine {
    /// Creates an engine with the default redirect policy.
    pub fn new() -> Result<Self, HttpError> {
        Ok(Self {
            default_client: build_client(true, DEFAULT_MAX_REDIRECTS)?,
        })
    }

    /// Executes a request until completion.
    pub async fn execute(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
    ) -> Result<HttpResponse, HttpError> {
        self.execute_cancellable(request, options, pending::<()>())
            .await
    }

    /// Executes a request while streaming its response body to a file.
    ///
    /// The destination is replaced only after the full response body has been written.
    pub async fn execute_to_file(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: &Path,
    ) -> Result<HttpResponse, HttpError> {
        self.execute_cancellable_to_file(request, options, output, pending::<()>())
            .await
    }

    /// Executes a request, cancelling it when `cancellation` completes.
    ///
    /// Dropping the execution future also cancels the underlying reqwest request.
    pub async fn execute_cancellable<C>(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        cancellation: C,
    ) -> Result<HttpResponse, HttpError>
    where
        C: Future + Send,
    {
        self.execute_with_cancellation(request, options, None, cancellation)
            .await
    }

    /// Executes a cancellable request while streaming its response body to a file.
    pub async fn execute_cancellable_to_file<C>(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: &Path,
        cancellation: C,
    ) -> Result<HttpResponse, HttpError>
    where
        C: Future + Send,
    {
        self.execute_with_cancellation(request, options, Some(output), cancellation)
            .await
    }

    async fn execute_with_cancellation<C>(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: Option<&Path>,
        cancellation: C,
    ) -> Result<HttpResponse, HttpError>
    where
        C: Future + Send,
    {
        tokio::pin!(cancellation);
        tokio::select! {
            biased;
            _ = &mut cancellation => Err(HttpError::Cancelled),
            response = self.execute_inner(request, options, output) => response,
        }
    }

    async fn execute_inner(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: Option<&Path>,
    ) -> Result<HttpResponse, HttpError> {
        let client = self.client_for(&request.settings)?;
        let builder = build_request(&client, request, options).await?;
        let started = Instant::now();
        let mut response = builder.send().await.map_err(map_reqwest_error)?;
        let expected_size = response.content_length();
        let status = response.status();
        let url = response.url().to_string();
        let headers = response_headers(response.headers());
        let body = match output {
            Some(output) => CollectedBody {
                preview: Vec::new(),
                size: stream_to_file(&mut response, output).await?,
                complete: false,
                file: None,
                retention_error: None,
            },
            None => {
                collect_bounded(
                    &mut response,
                    options.response_cache.as_ref(),
                    expected_size,
                )
                .await?
            }
        };
        Ok(HttpResponse {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or_default().to_owned(),
            url,
            duration: started.elapsed(),
            size: body.size,
            headers,
            body: body.preview,
            body_complete: body.complete,
            body_file: body.file,
            body_retention_error: body.retention_error,
        })
    }

    fn client_for(&self, settings: &RequestSettings) -> Result<Cow<'_, Client>, HttpError> {
        let follow = settings.follow_redirects.unwrap_or(true);
        let maximum = settings.max_redirects.unwrap_or(DEFAULT_MAX_REDIRECTS);
        if follow && maximum == DEFAULT_MAX_REDIRECTS {
            Ok(Cow::Borrowed(&self.default_client))
        } else {
            build_client(follow, maximum).map(Cow::Owned)
        }
    }
}

fn build_client(follow_redirects: bool, maximum: usize) -> Result<Client, HttpError> {
    let policy = if follow_redirects {
        Policy::limited(maximum)
    } else {
        Policy::none()
    };
    Client::builder()
        .redirect(policy)
        .build()
        .map_err(|error| HttpError::ClientConfiguration(error.to_string()))
}

fn response_headers(headers: &HeaderMap) -> Vec<ResponseHeader> {
    let mut headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| ResponseHeader {
            name: name.as_str().to_owned(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect();
    headers.sort_by(|left, right| (&left.name, &left.value).cmp(&(&right.name, &right.value)));
    headers
}
