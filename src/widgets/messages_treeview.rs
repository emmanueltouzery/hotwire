use super::win;
use crate::message_parser::{self, AnyMessagesData, AnyStreamGlobals};
use crate::message_parser::{
    ClientServerInfo, MessageData, MessageInfo, MessageParser, StreamData,
};
use crate::search_expr;
use crate::search_expr::OperatorNegation;
use crate::tshark_communication::TcpStreamId;
use crate::widgets::comm_target_card::{CommTargetCardData, CommTargetCardKey};
use crate::win::{RefreshOngoing, RefreshRemoteIpsAndStreams};
use crate::BgFunc;
use gtk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc;
use std::time::Instant;

/// I considered making this a relm widget, but decided against it because
/// I'd have to constantly pass around (copy) StreamData objects around, these can be
/// quite large... So in the end I structured this as a series of utility functions

// when we reload treeview, tons of selection change signals
// get emitted. So while we do that we disable those.
// but in that time we still allow row selection,
// which are always explicit user clicks.
// And row activation is only active when loading.
// Selection change is more precise: also follows keyboard
// actions for instance
#[derive(Debug)]
struct TreeViewSignals {
    selection_change_signal_id: glib::SignalHandlerId,
}

type DetailsCallback = Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)>;

pub struct MessagesTreeviewState {
    comm_remote_servers_stack: gtk::Stack,
    message_treeviews: Vec<(gtk::TreeView, TreeViewSignals)>,
    details_component_emitters: Vec<DetailsCallback>,
    details_adjustments: Vec<gtk::Adjustment>,
    cur_liststore: Option<(CommTargetCardKey, gtk::ListStore)>,
}

impl MessagesTreeviewState {
    pub fn file_closed(&mut self) {
        self.cur_liststore = None;
    }
}

pub fn init_grids_and_panes(
    relm: &relm::Relm<win::Win>,
    bg_sender: &mpsc::Sender<BgFunc>,
    comm_remote_servers_stack: gtk::Stack,
) -> MessagesTreeviewState {
    let mut message_treeviews = vec![];
    let mut details_component_emitters = vec![];
    let mut details_adjustments = vec![];
    for (idx, message_parser) in win::get_message_parsers().iter().enumerate() {
        let (tv, dtl_emitter, dtl_adj) = add_message_parser_grid_and_pane(
            &comm_remote_servers_stack,
            relm,
            bg_sender,
            message_parser.as_ref(),
            idx,
        );
        message_treeviews.push(tv);
        details_component_emitters.push(dtl_emitter);
        details_adjustments.push(dtl_adj);
    }
    MessagesTreeviewState {
        comm_remote_servers_stack,
        message_treeviews,
        details_component_emitters,
        details_adjustments,
        cur_liststore: None,
    }
}

