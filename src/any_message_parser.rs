use super::message_parser::{AnyStreamGlobals, MessageParser, StreamData};
use crate::{
    message_parser::MessagesData,
    tshark_communication::{TSharkPacket, TcpStreamId},
};
use std::collections::HashMap;

pub fn wrap_message_parser<MP: MessageParser>(
    p: MP,
) -> impl MessageParser<StreamGlobalsType = AnyStreamGlobals> {
    AnyMessageParser { p }
}

struct AnyMessageParser<MP: MessageParser> {
    p: MP,
}

impl<MP: MessageParser> AnyMessageParser<MP> {
    fn convert_stream_from_any(
        &self,
        stream: StreamData<AnyStreamGlobals>,
    ) -> StreamData<MP::StreamGlobalsType> {
        StreamData {
            parser_index: stream.parser_index,
            stream_globals: self
                .p
                .extract_stream_globals(stream.stream_globals)
                .unwrap(),
            client_server: stream.client_server,
            messages: stream.messages,
            summary_details: stream.summary_details,
        }
    }
}

impl<MP: MessageParser> AnyMessageParser<MP> {
    fn convert_stream_to_any(
        &self,
        stream: StreamData<MP::StreamGlobalsType>,
    ) -> StreamData<AnyStreamGlobals> {
        StreamData {
            parser_index: stream.parser_index,
            stream_globals: self.p.to_any_stream_globals(stream.stream_globals),
            client_server: stream.client_server,
            messages: stream.messages,
            summary_details: stream.summary_details,
        }
    }
}

impl<MP: MessageParser> MessageParser for AnyMessageParser<MP> {
    type StreamGlobalsType = AnyStreamGlobals;

    fn is_my_message(&self, msg: &TSharkPacket) -> bool {
        self.p.is_my_message(msg)
    }

    fn tshark_filter_string(&self) -> &'static str {
        self.p.tshark_filter_string()
    }

    fn protocol_icon(&self) -> crate::icons::Icon {
        self.p.protocol_icon()
    }

    fn protocol_name(&self) -> &'static str {
        self.p.protocol_name()
    }

    fn to_any_stream_globals(&self, g: Self::StreamGlobalsType) -> AnyStreamGlobals {
        g
    }

    fn initial_globals(&self) -> Self::StreamGlobalsType {
        self.p.to_any_stream_globals(self.p.initial_globals())
    }

    fn empty_messages_data(&self) -> MessagesData {
        self.p.empty_messages_data()
    }

    fn extract_stream_globals(&self, g: AnyStreamGlobals) -> Option<Self::StreamGlobalsType> {
        Some(g)
    }

    fn add_to_stream(
        &self,
        stream: StreamData<AnyStreamGlobals>,
        new_packet: TSharkPacket,
    ) -> Result<StreamData<AnyStreamGlobals>, String> {
        let s2 = self.convert_stream_from_any(stream);
        self.p
            .add_to_stream(s2, new_packet)
            .map(|s| self.convert_stream_to_any(s))
    }

    fn finish_stream(
        &self,
        stream: StreamData<AnyStreamGlobals>,
    ) -> Result<StreamData<AnyStreamGlobals>, String> {
        self.p
            .finish_stream(self.convert_stream_from_any(stream))
            .map(|s| self.convert_stream_to_any(s))
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        self.p.prepare_treeview(tv)
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        self.p.get_empty_liststore()
    }

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        messages: &MessagesData,
        start_idx: usize,
        item_count: usize,
    ) {
        self.p
            .populate_treeview(ls, session_id, messages, start_idx, item_count)
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        self.p.end_populate_treeview(tv, ls)
    }

    fn requests_details_overlay(&self) -> bool {
        self.p.requests_details_overlay()
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: std::sync::mpsc::Sender<crate::BgFunc>,
        win_msg_sender: relm::StreamHandle<crate::widgets::win::Msg>,
    ) -> Box<dyn Fn(std::sync::mpsc::Sender<crate::BgFunc>, crate::message_parser::MessageInfo)>
    {
        self.p
            .add_details_to_scroll(parent, overlay, bg_sender, win_msg_sender)
    }

    fn supported_filter_keys(&self) -> &'static [&'static str] {
        self.p.supported_filter_keys()
    }

    fn matches_filter(
        &self,
        filter: &crate::search_expr::SearchOpExpr,
        messages_by_stream: &HashMap<TcpStreamId, &MessagesData>,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool {
        self.p
            .matches_filter(filter, &messages_by_stream, model, iter)
    }
}
