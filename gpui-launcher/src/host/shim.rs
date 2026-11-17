//! The launcher interfaces used by local hosting and sharing: the data folder,
//! progress events, and the shared HostState.
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct AppHandle {
    data: PathBuf,
    host: Arc<super::hosted::HostState>,
    /// Latest payload per event name ("hosted-install-progress", "share-install-progress").
    pub events: Arc<Mutex<HashMap<String, Value>>>,
}

impl AppHandle {
    pub fn new(data: PathBuf) -> Self {
        Self { data, host: Arc::new(super::hosted::HostState::default()), events: Arc::new(Mutex::new(HashMap::new())) }
    }
    pub fn event(&self, name: &str) -> Option<Value> {
        self.events.lock().ok()?.get(name).cloned()
    }
    pub fn clear_event(&self, name: &str) {
        if let Ok(mut e) = self.events.lock() {
            e.remove(name);
        }
    }
    pub fn host(&self) -> &super::hosted::HostState {
        &self.host
    }
}

pub trait Emitter {
    fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String>;
}

impl Emitter for AppHandle {
    fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
        let v = serde_json::to_value(payload).map_err(|e| e.to_string())?;
        self.events.lock().map_err(|e| e.to_string())?.insert(event.to_string(), v);
        Ok(())
    }
}

/// `app.state::<HostState>()`.
pub trait Manager {
    fn state<T: FromApp>(&self) -> &T;
}
pub trait FromApp {
    fn from_app(app: &AppHandle) -> &Self;
}
impl FromApp for super::hosted::HostState {
    fn from_app(app: &AppHandle) -> &Self {
        &app.host
    }
}
impl Manager for AppHandle {
    fn state<T: FromApp>(&self) -> &T {
        T::from_app(self)
    }
}

/// `store::data_dir(app)`.
pub fn data_dir(app: &AppHandle) -> PathBuf {
    app.data.clone()
}

pub use crate::store::write_json;