fn add_message_parser_grid_and_pane(
    comm_remote_servers_stack: &gtk::Stack,
    relm: &relm::Relm<win::Win>,
    bg_sender: &mpsc::Sender<BgFunc>,
    message_parser: &dyn MessageParser<
        StreamGlobalsType = AnyStreamGlobals,
        MessagesType = AnyMessagesData,
    >,
    mp_idx: usize,
) -> (
    (gtk::TreeView, TreeViewSignals),
    DetailsCallback,
    gtk::Adjustment,
) {
    let tv = gtk::TreeViewBuilder::new()
        .activate_on_single_click(true)
        .build();
    message_parser.prepare_treeview(&tv);

    let selection_change_signal_id = {
        let rstream = relm.stream().clone();
        let tv = tv.clone();
        tv.selection().connect_changed(move |selection| {
            if let Some((model, iter)) = selection.selected() {
                let stree = model.dynamic_cast::<gtk::TreeModelSort>().unwrap();
                let smodel = stree.model();
                match smodel.clone().dynamic_cast::<gtk::TreeModelFilter>() {
                    Ok(modelfilter) => {
                        let model = modelfilter.model().unwrap();
                        let store = model.dynamic_cast::<gtk::ListStore>().unwrap();
                        let path = stree
                            .path(&iter)
                            .and_then(|p| stree.convert_path_to_child_path(&p));
                        if let Some(childpath) =
                            path.and_then(|p| modelfilter.convert_path_to_child_path(&p))
                        {
                            row_selected(&store, &childpath, &rstream);
                        }
                    }
                    _ => {
                        let path = stree.path(&iter);
                        let store = smodel.dynamic_cast::<gtk::ListStore>().unwrap();
                        if let Some(childpath) =
                            path.and_then(|p| stree.convert_path_to_child_path(&p))
                        {
                            row_selected(&store, &childpath, &rstream);
                        }
                    }
                };
            }
        })
    };
    // let rstream2 = self.model.relm.stream().clone();
    // let st2 = store.clone();
    // let ms2 = modelsort.clone();
    // let row_activation_signal_id = tv.connect_row_activated(move |_tv, sort_path, _col| {
    //     let mpath = ms2.convert_path_to_child_path(&sort_path);
    //     if let Some(path) = mpath {
    //         Self::row_selected(&st2, &path, &rstream2);
    //     }
    // });
    // tv.block_signal(&row_activation_signal_id);

    let scroll = gtk::ScrolledWindowBuilder::new()
        .expand(true)
        .child(&tv)
        .build();
    let paned = gtk::PanedBuilder::new()
        .orientation(gtk::Orientation::Vertical)
        .build();
    paned.pack1(&scroll, true, true);

    let scroll2 = gtk::ScrolledWindowBuilder::new().margin_start(3).build();
    scroll2.set_height_request(200);

    let (child, overlay) = if message_parser.requests_details_overlay() {
        let overlay = gtk::OverlayBuilder::new().child(&scroll2).build();
        (
            overlay.clone().dynamic_cast::<gtk::Widget>().unwrap(),
            Some(overlay),
        )
    } else {
        (scroll2.clone().dynamic_cast::<gtk::Widget>().unwrap(), None)
    };
    paned.pack2(&child, false, true);
    let component_emitter = message_parser.add_details_to_scroll(
        &scroll2,
        overlay.as_ref(),
        bg_sender.clone(),
        relm.stream().clone(),
    );
    let adj = scroll2.vadjustment();

    comm_remote_servers_stack.add_named(&paned, &mp_idx.to_string());
    paned.show_all();
    (
        (
            tv,
            TreeViewSignals {
                selection_change_signal_id,
                // row_activation_signal_id,
            },
        ),
        component_emitter,
        adj,
    )
}

fn row_selected(
    store: &gtk::ListStore,
    path: &gtk::TreePath,
    rstream: &relm::StreamHandle<win::Msg>,
) {
    let iter = store.iter(path).unwrap();
    let stream_id = store.value(&iter, message_parser::TREE_STORE_STREAM_ID_COL_IDX as i32);
    let idx = store.value(
        &iter,
        message_parser::TREE_STORE_MESSAGE_INDEX_COL_IDX as i32,
    );
    rstream.emit(win::Msg::DisplayDetails(
        TcpStreamId(stream_id.get::<u32>().unwrap()),
        idx.get::<u32>().unwrap(),
    ));
}

