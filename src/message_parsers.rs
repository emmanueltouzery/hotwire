use std::sync::mpsc;

use crate::{
    http::http_message_parser::Http, http2::http2_message_parser::Http2, icons::Icon,
    message_parser::MessageInfo, message_parser::MessageParser,
    pgsql::postgres_message_parser::Postgres, search_expr, streams::Streams,
    tshark_communication::TcpStreamId, widgets::win, BgFunc,
};

// gonna put the concrete type here later
pub const MESSAGE_PARSERS: (Http, Postgres, Http2) = (Http, Postgres, Http2);

pub trait MessageParserList {
    // let filter = get_message_parsers()
    //     .into_iter()
    //     .map(|p| p.tshark_filter_string())
    //     .join(" || ");
    fn combine_tshark_filter_strings(&self) -> String;

    fn protocol_indices(&self) -> &[usize];

    fn supported_filter_keys(&self, protocol_index: usize) -> &'static [&'static str];

    fn protocol_icon(&self, protocol_index: usize) -> Icon;

    fn get_empty_liststore(&self, protocol_index: usize) -> gtk::ListStore;

    fn populate_treeview(
        &self,
        protocol_index: usize,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        messages: &Box<dyn Streams>,
        start_idx: usize,
        item_count: usize,
    );

    fn end_populate_treeview(&self, protocol_index: usize, tv: &gtk::TreeView, ls: &gtk::ListStore);

    fn prepare_treeview(&self, protocol_index: usize, tv: &gtk::TreeView);
    fn requests_details_overlay(&self, protocol_index: usize) -> bool;

    fn add_details_to_scroll(
        &self,
        protocol_index: usize,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)>;

    fn matches_filter(
        &self,
        protocol_index: usize,
        filter: &search_expr::SearchOpExpr,
        streams: &Box<dyn Streams>,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool;
}

// TODO the plan is to generate this implementation using a macro.
// See https://users.rust-lang.org/t/need-heterogeneous-list-thats-not-statically-typed/68961/31
impl MessageParserList for (Http, Postgres, Http2) {
    fn combine_tshark_filter_strings(&self) -> String {
        // let filter = get_message_parsers()
        //     .into_iter()
        //     .map(|p| p.tshark_filter_string())
        //     .join(" || ");
        format!(
            "{} || {} || {}",
            Http.tshark_filter_string(),
            Postgres.tshark_filter_string(),
            Http2.tshark_filter_string()
        )
    }

    fn protocol_indices(&self) -> &[usize] {
        &[0, 1, 2]
    }

    fn supported_filter_keys(&self, protocol_index: usize) -> &'static [&'static str] {
        match protocol_index {
            0 => Http.supported_filter_keys(),
            1 => Postgres.supported_filter_keys(),
            2 => Http2.supported_filter_keys(),
            _ => panic!(),
        }
    }

    fn protocol_icon(&self, protocol_index: usize) -> Icon {
        match protocol_index {
            0 => Http.protocol_icon(),
            1 => Postgres.protocol_icon(),
            2 => Http2.protocol_icon(),
            _ => panic!(),
        }
    }

    fn get_empty_liststore(&self, protocol_index: usize) -> gtk::ListStore {
        match protocol_index {
            0 => Http.get_empty_liststore(),
            1 => Postgres.get_empty_liststore(),
            2 => Http2.get_empty_liststore(),
            _ => panic!(),
        }
    }

    fn populate_treeview<T: Streams>(
        &self,
        protocol_index: usize,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        messages: &T,
        start_idx: usize,
        item_count: usize,
    ) {
        match protocol_index {
            0 => Http.populate_treeview(ls, session_id, messages, start_idx, item_count),
            1 => Postgres.populate_treeview(ls, session_id, messages, start_idx, item_count),
            2 => Http2.populate_treeview(ls, session_id, messages, start_idx, item_count),
            _ => panic!(),
        }
    }

    fn end_populate_treeview(
        &self,
        protocol_index: usize,
        tv: &gtk::TreeView,
        ls: &gtk::ListStore,
    ) {
        match protocol_index {
            0 => Http.end_populate_treeview(tv, ls),
            1 => Postgres.end_populate_treeview(tv, ls),
            2 => Http2.end_populate_treeview(tv, ls),
            _ => panic!(),
        }
    }

    fn prepare_treeview(&self, protocol_index: usize, tv: &gtk::TreeView) {
        match protocol_index {
            0 => Http.prepare_treeview(tv),
            1 => Postgres.prepare_treeview(tv),
            2 => Http2.prepare_treeview(tv),
            _ => panic!(),
        }
    }
    fn requests_details_overlay(&self, protocol_index: usize) -> bool {
        match protocol_index {
            0 => Http.requests_details_overlay(),
            1 => Http.requests_details_overlay(),
            2 => Http.requests_details_overlay(),
            _ => panic!(),
        }
    }

    fn add_details_to_scroll(
        &self,
        protocol_index: usize,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)> {
        match protocol_index {
            0 => Http.add_details_to_scroll(parent, overlay, bg_sender, win_msg_sender),
            1 => Postgres.add_details_to_scroll(parent, overlay, bg_sender, win_msg_sender),
            2 => Http2.add_details_to_scroll(parent, overlay, bg_sender, win_msg_sender),
            _ => panic!(),
        }
    }

    fn matches_filter(
        &self,
        protocol_index: usize,
        filter: &search_expr::SearchOpExpr,
        streams: &Box<dyn Streams>,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool {
        match protocol_index {
            0 => Http.matches_filter(filter, streams, model, iter),
            1 => Postgres.matches_filter(filter, streams, model, iter),
            2 => Http2.matches_filter(filter, streams, model, iter),
            _ => panic!(),
        }
    }
}
