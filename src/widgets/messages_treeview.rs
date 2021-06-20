use super::win;
use crate::message_parser::ClientServerInfo;
use crate::message_parser::MessageInfo;
use crate::message_parser::MessageParser;
use crate::message_parser::StreamData;
use crate::tshark_communication::TcpStreamId;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_target_card::CommTargetCardData;
use crate::widgets::comm_target_card::CommTargetCardKey;
use crate::win::{RefreshOngoing, RefreshRemoteIpsAndStreams};
use crate::BgFunc;
use gtk::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::net::IpAddr;
use std::net::Ipv4Addr;
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
    cur_liststore: Option<(CommTargetCardKey, gtk::ListStore, i32)>,
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
            &message_parser,
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
    message_parser: &Box<dyn MessageParser>,
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
        tv.get_selection().connect_changed(move |selection| {
            if let Some((model, iter)) = selection.get_selected() {
                let (modelsort, path) = if model.is::<gtk::TreeModelFilter>() {
                    let modelfilter = model.dynamic_cast::<gtk::TreeModelFilter>().unwrap();
                    let model = modelfilter.get_model().unwrap();
                    (
                        model.dynamic_cast::<gtk::TreeModelSort>().unwrap(),
                        modelfilter
                            .get_path(&iter)
                            .and_then(|p| modelfilter.convert_path_to_child_path(&p)),
                    )
                } else {
                    let smodel = model.dynamic_cast::<gtk::TreeModelSort>().unwrap();
                    let path = smodel.get_path(&iter);
                    (smodel, path)
                };
                let model = modelsort
                    .get_model()
                    .dynamic_cast::<gtk::ListStore>()
                    .unwrap();
                if let Some(childpath) = path.and_then(|p| modelsort.convert_path_to_child_path(&p))
                {
                    row_selected(&model, &childpath, &rstream);
                }
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
    scroll2.set_property_height_request(200);

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
    let adj = scroll2.get_vadjustment().unwrap();

    comm_remote_servers_stack.add_named(&paned, &mp_idx.to_string());
    paned.show_all();
    (
        (
            tv.clone(),
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
    let iter = store.get_iter(&path).unwrap();
    let stream_id = store.get_value(&iter, 2);
    let idx = store.get_value(&iter, 3);
    println!(
        "stream: {} idx: {}",
        stream_id.get::<u32>().unwrap().unwrap(),
        idx.get::<u32>().unwrap().unwrap()
    );
    rstream.emit(win::Msg::DisplayDetails(
        TcpStreamId(stream_id.get::<u32>().unwrap().unwrap()),
        idx.get::<u32>().unwrap().unwrap(),
    ));
}

pub fn refresh_remote_servers(
    tv_state: &MessagesTreeviewState,
    win_stream: &relm::StreamHandle<win::Msg>,
    selected_card: Option<&CommTargetCardData>,
    streams: &HashMap<TcpStreamId, StreamData>,
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
        let target_ip = card.ip;
        let target_port = card.port;
        let mut by_remote_ip = HashMap::new();
        let parsers = win::get_message_parsers();
        for (stream_id, messages) in streams {
            if messages.client_server.as_ref().map(|cs| cs.server_ip) != Some(target_ip)
                || messages.client_server.as_ref().map(|cs| cs.server_port) != Some(target_port)
            {
                continue;
            }
            let allowed_all = constrain_remote_ips.is_empty() && constrain_stream_ids.is_empty();

            let allowed_ip = messages
                .client_server
                .as_ref()
                .filter(|cs| constrain_remote_ips.contains(&cs.client_ip))
                .is_some();
            let allowed_stream = constrain_stream_ids.contains(&stream_id);
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
                        .unwrap_or(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))),
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
        for (remote_ip, tcp_sessions) in &by_remote_ip {
            for (session_id, session) in tcp_sessions {
                let mut idx = 0;
                for chunk in session.messages.chunks(100) {
                    mp.populate_treeview(&ls, **session_id, chunk, idx);
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
        return RefreshRemoteIpsAndStreams::Yes(card, ip_hash);
    }
    RefreshRemoteIpsAndStreams::No
}

pub fn refresh_remote_servers_after(
    tv_state: &MessagesTreeviewState,
    selected_card: Option<&CommTargetCardData>,
    remote_ips_streams_treeview: &gtk::TreeView,
    sidebar_selection_change_signal_id: Option<&glib::SignalHandlerId>,
) {
    setup_selection_signals(
        tv_state,
        &remote_ips_streams_treeview,
        sidebar_selection_change_signal_id,
        RefreshOngoing::No,
    );
    if let Some(card) = selected_card.cloned() {
        tv_state
            .message_treeviews
            .get(card.protocol_index)
            .unwrap()
            .0
            .get_selection()
            .select_path(&gtk::TreePath::new_first());
    }
}

fn setup_selection_signals(
    tv_state: &MessagesTreeviewState,
    remote_ips_streams_treeview: &gtk::TreeView,
    sidebar_selection_change_signal_id: Option<&glib::SignalHandlerId>,
    refresh_ongoing: RefreshOngoing,
) {
    dbg!(&refresh_ongoing);
    match refresh_ongoing {
        RefreshOngoing::Yes => {
            for (tv, signals) in &tv_state.message_treeviews {
                remote_ips_streams_treeview
                    .get_selection()
                    .block_signal(sidebar_selection_change_signal_id.unwrap());
                tv.get_selection()
                    .block_signal(&signals.selection_change_signal_id);
                // tv.unblock_signal(&signals.row_activation_signal_id);
            }
        }
        RefreshOngoing::No => {
            for (tv, signals) in &tv_state.message_treeviews {
                remote_ips_streams_treeview
                    .get_selection()
                    .unblock_signal(sidebar_selection_change_signal_id.as_ref().unwrap());
                tv.get_selection()
                    .unblock_signal(&signals.selection_change_signal_id);
                // tv.block_signal(&signals.row_activation_signal_id);
            }
        }
    }
}

fn get_model_sort(
    tv_state: &MessagesTreeviewState,
    protocol_index: usize,
) -> (&gtk::TreeView, gtk::TreeModelSort) {
    let (ref tv, ref _signals) = tv_state.message_treeviews.get(protocol_index).unwrap();
    let model_sort = tv
        .get_model()
        .unwrap()
        .dynamic_cast::<gtk::TreeModelSort>()
        .unwrap_or_else(|_| {
            tv.get_model()
                .unwrap()
                .dynamic_cast::<gtk::TreeModelFilter>()
                .unwrap()
                .get_model()
                .unwrap()
                .dynamic_cast::<gtk::TreeModelSort>()
                .unwrap()
        });
    (tv, model_sort)
}

pub fn search_text_changed(tv_state: &MessagesTreeviewState, protocol_index: usize, txt: &str) {
    let (tv, model_sort) = get_model_sort(tv_state, protocol_index);
    let parsers = win::get_message_parsers();
    let new_model_filter = gtk::TreeModelFilter::new(&model_sort, None);
    let txt_string = txt.to_string();
    new_model_filter.set_visible_func(move |model, iter| {
        let mp = parsers.get(protocol_index).unwrap();
        mp.matches_filter(&txt_string, model, iter)
    });
    tv.set_model(Some(&new_model_filter));
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum FollowPackets {
    Follow,
    DontFollow,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ViewAddedInfo {
    AddedFirstMessages,
    DidntAddFirstMessages,
}

pub fn refresh_grids_new_messages(
    tv_state: &mut MessagesTreeviewState,
    selected_card: Option<CommTargetCardData>,
    stream_id: TcpStreamId,
    parser_index: usize,
    message_count_before: usize,
    stream_data: &StreamData,
    follow_packets: FollowPackets,
) -> ViewAddedInfo {
    let mut r = ViewAddedInfo::DidntAddFirstMessages;
    let parsers = win::get_message_parsers();
    let parser = parsers.get(parser_index).unwrap();
    let added_messages = stream_data.messages.len() - message_count_before;
    // self.refresh_comm_targets();

    // self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
    match (stream_data.client_server, selected_card) {
        (Some(client_server), Some(card)) => {
            if client_server.server_ip == card.ip
                && client_server.server_port == card.port
                && parser_index == card.protocol_index
            {
                let ls = tv_state
                    .cur_liststore
                    .as_ref()
                    .filter(|(c, _s, _l)| {
                        c.ip == card.ip
                            && c.port == card.port
                            && c.protocol_index == card.protocol_index
                    })
                    .map(|(_c, s, _l)| s.clone())
                    .unwrap_or_else(|| {
                        let key = card.to_key();
                        let ls = parser.get_empty_liststore();
                        tv_state.cur_liststore = Some((key, ls.clone(), 0));
                        let (ref tv, ref _signals) =
                            &tv_state.message_treeviews.get(card.protocol_index).unwrap();
                        parser.end_populate_treeview(tv, &ls);
                        ls
                    });
                // refresh_remote_ips_streams_tree() // <------
                parser.populate_treeview(
                    &ls,
                    stream_id,
                    &stream_data.messages[stream_data.messages.len() - added_messages..],
                    (stream_data.messages.len() - added_messages) as i32,
                );
                let mut store = tv_state.cur_liststore.take().unwrap();
                store.2 += added_messages as i32;
                tv_state.cur_liststore = Some(store);

                if follow_packets == FollowPackets::Follow {
                    // we're capturing network traffic. scroll to
                    // reveal new packets
                    let scrolledwindow = tv_state
                        .comm_remote_servers_stack
                        .get_visible_child()
                        .unwrap()
                        .dynamic_cast::<gtk::Paned>()
                        .unwrap()
                        .get_child1()
                        .unwrap()
                        .dynamic_cast::<gtk::ScrolledWindow>()
                        .unwrap();
                    let vadj = scrolledwindow.get_vadjustment().unwrap();
                    // new packets were added to the view,
                    // => scroll to reveal new packets
                    vadj.set_value(vadj.get_upper());
                }

                if stream_data.messages.len() == added_messages {
                    // just added the first rows to the grid. select the first row.
                    tv_state
                        .message_treeviews
                        .get(card.protocol_index)
                        .unwrap()
                        .0
                        .get_selection()
                        .select_path(&gtk::TreePath::new_first());

                    r = ViewAddedInfo::AddedFirstMessages;
                }
            }
        }
        _ => {}
    }
    r
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
