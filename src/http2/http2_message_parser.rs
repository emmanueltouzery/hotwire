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
        todo!()
    }

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    ) {
        todo!()
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)> {
        todo!()
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        todo!()
    }
    fn prepare_treeview(&self, _: &gtk::TreeView) {
        todo!()
    }
    fn requests_details_overlay(&self) -> bool {
        todo!()
    }
    fn end_populate_treeview(&self, _: &gtk::TreeView, _: &gtk::ListStore) {
        todo!()
    }
}
