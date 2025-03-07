use futures_lite::{io, prelude::*, ready};
#[cfg(feature = "serde")]
use serde_crate::{de::DeserializeOwned, Serialize};

use std::convert::TryFrom;
use std::fmt::{self, Debug};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::mime::{self, Mime};
use crate::{Status, StatusCode};

pin_project_lite::pin_project! {
    /// A streaming HTTP body.
    ///
    /// `Body` represents the HTTP body of both `Request` and `Response`. It's completely
    /// streaming, and implements `AsyncBufRead` to make reading from it both convenient and
    /// performant.
    ///
    /// Both `Request` and `Response` take `Body` by `Into<Body>`, which means that passing string
    /// literals, byte vectors, but also concrete `Body` instances are all valid. This makes it
    /// easy to create both quick HTTP requests, but also have fine grained control over how bodies
    /// are streamed out.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Body, Response, StatusCode};
    /// use async_std::io::Cursor;
    ///
    /// let mut req = Response::new(StatusCode::Ok);
    /// req.set_body("Hello Chashu");
    ///
    /// let mut req = Response::new(StatusCode::Ok);
    /// let cursor = Cursor::new("Hello Nori");
    /// let body = Body::from_reader(cursor, Some(10)); // set the body length
    /// req.set_body(body);
    /// ```
    ///
    /// # Length
    ///
    /// One of the details of `Body` to be aware of is the `length` parameter. The value of
    /// `length` is used by HTTP implementations to determine how to treat the stream. If a length
    /// is known ahead of time, it's _strongly_ recommended to pass it.
    ///
    /// Casting from `Vec<u8>`, `String`, or similar to `Body` will automatically set the value of
    /// `length`.
    ///
    /// # Content Encoding
    ///
    /// By default `Body` will come with a fallback Mime type that is used by `Request` and
    /// `Response` if no other type has been set, and no other Mime type can be inferred.
    ///
    /// It's _strongly_ recommended to always set a mime type on both the `Request` and `Response`,
    /// and not rely on the fallback mechanisms. However, they're still there if you need them.
    pub struct Body {
        #[pin]
        reader: Box<dyn AsyncBufRead + Unpin + 'static>,
        mime: Option<Mime>,
        length: Option<u64>,
        bytes_read: u64,
    }
}

impl Body {
    /// Create a new empty `Body`.
    ///
    /// The body will have a length of `0`, and the Mime type set to `application/octet-stream` if
    /// no other mime type has been set or can be sniffed.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Body, Response, StatusCode};
    ///
    /// let mut req = Response::new(StatusCode::Ok);
    /// req.set_body(Body::empty());
    /// ```
    pub fn empty() -> Self {
        Self {
            reader: Box::new(io::empty()),
            mime: Some(mime::BYTE_STREAM),
            length: Some(0),
            bytes_read: 0,
        }
    }

