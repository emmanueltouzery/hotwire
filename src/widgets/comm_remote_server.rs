use super::http_comm_entry::{HttpCommEntry, HttpMessageData};
use super::postgres_comm_entry::{PostgresCommEntry, PostgresMessageData};
use crate::icons::Icon;
use crate::BgFunc;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

pub trait MessageParser {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool;
    fn protocol_icon(&self) -> Icon;
    fn parse_stream(&self, stream: &[TSharkCommunication]) -> Vec<MessageData>;
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
    _http_comm_entry_components: Vec<Component<HttpCommEntry>>,
    _postgres_comm_entry_components: Vec<Component<PostgresCommEntry>>,
}

#[widget]
impl Widget for CommRemoteServer {
    fn init_view(&mut self) {
        self.refresh_comm_entries();
    }

    fn model(relm: &relm::Relm<Self>, data: CommRemoteServerData) -> Model {
        Model {
            data,
            _http_comm_entry_components: vec![],
            _postgres_comm_entry_components: vec![],
        }
    }

    fn update(&mut self, event: Msg) {}

    fn refresh_comm_entries(&mut self) {
        for child in self.widgets.comm_entries.get_children() {
            self.widgets.comm_entries.remove(&child);
        }
        let mut comm_entries_group_start_indexes = HashMap::new();
        let mut row_idx = 0;
        let mut http_components = vec![];
        let mut pg_components = vec![];
        for tcp_session in &self.model.data.tcp_sessions {
            comm_entries_group_start_indexes.insert(
                row_idx,
                format!(
                    "TCP session {}",
                    tcp_session
                        .0
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "-".to_string())
                ),
            );
            for msg in &tcp_session.1 {
                match msg {
                    MessageData::Http(http) => http_components.push(
                        self.widgets
                            .comm_entries
                            .add_widget::<HttpCommEntry>(((*http).clone())),
                    ),
                    MessageData::Postgres(pg) => pg_components.push(
                        self.widgets
                            .comm_entries
                            .add_widget::<PostgresCommEntry>((*pg).clone()),
                    ),
                };
                row_idx += 1;
            }
        }
        self.widgets
            .comm_entries
            .set_header_func(Some(Box::new(move |row, _h| {
                if let Some(group_name) =
                    comm_entries_group_start_indexes.get(&(row.get_index() as usize))
                {
                    let vbox = gtk::BoxBuilder::new()
                        .orientation(gtk::Orientation::Vertical)
                        .build();
                    vbox.add(&gtk::SeparatorBuilder::new().build());
                    let label = gtk::LabelBuilder::new()
                        .label(group_name)
                        .xalign(0.0)
                        .build();
                    label.get_style_context().add_class("tcp_session_header");
                    vbox.add(&label);
                    vbox.show_all();
                    row.set_header(Some(&vbox));
                } else {
                    row.set_header::<gtk::ListBoxRow>(None)
                }
            })));
        self.model._http_comm_entry_components = http_components;
        self.model._postgres_comm_entry_components = pg_components;
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            #[style_class="comm_remote_server_ip"]
            gtk::Label {
                label: &self.model.data.remote_ip,
                xalign: 0.0,
            },
            #[name="comm_entries"]
            gtk::ListBox {
            },
        }
    }
}
