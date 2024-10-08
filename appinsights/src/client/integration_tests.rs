use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::{DateTime, Utc};
use http_body_util::Full;
use hyper::{
    body::{Bytes, Incoming},
    service::Service,
    Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use lazy_static::lazy_static;
use matches::assert_matches;
use parking_lot::Mutex;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{
    mpsc::{self, Receiver},
    oneshot,
};

use crate::{timeout, TelemetryClient, TelemetryConfig};

lazy_static! {
    /// A global lock since most tests need to run in serial.
    static ref SERIAL_TEST_MUTEX: Mutex<()> = Mutex::new(());
}

macro_rules! manual_timeout_test {
    (async fn $name: ident() $body: block) => {
        #[test]
        fn $name() {
            let _guard = SERIAL_TEST_MUTEX.lock();

            let rt = tokio::runtime::Runtime::new().expect("runtime");
            rt.block_on(async {
                timeout::init();

                $body;

                timeout::reset();
            });
        }
    };
}

manual_timeout_test! {
    async fn it_sends_one_telemetry_item() {
        let mut server = server().status(StatusCode::OK).create();

        let client = create_client(server.url());
        client.track_event("--event--");

        timeout::expire();

        // expect one requests available so far
        assert_matches!(server.next_request_timeout().await, Ok(_));

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_does_not_resend_submitted_telemetry_items() {
        let mut server = server().status(StatusCode::OK).create();

        let client = create_client(server.url());
        client.track_event("--event--");

        // verify 1 items is sent after first interval expired

        // "wait" until interval expired
        timeout::expire();
        assert_matches!(server.next_request_timeout().await, Ok(_));

        // verify no items is sent after next interval expired
        timeout::expire();
        assert_matches!(
            server.next_request_timeout().await,
            Err(RecvTimeoutError::Timeout)
        );

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_sends_telemetry_items_in_several_batches() {
        let mut server = server().status(StatusCode::OK).status(StatusCode::OK).create();

        let client = create_client(server.url());

        // send 10 items and then interval expired
        for i in 0..10 {
            client.track_event(format!("--event {}--", i));
        }

        // "wait" until interval expired
        timeout::expire();

        // send next 5 items and then interval expired
        for i in 10..15 {
            client.track_event(format!("--event {}--", i));
        }

        // "wait" until next interval expired
        timeout::expire();

        // verify that all items were send
        let requests = server.wait_for_requests(2).await;
        let content = requests.into_iter().fold(String::new(), |mut content, body| {
            content.push_str(&body);
            content
        });
        let items_count = (0..15)
            .filter(|i| content.contains(&format!("--event {}--", i)))
            .count();
        assert_eq!(items_count, 15);

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_flushes_all_pending_telemetry_items() {
        let mut server = server().status(StatusCode::OK).status(StatusCode::OK).create();

        let client = create_client(server.url());

        // send 15 items and then interval expired
        for i in 0..15 {
            client.track_event(format!("--event {}--", i));
        }

        // force client to send all items to the server
        client.flush_channel();

        // NOTE no timeout expired
        // assert that 1 request has been sent
        let requests = server.wait_for_requests(1).await;
        assert_eq!(requests.len(), 1);

        // verify request contains all items we submitted to the client
        let content = requests.into_iter().fold(String::new(), |mut content, body| {
            content.push_str(&body);
            content
        });
        let items_count = (0..15)
            .filter(|i| content.contains(&format!("--event {}--", i)))
            .count();
        assert_eq!(items_count, 15);

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_does_not_send_any_pending_telemetry_items_when_drop_client() {
        let mut server = server().status(StatusCode::OK).status(StatusCode::OK).create();

        let client = create_client(server.url());

        // send 15 items and then interval expired
        for i in 0..15 {
            client.track_event(format!("--event {}--", i));
        }

        // drop client
        drop(client);

        // verify that nothing has been sent to the server
        assert_matches!(
            server.next_request_timeout().await,
            Err(RecvTimeoutError::Timeout)
        );

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_tries_to_send_pending_telemetry_items_when_close_channel_requested() {
        let mut server = server().status(StatusCode::OK).status(StatusCode::OK).create();

        let client = create_client(server.url());

        // send 15 items and then interval expired
        for i in 0..15 {
            client.track_event(format!("--event {}--", i));
        }

        // close internal channel means that client will make an attempt to send telemetry items once
        // and then tear down submission flow
        client.close_channel().await;

        // NOTE no timeout expired
        // verify that 1 request has been sent
        let requests = server.wait_for_requests(1).await;
        assert_eq!(requests.len(), 1);

        // verify request contains all items we submitted to the client
        let content = requests.into_iter().fold(String::new(), |mut content, body| {
            content.push_str(&body);
            content
        });
        let items_count = (0..15)
            .filter(|i| content.contains(&format!("--event {}--", i)))
            .count();
        assert_eq!(items_count, 15);

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_does_not_try_to_send_pending_telemetry_items_when_client_terminated() {
        let mut server = server().status(StatusCode::OK).status(StatusCode::OK).create();

        let client = create_client(server.url());

        // send 15 items and then interval expired
        for i in 0..15 {
            client.track_event(format!("--event {}--", i));
        }

        // terminate client
        client.terminate().await;

        // NOTE no timeout expired
        // verify that no request has been sent
        let requests = server.wait_for_requests(1).await;
        assert!(requests.is_empty());

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_retries_when_previous_submission_failed() {
        let mut server = server()
            .response(StatusCode::INTERNAL_SERVER_ERROR, json!({}), None)
            .response(
                StatusCode::OK,
                json!(
                {
                    "itemsAccepted": 15,
                    "itemsReceived": 15,
                    "errors": [],
                }),
                None,
            )
            .create();

        let client = create_client(server.url());

        // send 15 items and then interval expired
        for i in 0..15 {
            client.track_event(format!("--event {}--", i));
        }

        // "wait" until interval expired
        timeout::expire();

        // "wait" until retry logic handled
        timeout::expire();

        // verify there are 2 identical requests
        let requests = server.wait_for_requests(2).await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);

        // terminate server
        server.terminate().await;
    }
}

manual_timeout_test! {
    async fn it_retries_when_partial_content() {
        let mut server = server()
            .response(
                StatusCode::PARTIAL_CONTENT,
                json!(
                {
                    "itemsAccepted": 12,
                    "itemsReceived": 15,
                    "errors": [
                        {
                            "index": 4,
                            "statusCode": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                            "message": "Internal Server Error"
                        },
                        {
                            "index": 9,
                            "statusCode": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                            "message": "Internal Server Error"
                        },
                        {
                            "index": 14,
                            "statusCode": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                            "message": "Internal Server Error"
                        }
                    ],
                }),
                None,
            )
            .response(
                StatusCode::OK,
                json!(
                {
                    "itemsAccepted": 3,
                    "itemsReceived": 3,
                    "errors": [],
                }),
                None,
            )
            .create();

        let client = create_client(server.url());

        // send 15 items and then interval expired
        for i in 0..15 {
            client.track_event(format!("--event {}--", i));
        }

        // "wait" until interval expired
        timeout::expire();

        // "wait" until retry logic handled
        timeout::expire();

        // verify it sends a first request with all items
        let requests = server.wait_for_requests(1).await;
        assert_eq!(requests.len(), 1);

        let content = requests.into_iter().fold(String::new(), |mut content, body| {
            content.push_str(&body);
            content
        });
        let items_count = (0..15)
            .filter(|i| content.contains(&format!("--event {}--", i)))
            .count();
        assert_eq!(items_count, 15);

        // verify it re-send only errors that previously were invalid
        let requests = server.wait_for_requests(1).await;
        assert_eq!(requests.len(), 1);

        let content = requests.into_iter().fold(String::new(), |mut content, body| {
            content.push_str(&body);
            content
        });
        let items_count = [4, 9, 14]
            .iter()
            .filter(|i| content.contains(&format!("--event {}--", i)))
            .count();
        assert_eq!(items_count, 3);

        // terminate server
        server.terminate().await;
    }
}

// TODO Check case when all retries exhausted. Pending items should not be lost

fn create_client(endpoint: &str) -> TelemetryClient {
    let config = TelemetryConfig::builder()
        .i_key("instrumentation key")
        .endpoint(endpoint)
        .interval(Duration::from_millis(300))
        .build();

    TelemetryClient::from_config(config)
}

fn server() -> Builder {
    Builder { responses: Vec::new() }
}

struct HyperTestServer {
    url: String,
    request_recv: Receiver<String>,
    shutdown_send: Option<oneshot::Sender<()>>,
}

impl HyperTestServer {
    fn url(&self) -> &str {
        &self.url
    }

    async fn next_request_timeout(&mut self) -> Result<String, RecvTimeoutError> {
        match tokio::time::timeout(Duration::from_millis(100), self.request_recv.recv()).await {
            Ok(Some(x)) => Ok(x),
            Ok(None) => Err(RecvTimeoutError::Disconnected),
            Err(_) => Err(RecvTimeoutError::Timeout),
        }
    }

    async fn wait_for_requests(&mut self, count: usize) -> Vec<String> {
        let mut requests = Vec::new();

        for _ in 0..count {
            match self.next_request_timeout().await {
                Result::Ok(request) => requests.push(request),
                Result::Err(err) => {
                    log::error!("{:?}", err);
                }
            }
        }

        requests
    }

    async fn terminate(mut self) {
        if let Some(shutdown) = self.shutdown_send.take() {
            shutdown.send(()).unwrap();
        }
    }
}

#[derive(Debug)]
enum RecvTimeoutError {
    Disconnected,
    Timeout,
}

struct Builder {
    responses: Vec<Response<String>>,
}

#[derive(Debug, Clone)]
struct TestServerService {
    counter: Arc<AtomicUsize>,
    requests_channel: tokio::sync::mpsc::Sender<String>,
    responses: Arc<Vec<Response<String>>>,
}

impl Service<Request<Incoming>> for TestServerService {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, request: Request<Incoming>) -> Self::Future {
        let sender = self.requests_channel.clone();
        let count = self.counter.fetch_add(1, Ordering::AcqRel);
        let response = self.responses.get(count).cloned();

        Box::pin(async move {
            // Read the request body
            // Then send into into the shared channel
            use http_body_util::BodyExt;
            let body = request.into_body().collect().await.expect("reading body");
            let slice = body.to_bytes().to_vec();
            let body = String::from_utf8_lossy(slice.as_slice()).to_string();
            sender.send(body).await.expect("send body into requests channel");

            let response = match response {
                Some(response) => {
                    let bytes = response.body().as_bytes();
                    Response::builder()
                        .status(response.status())
                        .body(Full::new(Bytes::copy_from_slice(bytes)))
                        .unwrap()
                }
                None => Response::builder().body(Full::new(Bytes::new())).unwrap(),
            };
            Ok(response)
        })
    }
}

impl Builder {
    fn response(mut self, status: StatusCode, body: impl ToString, retry_after: Option<DateTime<Utc>>) -> Self {
        let mut builder = Response::builder().status(status);

        if let Some(retry_after) = retry_after {
            let retry_after = retry_after.to_rfc2822();
            builder = builder.header("Retry-After", retry_after);
        }

        let response = builder.body(body.to_string()).unwrap();
        self.responses.push(response);

        self
    }

    fn status(self, status: StatusCode) -> Self {
        self.response(
            status,
            json!(
            {
                "itemsAccepted": 1,
                "itemsReceived": 1,
                "errors": [],
            }),
            None,
        )
    }

    fn create(self) -> HyperTestServer {
        let (shutdown_send, shutdown_recv) = oneshot::channel();
        let (request_sender, request_receiver) = mpsc::channel(100);

        let responses = Arc::new(self.responses);
        let counter = Arc::new(AtomicUsize::new(0));

        let shutdown = graceful_shutdown::Shutdown::new();
        tokio::spawn(shutdown.shutdown_after(shutdown_recv));

        let addr = {
            let addr = SocketAddr::from(([0, 0, 0, 0], 0));
            let std_listener = std::net::TcpListener::bind(addr).expect("bind to localhost");
            std_listener
                .set_nonblocking(true)
                .expect("convert std::net::TcpListener to non-blocking");
            let listener = TcpListener::from_std(std_listener).expect("from std::net::TcpListener");
            let addr = listener.local_addr().expect("localhost local_addr");

            tokio::spawn(async move {
                // Initialize the service that will be cloned between each served connection,
                // effectively allowing us shared state access in our handler.
                let service = TestServerService {
                    counter: counter.clone(),
                    requests_channel: request_sender.clone(),
                    responses: responses.clone(),
                };

                loop {
                    let stream = match shutdown.cancel_on_shutdown(listener.accept()).await {
                        Some(Ok((conn, _))) => conn,
                        Some(Err(_)) => break,
                        None => break,
                    };
                    let io = TokioIo::new(stream);
                    let service = service.clone();

                    tokio::spawn(async move {
                        hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service)
                            .await
                            .expect("serve local connection");
                    });
                }
            });

            addr
        };

        let url = format!("http://{}", addr);

        HyperTestServer {
            url,
            request_recv: request_receiver,
            shutdown_send: Some(shutdown_send),
        }
    }
}
