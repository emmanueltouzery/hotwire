use super::http_comm_entry::HttpMessageData;
use super::postgres_comm_entry::PostgresMessageData;
use super::tls_comm_entry::TlsMessageData;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageData {
    Http(HttpMessageData),
    Postgres(PostgresMessageData),
    Tls(TlsMessageData),
}

impl MessageData {
    pub fn as_http(&self) -> Option<&HttpMessageData> {
        match &self {
            MessageData::Http(x) => Some(x),
            _ => None,
        }
    }

    pub fn as_postgres(&self) -> Option<&PostgresMessageData> {
        match &self {
            MessageData::Postgres(x) => Some(x),
            _ => None,
        }
    }
}

pub struct CommRemoteServerData {
    pub remote_ip: String,
    pub tcp_sessions: Vec<(Option<u32>, Vec<MessageData>)>,
}

pub struct Model {
    data: CommRemoteServerData,
}

#[widget]
impl Widget for CommRemoteServer {
    fn model(relm: &relm::Relm<Self>, data: CommRemoteServerData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            #[style_class="comm_remote_server_ip"]
            gtk::Label {
                label: &self.model.data.remote_ip,
                xalign: 0.0,
            },
        }
    }
}
