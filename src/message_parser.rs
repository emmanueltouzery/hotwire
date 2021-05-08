use crate::icons::Icon;
use crate::tshark_communication::TSharkPacket;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use std::path::PathBuf;
use std::sync::mpsc;

pub struct StreamData {
    // need to say who is the server. i have 50:50 chance
    // that the first message that was capture is from the
    // client contacting the server, or the server responding
    // to the client
    pub server_ip: String,
    pub server_port: u32,
    pub client_ip: String,
    pub messages: Vec<MessageData>,
    pub summary_details: Option<String>,
}

pub trait MessageParser {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool;
    fn protocol_icon(&self) -> Icon;
    fn parse_stream(&self, stream: Vec<TSharkPacket>) -> StreamData;
    fn prepare_treeview(&self, tv: &gtk::TreeView);
    fn get_empty_liststore(&self) -> gtk::ListStore;
    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    );
    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore);
    fn requests_details_overlay(&self) -> bool;
    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)>;
}

#[derive(Debug)]
pub struct MessageInfo {
    pub stream_id: u32,
    pub client_ip: String,
    pub message_data: MessageData,
}
