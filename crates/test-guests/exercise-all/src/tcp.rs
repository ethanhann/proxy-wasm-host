//! The TCP context, which reads and changes the data in each direction.

use proxy_wasm::traits::{Context, StreamContext};
use proxy_wasm::types::{Action, PeerType};

use crate::{info, text};

pub(crate) struct Tcp {
    context_id: u32,
}

impl Tcp {
    pub(crate) fn new(context_id: u32) -> Self {
        Self { context_id }
    }
}

fn peer(peer: PeerType) -> &'static str {
    match peer {
        PeerType::Local => "Local",
        PeerType::Remote => "Remote",
        _ => "Unknown",
    }
}

impl Context for Tcp {}

impl StreamContext for Tcp {
    fn on_new_connection(&mut self) -> Action {
        info(&format!("new_connection context={}", self.context_id));
        Action::Continue
    }

    fn on_downstream_data(&mut self, data_size: usize, end_of_stream: bool) -> Action {
        let data = text(self.get_downstream_data(0, data_size));
        info(&format!(
            "downstream_data size={data_size} end={end_of_stream} data={data}"
        ));
        if data == "pause" {
            return Action::Pause;
        }
        self.set_downstream_data(0, data_size, data.to_uppercase().as_bytes());
        Action::Continue
    }

    fn on_upstream_data(&mut self, data_size: usize, end_of_stream: bool) -> Action {
        let data = text(self.get_upstream_data(0, data_size));
        info(&format!(
            "upstream_data size={data_size} end={end_of_stream} data={data}"
        ));
        self.set_upstream_data(0, data_size, data.to_uppercase().as_bytes());
        self.resume_downstream();
        self.close_upstream();
        Action::Continue
    }

    fn on_downstream_close(&mut self, peer_type: PeerType) {
        info(&format!("downstream_close peer={}", peer(peer_type)));
    }

    fn on_upstream_close(&mut self, peer_type: PeerType) {
        info(&format!("upstream_close peer={}", peer(peer_type)));
    }

    fn on_log(&mut self) {
        info(&format!("log context={}", self.context_id));
    }
}
