mod logger;

use std::{
    env,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use appinsights::{telemetry::SeverityLevel, TelemetryClient};
use hyper::Uri;

async fn panicking(message: &'static str) {
    panic!("{}", message);
}

#[tokio::test]
async fn it_tracks_all_telemetry_items() {
    let entries = Arc::new(RwLock::new(Vec::new()));
    logger::builder(entries.clone()).output(true).init();

    let i_key = env::var("APPINSIGHTS_INSTRUMENTATIONKEY").expect("Set APPINSIGHTS_INSTRUMENTATIONKEY first");
    let ai = Arc::new(Mutex::new(TelemetryClient::new(i_key)));

    ai.lock().unwrap().track_event("event happened");
    ai.lock()
        .unwrap()
        .track_trace("Unable to connect to a gateway", SeverityLevel::Warning);
    ai.lock().unwrap().track_metric("gateway_latency_ms", 113.0);
    ai.lock().unwrap().track_request(
        "GET /dmolokanov/appinsights-rs".to_string(),
        "https://api.github.com/dmolokanov/appinsights-rs"
            .parse::<Uri>()
            .unwrap(),
        Duration::from_millis(100),
        "200".to_string(),
    );
    ai.lock().unwrap().track_remote_dependency(
        "GET https://api.github.com/dmolokanov/appinsights-rs",
        "HTTP",
        "api.github.com",
        true,
    );
    ai.lock().unwrap().track_availability(
        "GET https://api.github.com/dmolokanov/appinsights-rs",
        Duration::from_secs(2),
        true,
    );

    let weak = Arc::downgrade(&ai);
    std::panic::set_hook(Box::new(move |info| {
        let exception_type = "Panic";
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let location = info.location().unwrap();
        let problem_id = format!("{}:{}", exception_type, location);
        let message = if let Some(str_message) = info.payload().downcast_ref::<&str>() {
            str_message.to_string()
        } else if let Some(string_message) = info.payload().downcast_ref::<String>() {
            string_message.to_owned()
        } else {
            "couldn't parse panic message".to_string()
        };

        if let Some(ai) = weak.upgrade() {
            ai.lock().unwrap().track_exception(
                format!("Panic occurred at {}: {}", location, message),
                exception_type,
                Some(backtrace),
                Some(problem_id),
            );
        }
    }));

    let _ = tokio::spawn(async move { panicking("This task panicked!").await }).await;

    match Arc::try_unwrap(ai) {
        Ok(m) => {
            m.into_inner().unwrap().close_channel().await;
        }
        _ => {
            println!("Couldn't close channel");
        }
    }

    logger::wait_until(&entries, "Successfully sent 7 items", Duration::from_secs(10)).await;
}
