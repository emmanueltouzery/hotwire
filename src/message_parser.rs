use crate::icons::Icon;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpStreamId};
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::StreamGlobals;
use crate::widgets::win;
use crate::BgFunc;
use std::net::IpAddr;
use std::sync::mpsc;

#[derive(Copy, Clone)]
pub struct ClientServerInfo {
    // need to say who is the server. i have 50:50 chance
    // that the first message that was capture is from the
    // client contacting the server, or the server responding
    // to the client
    pub server_ip: IpAddr,
    pub server_port: NetworkPort,
    pub client_ip: IpAddr,
}

pub struct StreamData {
    pub parser_index: usize,
    pub stream_globals: StreamGlobals,
    pub client_server: Option<ClientServerInfo>,
    pub messages: Vec<MessageData>,
    pub summary_details: Option<String>,
}

pub trait MessageParser {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool;
    fn protocol_icon(&self) -> Icon;
    fn initial_globals(&self) -> StreamGlobals;
    fn add_to_stream(
        &self,
        stream: StreamData,
        new_packet: TSharkPacket,
    ) -> Result<StreamData, String>;
    fn finish_stream(&self, stream: StreamData) -> Result<StreamData, String>;
    fn prepare_treeview(&self, tv: &gtk::TreeView);
    fn get_empty_liststore(&self) -> gtk::ListStore;
    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        messages: &[MessageData],
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
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)>;
}

#[derive(Debug)]
pub struct MessageInfo {
    pub stream_id: TcpStreamId,
    pub client_ip: IpAddr,
    pub message_data: MessageData,
}
