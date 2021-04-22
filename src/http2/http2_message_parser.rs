use crate::http::http_message_parser;
use crate::icons;
use crate::message_parser::{MessageInfo, MessageParser, StreamData};
use crate::tshark_communication::TSharkCommunication;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use std::path::PathBuf;
use std::sync::mpsc;

pub struct Http2;

impl MessageParser for Http2 {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.http2.is_some()
    }

    fn protocol_icon(&self) -> icons::Icon {
        icons::Icon::HTTP
    }

    fn parse_stream(&self, stream: Vec<TSharkCommunication>) -> StreamData {
        let mut server_ip = stream
            .first()
            .unwrap()
            .source
            .layers
            .ip
            .as_ref()
            .unwrap()
            .ip_dst
            .clone();
        let mut server_port = stream
            .first()
            .unwrap()
            .source
            .layers
            .tcp
            .as_ref()
            .unwrap()
            .port_dst;
        let mut summary_details = None;
        let mut messages = vec![];
        for msg in stream {
            dbg!(msg.source.layers.http2);
        }
        StreamData {
            server_ip,
            server_port,
            messages,
            summary_details,
        }
    }

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    ) {
        http_message_parser::Http.populate_treeview(ls, session_id, messages, start_idx)
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)> {
        http_message_parser::Http.add_details_to_scroll(parent, overlay, bg_sender, win_msg_sender)
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        http_message_parser::Http.get_empty_liststore()
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        http_message_parser::Http.prepare_treeview(tv);
    }

    fn requests_details_overlay(&self) -> bool {
        http_message_parser::Http.requests_details_overlay()
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        http_message_parser::Http.end_populate_treeview(tv, ls);
    }
}