pub fn refresh_remote_servers(
    tv_state: &mut MessagesTreeviewState,
    selected_card: Option<&CommTargetCardData>,
    streams: &HashMap<TcpStreamId, StreamData<AnyStreamGlobals, AnyMessagesData>>,
    remote_ips_streams_treeview: &gtk::TreeView,
    sidebar_selection_change_signal_id: Option<&glib::SignalHandlerId>,
    constrain_remote_ips: &[IpAddr],
    constrain_stream_ids: &[TcpStreamId],
) -> RefreshRemoteIpsAndStreams {
    setup_selection_signals(
        tv_state,
        remote_ips_streams_treeview,
        sidebar_selection_change_signal_id,
        RefreshOngoing::Yes,
    );
    if let Some(card) = selected_card.cloned() {
        let mut by_remote_ip = HashMap::new();
        let parsers = win::get_message_parsers();
        for (stream_id, messages) in streams {
            if !matches!(messages.client_server, Some(cs) if card.to_key().matches_server(cs)) {
                continue;
            }
            let allowed_all = constrain_remote_ips.is_empty() && constrain_stream_ids.is_empty();

            let allowed_ip = messages
                .client_server
                .as_ref()
                .filter(|cs| constrain_remote_ips.contains(&cs.client_ip))
                .is_some();
            let allowed_stream = constrain_stream_ids.contains(stream_id);
            let allowed = allowed_all || allowed_ip || allowed_stream;

            if !allowed {
                continue;
            }
            let remote_server_streams = by_remote_ip
                .entry(
                    messages
                        .client_server
                        .as_ref()
                        .map(|cs| cs.client_ip)
                        .unwrap_or_else(|| IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))),
                )
                .or_insert_with(Vec::new);
            remote_server_streams.push((stream_id, messages));
        }
        let mp = parsers.get(card.protocol_index).unwrap();
        tv_state
            .comm_remote_servers_stack
            .set_visible_child_name(&card.protocol_index.to_string());
        let (ref tv, ref _signals) = &tv_state.message_treeviews.get(card.protocol_index).unwrap();
        let ls = mp.get_empty_liststore();
        for tcp_sessions in by_remote_ip.values() {
            for (session_id, session) in tcp_sessions {
                let mut idx = 0;
                while idx < session.messages.len() {
                    mp.populate_treeview(&ls, **session_id, &session.messages, idx, 100);
                    idx += 100;
                    // https://developer.gnome.org/gtk3/stable/gtk3-General.html#gtk-events-pending
                    // I've had this loop last almost 3 seconds!!
                    let start = Instant::now();
                    while gtk::events_pending() {
                        gtk::main_iteration();
                        if start.elapsed().as_millis() >= 70 {
                            break;
                        }
                    }
                }
            }
        }
        mp.end_populate_treeview(tv, &ls);
        let ip_hash = by_remote_ip.keys().copied().collect::<HashSet<_>>();

        tv_state.cur_liststore = Some((card.to_key(), ls));
        return RefreshRemoteIpsAndStreams::Yes(card, ip_hash);
    }
    RefreshRemoteIpsAndStreams::No
}

pub fn refresh_remote_servers_handle_selection(
    tv_state: &MessagesTreeviewState,
    selected_card: Option<&CommTargetCardData>,
    remote_ips_streams_treeview: &gtk::TreeView,
    sidebar_selection_change_signal_id: Option<&glib::SignalHandlerId>,
) {
    setup_selection_signals(
        tv_state,
        remote_ips_streams_treeview,
        sidebar_selection_change_signal_id,
        RefreshOngoing::No,
    );
    if let Some(card) = selected_card.cloned() {
        tv_state
            .message_treeviews
            .get(card.protocol_index)
            .unwrap()
            .0
            .selection()
            .select_path(&gtk::TreePath::new_first());
    }
}

fn setup_selection_signals(
    tv_state: &MessagesTreeviewState,
    remote_ips_streams_treeview: &gtk::TreeView,
    sidebar_selection_change_signal_id: Option<&glib::SignalHandlerId>,
    refresh_ongoing: RefreshOngoing,
) {
    match refresh_ongoing {
        RefreshOngoing::Yes => {
            for (tv, signals) in &tv_state.message_treeviews {
                remote_ips_streams_treeview
                    .selection()
                    .block_signal(sidebar_selection_change_signal_id.unwrap());
                tv.selection()
                    .block_signal(&signals.selection_change_signal_id);
                // tv.unblock_signal(&signals.row_activation_signal_id);
            }
        }
        RefreshOngoing::No => {
            for (tv, signals) in &tv_state.message_treeviews {
                remote_ips_streams_treeview
                    .selection()
                    .unblock_signal(sidebar_selection_change_signal_id.as_ref().unwrap());
                tv.selection()
                    .unblock_signal(&signals.selection_change_signal_id);
                // tv.block_signal(&signals.row_activation_signal_id);
            }
        }
    }
}

/// the model may in the end by held by a TreeModelSort or a
/// TreeModelFilter. The hierarchy can be either:
/// 1. TreeModelSort / TreeModelFilter / ListStore
/// 2. TreeModelSort / ListStore
/// If the TreeModelSort is not at the toplevel, the user can't
/// sort by clicking on column headers in the GUI.
fn get_store_holding_model(
    tv_state: &MessagesTreeviewState,
    protocol_index: usize,
) -> (&gtk::TreeView, gtk::TreeModel) {
    let (ref tv, ref _signals) = tv_state.message_treeviews.get(protocol_index).unwrap();
    let model_sort = tv
        .model()
        .unwrap()
        .dynamic_cast::<gtk::TreeModelSort>()
        .unwrap();

    // does the ModelSort contain directly the ListStore?
    let store_holding_model = if model_sort.model().dynamic_cast::<gtk::ListStore>().is_ok() {
        // YES => we want to return the ModelSort
        model_sort.model()
    } else {
        // NO => it must be a ModelFilter, and the ListStore's in there, return that
        model_sort
            .model()
            .dynamic_cast::<gtk::TreeModelFilter>()
            .unwrap()
            .model()
            .unwrap()
    };
    (tv, store_holding_model)
}