    /// Create a `Body` from a reader with an optional length.
    ///
    /// The Mime type is set to `application/octet-stream` if no other mime type has been set or can
    /// be sniffed. If a `Body` has no length, HTTP implementations will often switch over to
    /// framed messages such as [Chunked
    /// Encoding](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Transfer-Encoding).
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Body, Response, StatusCode};
    /// use async_std::io::Cursor;
    ///
    /// let mut req = Response::new(StatusCode::Ok);
    ///
    /// let cursor = Cursor::new("Hello Nori");
    /// let len = 10;
    /// req.set_body(Body::from_reader(cursor, Some(len)));
    /// ```
    pub fn from_reader(
        reader: impl AsyncBufRead + Unpin + 'static,
        length: Option<u64>,
    ) -> Self {
        Self {
            reader: Box::new(reader),
            mime: Some(mime::BYTE_STREAM),
            length,
            bytes_read: 0,
        }
    }

    /// Get the inner reader from the `Body`
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::io::prelude::*;
    /// use http_types::Body;
    /// use async_std::io::Cursor;
    ///
    /// let cursor = Cursor::new("Hello Nori");
    /// let body = Body::from_reader(cursor, None);
    /// let _ = body.into_reader();
    /// ```
    pub fn into_reader(self) -> Box<dyn AsyncBufRead + Unpin + 'static> {
        self.reader
    }

    /// Create a `Body` from a Vec of bytes.
    ///
    /// The Mime type is set to `application/octet-stream` if no other mime type has been set or can
    /// be sniffed. If a `Body` has no length, HTTP implementations will often switch over to
    /// framed messages such as [Chunked
    /// Encoding](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Transfer-Encoding).
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Body, Response, StatusCode};
    /// use async_std::io::Cursor;
    ///
    /// let mut req = Response::new(StatusCode::Ok);
    ///
    /// let input = vec![1, 2, 3];
    /// req.set_body(Body::from_bytes(input));
    /// ```
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            mime: Some(mime::BYTE_STREAM),
            length: Some(bytes.len() as u64),
            reader: Box::new(io::Cursor::new(bytes)),
            bytes_read: 0,
        }
    }

    /// Parse the body into a `Vec<u8>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::Body;
    ///
    /// let bytes = vec![1, 2, 3];
    /// let body = Body::from_bytes(bytes);
    ///
    /// let bytes: Vec<u8> = body.into_bytes().await?;
    /// assert_eq!(bytes, vec![1, 2, 3]);
    /// # Ok(()) }) }
    /// ```
    pub async fn into_bytes(mut self) -> crate::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(1024);
        self.read_to_end(&mut buf)
            .await
            .status(StatusCode::UnprocessableEntity)?;
        Ok(buf)
    }

    /// Create a `Body` from a String
    ///
    /// The Mime type is set to `text/plain` if no other mime type has been set or can
    /// be sniffed. If a `Body` has no length, HTTP implementations will often switch over to
    /// framed messages such as [Chunked
    /// Encoding](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Transfer-Encoding).
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Body, Response, StatusCode};
    /// use async_std::io::Cursor;
    ///
    /// let mut req = Response::new(StatusCode::Ok);
    ///
    /// let input = String::from("hello Nori!");
    /// req.set_body(Body::from_string(input));
    /// ```
    pub fn from_string(s: String) -> Self {
        Self {
            mime: Some(mime::PLAIN),
            length: Some(s.len() as u64),
            reader: Box::new(io::Cursor::new(s.into_bytes())),
            bytes_read: 0,
        }
    }

    /// Read the body as a string
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::Body;
    /// use async_std::io::Cursor;
    ///
    /// let cursor = Cursor::new("Hello Nori");
    /// let body = Body::from_reader(cursor, None);
    /// assert_eq!(&body.into_string().await.unwrap(), "Hello Nori");
    /// # Ok(()) }) }
    /// ```
    pub async fn into_string(mut self) -> crate::Result<String> {
        let len = usize::try_from(self.len().unwrap_or(0)).status(StatusCode::PayloadTooLarge)?;
        let mut result = String::with_capacity(len);
        self.read_to_string(&mut result)
            .await
            .status(StatusCode::UnprocessableEntity)?;
        Ok(result)
    }

    /// Creates a `Body` from a type, serializing it as JSON.
    ///
    /// # Mime
    ///
    /// The encoding is set to `application/json`.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::{Body, convert::json};
    ///
    /// let body = Body::from_json(&json!({ "name": "Chashu" }));
    /// # drop(body);
    /// ```
    #[cfg(feature = "serde")]
    pub fn from_json(json: &impl Serialize) -> crate::Result<Self> {
        let bytes = serde_json::to_vec(&json)?;
        let body = Self {
            length: Some(bytes.len() as u64),
            reader: Box::new(io::Cursor::new(bytes)),
            mime: Some(mime::JSON),
            bytes_read: 0,
        };
        Ok(body)
    }

    /// Parse the body as JSON, serializing it to a struct.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::Body;
    /// use http_types::convert::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// # #[serde(crate = "serde_crate")]
    /// struct Cat { name: String }
    ///
    /// let cat = Cat { name: String::from("chashu") };
    /// let body = Body::from_json(&cat)?;
    ///
    /// let cat: Cat = body.into_json().await?;
    /// assert_eq!(&cat.name, "chashu");
    /// # Ok(()) }) }
    /// ```
    #[cfg(feature = "serde")]
    pub async fn into_json<T: DeserializeOwned>(mut self) -> crate::Result<T> {
        let mut buf = Vec::with_capacity(1024);
        self.read_to_end(&mut buf).await?;
        Ok(serde_json::from_slice(&buf).status(StatusCode::UnprocessableEntity)?)
    }

    /// Creates a `Body` from a type, serializing it using form encoding.
    ///
    /// # Mime
    ///
    /// The encoding is set to `application/x-www-form-urlencoded`.
    ///
    /// # Errors
    ///
    /// An error will be returned if the encoding failed.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::Body;
    /// use http_types::convert::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// # #[serde(crate = "serde_crate")]
    /// struct Cat { name: String }
    ///
    /// let cat = Cat { name: String::from("chashu") };
    /// let body = Body::from_form(&cat)?;
    ///
    /// let cat: Cat = body.into_form().await?;
    /// assert_eq!(&cat.name, "chashu");
    /// # Ok(()) }) }
    /// ```
    #[cfg(feature = "serde")]
    pub fn from_form(form: &impl Serialize) -> crate::Result<Self> {
        let query = serde_urlencoded::to_string(form)?;
        let bytes = query.into_bytes();

        let body = Self {
            length: Some(bytes.len() as u64),
            reader: Box::new(io::Cursor::new(bytes)),
            mime: Some(mime::FORM),
            bytes_read: 0,
        };
        Ok(body)
    }

    /// Parse the body from form encoding into a type.
    ///
    /// # Errors
    ///
    /// An error is returned if the underlying IO stream errors, or if the body
    /// could not be deserialized into the type.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::Body;
    /// use http_types::convert::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// # #[serde(crate = "serde_crate")]
    /// struct Cat { name: String }
    ///
    /// let cat = Cat { name: String::from("chashu") };
    /// let body = Body::from_form(&cat)?;
    ///
    /// let cat: Cat = body.into_form().await?;
    /// assert_eq!(&cat.name, "chashu");
    /// # Ok(()) }) }
    /// ```
    #[cfg(feature = "serde")]
    pub async fn into_form<T: DeserializeOwned>(self) -> crate::Result<T> {
        let s = self.into_string().await?;
        Ok(serde_urlencoded::from_str(&s).status(StatusCode::UnprocessableEntity)?)
    }

    /// Create a `Body` from a file named by a path.
    ///
    /// The Mime type is sniffed from the file contents if possible, otherwise
    /// it is inferred from the path's extension if possible, otherwise is set
    /// to `application/octet-stream`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::{Body, Response, StatusCode};
    ///
    /// let mut res = Response::new(StatusCode::Ok);
    /// res.set_body(Body::from_path("/path/to/file").await?);
    /// # Ok(()) }) }
    /// ```
    #[cfg(all(feature = "fs", not(target_os = "unknown")))]
    pub async fn from_path<P>(path: P) -> io::Result<Self>
    where
        P: AsRef<std::path::Path>,
    {
        let path = path.as_ref();
        let file = async_std::fs::File::open(path).await?;
        Self::from_file_with_path(file, path).await
    }

    /// Create a `Body` from an already-open file.
    ///
    /// The Mime type is sniffed from the file contents if possible, otherwise
    /// is set to `application/octet-stream`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::{Body, Response, StatusCode};
    ///
    /// let mut res = Response::new(StatusCode::Ok);
    /// let path = std::path::Path::new("/path/to/file");
    /// let file = async_std::fs::File::open(path).await?;
    /// res.set_body(Body::from_file(file).await?);
    /// # Ok(()) }) }
    /// ```
    #[cfg(all(feature = "fs", not(target_os = "unknown")))]
    #[inline]
    pub async fn from_file(file: async_std::fs::File) -> io::Result<Self> {
        Self::from_file_with_path(file, std::path::Path::new("")).await
    }

    /// Create a `Body` from an already-open file.
    ///
    /// The Mime type is sniffed from the file contents if possible, otherwise
    /// it is inferred from the path's extension if possible, otherwise is set
    /// to `application/octet-stream`.
    ///
    /// The path here is only used to provide an extension for guessing the Mime
    /// type, and may be empty if the path is unknown.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::{Body, Response, StatusCode};
    ///
    /// let mut res = Response::new(StatusCode::Ok);
    /// let path = std::path::Path::new("/path/to/file");
    /// let file = async_std::fs::File::open(path).await?;
    /// res.set_body(Body::from_file_with_path(file, path).await?);
    /// # Ok(()) }) }
    /// ```
    #[cfg(all(feature = "fs", not(target_os = "unknown")))]
    pub async fn from_file_with_path(
        mut file: async_std::fs::File,
        path: &std::path::Path,
    ) -> io::Result<Self> {
        let len = file.metadata().await?.len();

        // Look at magic bytes first, look at extension second, fall back to
        // octet stream.
        let mime = peek_mime(&mut file)
            .await?
            .or_else(|| guess_ext(path))
            .unwrap_or(mime::BYTE_STREAM);

        Ok(Self {
            mime: Some(mime),
            length: Some(len),
            reader: Box::new(io::BufReader::new(file)),
            bytes_read: 0,
        })
    }

    /// Get the length of the body in bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_types::Body;
    /// use async_std::io::Cursor;
    ///
    /// let cursor = Cursor::new("Hello Nori");
    /// let len = 10;
    /// let body = Body::from_reader(cursor, Some(len));
    /// assert_eq!(body.len(), Some(10));
    /// ```
    pub fn len(&self) -> Option<u64> {
        self.length
    }

    /// Returns `true` if the body has a length of zero, and `false` otherwise.
    pub fn is_empty(&self) -> Option<bool> {
        self.length.map(|length| length == 0)
    }

    /// Returns the mime type of this Body.
    pub fn mime(&self) -> Option<&Mime> {
        self.mime.as_ref()
    }

    /// Sets the mime type of this Body.
    ///
    /// # Examples
    /// ```
    /// use http_types::Body;
    /// use http_types::mime;
    ///
    /// let mut body = Body::empty();
    /// body.set_mime(Some(mime::CSS));
    /// assert_eq!(body.mime(), Some(&mime::CSS));
    ///
    /// body.set_mime(None);
    /// assert_eq!(body.mime(), None);
    /// ```
    pub fn set_mime(&mut self, mime: Option<Mime>) {
        self.mime = mime;
    }

    /// Create a Body by chaining another Body after this one, consuming both.
    ///
    /// If both Body instances have a length, and their sum does not overflow,
    /// the resulting Body will have a length.
    ///
    /// If both Body instances have the same fallback MIME type, the resulting
    /// Body will have the same fallback MIME type; otherwise, the resulting
    /// Body will have the fallback MIME type `application/octet-stream`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> http_types::Result<()> { async_std::task::block_on(async {
    /// use http_types::Body;
    /// use async_std::io::Cursor;
    ///
    /// let cursor = Cursor::new("Hello ");
    /// let body = Body::from_reader(cursor, None).chain(Body::from("Nori"));
    /// assert_eq!(&body.into_string().await.unwrap(), "Hello Nori");
    /// # Ok(()) }) }
    /// ```
    pub fn chain(self, other: Body) -> Self {
        let mime = if self.mime == other.mime {
            self.mime.clone()
        } else {
            Some(mime::BYTE_STREAM)
        };
        let length = match (self.length, other.length) {
            (Some(l1), Some(l2)) => (l1 - self.bytes_read).checked_add(l2 - other.bytes_read),
            _ => None,
        };
        Self {
            mime,
            length,
            reader: Box::new(futures_lite::io::AsyncReadExt::chain(self, other)),
            bytes_read: 0,
        }
    }
}

