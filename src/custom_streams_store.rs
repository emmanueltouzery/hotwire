use crate::icons::Icon;
use crate::search_expr;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use gtk::prelude::*;
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

pub const TREE_STORE_STREAM_ID_COL_IDX: u32 = 2;
pub const TREE_STORE_MESSAGE_INDEX_COL_IDX: u32 = 3;

/// A custom streams store parses, stores & displays messages related to
/// a certain protocol, for instance HTTP. The custom streams store deals with
/// parsing packets, storing them by TCP stream as well as displaying them.
///
/// The first step is to take TSharkPackets and build streams (list of
/// messages and meta-data) from it.
///
/// So, one TSharkPacket may cause you to create zero, one, or multiple
/// messages in your stream. You also must try to populate the
/// ClientServerInfo, letting hotwire know who is the server and who is
/// the client, based on your protocol knowledge.
/// Because we can parse realtime traffic that is being recorded live,
/// you don't get the list of packets all at once, rather you can store
/// your state in your 'globals' object, and you get fed new packets
/// through 'add_to_packet'. After the last packet, 'finish_stream' will
/// be called for you to clean up your state.
///
/// Then there are methods for populating the GUI treeview, the details
/// area, and others.
pub trait CustomStreamsStore {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool;

    /// by restricting tshark to only the packets we can decode,
    /// we can possibly speed up things... so tell me how to filter
    /// for your protocol (for instance 'http2', 'pgsql' and so on)
    fn tshark_filter_string(&self) -> &'static str;

    fn protocol_icon(&self) -> Icon;

    fn protocol_name(&self) -> &'static str;

    fn tcp_stream_ids(&self) -> Vec<TcpStreamId>;

    fn has_stream_id(&self, stream_id: TcpStreamId) -> bool;

    fn is_empty(&self) -> bool;

    fn stream_client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo>;

    // parsing
    fn reset(&mut self);

    fn stream_message_count(&self, stream_id: TcpStreamId) -> Option<usize>;
    fn stream_summary_details(&self, stream_id: TcpStreamId) -> Option<&str>;

    fn add_to_stream(
        &mut self,
        stream_id: TcpStreamId,
        new_packet: TSharkPacket,
    ) -> Result<Option<ClientServerInfo>, String>;
    fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String>;

    // treeview
    fn prepare_treeview(&self, tv: &gtk::TreeView);
    fn get_empty_liststore(&self) -> gtk::ListStore;
    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        start_idx: usize,
        item_count: usize,
    );
    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore);

    fn display_in_details_widget(
        &self,
        bg_sender: mpsc::Sender<BgFunc>,
        stream_id: TcpStreamId,
        msg_idx: usize,
    );

    // details
    fn requests_details_overlay(&self) -> bool;
    fn add_details_to_scroll(
        &mut self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    );

    // search
    fn supported_filter_keys(&self) -> &'static [&'static str];
    fn matches_filter(
        &self,
        filter: &search_expr::SearchOpExpr,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool;
}

pub fn get_message_helper(model: &gtk::TreeModel, iter: &gtk::TreeIter) -> (TcpStreamId, u32) {
    let stream_id = TcpStreamId(
        model
            .value(iter, TREE_STORE_STREAM_ID_COL_IDX as i32)
            .get::<u32>()
            .unwrap(),
    );
    let idx = model
        .value(iter, TREE_STORE_MESSAGE_INDEX_COL_IDX as i32)
        .get::<u32>()
        .unwrap();
    (stream_id, idx)
}
