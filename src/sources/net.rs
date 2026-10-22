//! Network sources.
//!
//! timemachine does not ship an HTTP client. You adapt whatever client your
//! code uses to the small [`Net`] trait (one request in, one response out),
//! and the [`crate::Ctx`] records and replays the exchanges.

use std::fmt;

/// A request: a method, a target (URL or any address string) and a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Method, e.g. `GET`.
    pub method: String,
    /// Target, e.g. `https://example.test/token`.
    pub target: String,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl Request {
    /// A request without a body.
    pub fn new(method: impl Into<String>, target: impl Into<String>) -> Self {
        Request {
            method: method.into(),
            target: target.into(),
            body: Vec::new(),
        }
    }

    /// Adds a body (builder style).
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

/// A response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Status code.
    pub status: u16,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// A response with a status and body.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Response {
            status,
            body: body.into(),
        }
    }
}

/// A failed exchange (connection refused, timeout, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetError {
    /// Short category, e.g. `timeout`.
    pub kind: String,
    /// Details.
    pub message: String,
}

impl NetError {
    /// Creates an error.
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        NetError {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for NetError {}

/// A request/response transport.
pub trait Net: Send {
    /// Performs one exchange.
    fn request(&mut self, req: &Request) -> Result<Response, NetError>;
}

/// The default transport: every request fails with `no_network`, so a test
/// that unexpectedly talks to the network fails loudly instead of reaching
/// a real host.
#[derive(Debug, Clone, Default)]
pub struct NoNet;

impl Net for NoNet {
    fn request(&mut self, req: &Request) -> Result<Response, NetError> {
        Err(NetError::new(
            "no_network",
            format!(
                "no network source configured (request: {} {})",
                req.method, req.target
            ),
        ))
    }
}

/// A transport backed by a closure, for simulated services.
pub struct StubNet<F> {
    handler: F,
}

impl<F> StubNet<F>
where
    F: FnMut(&Request) -> Result<Response, NetError> + Send,
{
    /// Wraps a handler.
    pub fn new(handler: F) -> Self {
        StubNet { handler }
    }
}

impl<F> Net for StubNet<F>
where
    F: FnMut(&Request) -> Result<Response, NetError> + Send,
{
    fn request(&mut self, req: &Request) -> Result<Response, NetError> {
        (self.handler)(req)
    }
}