impl Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body")
            .field("reader", &"<hidden>")
            .field("length", &self.length)
            .field("bytes_read", &self.bytes_read)
            .finish()
    }
}

#[cfg(feature = "serde")]
impl From<serde_json::Value> for Body {
    fn from(json_value: serde_json::Value) -> Self {
        Self::from_json(&json_value).unwrap()
    }
}

impl From<String> for Body {
    fn from(s: String) -> Self {
        Self::from_string(s)
    }
}

impl<'a> From<&'a str> for Body {
    fn from(s: &'a str) -> Self {
        Self::from_string(s.to_owned())
    }
}

impl From<Vec<u8>> for Body {
    fn from(b: Vec<u8>) -> Self {
        Self::from_bytes(b)
    }
}

impl<'a> From<&'a [u8]> for Body {
    fn from(b: &'a [u8]) -> Self {
        Self::from_bytes(b.to_owned())
    }
}

impl AsyncRead for Body {
    #[allow(rustdoc::missing_doc_code_examples)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let buf = match self.length {
            None => buf,
            Some(length) if length == self.bytes_read => return Poll::Ready(Ok(0)),
            Some(length) => {
                // Compute `min` using u64, then truncate back to usize. Since
                // buf.len() is a usize, this can never overflow.
                let max_len = (length - self.bytes_read).min(buf.len() as u64) as usize;
                &mut buf[0..max_len]
            }
        };

        let bytes = ready!(Pin::new(&mut self.reader).poll_read(cx, buf))?;
        self.bytes_read += bytes as u64;
        Poll::Ready(Ok(bytes))
    }
}