fn get_messages_by_stream(
    streams: &HashMap<TcpStreamId, StreamData<AnyStreamGlobals, AnyMessagesData>>,
) -> HashMap<TcpStreamId, &AnyMessagesData> {
    streams
        .iter()
        .map(|(tcp, sd)| (*tcp, &sd.messages))
        .collect()
}

fn matches_filter(
    mp: &dyn MessageParser<StreamGlobalsType = AnyStreamGlobals, MessagesType = AnyMessagesData>,
    f: &search_expr::SearchExpr,
    streams: &HashMap<TcpStreamId, StreamData<AnyStreamGlobals, AnyMessagesData>>,
    model: &gtk::TreeModel,
    iter: &gtk::TreeIter,
) -> bool {
    match f {
        search_expr::SearchExpr::And(a, b) => {
            matches_filter(mp, a, streams, model, iter)
                && matches_filter(mp, b, streams, model, iter)
        }
        search_expr::SearchExpr::Or(a, b) => {
            matches_filter(mp, a, streams, model, iter)
                || matches_filter(mp, b, streams, model, iter)
        }
        search_expr::SearchExpr::SearchOpExpr(expr)
            if expr.op_negation == OperatorNegation::Negated =>
        {
            let msg_by_stream = get_messages_by_stream(streams);
            !mp.matches_filter(expr, &msg_by_stream, model, iter)
        }
        search_expr::SearchExpr::SearchOpExpr(expr) => {
            let msg_by_stream = get_messages_by_stream(streams);
            mp.matches_filter(expr, &msg_by_stream, model, iter)
        }
    }
}

