//! Http response.
use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};

#[cfg(feature = "cookie")]
use cookie::{Cookie, CookieJar};
use futures_util::stream::{Stream, TryStreamExt};
use http::header::{HeaderMap, HeaderValue, IntoHeaderName, SET_COOKIE};
pub use http::response::Parts;
use http::version::Version;
use mime::Mime;

use super::errors::*;
use crate::http::StatusCode;
use crate::{Error, Piece};
use bytes::Bytes;

pub use crate::http::body::ResBody;

/// Represents an HTTP response
pub struct Response {
    status_code: Option<StatusCode>,
    pub(crate) status_error: Option<StatusError>,
    headers: HeaderMap,
    version: Version,
    #[cfg(feature = "cookie")]
    pub(crate) cookies: CookieJar,
    pub(crate) body: ResBody,
}
impl Default for Response {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
impl From<hyper::Response<ResBody>> for Response {
    #[inline]
    fn from(res: hyper::Response<ResBody>) -> Self {
        let (
            http::response::Parts {
                status,
                version,
                headers,
                // extensions,
                ..
            },
            body,
        ) = res.into_parts();
        #[cfg(feature = "cookie")]
        // Set the request cookies, if they exist.
        let cookies = if let Some(header) = headers.get(SET_COOKIE) {
            let mut cookie_jar = CookieJar::new();
            if let Ok(header) = header.to_str() {
                for cookie_str in header.split(';').map(|s| s.trim()) {
                    if let Ok(cookie) = Cookie::parse_encoded(cookie_str).map(|c| c.into_owned()) {
                        cookie_jar.add(cookie);
                    }
                }
            }
            cookie_jar
        } else {
            CookieJar::new()
        };

        Response {
            status_code: Some(status),
            status_error: None,
            body,
            version,
            headers,
            #[cfg(feature = "cookie")]
            cookies,
        }
    }
}

impl Response {
    /// Creates a new blank `Response`.
    #[inline]
    pub fn new() -> Response {
        Response {
            status_code: None,
            status_error: None,
            body: ResBody::None,
            version: Version::default(),
            headers: HeaderMap::new(),
            #[cfg(feature = "cookie")]
            cookies: CookieJar::default(),
        }
    }

    /// Creates a new blank `Response`.
    #[cfg(feature = "cookie")]
    #[inline]
    pub fn with_cookies(cookies: CookieJar) -> Response {
        Response {
            status_code: None,
            status_error: None,
            body: ResBody::None,
            version: Version::default(),
            headers: HeaderMap::new(),
            cookies,
        }
    }

    /// Get headers reference.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    /// Get mutable headers reference.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
    /// Sets headers.
    #[inline]
    pub fn set_headers(&mut self, headers: HeaderMap) {
        self.headers = headers
    }

    /// Modify a header for this response.
    ///
    /// When `overwrite` is set to `true`, If the header is already present, the value will be replaced.
    /// When `overwrite` is set to `false`, The new header is always appended to the request, even if the header already exists.
    pub fn add_header<N, V>(&mut self, name: N, value: V, overwrite: bool) -> crate::Result<&mut Self>
    where
        N: IntoHeaderName,
        V: TryInto<HeaderValue>,
    {
        let value = value
            .try_into()
            .map_err(|_| Error::Other("invalid header value".into()))?;
        if overwrite {
            self.headers.insert(name, value);
        } else {
            self.headers.append(name, value);
        }
        Ok(self)
    }

    /// Get version.
    #[inline]
    pub fn version(&self) -> Version {
        self.version
    }
    /// Get mutable version reference.
    #[inline]
    pub fn version_mut(&mut self) -> &mut Version {
        &mut self.version
    }

    /// Get body reference.
    #[inline]
    pub fn body(&self) -> &ResBody {
        &self.body
    }
    /// Get mutable body reference.
    #[inline]
    pub fn body_mut(&mut self) -> &mut ResBody {
        &mut self.body
    }
    /// Sets body.
    #[inline]
    pub fn set_body(&mut self, body: ResBody) {
        self.body = body;
    }
    /// Sets body.
    #[inline]
    pub fn with_body(&mut self, body: ResBody) -> &mut Self {
        self.set_body(body);
        self
    }

    /// Sets body to a new value and returns old value.
    #[inline]
    pub fn replace_body(&mut self, body: ResBody) -> ResBody {
        std::mem::replace(&mut self.body, body)
    }

