use super::message_parser::{AnyStreamGlobals, FromToStreamGlobal, MessageParser, StreamData};
use crate::{
    message_parser::{AnyMessagesData, FromToAnyMessages},
    tshark_communication::{TSharkPacket, TcpStreamId},
};
use std::collections::HashMap;

pub fn wrap_message_parser<MP: MessageParser>(
    p: MP,
) -> impl MessageParser<StreamGlobalsType = AnyStreamGlobals, MessagesType = AnyMessagesData> {
    AnyMessageParser { p }
}

struct AnyMessageParser<MP: MessageParser> {
    p: MP,
}

impl<MP: MessageParser> AnyMessageParser<MP> {
    fn convert_stream_from_any(
        &self,
        stream: StreamData<AnyStreamGlobals, AnyMessagesData>,
    ) -> StreamData<MP::StreamGlobalsType, MP::MessagesType> {
        StreamData {
            parser_index: stream.parser_index,
            stream_globals: MP::StreamGlobalsType::extract_stream_globals(stream.stream_globals)
                .unwrap(),
            client_server: stream.client_server,
            messages: MP::MessagesType::extract_messages(stream.messages).unwrap(),
            summary_details: stream.summary_details,
        }
    }
}

impl<MP: MessageParser> AnyMessageParser<MP> {
    fn convert_stream_to_any(
        &self,
        stream: StreamData<MP::StreamGlobalsType, MP::MessagesType>,
    ) -> StreamData<AnyStreamGlobals, AnyMessagesData> {
        StreamData {
            parser_index: stream.parser_index,
            stream_globals: stream.stream_globals.to_any_stream_globals(),
            client_server: stream.client_server,
            messages: stream.messages.to_any_messages(),
            summary_details: stream.summary_details,
        }
    }
}

impl<MP: MessageParser> MessageParser for AnyMessageParser<MP> {
    type StreamGlobalsType = AnyStreamGlobals;
    type MessagesType = AnyMessagesData;

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

    fn initial_globals(&self) -> Self::StreamGlobalsType {
        self.p.initial_globals().to_any_stream_globals()
    }

    fn empty_messages_data(&self) -> AnyMessagesData {
        self.p.empty_messages_data().to_any_messages()
    }

    fn add_to_stream(
        &self,
        stream: StreamData<AnyStreamGlobals, AnyMessagesData>,
        new_packet: TSharkPacket,
    ) -> Result<StreamData<AnyStreamGlobals, AnyMessagesData>, String> {
        let s2 = self.convert_stream_from_any(stream);
        self.p
            .add_to_stream(s2, new_packet)
            .map(|s| self.convert_stream_to_any(s))
    }

    fn finish_stream(
        &self,
        stream: StreamData<AnyStreamGlobals, AnyMessagesData>,
    ) -> Result<StreamData<AnyStreamGlobals, AnyMessagesData>, String> {
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
        messages: &AnyMessagesData,
        start_idx: usize,
        item_count: usize,
    ) {
        let typed_messages = MP::MessagesType::extract_messages_ref(messages).unwrap();
        self.p
            .populate_treeview(ls, session_id, typed_messages, start_idx, item_count)
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
        messages_by_stream: &HashMap<TcpStreamId, &AnyMessagesData>,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool {
        let messages_typed = messages_by_stream
            .iter()
            .map(|(tcp, msg_any)| {
                (
                    *tcp,
                    MP::MessagesType::extract_messages_ref(*msg_any).unwrap(),
                )
            })
            .collect();
        self.p.matches_filter(filter, &messages_typed, model, iter)
    }
}
