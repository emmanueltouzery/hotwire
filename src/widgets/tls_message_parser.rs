use super::message_parser::{MessageParser, StreamData};
use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::message_parser::MessageInfo;
use crate::BgFunc;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::{ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::path::PathBuf;
use std::sync::mpsc;

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct TlsMessageData;

pub struct Tls;

impl MessageParser for Tls {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.tls.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::LOCK
    }

    fn parse_stream(&self, stream: Vec<TSharkCommunication>) -> StreamData {
        let server_ip = stream
            .first()
            .as_ref()
            .unwrap()
            .source
            .layers
            .ip
            .as_ref()
            .unwrap()
            .ip_dst
            .clone();
        let server_port = stream
            .first()
            .as_ref()
            .unwrap()
            .source
            .layers
            .tcp
            .as_ref()
            .unwrap()
            .port_dst;
        StreamData {
            server_ip,
            server_port,
            messages: vec![MessageData::Tls(TlsMessageData {})],
            summary_details: None,
        }
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        let data_col = gtk::TreeViewColumnBuilder::new()
            .title("TLS")
            .expand(true)
            .resizable(true)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new()
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        data_col.pack_start(&cell_r_txt, true);
        data_col.add_attribute(&cell_r_txt, "text", 0);
        tv.append_column(&data_col);
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        gtk::ListStore::new(&[
            String::static_type(), // description
            i32::static_type(), // dummy (win has list store columns 2 & 3 hardcoded for stream & row idx)
            u32::static_type(), // stream_id
            u32::static_type(), // index of the comm in the model vector
        ])
    }

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    ) {
        ls.insert_with_values(
            None,
            &[0, 2, 3],
            &[
                &"Encrypted TLS stream".to_value(),
                &session_id.to_value(),
                &0.to_value(),
            ],
        );
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        let model_sort = gtk::TreeModelSort::new(ls);
        tv.set_model(Some(&model_sort));
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        bg_sender: mpsc::Sender<BgFunc>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)> {
        let component = Box::leak(Box::new(
            parent.add_widget::<TlsCommEntry>(TlsMessageData {}),
        ));
        Box::new(move |bg_sender, path, message_info| {
            component
                .stream()
                .emit(Msg::DisplayDetails(bg_sender, path, message_info))
        })
    }
}

pub struct Model {}

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, PathBuf, MessageInfo),
}

#[widget]
impl Widget for TlsCommEntry {
    fn model(relm: &relm::Relm<Self>, data: TlsMessageData) -> Model {
        Model {}
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Box {
            gtk::Label {
                label: "The contents of this stream are encrypted."
            }
        }
    }
}