impl AsyncBufRead for Body {
    #[allow(rustdoc::missing_doc_code_examples)]
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&'_ [u8]>> {
        self.project().reader.poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        Pin::new(&mut self.reader).consume(amt)
    }
}

/// Look at first few bytes of a file to determine the mime type.
/// This is used for various binary formats such as images and videos.
#[cfg(all(feature = "fs", not(target_os = "unknown")))]
async fn peek_mime(file: &mut async_std::fs::File) -> io::Result<Option<Mime>> {
    // We need to read the first 300 bytes to correctly infer formats such as tar.
    let mut buf = [0_u8; 300];
    file.read(&mut buf).await?;
    let mime = Mime::sniff(&buf).ok();

    // Reset the file cursor back to the start.
    file.seek(io::SeekFrom::Start(0)).await?;
    Ok(mime)
}

/// Look at the extension of a file to determine the mime type.
/// This is useful for plain-text formats such as HTML and CSS.
#[cfg(all(feature = "fs", not(target_os = "unknown")))]
fn guess_ext(path: &std::path::Path) -> Option<Mime> {
    let ext = path.extension().map(|p| p.to_str()).flatten();
    ext.and_then(Mime::from_extension)
}

#[cfg(test)]
mod test {
    use super::*;
    use async_std::io::Cursor;
    use serde_crate::Deserialize;

