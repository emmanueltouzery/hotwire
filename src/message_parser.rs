use crate::icons::Icon;
use crate::tshark_communication::TSharkPacket;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::StreamGlobals;
use crate::widgets::win;
use crate::BgFunc;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::mpsc;

pub struct ClientServerInfo {
    // need to say who is the server. i have 50:50 chance
    // that the first message that was capture is from the
    // client contacting the server, or the server responding
    // to the client
    pub server_ip: IpAddr,
    pub server_port: u32,
    pub client_ip: IpAddr,
}

pub struct StreamData<MessageType, StreamGlobalsType> {
    pub stream_globals: StreamGlobalsType,
    pub client_server: Option<ClientServerInfo>,
    pub messages: Vec<MessageType>,
    pub summary_details: Option<String>,
}

pub trait MessageParser {
    type MessageType;
    type StreamGlobalsType;
    fn is_my_message(&self, msg: &TSharkPacket) -> bool;
    fn protocol_icon(&self) -> Icon;
    fn initial_globals(&self) -> Self::StreamGlobalsType;
    fn add_to_stream(
        &self,
        stream: &mut StreamData<Self::MessageType, Self::StreamGlobalsType>,
        new_packet: TSharkPacket,
    ) -> Result<(), String>;
    fn prepare_treeview(&self, tv: &gtk::TreeView);
    fn get_empty_liststore(&self) -> gtk::ListStore;
    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[Self::MessageType],
        start_idx: i32,
    );
    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore);
    fn requests_details_overlay(&self) -> bool;
    fn matches_filter(&self, filter: &str, model: &gtk::TreeModel, iter: &gtk::TreeIter) -> bool;
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
    pub client_ip: IpAddr,
    pub message_data: MessageData,
}
