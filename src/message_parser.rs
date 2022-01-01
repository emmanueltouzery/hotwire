use crate::http::http_message_parser::HttpMessageData;
use crate::http::http_message_parser::HttpStreamGlobals;
use crate::http2::http2_message_parser::Http2StreamGlobals;
use crate::icons::Icon;
use crate::pgsql::postgres_message_parser::PostgresMessageData;
use crate::pgsql::postgres_message_parser::PostgresStreamGlobals;
use crate::search_expr;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use gtk::prelude::*;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc;

// there's a bit of a circular dependency problem here, with
// message parsers depending on this file, and this file depending on them...
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessagesData {
    Http(Vec<HttpMessageData>),
    Postgres(Vec<PostgresMessageData>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageData {
    Http(HttpMessageData),
    Postgres(PostgresMessageData),
}

impl MessagesData {
    pub fn len(&self) -> usize {
        match &self {
            MessagesData::Http(x) => x.len(),
            MessagesData::Postgres(x) => x.len(),
        }
    }

    pub fn get(&self, idx: usize) -> Option<MessageData> {
        match &self {
            MessagesData::Http(x) => x.get(idx).cloned().map(MessageData::Http),
            MessagesData::Postgres(x) => x.get(idx).cloned().map(MessageData::Postgres),
        }
    }

    pub fn as_http(&self) -> Option<&[HttpMessageData]> {
        match &self {
            MessagesData::Http(x) => Some(&x),
            _ => None,
        }
    }

    pub fn as_postgres(&self) -> Option<&[PostgresMessageData]> {
        match &self {
            MessagesData::Postgres(x) => Some(&x),
            _ => None,
        }
    }

    pub fn get_http(self) -> Option<Vec<HttpMessageData>> {
        match self {
            MessagesData::Http(x) => Some(x),
            _ => None,
        }
    }

    pub fn get_postgres(self) -> Option<Vec<PostgresMessageData>> {
        match self {
            MessagesData::Postgres(x) => Some(x),
            _ => None,
        }
    }
}

pub enum AnyStreamGlobals {
    Postgres(PostgresStreamGlobals),
    Http2(Http2StreamGlobals),
    Http(HttpStreamGlobals),
    None,
}

impl AnyStreamGlobals {
    pub fn extract_postgres(self) -> Option<PostgresStreamGlobals> {
        match self {
            AnyStreamGlobals::Postgres(x) => Some(x),
            _ => None,
        }
    }

    pub fn extract_http(self) -> Option<HttpStreamGlobals> {
        match self {
            AnyStreamGlobals::Http(x) => Some(x),
            _ => None,
        }
    }

    pub fn extract_http2(self) -> Option<Http2StreamGlobals> {
        match self {
            AnyStreamGlobals::Http2(x) => Some(x),
            _ => None,
        }
    }
}

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

pub struct StreamData<G> {
    pub parser_index: usize,
    pub stream_globals: G,
    pub client_server: Option<ClientServerInfo>,
    pub messages: MessagesData,
    pub summary_details: Option<String>,
}

pub const TREE_STORE_STREAM_ID_COL_IDX: u32 = 2;
pub const TREE_STORE_MESSAGE_INDEX_COL_IDX: u32 = 3;

/// A MessageParser allows hotwire to parse & display messages related to
/// a certain protocol, for instance HTTP. The message parser deals with
/// parsing packets as well as displaying them.
///
/// The first step is to take TSharkPackets and build a StreamData from it.
/// The StreamData contains among other things a list of messages, which
/// the parser builds from the packets. Conceptually the MessageParser
/// trait "should" have two associated types: one for a StreamGlobals type
/// specific for the parser, and one for a MessageData type specific
/// for the parser.
/// This was attempted in the 'better_types' branch, but it complicated
/// many things, because then it was not possible to iterate over parsers
/// or over messages from different parsers.
///
/// Instead we now have a "workaround" where MessageData and StreamGlobals
/// are enums and basically each parser puts and expects to find its own
/// type. Hopefully a better solution can be found in the future.
///
/// So, one TSharkPacket may cause you to create zero, one, or multiple
/// MessageData messages. You also must try to populate the
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
pub trait MessageParser {
    type StreamGlobalsType;

    fn is_my_message(&self, msg: &TSharkPacket) -> bool;

    /// by restricting tshark to only the packets we can decode,
    /// we can possibly speed up things... so tell me how to filter
    /// for your protocol (for instance 'http2', 'pgsql' and so on)
    fn tshark_filter_string(&self) -> &'static str;

    fn protocol_icon(&self) -> Icon;

    fn protocol_name(&self) -> &'static str;

    fn to_any_stream_globals(&self, g: Self::StreamGlobalsType) -> AnyStreamGlobals;
    fn extract_stream_globals(&self, g: AnyStreamGlobals) -> Option<Self::StreamGlobalsType>;

    // parsing
    fn initial_globals(&self) -> Self::StreamGlobalsType;
    fn empty_messages_data(&self) -> MessagesData;

    fn add_to_stream(
        &self,
        stream: StreamData<Self::StreamGlobalsType>,
        new_packet: TSharkPacket,
    ) -> Result<StreamData<Self::StreamGlobalsType>, String>;
    fn finish_stream(
        &self,
        stream: StreamData<Self::StreamGlobalsType>,
    ) -> Result<StreamData<Self::StreamGlobalsType>, String>;

    // treeview
    fn prepare_treeview(&self, tv: &gtk::TreeView);
    fn get_empty_liststore(&self) -> gtk::ListStore;
    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        messages: &MessagesData,
        start_idx: usize,
        item_count: usize,
    );
    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore);

    // details
    fn requests_details_overlay(&self) -> bool;
    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)>;

    // search
    fn supported_filter_keys(&self) -> &'static [&'static str];
    fn matches_filter(
        &self,
        filter: &search_expr::SearchOpExpr,
        messages_by_stream: &HashMap<TcpStreamId, &MessagesData>,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool;
}

#[derive(Debug)]
pub struct MessageInfo {
    pub stream_id: TcpStreamId,
    pub client_ip: IpAddr,
    pub message_data: MessageData,
}

pub fn get_message<'a, 'b>(
    messages_by_stream: &'a HashMap<TcpStreamId, &MessagesData>,
    model: &'b gtk::TreeModel,
    iter: &'b gtk::TreeIter,
) -> Option<(usize, &'a MessagesData)> {
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
    messages_by_stream
        .get(&stream_id)
        .map(|m| (idx as usize, *m))
}
