use chrono::{DateTime, Utc};
use http::{header::RETRY_AFTER, StatusCode};
use log::debug;
use reqwest::Client;

use crate::{
    contracts::{Envelope, Transmission, TransmissionItem},
    Result,
};

#[derive(Debug, PartialEq)]
pub enum Response {
    Success,
    Retry(Vec<Envelope>),
    Throttled(DateTime<Utc>, Vec<Envelope>),
    NoRetry,
}

/// Sends telemetry items to the server.
pub struct Transmitter {
    url: String,
    client: Client,
}

impl Transmitter {
    /// Creates a new instance of telemetry items sender.
    pub fn new(url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        Self {
            url: url.into(),
            client,
        }
    }

    /// Sends a telemetry items to the server.
    pub async fn send(&self, mut items: Vec<Envelope>) -> Result<Response> {
        let payload = serde_json::to_string(&items)?;

        let response = self.client.post(&self.url).body(payload).send().await?;
        let response = match response.status() {
            StatusCode::OK => {
                debug!("Successfully sent {} items", items.len());
                Response::Success
            }
            StatusCode::PARTIAL_CONTENT => {
                let content: Transmission = response.json().await?;
                let log_prefix = format!(
                    "Successfully sent {}/{} telemetry items",
                    content.items_accepted, content.items_received
                );
                if content.items_received == content.items_accepted {
                    debug!("{}", log_prefix);
                    Response::Success
                } else {
                    retain_retry_items(&mut items, content);
                    if items.is_empty() {
                        debug!("{}. Nothing to re-send", log_prefix);
                        Response::NoRetry
                    } else {
                        debug!("{}. Retry sending {} items", log_prefix, items.len());
                        Response::Retry(items)
                    }
                }
            }
            StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT => {
                let retry_after = response.headers().get(RETRY_AFTER).cloned();

                if let Ok(content) = response.json::<Transmission>().await {
                    retain_retry_items(&mut items, content);
                }

                if let Some(retry_after) = retry_after {
                    let retry_after = retry_after.to_str()?;
                    let retry_after = DateTime::parse_from_rfc2822(retry_after)?.with_timezone(&Utc);
                    debug!(
                        "Some items were discarded. Retry sending {} items after {}",
                        items.len(),
                        retry_after
                    );
                    Response::Throttled(retry_after, items)
                } else {
                    debug!("Some items were discarded. Retry sending {} items", items.len());
                    Response::Retry(items)
                }
            }
            StatusCode::SERVICE_UNAVAILABLE => {
                debug!("Service unavailable. Retry sending {} items", items.len());
                Response::Retry(items.to_vec())
            }
            StatusCode::INTERNAL_SERVER_ERROR => {
                if let Ok(content) = response.json::<Transmission>().await {
                    retain_retry_items(&mut items, content);
                    if items.is_empty() {
                        debug!("Service error. Nothing to re-send");
                        Response::NoRetry
                    } else {
                        debug!("Service error. Retry sending {} items", items.len());
                        Response::Retry(items)
                    }
                } else {
                    debug!("Service error. Retry sending {} items", items.len());
                    Response::Retry(items.to_vec())
                }
            }
            _ => {
                debug!(
                    "Unknown status: {}. {}. Nothing to re-send",
                    response.status(),
                    response.text().await.unwrap_or_default()
                );
                Response::NoRetry
            }
        };

        Ok(response)
    }
}

/// Filters out those telemetry items that cannot be re-sent.
fn retain_retry_items(items: &mut Vec<Envelope>, content: Transmission) {
    let mut retry_items = Vec::default();
    for error in content.errors.iter().filter(|error| can_retry_item(error)) {
        retry_items.push(items.remove(error.index - retry_items.len()));
    }

    *items = retry_items;
}