    /// Take body from response.
    #[inline]
    pub fn take_body(&mut self) -> ResBody {
        self.replace_body(ResBody::None)
    }

    // If return `true`, it means this response is ready for write back and the reset handlers should be skipped.
    #[doc(hidden)]
    #[inline]
    pub fn is_stamped(&mut self) -> bool {
        if let Some(code) = self.status_code() {
            if code.is_client_error() || code.is_server_error() || code.is_redirection() {
                return true;
            }
        }
        false
    }

    #[cfg(feature = "cookie")]
    #[doc(hidden)]
    #[inline]
    pub(crate) fn write_cookies_to_headers(&mut self) {
        for cookie in self.cookies.delta() {
            if let Ok(hv) = cookie.encoded().to_string().parse() {
                self.headers.append(SET_COOKIE, hv);
            }
        }
        self.cookies = CookieJar::new();
    }

    /// `write_back` is used to put all the data added to `self`
    /// back onto an `hyper::Response` so that it is sent back to the
    /// client.
    ///
    /// `write_back` consumes the `Response`.
    #[inline]
    pub(crate) async fn write_back(mut self, res: &mut hyper::Response<ResBody>) {
        #[cfg(feature = "cookie")]
        self.write_cookies_to_headers();
        let Self {
            status_code,
            headers,
            body,
            ..
        } = self;
        *res.headers_mut() = headers;

        // Default to a 404 if no response code was set
        *res.status_mut() = status_code.unwrap_or(StatusCode::NOT_FOUND);
        *res.body_mut() = body;
    }

    cfg_feature! {
        #![feature = "cookie"]
        /// Get cookies reference.
        #[inline]
        pub fn cookies(&self) -> &CookieJar {
            &self.cookies
        }
        /// Get mutable cookies reference.
        #[inline]
        pub fn cookies_mut(&mut self) -> &mut CookieJar {
            &mut self.cookies
        }
        /// Helper function for get cookie.
        #[inline]
        pub fn cookie<T>(&self, name: T) -> Option<&Cookie<'static>>
        where
            T: AsRef<str>,
        {
            self.cookies.get(name.as_ref())
        }
        /// Helper function for add cookie.
        #[inline]
        pub fn add_cookie(&mut self, cookie: Cookie<'static>)-> &mut Self {
            self.cookies.add(cookie);
            self
        }

        /// Helper function for remove cookie.
        ///
        /// Removes `cookie` from this CookieJar. If an _original_ cookie with the same
        /// name as `cookie` is present in the jar, a _removal_ cookie will be
        /// present in the `delta` computation.
        ///
        /// A "removal" cookie is a cookie that has the same name as the original
        /// cookie but has an empty value, a max-age of 0, and an expiration date
        /// far in the past. Read more about [removal cookies](https://docs.rs/cookie/0.16.1/cookie/struct.CookieJar.html#method.remove).
        #[inline]
        pub fn remove_cookie(&mut self, name: &str) -> &mut Self
        {
            if let Some(cookie) = self.cookies.get(name).cloned() {
                self.cookies.remove(cookie);
            }
            self
        }
    }

    /// Get status code.
    #[inline]
    pub fn status_code(&self) -> Option<StatusCode> {
        self.status_code
    }

    /// Sets status code.
    #[inline]
    pub fn set_status_code(&mut self, code: StatusCode) {
        self.status_code = Some(code);
        if !code.is_success() {
            self.status_error = StatusError::from_code(code);
        }
    }

    /// Sets status code.
    #[inline]
    pub fn with_status_code(&mut self, code: StatusCode) -> &mut Self {
        self.set_status_code(code);
        self
    }

    /// Get content type.
    #[inline]
    pub fn content_type(&self) -> Option<Mime> {
        self.headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .and_then(|v| v.parse().ok())
    }

    /// Get http error if exists, only exists after use `set_status_error` set http error.
    #[inline]
    pub fn status_error(&self) -> Option<&StatusError> {
        self.status_error.as_ref()
    }
    /// Sets http error.
    #[inline]
    pub fn set_status_error(&mut self, e: StatusError) {
        self.status_code = Some(e.code);
        self.status_error = Some(e);
    }
    /// Sets http error.
    #[inline]
    pub fn with_status_error(&mut self, e: StatusError) -> &mut Self {
        self.set_status_error(e);
        self
    }

