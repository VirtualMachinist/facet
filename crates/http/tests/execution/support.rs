use std::{
    io,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use probe_core::HttpRequest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

#[derive(Debug)]
pub(crate) struct CapturedRequest {
    pub(crate) request_line: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl CapturedRequest {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}

pub(crate) async fn serve_once(
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> io::Result<(String, JoinHandle<io::Result<CapturedRequest>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let status = status.to_owned();
    let headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let body = body.to_vec();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let captured = read_request(&mut stream).await?;
        write_response(&mut stream, &status, &headers, &body).await?;
        Ok(captured)
    });
    Ok((format!("http://{address}"), handle))
}

pub(crate) async fn read_request(stream: &mut TcpStream) -> io::Result<CapturedRequest> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
        read_more(stream, &mut bytes).await?;
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let headers: Vec<_> = lines
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect();
    let mut encoded_body = bytes[header_end..].to_vec();
    let body = if let Some(length) = header(&headers, "content-length") {
        let length = length.parse::<usize>().map_err(io::Error::other)?;
        while encoded_body.len() < length {
            read_more(stream, &mut encoded_body).await?;
        }
        encoded_body.truncate(length);
        encoded_body
    } else if header(&headers, "transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        decode_chunked(stream, encoded_body).await?
    } else {
        Vec::new()
    };
    Ok(CapturedRequest {
        request_line,
        headers,
        body,
    })
}

pub(crate) async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> io::Result<()> {
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("Connection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await
}

pub(crate) fn request(method: &str, url: String) -> HttpRequest {
    HttpRequest {
        method: Some(method.to_owned()),
        url: Some(url),
        ..HttpRequest::default()
    }
}

pub(crate) async fn delayed_server() -> (String, JoinHandle<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        read_request(&mut stream).await?;
        tokio::time::sleep(Duration::from_secs(10)).await;
        write_response(&mut stream, "200 OK", &[], b"late").await
    });
    (format!("http://{address}/slow"), handle)
}

pub(crate) fn temporary_path(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "probe-http-{}-{unique}-{suffix}",
        std::process::id()
    ))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

async fn decode_chunked(stream: &mut TcpStream, mut encoded: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut decoded = Vec::new();
    loop {
        let line_end = loop {
            if let Some(position) = find_bytes(&encoded, b"\r\n") {
                break position;
            }
            read_more(stream, &mut encoded).await?;
        };
        let line = String::from_utf8_lossy(&encoded[..line_end]);
        let size_text = line.split(';').next().unwrap_or_default();
        let size = usize::from_str_radix(size_text.trim(), 16).map_err(io::Error::other)?;
        encoded.drain(..line_end + 2);
        if size == 0 {
            return Ok(decoded);
        }
        while encoded.len() < size + 2 {
            read_more(stream, &mut encoded).await?;
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded.drain(..size + 2);
    }
}

async fn read_more(stream: &mut TcpStream, bytes: &mut Vec<u8>) -> io::Result<()> {
    let mut buffer = [0_u8; 8 * 1024];
    let count = stream.read(&mut buffer).await?;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection closed while reading request",
        ));
    }
    bytes.extend_from_slice(&buffer[..count]);
    Ok(())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