/// Determines that a telemetry item can be re-send corresponding to this submission status
/// descriptor.
fn can_retry_item(item: &TransmissionItem) -> bool {
    item.status_code == StatusCode::PARTIAL_CONTENT
        || item.status_code == StatusCode::REQUEST_TIMEOUT
        || item.status_code == StatusCode::INTERNAL_SERVER_ERROR
        || item.status_code == StatusCode::SERVICE_UNAVAILABLE
        || item.status_code == StatusCode::TOO_MANY_REQUESTS
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use chrono::TimeZone;
    use http::{Request, StatusCode};
    use http_body_util::Full;
    use hyper::body::{Bytes, Incoming};
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use serde_json::{json, Value};
    use test_case::test_case;
    use tokio::net::TcpListener;

    use super::*;

    #[test_case(items(), StatusCode::OK, None, Some(all_accepted()), Response::Success; "success")]
    #[test_case(items(), StatusCode::PARTIAL_CONTENT, None, Some(partial_some_retries()), Response::Retry(retry_items()); "partial. resend some items")]
    #[test_case(items(), StatusCode::PARTIAL_CONTENT, None, Some(partial_no_retries()), Response::NoRetry; "partial. nothing to resend")]
    #[test_case(items(), StatusCode::PARTIAL_CONTENT, None, Some(none_accepted()), Response::Retry(items()); "partial. resend everything")]
    #[test_case(items(), StatusCode::PARTIAL_CONTENT, None, Some(all_accepted()), Response::Success; "partial. everything accepted")]
    #[test_case(items(), StatusCode::BAD_REQUEST, None, None, Response::NoRetry; "bad request. no retry")]
    #[test_case(items(), StatusCode::REQUEST_TIMEOUT, None, None, Response::Retry(items()); "timeout. resend everything")]
    #[test_case(items(), StatusCode::REQUEST_TIMEOUT, Some(retry_after_str()), None, Response::Throttled(retry_after(), items()); "timeout. throttled")]
    #[test_case(items(), StatusCode::TOO_MANY_REQUESTS, None, None,Response::Retry(items()); "too many requests. no retry-after. resend everything")]
    #[test_case(items(), StatusCode::TOO_MANY_REQUESTS, Some(retry_after_str()), None, Response::Throttled(retry_after(), items()); "too many requests. retry-after. throttled")]
    #[test_case(items(), StatusCode::INTERNAL_SERVER_ERROR, None, None, Response::Retry(items()); "server error. resend everything")]
    #[test_case(items(), StatusCode::SERVICE_UNAVAILABLE, None, None, Response::Retry(items()); "service unavailable. resend everything")]
    #[test_case(items(), StatusCode::UNAUTHORIZED, None, None, Response::NoRetry; "unauthorized. no retry")]
    #[test_case(items(), StatusCode::REQUEST_TIMEOUT, None, Some(partial_some_retries()), Response::Retry(retry_items()); "timeout. resend some items")]
    #[test_case(items(), StatusCode::INTERNAL_SERVER_ERROR, None, Some(partial_some_retries()), Response::Retry(retry_items()); "server error. resend some items")]
    fn it_sends_telemetry_and_handles_server_response(
        items: Vec<Envelope>,
        status_code: StatusCode,
        retry_after: Option<&'static str>,
        body: Option<Value>,
        expected: Response,
    ) {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let url = create_server(status_code, retry_after, body);

            let transmitter = Transmitter::new(&format!("{}/track", url));

            let response = transmitter.send(items).await.unwrap();

            assert_eq!(response, expected);
        });
    }

    fn create_server(status_code: StatusCode, retry_after: Option<&'static str>, body: Option<Value>) -> String {
        // Serve a dummy port
        let addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let std_listener = std::net::TcpListener::bind(addr).expect("bind to localhost");
        std_listener
            .set_nonblocking(true)
            .expect("convert std::net::TcpListener to non-blocking");
        let listener = TcpListener::from_std(std_listener).expect("from std::net::TcpListener");
        let addr = listener.local_addr().expect("localhost local_addr");

        let retry_after = retry_after.map(ToString::to_string);
        let body = body.clone();

        let task = async move {
            let (conn, _) = listener.accept().await.expect("valid connection");
            let io = TokioIo::new(conn);

            let service = service_fn(|_req: Request<Incoming>| {
                let retry_after = retry_after.clone();
                let body = body.clone();
                async move {
                    let retry_after = retry_after;
                    let body = body;

                    let mut builder = hyper::Response::builder().status(status_code);

                    if let Some(retry_after) = retry_after {
                        builder = builder.header("Retry-After", retry_after);
                    }

                    let body = body
                        .map(move |body| Full::new(Bytes::copy_from_slice(body.to_string().as_bytes())))
                        .unwrap_or_default();

                    builder.body(body)
                }
            });

            hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
                .expect("serve connection");
        };

        tokio::spawn(task);

        format!("http://{addr}")
    }

    fn partial_no_retries() -> Value {
        json!({
            "itemsAccepted": 3,
            "itemsReceived": 5,
            "errors": [
                {
                    "index": 2,
                    "statusCode": 400,
                    "message": "Bad 1"
                },
                {
                    "index": 4,
                    "statusCode": 400,
                    "message": "Bad 2"
                },
            ],
        })
    }

    fn partial_some_retries() -> Value {
        json!({
            "itemsAccepted": 2,
            "itemsReceived": 5,
            "errors": [
                {
                    "index": 2,
                    "statusCode": 400,
                    "message": "Bad 1"
                },
                {
                    "index": 4,
                    "statusCode": 408,
                    "message": "OK Later"
                },
            ],
        })
    }

    fn none_accepted() -> Value {
        json!({
            "itemsAccepted": 0,
            "itemsReceived": 5,
            "errors": [
                {
                    "index": 0,
                    "statusCode": 500,
                    "message": "Bad 1"
                },
                {
                    "index": 1,
                    "statusCode": 500,
                    "message": "Bad 2"
                },
                {
                    "index": 2,
                    "statusCode": 500,
                    "message": "Bad 3"
                },
                {
                    "index": 3,
                    "statusCode": 500,
                    "message": "Bad 4"
                },
                {
                    "index": 4,
                    "statusCode": 500,
                    "message": "Bad 5"
                },
            ],
        })
    }

    fn all_accepted() -> Value {
        json!({
            "itemsAccepted": 5,
            "itemsReceived": 5,
            "errors": [],
        })
    }

    fn retry_after_str() -> &'static str {
        "Wed, 09 Aug 2017 23:43:57 GMT"
    }

    fn retry_after() -> DateTime<Utc> {
        Utc.ymd(2017, 8, 9).and_hms(23, 43, 57)
    }

    fn items() -> Vec<Envelope> {
        (0..5)
            .map(|i| Envelope {
                name: format!("event {}", i),
                ..Envelope::default()
            })
            .collect()
    }

    fn retry_items() -> Vec<Envelope> {
        vec![Envelope {
            name: "event 4".into(),
            ..Envelope::default()
        }]
    }
}