    /// Render content.
    #[inline]
    pub fn render<P>(&mut self, piece: P)
    where
        P: Piece,
    {
        piece.render(self);
    }

    /// Render content.
    #[inline]
    pub fn with_render<P>(&mut self, piece: P) -> &mut Self
    where
        P: Piece,
    {
        self.render(piece);
        self
    }

    /// Render content with status code.
    #[inline]
    pub fn stuff<P>(&mut self, code: StatusCode, piece: P)
    where
        P: Piece,
    {
        self.status_code = Some(code);
        piece.render(self);
    }
    /// Render content with status code.
    #[inline]
    pub fn with_stuff<P>(&mut self, code: StatusCode, piece: P) -> &mut Self
    where
        P: Piece,
    {
        self.stuff(code, piece);
        self
    }

    /// Write bytes data to body. If body is none, a new `ResBody` will created.
    #[inline]
    pub fn write_body(&mut self, data: impl Into<Bytes>) -> crate::Result<()> {
        match self.body_mut() {
            ResBody::None => {
                self.body = ResBody::Once(data.into());
            }
            ResBody::Once(ref bytes) => {
                let mut chunks = VecDeque::new();
                chunks.push_back(bytes.clone());
                chunks.push_back(data.into());
                self.body = ResBody::Chunks(chunks);
            }
            ResBody::Chunks(chunks) => {
                chunks.push_back(data.into());
            }
            ResBody::Hyper(_) => {
                tracing::error!("current body's kind is `ResBody::Hyper`, it is not allowed to write bytes");
                return Err(Error::other(
                    "current body's kind is `ResBody::Hyper`, it is not allowed to write bytes",
                ));
            }
            ResBody::Stream(_) => {
                tracing::error!("current body's kind is `ResBody::Stream`, it is not allowed to write bytes");
                return Err(Error::other(
                    "current body's kind is `ResBody::Stream`, it is not allowed to write bytes",
                ));
            }
        }
        Ok(())
    }
    /// Write streaming data.
    #[inline]
    pub fn streaming<S, O, E>(&mut self, stream: S) -> crate::Result<()>
    where
        S: Stream<Item = Result<O, E>> + Send + 'static,
        O: Into<Bytes> + 'static,
        E: Into<Box<dyn StdError + Send + Sync>> + 'static,
    {
        match &self.body {
            ResBody::Once(_) => {
                return Err(Error::other("current body kind is `ResBody::Once` already"));
            }
            ResBody::Chunks(_) => {
                return Err(Error::other("current body kind is `ResBody::Chunks` already"));
            }
            ResBody::Stream(_) => {
                return Err(Error::other("current body kind is `ResBody::Stream` already"));
            }
            _ => {}
        }
        let mapped = stream.map_ok(Into::into).map_err(Into::into);
        self.body = ResBody::Stream(Box::pin(mapped));
        Ok(())
    }
}

impl fmt::Debug for Response {
    #[inline]
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        writeln!(
            f,
            "HTTP/1.1 {}\n{:?}",
            self.status_code.unwrap_or(StatusCode::NOT_FOUND),
            self.headers
        )
    }
}

impl Display for Response {
    #[inline]
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod test {
    use bytes::BytesMut;
    use futures_util::stream::{iter, StreamExt};
    use std::error::Error;

    use super::*;

    #[test]
    fn test_body_empty() {
        let body = ResBody::Once(Bytes::from("hello"));
        assert!(!body.is_none());
        let body = ResBody::None;
        assert!(body.is_none());
    }

    #[tokio::test]
    async fn test_body_stream1() {
        let mut body = ResBody::Once(Bytes::from("hello"));

        let mut result = bytes::BytesMut::new();
        while let Some(Ok(data)) = body.next().await {
            result.extend_from_slice(&data)
        }

        assert_eq!("hello", &result)
    }

    #[tokio::test]
    async fn test_body_stream2() {
        let mut body = ResBody::Stream(Box::pin(iter(vec![
            Result::<_, Box<dyn Error + Send + Sync>>::Ok(BytesMut::from("Hello").freeze()),
            Result::<_, Box<dyn Error + Send + Sync>>::Ok(BytesMut::from(" World").freeze()),
        ])));

        let mut result = bytes::BytesMut::new();
        while let Some(Ok(data)) = body.next().await {
            result.extend_from_slice(&data)
        }

        assert_eq!("Hello World", &result)
    }
}
