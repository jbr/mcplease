//! Resource providers: routing and URI interpretation belong to the application.
use crate::types::{
    JsonRpcError, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResult, RequestContext,
};
use std::future::Future;

/// A collection of resources backed by the server's shared `State`, installed
/// with [`ServerConfig::with_resources`](crate::ServerConfig::with_resources).
///
/// The provider is a value, so it can carry its own configuration — a root
/// directory, a URI prefix — alongside the `State` every request is handed.
/// All methods receive per-request context, including discovery, and return
/// `Send` futures so transports can run them concurrently. Providers may use
/// dynamic discovery, opaque cursors, and arbitrary URI schemes. No routing,
/// query parsing, template expansion, subscriptions, or completion is imposed
/// here. Errors are JSON-RPC errors, never tool `isError` envelopes. A missing
/// or invalid resource can use [`JsonRpcError::invalid_params`].
///
/// This interface returns complete read results; the input-required/MRTR
/// response flow is not implemented by this dispatcher.
pub trait Resources<State>: Send + Sync {
    fn list(
        &self,
        state: State,
        params: PaginatedRequestParams,
        context: RequestContext,
    ) -> impl Future<Output = Result<ListResourcesResult, JsonRpcError>> + Send;

    fn list_templates(
        &self,
        state: State,
        params: PaginatedRequestParams,
        context: RequestContext,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, JsonRpcError>> + Send;

    fn read(
        &self,
        state: State,
        params: ReadResourceRequestParams,
        context: RequestContext,
    ) -> impl Future<Output = Result<ReadResourceResult, JsonRpcError>> + Send;
}

/// The [`ServerConfig`](crate::ServerConfig) default: no provider installed, so
/// the resources capability is not advertised and resource methods are
/// answered with method-not-found. Uninhabited, so it is never called.
#[derive(Debug, Clone, Copy)]
pub enum NoResources {}

impl<State: Send> Resources<State> for NoResources {
    async fn list(
        &self,
        _: State,
        _: PaginatedRequestParams,
        _: RequestContext,
    ) -> Result<ListResourcesResult, JsonRpcError> {
        match *self {}
    }

    async fn list_templates(
        &self,
        _: State,
        _: PaginatedRequestParams,
        _: RequestContext,
    ) -> Result<ListResourceTemplatesResult, JsonRpcError> {
        match *self {}
    }

    async fn read(
        &self,
        _: State,
        _: ReadResourceRequestParams,
        _: RequestContext,
    ) -> Result<ReadResourceResult, JsonRpcError> {
        match *self {}
    }
}