pub fn search_text_changed(
    tv_state: &MessagesTreeviewState,
    streams: &HashMap<TcpStreamId, StreamData<AnyStreamGlobals, AnyMessagesData>>,
    protocol_index: usize,
    filter: Option<&search_expr::SearchExpr>,
) {
    let (tv, m) = get_store_holding_model(tv_state, protocol_index);
    let parsers = win::get_message_parsers();
    // compute all the row indexes to show right here. then in the callback only check the row id,
    // because i can't share the streams with the set_visible_func callback (which needs 'static lifetime)
    let mut shown = HashSet::new();
    let store = m
        .clone()
        .dynamic_cast::<gtk::ListStore>()
        .unwrap_or_else(|_| {
            m.clone()
                .dynamic_cast::<gtk::TreeModelFilter>()
                .unwrap()
                .model()
                .unwrap()
                .dynamic_cast::<gtk::ListStore>()
                .unwrap()
        });
    let mp = parsers.get(protocol_index).unwrap();
    let cur_iter_o = m.iter_first();
    if let Some(cur_iter) = cur_iter_o {
        if let Some(f) = filter {
            loop {
                if matches_filter(mp.as_ref(), f, streams, &m, &cur_iter) {
                    let stream_id = store
                        .value(
                            &cur_iter,
                            message_parser::TREE_STORE_STREAM_ID_COL_IDX as i32,
                        )
                        .get::<u32>()
                        .unwrap();
                    let idx = store
                        .value(
                            &cur_iter,
                            message_parser::TREE_STORE_MESSAGE_INDEX_COL_IDX as i32,
                        )
                        .get::<u32>()
                        .unwrap();
                    shown.insert((stream_id, idx));
                }
                if !m.iter_next(&cur_iter) {
                    break;
                }
            }
        }
    }
    let new_model_filter = gtk::TreeModelFilter::new(&store, None);
    if filter.is_some() {
        new_model_filter.set_visible_func(move |model, iter| {
            let stream_id = model
                .value(iter, message_parser::TREE_STORE_STREAM_ID_COL_IDX as i32)
                .get::<u32>()
                .unwrap();
            let idx = model
                .value(
                    iter,
                    message_parser::TREE_STORE_MESSAGE_INDEX_COL_IDX as i32,
                )
                .get::<u32>()
                .unwrap();
            shown.contains(&(stream_id, idx))
        });
    }
    let previous_sort = tv
        .model()
        .unwrap()
        .dynamic_cast::<gtk::TreeModelSort>()
        .unwrap()
        .sort_column_id();
    let new_sort = gtk::TreeModelSort::new(&new_model_filter);
    if let Some((col, typ)) = previous_sort {
        new_sort.set_sort_column_id(col, typ);
    }
    tv.set_model(Some(&new_sort));
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum FollowPackets {
    Follow,
    DontFollow,
}

pub fn refresh_grids_new_messages(
    tv_state: &mut MessagesTreeviewState,
    rstream: &relm::StreamHandle<win::Msg>,
    selected_card: Option<CommTargetCardData>,
    stream_id: TcpStreamId,
    message_count_before: usize,
    stream_data: &StreamData<AnyStreamGlobals, AnyMessagesData>,
    follow_packets: FollowPackets,
) {
    let parsers = win::get_message_parsers();
    let parser = parsers.get(stream_data.parser_index).unwrap();
    let added_messages = stream_data.messages.len() - message_count_before;
    // self.refresh_comm_targets();

    // self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
    if let (Some(client_server), Some(card)) = (stream_data.client_server, selected_card) {
        if client_server.server_ip == card.ip
            && client_server.server_port == card.port
            && stream_data.parser_index == card.protocol_index
        {
            let ls = tv_state
                .cur_liststore
                .as_ref()
                .filter(|(c, _s)| {
                    c.ip == card.ip
                        && c.port == card.port
                        && c.protocol_index == card.protocol_index
                })
                .map(|(_c, s)| s.clone())
                .unwrap_or_else(|| {
                    let key = card.to_key();
                    let ls = parser.get_empty_liststore();
                    tv_state.cur_liststore = Some((key, ls.clone()));
                    let (ref tv, ref _signals) =
                        &tv_state.message_treeviews.get(card.protocol_index).unwrap();
                    parser.end_populate_treeview(tv, &ls);
                    ls
                });
            // refresh_remote_ips_streams_tree() // <------
            parser.populate_treeview(
                &ls,
                stream_id,
                &stream_data.messages,
                stream_data.messages.len() - added_messages,
                added_messages,
            );

            packets_added_trigger_events(
                tv_state,
                stream_data,
                rstream,
                added_messages,
                follow_packets,
            );
        }
    }
}

fn packets_added_trigger_events(
    tv_state: &MessagesTreeviewState,
    stream_data: &StreamData<AnyStreamGlobals, AnyMessagesData>,
    rstream: &relm::StreamHandle<win::Msg>,
    added_messages: usize,
    follow_packets: FollowPackets,
) {
    if follow_packets == FollowPackets::Follow {
        // we're capturing network traffic. scroll to
        // reveal new packets -- but schedule it when the
        // GUI thread will be idle, so it runs when the
        // items will be added, now would be too early
        let stack = tv_state.comm_remote_servers_stack.clone();
        glib::idle_add_local(move || {
            let scrolledwindow = stack
                .visible_child()
                .unwrap()
                .dynamic_cast::<gtk::Paned>()
                .unwrap()
                .child1()
                .unwrap()
                .dynamic_cast::<gtk::ScrolledWindow>()
                .unwrap();
            let vadj = scrolledwindow.vadjustment();
            // new packets were added to the view,
            // => scroll to reveal new packets
            vadj.set_value(vadj.upper());
            glib::Continue(false)
        });
    }

    if stream_data.messages.len() == added_messages {
        // just added the first rows to the grid. select the first row.
        tv_state
            .message_treeviews
            .get(stream_data.parser_index)
            .unwrap()
            .0
            .selection()
            .select_path(&gtk::TreePath::new_first());

        rstream.emit(win::Msg::OpenFileFirstPacketDisplayed);
    }
}

pub fn handle_display_details(
    state: &MessagesTreeviewState,
    bg_sender: &mpsc::Sender<BgFunc>,
    stream_id: TcpStreamId,
    stream_client_server: &Option<ClientServerInfo>,
    msg_data: &MessageData,
) {
    for adj in &state.details_adjustments {
        adj.set_value(0.0);
    }
    for component_emitter in &state.details_component_emitters {
        component_emitter(
            bg_sender.clone(),
            MessageInfo {
                stream_id,
                client_ip: stream_client_server.as_ref().unwrap().client_ip,
                message_data: msg_data.clone(),
            },
        );
    }
}
