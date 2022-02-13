use super::win;
use crate::colors;
use crate::custom_streams_store::ClientServerInfo;
use crate::streams::Streams;
use crate::tshark_communication::TcpStreamId;
use crate::widgets::comm_target_card::CommTargetCardData;
use gtk::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::net::IpAddr;

/// I considered making this a relm widget, but that came a little late,
/// and some code expects a sync operation to update this treeview, while
/// relm components offer async updates. maybe some day...

pub struct IpsAndStreamsTreeviewState {
    remote_ips_streams_treestore: gtk::TreeStore,
    remote_ips_streams_iptopath: HashMap<IpAddr, gtk::TreePath>,
}

impl IpsAndStreamsTreeviewState {
    pub fn file_closed(&mut self) {
        self.remote_ips_streams_iptopath.clear();
    }

    pub fn remote_ips(&self) -> HashSet<IpAddr> {
        self.remote_ips_streams_iptopath.keys().copied().collect()
    }
}

pub fn init_remote_ip_streams_tv(
    remote_ips_streams_treeview: &gtk::TreeView,
) -> IpsAndStreamsTreeviewState {
    let remote_ip_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("Incoming conns")
        .expand(true)
        .build();
    let cell_r_txt = gtk::builders::CellRendererTextBuilder::new()
        .weight(1)
        .weight_set(true)
        .build();
    remote_ip_col.pack_start(&cell_r_txt, true);
    remote_ip_col.add_attribute(&cell_r_txt, "markup", 0);
    remote_ip_col.add_attribute(&cell_r_txt, "weight", 1);
    remote_ips_streams_treeview.append_column(&remote_ip_col);

    let message_count_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("# msg")
        .build();
    let cell_r2_txt = gtk::builders::CellRendererTextBuilder::new().build();
    message_count_col.pack_start(&cell_r2_txt, true);
    message_count_col.add_attribute(&cell_r2_txt, "text", 3);
    remote_ips_streams_treeview.append_column(&message_count_col);

    IpsAndStreamsTreeviewState {
        remote_ips_streams_iptopath: HashMap::new(),
        remote_ips_streams_treestore: init_treestore(),
    }
}

fn init_treestore() -> gtk::TreeStore {
    gtk::TreeStore::new(&[
        String::static_type(),        // first column markup
        pango::Weight::static_type(), // first column font weight
        u32::static_type(),           // stream id
        String::static_type(),        // number of messages in the stream, as string
    ])
}

pub fn got_packet_refresh_remote_ips_treeview(
    treeview_state: &mut IpsAndStreamsTreeviewState,
    client_server: Option<&ClientServerInfo>,
    packet_stream_id: TcpStreamId,
) {
    let treestore = treeview_state.remote_ips_streams_treestore.clone();

    let remote_ip_iter = treeview_state
        .remote_ips_streams_iptopath
        .get(&client_server.unwrap().client_ip)
        .and_then(|path| treestore.iter(path))
        .unwrap_or_else(|| {
            let new_iter = treestore.insert_with_values(
                None,
                None,
                &[
                    (0, &client_server.unwrap().client_ip.to_string().to_value()),
                    (1, &pango::Weight::Normal.to_value()),
                ],
            );
            treeview_state.remote_ips_streams_iptopath.insert(
                client_server.unwrap().client_ip,
                treestore.path(&new_iter).unwrap(),
            );
            new_iter
        });
    tv_insert_stream_leaf(
        treeview_state,
        &remote_ip_iter,
        &packet_stream_id,
        // we don't update the message counts in streams for each new
        // packet when we load the file (seems wasteful of CPU and
        // distracting for the user). Instead we refresh the display
        // when the loading completes
        &"⌛".to_value(),
    );
}

pub fn refresh_remote_ip_stream(
    rstream: &relm::StreamHandle<win::Msg>,
    selected_card: Option<&CommTargetCardData>,
    remote_ips_streams_treeview: &gtk::TreeView,
    paths: &mut [gtk::TreePath],
) {
    let mut allowed_ips = vec![];
    let mut allowed_stream_ids = vec![];
    let remote_ips_streams_tree_store = remote_ips_streams_treeview.model().unwrap();
    for path in paths {
        match path.indices_with_depth().as_slice() {
            &[0] => {
                // everything is allowed
                allowed_ips.clear();
                allowed_stream_ids.clear();
                break;
            }
            x if x.len() == 1 => {
                // remote ip
                if let Some(iter) = remote_ips_streams_tree_store.iter(path) {
                    let remote_ip: Option<String> =
                        remote_ips_streams_tree_store.value(&iter, 0).get().unwrap();
                    allowed_ips.push(remote_ip.unwrap().parse::<IpAddr>().unwrap());
                }
            }
            x if x.len() == 2 => {
                // stream
                let stream_iter = remote_ips_streams_tree_store.iter(path).unwrap();
                let stream_id = remote_ips_streams_tree_store.value(&stream_iter, 2);
                allowed_stream_ids.push(TcpStreamId(stream_id.get().unwrap()));
            }
            _ => panic!("unexpected path depth: {}", path.depth()),
        }
    }
    if let Some(card) = selected_card {
        rstream.emit(win::Msg::SelectCardFromRemoteIpsAndStreams(
            card.clone(),
            allowed_ips,
            allowed_stream_ids,
        ));
    }
}

