use warp::Filter;
use std::net::SocketAddr;
use reqwest::Client;
use std::convert::Infallible;
use warp::http::{HeaderMap, Response, StatusCode};
use futures::stream::StreamExt;
use std::sync::Arc;
use super::online_resolver::VideoPlatform;

pub async fn start_server(port: u16) {
    let client = Client::new();
    let client = Arc::new(client);

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

    let routes = proxy_route.with(cors);

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    println!("Starting video proxy server on http://{}", addr);

    warp::serve(routes).run(addr).await;
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

            // Create response builder
            let mut response_builder = Response::builder().status(status.as_u16());

            // Forward headers
            for (key, value) in headers.iter() {
                // Forward relevant headers
                if key == "content-length" || key == "content-type" || key == "content-range" || key == "accept-ranges" {
                    if let Ok(v) = value.to_str() {
                        response_builder = response_builder.header(key.as_str(), v);
                    }
                }
            }

            // Stream body
            let stream = resp.bytes_stream().map(|result| {
                result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            });
            let body = warp::hyper::Body::wrap_stream(stream);

            Ok(response_builder.body(body).unwrap())
        },
        Err(e) => {
            println!("Proxy request failed: {}", e);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(warp::hyper::Body::from("Internal Server Error"))
                .unwrap())
        }
    }
}
