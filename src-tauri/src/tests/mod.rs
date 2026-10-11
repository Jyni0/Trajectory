//! Rust unit & integration tests.
//!
//! Network protocols are tested against in-process scripted mock servers
//! (`mock_servers`) and `wiremock` (REST APIs), so the suite runs offline.

mod api_mock_tests;
mod autodetect_tests;
mod bimi_tests;
mod db_tests;
mod fixtures;
mod imap_mock_tests;
mod mime_tests;
mod mock_servers;
mod oauth_tests;
mod pop3_mock_tests;
mod service_tests;
mod smtp_mock_tests;
mod tags_tests;
mod thread_tests;
mod updater_tests;

use std::sync::{Arc, Mutex};

use crate::db::Database;
use crate::secrets::MemoryStore;
use crate::service::{AppContext, EventSink};

/// Captures emitted events for assertions.
#[derive(Default)]
pub struct RecordingSink {
    pub events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl RecordingSink {
    pub fn names(&self) -> Vec<String> {
        self.events.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn find(&self, name: &str) -> Option<serde_json::Value> {
        self.events.lock().unwrap().iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.lock().unwrap().push((event.to_string(), payload));
    }
}

pub fn test_context() -> (Arc<AppContext>, Arc<RecordingSink>, Arc<MemoryStore>) {
    let sink = Arc::new(RecordingSink::default());
    let secrets = Arc::new(MemoryStore::default());
    let ctx = AppContext::new(Database::open_in_memory().unwrap(), secrets.clone(), sink.clone());
    (ctx, sink, secrets)
}
