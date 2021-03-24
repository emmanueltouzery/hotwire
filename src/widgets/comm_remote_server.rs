use super::http_comm_entry::HttpMessageData;
use super::postgres_comm_entry::PostgresMessageData;
use super::tls_comm_entry::TlsMessageData;
use crate::icons::Icon;
use crate::BgFunc;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::path::PathBuf;
use std::sync::mpsc;

pub struct StreamData {
    pub messages: Vec<MessageData>,
    pub summary_details: Option<String>,
}

pub trait MessageParser {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool;
    fn protocol_icon(&self) -> Icon;
    fn parse_stream(&self, stream: Vec<TSharkCommunication>) -> StreamData;
    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore;
    fn populate_treeview(&self, ls: &gtk::ListStore, session_id: u32, messages: &Vec<MessageData>);
    fn add_details_to_scroll(
        &self,
        paned: &gtk::ScrolledWindow,
        bg_sender: mpsc::Sender<BgFunc>,
    ) -> relm::StreamHandle<MessageParserDetailsMsg>;
}

#[derive(Msg)]
pub enum MessageParserDetailsMsg {
    DisplayDetails(mpsc::Sender<BgFunc>, PathBuf, MessageData),

    GotImage(Vec<u8>), // TODO this http-specific...
}

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