    #[async_std::test]
    async fn json_status() {
        #[derive(Debug, Deserialize)]
        #[serde(crate = "serde_crate")]
        struct Foo {
            #[allow(dead_code)]
            inner: String,
        }
        let body = Body::empty();
        let res = body.into_json::<Foo>().await;
        assert_eq!(res.unwrap_err().status(), 422);
    }

    #[async_std::test]
    async fn form_status() {
        #[derive(Debug, Deserialize)]
        #[serde(crate = "serde_crate")]
        struct Foo {
            #[allow(dead_code)]
            inner: String,
        }
        let body = Body::empty();
        let res = body.into_form::<Foo>().await;
        assert_eq!(res.unwrap_err().status(), 422);
    }

    async fn read_with_buffers_of_size<R>(reader: &mut R, size: usize) -> crate::Result<String>
    where
        R: AsyncRead + Unpin,
    {
        let mut return_buffer = vec![];
        loop {
            let mut buf = vec![0; size];
            match reader.read(&mut buf).await? {
                0 => break Ok(String::from_utf8(return_buffer)?),
                bytes_read => return_buffer.extend_from_slice(&buf[..bytes_read]),
            }
        }
    }

    #[async_std::test]
    async fn attempting_to_read_past_length() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from_reader(Cursor::new("hello world"), Some(5));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello"
            );
            assert_eq!(body.bytes_read, 5);
        }

        Ok(())
    }

    #[async_std::test]
    async fn attempting_to_read_when_length_is_greater_than_content() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from_reader(Cursor::new("hello world"), Some(15));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn attempting_to_read_when_length_is_exactly_right() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from_reader(Cursor::new("hello world"), Some(11));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn reading_in_various_buffer_lengths_when_there_is_no_length() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from_reader(Cursor::new("hello world"), None);
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_strings() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from("hello ").chain(Body::from("world"));
            assert_eq!(body.len(), Some(11));
            assert_eq!(body.mime(), Some(&mime::PLAIN));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_mixed_bytes_string() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from(&b"hello "[..]).chain(Body::from("world"));
            assert_eq!(body.len(), Some(11));
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_mixed_reader_string() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body =
                Body::from_reader(Cursor::new("hello "), Some(6)).chain(Body::from("world"));
            assert_eq!(body.len(), Some(11));
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_mixed_nolen_len() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body =
                Body::from_reader(Cursor::new("hello "), None).chain(Body::from("world"));
            assert_eq!(body.len(), None);
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_mixed_len_nolen() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body =
                Body::from("hello ").chain(Body::from_reader(Cursor::new("world"), None));
            assert_eq!(body.len(), None);
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_short() -> crate::Result<()> {
        for buf_len in 1..26 {
            let mut body = Body::from_reader(Cursor::new("hello xyz"), Some(6))
                .chain(Body::from_reader(Cursor::new("world abc"), Some(5)));
            assert_eq!(body.len(), Some(11));
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_many() -> crate::Result<()> {
        for buf_len in 1..13 {
            let mut body = Body::from("hello")
                .chain(Body::from(&b" "[..]))
                .chain(Body::from("world"));
            assert_eq!(body.len(), Some(11));
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }

    #[async_std::test]
    async fn chain_skip_start() -> crate::Result<()> {
        for buf_len in 1..26 {
            let mut body1 = Body::from_reader(Cursor::new("1234 hello xyz"), Some(11));
            let mut buf = vec![0; 5];
            body1.read(&mut buf).await?;
            assert_eq!(buf, b"1234 ");

            let mut body2 = Body::from_reader(Cursor::new("321 world abc"), Some(9));
            let mut buf = vec![0; 4];
            body2.read(&mut buf).await?;
            assert_eq!(buf, b"321 ");

            let mut body = body1.chain(body2);
            assert_eq!(body.len(), Some(11));
            assert_eq!(body.mime(), Some(&mime::BYTE_STREAM));
            assert_eq!(
                read_with_buffers_of_size(&mut body, buf_len).await?,
                "hello world"
            );
            assert_eq!(body.bytes_read, 11);
        }

        Ok(())
    }
}
