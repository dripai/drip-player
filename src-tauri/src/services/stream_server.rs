use super::online_resolver::VideoPlatform;
use reqwest::Client;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use warp::http::{HeaderMap, StatusCode};
use warp::{Filter, Reply};

pub async fn start_server(port: u16) {
    let routes = proxy_routes(Arc::new(Client::new()));
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    println!("Starting video proxy server on http://{}", addr);
    warp::serve(routes).run(addr).await;
}

fn proxy_routes(
    client: Arc<Client>,
) -> impl Filter<Extract = (impl Reply,), Error = warp::Rejection> + Clone {
    let client_filter = warp::any().map(move || client.clone());

    // Video proxy route (for bilibili, douyin, tencent etc)
    let proxy_route = warp::path("video_proxy")
        .and(warp::query::<ProxyParams>())
        .and(warp::header::headers_cloned())
        .and(client_filter)
        .and_then(handle_proxy);

    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["Range", "Content-Type", "User-Agent"])
        .allow_methods(vec!["GET", "HEAD", "OPTIONS"]);

    proxy_route.with(cors).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn forwards_range_and_preserves_binary_partial_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0 && request.len() < 8192);
                request.extend_from_slice(&buffer[..count]);
            }
            assert!(String::from_utf8(request)
                .unwrap()
                .to_lowercase()
                .contains("range: bytes=1-3"));
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Length: 3\r\nContent-Range: bytes 1-3/5\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n").await.unwrap();
            socket.write_all(&[0, 127, 255]).await.unwrap();
        });
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("url", &format!("http://{addr}/video"))
            .finish();
        let routes = proxy_routes(Arc::new(Client::builder().no_proxy().build().unwrap()));
        let response = timeout(
            Duration::from_secs(5),
            warp::test::request()
                .path(&format!("/video_proxy?{query}"))
                .header("Range", "bytes=1-3")
                .reply(&routes),
        )
        .await
        .unwrap();
        upstream.await.unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()["content-range"], "bytes 1-3/5");
        assert_eq!(response.headers()["accept-ranges"], "bytes");
        assert_eq!(response.headers()["content-type"], "video/mp4");
        assert_eq!(response.body().as_ref(), &[0, 127, 255]);
    }

    #[tokio::test]
    async fn upstream_disconnect_returns_an_error_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        });
        let routes = proxy_routes(Arc::new(Client::builder().no_proxy().build().unwrap()));
        let response = timeout(
            Duration::from_secs(5),
            warp::test::request()
                .path(&format!("/video_proxy?url=http://{addr}/video"))
                .reply(&routes),
        )
        .await
        .unwrap();
        upstream.await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.body().as_ref(), b"Internal Server Error");
    }
}

#[derive(serde::Deserialize)]
struct ProxyParams {
    url: String,
}

async fn handle_proxy(
    params: ProxyParams,
    headers: HeaderMap,
    client: Arc<Client>,
) -> Result<impl warp::Reply, Infallible> {
    let target_url = params.url;

    // Build request with appropriate headers based on target URL
    let mut req_builder = client.get(&target_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36");

    // Add platform-specific referer based on stream URL
    if let Some(platform) = VideoPlatform::matches_stream_url(&target_url) {
        if let Some(referer) = platform.get_referer() {
            req_builder = req_builder.header("Referer", referer);
        }
    }

    // Forward Range header
    if let Some(range) = headers.get("range") {
        if let Ok(range_str) = range.to_str() {
            req_builder = req_builder.header("Range", range_str);
        }
    }

    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();

            let mut response = warp::reply::stream(resp.bytes_stream()).into_response();
            *response.status_mut() = status;

            // Forward headers
            for (key, value) in headers.iter() {
                // Forward relevant headers
                if key == "content-length"
                    || key == "content-type"
                    || key == "content-range"
                    || key == "accept-ranges"
                {
                    response.headers_mut().insert(key.clone(), value.clone());
                }
            }

            Ok(response)
        }
        Err(e) => {
            println!("Proxy request failed: {}", e);
            Ok(
                warp::reply::with_status(
                    "Internal Server Error",
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .into_response(),
            )
        }
    }
}