pub fn init_remote_ips_streams_tree(treeview_state: &mut IpsAndStreamsTreeviewState) {
    treeview_state.remote_ips_streams_iptopath.clear();
    treeview_state.remote_ips_streams_treestore = init_treestore();
    treeview_state
        .remote_ips_streams_treestore
        .insert_with_values(
            None,
            None,
            &[(0, &"All".to_value()), (1, &pango::Weight::Bold.to_value())],
        );
}

pub fn connect_remote_ips_streams_tree(
    treeview_state: &IpsAndStreamsTreeviewState,
    remote_ips_streams_treeview: &gtk::TreeView,
) {
    let model_sort = gtk::TreeModelSort::new(&treeview_state.remote_ips_streams_treestore);
    model_sort.set_sort_column_id(gtk::SortColumn::Index(2), gtk::SortType::Ascending);
    remote_ips_streams_treeview.set_model(Some(&model_sort));
    remote_ips_streams_treeview.expand_all();
}

fn tv_insert_stream_leaf(
    treeview_state: &mut IpsAndStreamsTreeviewState,
    remote_ip_iter: &gtk::TreeIter,
    stream_id: &TcpStreamId,
    stream_message_count_val: &glib::Value,
) {
    treeview_state.remote_ips_streams_treestore.insert_with_values(
                    Some(remote_ip_iter),
                    None,
                    &[
                        (0,&format!(
                            r#"<span foreground="{}" size="smaller">⬤</span> <span rise="-1700">Stream {}</span>"#,
                            colors::STREAM_COLORS
                                [stream_id.as_u32() as usize % colors::STREAM_COLORS.len()],
                            stream_id.as_u32()
                        )
                        .to_value()),
                        (1,&pango::Weight::Normal.to_value()),
                        (2,&stream_id.as_u32().to_value()),
                        (3,stream_message_count_val),
                    ],
                );
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum IsNewDataStillIncoming {
    Yes,
    No,
}

/// if there is still data incoming, we won't attempt to put in
/// the treeview the count of messages per stream, we'll just
/// put a unicode hourglass. If there no more data incoming
/// (file fully loaded) then we'll compute the message counts
pub fn refresh_remote_ips_streams_tree(
    treeview_state: &mut IpsAndStreamsTreeviewState,
    remote_ips_streams_treeview: &gtk::TreeView,
    streams: &Streams,
    card: &CommTargetCardData,
    remote_ips: &HashSet<IpAddr>,
    is_new_data_incoming: IsNewDataStillIncoming,
) {
    // self.widgets.remote_ips_streams_treeview.set_cursor(
    //     &gtk::TreePath::new_first(),
    //     None::<&gtk::TreeViewColumn>,
    //     false,
    // );
    let target_ip = card.ip;
    let target_port = card.port;

    for remote_ip in remote_ips {
        let remote_ip_iter = treeview_state
            .remote_ips_streams_treestore
            .insert_with_values(
                None,
                None,
                &[
                    (0, &remote_ip.to_string().to_value()),
                    (1, &pango::Weight::Normal.to_value()),
                ],
            );
        treeview_state.remote_ips_streams_iptopath.insert(
            *remote_ip,
            treeview_state
                .remote_ips_streams_treestore
                .path(&remote_ip_iter)
                .unwrap(),
        );
        for stream_id in &streams.tcp_stream_ids() {
            let client_server = streams.get_client_server(*stream_id);
            if client_server.map(|cs| cs.server_ip) != Some(target_ip)
                || client_server.map(|cs| cs.server_port) != Some(target_port)
                || client_server.map(|cs| cs.client_ip) != Some(*remote_ip)
            {
                continue;
            }
            tv_insert_stream_leaf(
                treeview_state,
                &remote_ip_iter,
                stream_id,
                &if is_new_data_incoming == IsNewDataStillIncoming::Yes {
                    "⌛".to_string()
                } else {
                    streams
                        .stream_message_count(*stream_id)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "error".to_string())
                }
                .to_value(),
            );
        }
    }

    connect_remote_ips_streams_tree(treeview_state, remote_ips_streams_treeview);
}
