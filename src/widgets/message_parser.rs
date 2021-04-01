use super::comm_remote_server::MessageData;
use crate::icons::Icon;
use crate::BgFunc;
use crate::TSharkCommunication;
use relm_derive::Msg;
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
    fn prepare_treeview(&self, tv: &gtk::TreeView) -> (gtk::TreeModelSort, gtk::ListStore);
    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    );
    fn add_details_to_scroll(
        &self,
        paned: &gtk::ScrolledWindow,
        bg_sender: mpsc::Sender<BgFunc>,
    ) -> relm::StreamHandle<MessageParserDetailsMsg>;
}

#[derive(Debug)]
pub struct MessageInfo {
    pub stream_id: u32,
    pub client_ip: String,
    pub message_data: MessageData,
}

#[derive(Msg, Debug)]
pub enum MessageParserDetailsMsg {
    DisplayDetails(mpsc::Sender<BgFunc>, PathBuf, MessageInfo),

    GotImage(Vec<u8>), // TODO this http-specific...
}