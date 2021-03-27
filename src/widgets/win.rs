use super::comm_remote_server::MessageData;
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use crate::colors;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::comm_remote_server::MessageParserDetailsMsg;
use crate::widgets::comm_target_card::SummaryDetails;
use crate::widgets::http_comm_entry::Http;
use crate::widgets::postgres_comm_entry::Postgres;
use crate::widgets::tls_comm_entry::Tls;
use crate::BgFunc;
use crate::TSharkCommunication;
use gdk::prelude::*;
use glib::translate::ToGlib;
use gtk::prelude::*;
use itertools::Itertools;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");

const WELCOME_STACK_NAME: &str = "welcome";
const LOADING_STACK_NAME: &str = "loading";
const NORMAL_STACK_NAME: &str = "normal";

pub fn get_message_parsers() -> Vec<Box<dyn MessageParser>> {
    vec![Box::new(Http), Box::new(Postgres), Box::new(Tls)]
}

pub type LoadedDataParams = (
    PathBuf,
    Vec<CommTargetCardData>,
    Vec<(StreamInfo, Vec<MessageData>)>,
);

#[derive(Msg, Debug)]
pub enum Msg {
    OpenFile,

    FinishedTShark,
    LoadedData(LoadedDataParams),

    SelectCard(Option<usize>),
    SelectRemoteIpStream(gtk::TreeSelection),

    SelectCardAll(CommTargetCardData),
    SelectCardFromRemoteIp(CommTargetCardData, String),
    SelectCardFromRemoteIpAndStream(CommTargetCardData, String, u32),

    DisplayDetails(u32, u32),

    Quit,
}

#[derive(Debug)]
pub struct StreamInfo {
    stream_id: u32,
    target_ip: String,
    target_port: u32,
    source_ip: String,
}

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
    // row activation disabled due to https://github.com/antoyo/relm/issues/281
    // row_activation_signal_id: glib::SignalHandlerId,
}

pub struct Model {
    relm: relm::Relm<Win>,
    bg_sender: mpsc::Sender<BgFunc>,

    window_subtitle: Option<String>,
    current_file_path: Option<PathBuf>,

    streams: Vec<(StreamInfo, Vec<MessageData>)>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,
    selected_server_or_stream: Option<gtk::TreePath>,

    remote_ips_streams_tree_store: gtk::TreeStore,
    comm_remote_servers_stores: Vec<gtk::ListStore>,
    comm_remote_servers_treeviews: Vec<(gtk::TreeView, TreeViewSignals)>,

    _finished_tshark_channel: relm::Channel<()>,
    finished_tshark_sender: relm::Sender<()>,

    _loaded_data_channel: relm::Channel<LoadedDataParams>,
    loaded_data_sender: relm::Sender<LoadedDataParams>,

    _comm_targets_components: Vec<Component<CommTargetCard>>,

    details_component_streams: Vec<relm::StreamHandle<MessageParserDetailsMsg>>,
}

#[derive(PartialEq, Eq)]
enum RefreshRemoteIpsAndStreams {
    Yes,
    No,
}

#[derive(PartialEq, Eq)]
pub enum TSharkMode {
    Json,
    JsonRaw,
}

pub fn invoke_tshark<T>(
    fname: &Path,
    tshark_mode: TSharkMode,
    filters: &str,
) -> Result<Vec<T>, Box<dyn std::error::Error>>
where
    T: serde::de::DeserializeOwned,
{
    // piping from tshark, not to load the entire JSON in ram...
    let tshark_child = Command::new("tshark")
        .args(&[
            "-r",
            fname.to_str().expect("invalid filename"),
            if tshark_mode == TSharkMode::Json {
                "-Tjson"
            } else {
                "-Tjsonraw"
            },
            "--no-duplicate-keys",
            filters,
            // "tcp.stream eq 104",
        ])
        .stdout(Stdio::piped())
        .spawn()?;
    let reader = BufReader::new(tshark_child.stdout.unwrap());
    Ok(serde_json::de::from_reader(reader)?)
}

#[derive(Debug)]
enum RefreshOngoing {
    Yes,
    No,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        if let Err(err) = self.load_style() {
            println!("Error loading the CSS: {}", err);
        }

        // https://bugzilla.gnome.org/show_bug.cgi?id=305277
        gtk::Settings::get_default()
            .unwrap()
            .set_property_gtk_alternative_sort_arrows(true);

        for (idx, message_parser) in get_message_parsers().iter().enumerate() {
            let tv = gtk::TreeViewBuilder::new()
                .activate_on_single_click(true)
                .build();
            let (modelsort, store) = message_parser.prepare_treeview(&tv);
            self.model.comm_remote_servers_stores.push(store.clone());

            let rstream = self.model.relm.stream().clone();
            let st = store.clone();
            let ms = modelsort.clone();
            let selection_change_signal_id = tv.get_selection().connect_changed(move |selection| {
                if let Some((model, iter)) = selection.get_selected() {
                    if let Some(path) = model
                        .get_path(&iter)
                        .and_then(|p| ms.convert_path_to_child_path(&p))
                    {
                        Self::row_selected(&st, &path, &rstream);
                    }
                }
            });
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

            self.model.comm_remote_servers_treeviews.push((
                tv.clone(),
                TreeViewSignals {
                    selection_change_signal_id,
                    // row_activation_signal_id,
                },
            ));
            let scroll = gtk::ScrolledWindowBuilder::new()
                .expand(true)
                .child(&tv)
                .build();
            let paned = gtk::PanedBuilder::new()
                .orientation(gtk::Orientation::Vertical)
                .build();
            paned.pack1(&scroll, true, true);

            let scroll2 = gtk::ScrolledWindowBuilder::new().build();
            self.model
                .details_component_streams
                .push(message_parser.add_details_to_scroll(&scroll2, self.model.bg_sender.clone()));
            scroll2.set_property_height_request(200);
            paned.pack2(&scroll2, false, true);
            self.widgets
                .comm_remote_servers_stack
                .add_named(&paned, &idx.to_string());
            paned.show_all();
        }

        self.init_remote_ip_streams_tv();

        self.refresh_comm_targets();
        self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, None, None);
    }

    fn init_remote_ip_streams_tv(&self) {
        let remote_ip_col = gtk::TreeViewColumnBuilder::new()
            .title("Incoming conns")
            .expand(true)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new()
            .weight(1)
            .weight_set(true)
            .build();
        remote_ip_col.pack_start(&cell_r_txt, true);
        remote_ip_col.add_attribute(&cell_r_txt, "markup", 0);
        remote_ip_col.add_attribute(&cell_r_txt, "weight", 1);
        self.widgets
            .remote_ips_streams_treeview
            .append_column(&remote_ip_col);
    }

    fn setup_selection_signals(&self, refresh_ongoing: RefreshOngoing) {
        dbg!(&refresh_ongoing);
        match refresh_ongoing {
            RefreshOngoing::Yes => {
                for (tv, signals) in &self.model.comm_remote_servers_treeviews {
                    tv.get_selection()
                        .block_signal(&signals.selection_change_signal_id);
                    // tv.unblock_signal(&signals.row_activation_signal_id);
                }
            }
            RefreshOngoing::No => {
                for (tv, signals) in &self.model.comm_remote_servers_treeviews {
                    tv.get_selection()
                        .unblock_signal(&signals.selection_change_signal_id);
                    // tv.block_signal(&signals.row_activation_signal_id);
                }
            }
        }
    }

    fn row_selected(
        store: &gtk::ListStore,
        path: &gtk::TreePath,
        rstream: &relm::StreamHandle<Msg>,
    ) {
        let iter = store.get_iter(&path).unwrap();
        let stream_id = store.get_value(&iter, 2);
        let idx = store.get_value(&iter, 3);
        println!(
            "stream: {} idx: {}",
            stream_id.get::<u32>().unwrap().unwrap(),
            idx.get::<u32>().unwrap().unwrap()
        );
        rstream.emit(Msg::DisplayDetails(
            stream_id.get::<u32>().unwrap().unwrap(),
            idx.get::<u32>().unwrap().unwrap(),
        ));
    }

    fn load_style(&self) -> Result<(), Box<dyn std::error::Error>> {
        let screen = self.widgets.window.get_screen().unwrap();
        let css = gtk::CssProvider::new();
        css.load_from_data(CSS_DATA)?;
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        Ok(())
    }

    fn model(relm: &relm::Relm<Self>, bg_sender: mpsc::Sender<BgFunc>) -> Model {
        gtk::IconTheme::get_default()
            .unwrap()
            .add_resource_path("/icons");
        let remote_ips_streams_tree_store = gtk::TreeStore::new(&[
            String::static_type(),
            pango::Weight::static_type(),
            u32::static_type(),
        ]);

        let stream = relm.stream().clone();
        let (_loaded_data_channel, loaded_data_sender) =
            relm::Channel::new(move |ch_data: LoadedDataParams| {
                stream.emit(Msg::LoadedData(ch_data));
            });

        let stream2 = relm.stream().clone();
        let (_finished_tshark_channel, finished_tshark_sender) = relm::Channel::new(move |_| {
            stream2.emit(Msg::FinishedTShark);
        });

        Model {
            relm: relm.clone(),
            bg_sender,
            _comm_targets_components: vec![],
            selected_card: None,
            selected_server_or_stream: None,
            remote_ips_streams_tree_store,
            comm_remote_servers_stores: vec![],
            comm_remote_servers_treeviews: vec![],
            details_component_streams: vec![],
            loaded_data_sender,
            _loaded_data_channel,
            finished_tshark_sender,
            _finished_tshark_channel,
            comm_target_cards: vec![],
            streams: vec![],
            current_file_path: None,
            window_subtitle: None,
        }
    }

    fn update(&mut self, event: Msg) {
        match &event {
            Msg::LoadedData(_) => println!("event: loadeddata"),
            _ => {
                dbg!(&event);
            }
        }
        match event {
            Msg::OpenFile => {
                self.open_file();
            }
            Msg::FinishedTShark => {
                self.widgets.loading_tshark_label.set_visible(false);
                self.widgets.loading_parsing_label.set_visible(true);
            }
            Msg::LoadedData((fname, comm_target_cards, streams)) => {
                self.widgets.loading_spinner.stop();
                self.model.window_subtitle = Some(
                    fname
                        // .file_name().unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
                self.model.current_file_path = Some(fname);
                self.model.comm_target_cards = comm_target_cards;
                self.model.streams = streams;
                self.refresh_comm_targets();
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, None, None);
                self.widgets
                    .root_stack
                    .set_visible_child_name(NORMAL_STACK_NAME);
            }
            Msg::SelectCard(maybe_idx) => {
                println!("card changed");
                let wait_cursor = gdk::Cursor::new_for_display(
                    &self.widgets.window.get_display(),
                    gdk::CursorType::Watch,
                );
                if let Some(p) = self.widgets.root_stack.get_parent_window() {
                    p.set_cursor(Some(&wait_cursor));
                }
                self.widgets.comm_target_list.set_sensitive(false);
                self.widgets
                    .remote_ips_streams_treeview
                    .set_sensitive(false);
                self.model.selected_card = maybe_idx
                    .and_then(|idx| self.model.comm_target_cards.get(idx as usize))
                    .cloned();
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, None, None);
                if let Some(p) = self.widgets.root_stack.get_parent_window() {
                    p.set_cursor(None);
                }
                self.widgets.comm_target_list.set_sensitive(true);
                self.widgets.remote_ips_streams_treeview.set_sensitive(true);
                self.model.selected_server_or_stream = Some(gtk::TreePath::new_first());
                // if let Some(vadj) = self.widgets.remote_servers_scroll.get_vadjustment() {
                //     vadj.set_value(0.0);
                // }
            }
            Msg::SelectRemoteIpStream(selection) => {
                if let Some((model, iter)) = selection.get_selected() {
                    if let Some(path) = model.get_path(&iter) {
                        if Some(&path) != self.model.selected_server_or_stream.as_ref() {
                            println!("remote selection changed");
                            self.refresh_remote_ip_stream(path);
                        }
                    }
                }
            }
            Msg::SelectCardAll(_) => {
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::No, None, None);
            }
            Msg::SelectCardFromRemoteIp(_, remote_ip) => {
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::No, Some(remote_ip), None);
            }
            Msg::SelectCardFromRemoteIpAndStream(_, remote_ip, stream_id) => {
                self.refresh_remote_servers(
                    RefreshRemoteIpsAndStreams::No,
                    Some(remote_ip),
                    Some(stream_id),
                );
            }
            Msg::DisplayDetails(stream_id, idx) => {
                if let Some(msg_data) = self
                    .model
                    .streams
                    .iter()
                    .find(|(stream_info, items)| stream_info.stream_id == stream_id)
                    .and_then(|s| s.1.get(idx as usize))
                {
                    for component_stream in &self.model.details_component_streams {
                        component_stream.emit(MessageParserDetailsMsg::DisplayDetails(
                            self.model.bg_sender.clone(),
                            self.model.current_file_path.as_ref().unwrap().clone(),
                            msg_data.clone(),
                        ));
                    }
                }
            }
            Msg::Quit => gtk::main_quit(),
        }
    }

    fn refresh_remote_ip_stream(&mut self, mut path: gtk::TreePath) {
        self.model.selected_server_or_stream = Some(path.clone());
        match path.get_indices_with_depth().as_slice() {
            &[0] => self.model.relm.stream().emit(Msg::SelectCardAll(
                self.model.selected_card.as_ref().unwrap().clone(),
            )),
            x if x.len() == 1 => {
                if let Some(iter) = self.model.remote_ips_streams_tree_store.get_iter(&path) {
                    let remote_ip = self.model.remote_ips_streams_tree_store.get_value(&iter, 0);
                    self.model.relm.stream().emit(Msg::SelectCardFromRemoteIp(
                        self.model.selected_card.as_ref().unwrap().clone(),
                        remote_ip.get().unwrap().unwrap(),
                    ));
                }
            }
            x if x.len() == 2 => {
                let stream_iter = self
                    .model
                    .remote_ips_streams_tree_store
                    .get_iter(&path)
                    .unwrap();
                let stream_id = self
                    .model
                    .remote_ips_streams_tree_store
                    .get_value(&stream_iter, 2);
                path.up();
                let remote_ip_iter = self
                    .model
                    .remote_ips_streams_tree_store
                    .get_iter(&path)
                    .unwrap();
                let remote_ip = self
                    .model
                    .remote_ips_streams_tree_store
                    .get_value(&remote_ip_iter, 0);
                self.model
                    .relm
                    .stream()
                    .emit(Msg::SelectCardFromRemoteIpAndStream(
                        self.model.selected_card.as_ref().unwrap().clone(),
                        remote_ip.get().unwrap().unwrap(),
                        stream_id.get().unwrap().unwrap(),
                    ));
            }
            _ => panic!(path.get_depth()),
        }
    }

    fn open_file(&mut self) {
        let dialog = gtk::FileChooserNativeBuilder::new()
            .action(gtk::FileChooserAction::Open)
            .title("Select file")
            .modal(true)
            .build();
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.pcap");
        dialog.set_filter(&filter);
        if dialog.run() == gtk::ResponseType::Accept {
            if let Some(fname) = dialog.get_filename() {
                self.widgets.loading_spinner.start();
                self.widgets.loading_parsing_label.set_visible(false);
                self.widgets.loading_tshark_label.set_visible(true);
                self.widgets
                    .root_stack
                    .set_visible_child_name(LOADING_STACK_NAME);
                let s = self.model.loaded_data_sender.clone();
                let t = self.model.finished_tshark_sender.clone();
                self.model
                    .bg_sender
                    .send(BgFunc::new(move || {
                        Self::load_file(fname.clone(), s.clone(), t.clone());
                    }))
                    .unwrap();
            }
        }
    }

    fn load_file(
        fname: PathBuf,
        sender: relm::Sender<LoadedDataParams>,
        finished_tshark: relm::Sender<()>,
    ) {
        let packets = invoke_tshark::<TSharkCommunication>(&fname, TSharkMode::Json, "tcp")
            .expect("tshark error");
        finished_tshark.send(()).unwrap();
        Self::handle_packets(fname, packets, sender)
    }

    fn handle_packets(
        fname: PathBuf,
        packets: Vec<TSharkCommunication>,
        sender: relm::Sender<LoadedDataParams>,
    ) {
        let mut by_stream: Vec<_> = packets
            .into_iter()
            // .filter(|p| p.source.layers.http.is_some())
            .map(|p| (p.source.layers.tcp.as_ref().map(|t| t.stream), p))
            .into_group_map()
            .into_iter()
            .collect();
        by_stream.sort_by_key(|p| Reverse(p.1.len()));

        let message_parsers = get_message_parsers();

        let mut parsed_streams: Vec<_> = by_stream
            .into_iter()
            .filter_map(|(id, comms)| {
                let parser = comms
                    .iter()
                    .find_map(|c| message_parsers.iter().find(|p| p.is_my_message(c)));

                if let Some(p) = parser {
                    let layers = &comms.first().unwrap().source.layers;
                    let card_key = (layers.ip_dst(), layers.tcp.as_ref().unwrap().port_dst);
                    let ip_src = layers.ip_src();
                    Some((p, id, ip_src, card_key, p.parse_stream(comms)))
                } else {
                    None
                }
            })
            .collect();
        parsed_streams.sort_by_key(|(_parser, id, _ip_src, _card_key, _pstream)| *id);

        let comm_target_cards = parsed_streams
            .iter()
            .fold(
                HashMap::<(String, u32), CommTargetCardData>::new(),
                |mut sofar, (parser, _stream_id, ip_src, card_key, items)| {
                    if let Some(target_card) = sofar.get_mut(&card_key) {
                        target_card.remote_hosts.insert(ip_src.to_string());
                        target_card.incoming_session_count += 1;
                        if target_card.summary_details.is_none() && items.summary_details.is_some()
                        {
                            target_card.summary_details = SummaryDetails::new(
                                items.summary_details.as_deref().unwrap(),
                                &target_card.ip,
                                target_card.port,
                            );
                        }
                    } else {
                        let mut remote_hosts = BTreeSet::new();
                        remote_hosts.insert(ip_src.to_string());
                        let protocol_index = message_parsers
                            .iter()
                            // comparing by the protocol icon is.. quite horrible..
                            .position(|p| p.protocol_icon() == parser.protocol_icon())
                            .unwrap();
                        sofar.insert(
                            card_key.clone(),
                            CommTargetCardData {
                                ip: card_key.0.clone(),
                                protocol_index,
                                protocol_icon: parser.protocol_icon(),
                                port: card_key.1,
                                remote_hosts,
                                incoming_session_count: 1,
                                summary_details: items
                                    .summary_details
                                    .as_ref()
                                    .and_then(|d| SummaryDetails::new(d, &card_key.0, card_key.1)),
                            },
                        );
                    }
                    sofar
                },
            )
            .into_iter()
            .map(|(k, v)| v)
            .collect();

        let streams = parsed_streams
            .into_iter()
            .map(|(_parser, id, ip_src, card_key, pstream)| {
                (
                    StreamInfo {
                        stream_id: id.unwrap(),
                        target_ip: card_key.0,
                        target_port: card_key.1,
                        source_ip: ip_src,
                    },
                    pstream.messages,
                )
            })
            .collect();

        sender.send((fname, comm_target_cards, streams)).unwrap();
    }

    fn refresh_comm_targets(&mut self) {
        for child in self.widgets.comm_target_list.get_children() {
            self.widgets.comm_target_list.remove(&child);
        }
        self.model._comm_targets_components = self
            .model
            .comm_target_cards
            .iter()
            .map(|card| {
                self.widgets
                    .comm_target_list
                    .add_widget::<CommTargetCard>(card.clone())
            })
            .collect();
        self.widgets
            .comm_target_list
            .select_row(self.widgets.comm_target_list.get_row_at_index(0).as_ref());
        // self.model.selected_card = self.model.comm_target_cards.first().cloned();
    }

    fn refresh_remote_ips_streams_tree(
        &mut self,
        card: &CommTargetCardData,
        remote_ips: &HashSet<String>,
    ) {
        self.model.remote_ips_streams_tree_store.clear();
        self.model.remote_ips_streams_tree_store.insert_with_values(
            None,
            None,
            &[0, 1],
            &[&"All".to_value(), &pango::Weight::Bold.to_glib().to_value()],
        );
        self.widgets.remote_ips_streams_treeview.set_cursor(
            &gtk::TreePath::new_first(),
            None::<&gtk::TreeViewColumn>,
            false,
        );
        let target_ip = card.ip.clone();
        let target_port = card.port;

        for remote_ip in remote_ips {
            let remote_ip_iter = self.model.remote_ips_streams_tree_store.insert_with_values(
                None,
                None,
                &[0, 1],
                &[
                    &remote_ip.to_value(),
                    &pango::Weight::Normal.to_glib().to_value(),
                ],
            );
            for (stream_info, _messages) in &self.model.streams {
                if stream_info.target_ip != target_ip
                    || stream_info.target_port != target_port
                    || stream_info.source_ip != *remote_ip
                {
                    continue;
                }
                self.model.remote_ips_streams_tree_store.insert_with_values(
                    Some(&remote_ip_iter),
                    None,
                    &[0, 1, 2],
                    &[
                        &format!(
                            r#"<span foreground="{}" size="smaller">⬤</span> <span rise="-1700">Stream {}</span>"#,
                            colors::STREAM_COLORS
                                [stream_info.stream_id as usize % colors::STREAM_COLORS.len()],
                            stream_info.stream_id
                        )
                        .to_value(),
                        &pango::Weight::Normal.to_glib().to_value(),
                        &stream_info.stream_id.to_value(),
                    ],
                );
            }
        }

        self.widgets.remote_ips_streams_treeview.expand_all();
    }

    fn refresh_remote_servers(
        &mut self,
        refresh_remote_ips_and_streams: RefreshRemoteIpsAndStreams,
        constrain_remote_ip: Option<String>,
        constrain_stream_id: Option<u32>,
    ) {
        self.setup_selection_signals(RefreshOngoing::Yes);
        for store in &self.model.comm_remote_servers_stores {
            store.clear();
        }
        if let Some(card) = self.model.selected_card.as_ref().cloned() {
            let target_ip = card.ip.clone();
            let target_port = card.port;
            let mut by_remote_ip = HashMap::new();
            let parsers = get_message_parsers();
            for (stream_info, messages) in &self.model.streams {
                if stream_info.target_ip != target_ip || stream_info.target_port != target_port {
                    continue;
                }
                let remote_ip = &stream_info.source_ip;
                if let Some(ref constrained_remote) = constrain_remote_ip {
                    if constrained_remote != remote_ip {
                        continue;
                    }
                }
                if constrain_stream_id.is_some()
                    && constrain_stream_id != Some(stream_info.stream_id)
                {
                    continue;
                }
                let remote_server_streams = by_remote_ip
                    .entry(remote_ip.clone())
                    .or_insert_with(Vec::new);
                remote_server_streams.push((stream_info.stream_id, messages));
            }
            let mp = parsers.get(card.protocol_index).unwrap();
            self.widgets
                .comm_remote_servers_stack
                .set_visible_child_name(&card.protocol_index.to_string());
            let store = &self
                .model
                .comm_remote_servers_stores
                .get(card.protocol_index)
                .unwrap();
            for (remote_ip, tcp_sessions) in &by_remote_ip {
                for (session_id, session) in tcp_sessions {
                    let mut idx = 0;
                    for chunk in session.chunks(100) {
                        mp.populate_treeview(&store, *session_id, chunk, idx);
                        idx += 100;
                        // https://developer.gnome.org/gtk3/stable/gtk3-General.html#gtk-events-pending
                        while gtk::events_pending() {
                            gtk::main_iteration();
                        }
                    }
                }
            }
            if refresh_remote_ips_and_streams == RefreshRemoteIpsAndStreams::Yes {
                let ip_hash = by_remote_ip
                    .keys()
                    .map(|c| c.to_string())
                    .collect::<HashSet<_>>();
                self.refresh_remote_ips_streams_tree(&card, &ip_hash);
            }
        }
        self.setup_selection_signals(RefreshOngoing::No);
        if let Some(card) = self.model.selected_card.as_ref().cloned() {
            self.model
                .comm_remote_servers_treeviews
                .get(card.protocol_index)
                .unwrap()
                .0
                .get_selection()
                .select_path(&gtk::TreePath::new_first());
        }
    }

    view! {
        #[name="window"]
        gtk::Window {
            titlebar: view! {
                gtk::HeaderBar {
                    show_close_button: true,
                    title: Some("Hotwire"),
                    subtitle: self.model.window_subtitle.as_deref(),
                    gtk::MenuButton {
                        image: Some(&gtk::Image::from_icon_name(Some("open-menu-symbolic"), gtk::IconSize::Menu)),
                        child: {
                            pack_type: gtk::PackType::End
                        },
                        active: false,
                        popover: view! {
                            gtk::Popover {
                                visible: false,
                                gtk::Box {
                                    margin_top: 10,
                                    margin_start: 10,
                                    margin_end: 10,
                                    margin_bottom: 10,
                                    gtk::ModelButton {
                                        label: "Open",
                                        hexpand: true,
                                        clicked => Msg::OpenFile,
                                    }
                                }
                            }
                        },
                    },
                }
            },
            #[name="root_stack"]
            gtk::Stack {
                gtk::Label {
                    child: {
                        name: Some(WELCOME_STACK_NAME)
                    },
                    label: "Welcome to Hotwire!"
                },
                gtk::Box {
                    child: {
                        name: Some(LOADING_STACK_NAME)
                    },
                    orientation: gtk::Orientation::Vertical,
                    valign: gtk::Align::Center,
                    gtk::Box {
                        spacing: 5,
                        #[name="loading_spinner"]
                        gtk::Spinner {
                            hexpand: true,
                            halign: gtk::Align::End,
                        },
                        #[style_class="title"]
                        gtk::Label {
                            label: "Loading the file, please wait",
                            hexpand: true,
                            xalign: 0.0,
                        },
                    },
                    #[style_class="subtitle"]
                    #[name="loading_tshark_label"]
                    gtk::Label {
                        label: "Communication with tshark",
                        hexpand: true,
                    },
                    #[style_class="subtitle"]
                    #[name="loading_parsing_label"]
                    gtk::Label {
                        label: "Parsing of the packets",
                        hexpand: true,
                    },
                },
                gtk::Box {
                    child: {
                        name: Some(NORMAL_STACK_NAME)
                    },
                    hexpand: true,
                    #[style_class="sidebar"]
                    gtk::ScrolledWindow {
                        property_width_request: 250,
                        #[name="comm_target_list"]
                        gtk::ListBox {
                            row_selected(_, row) =>
                                Msg::SelectCard(row.map(|r| r.get_index() as usize))
                        }
                    },
                    gtk::ScrolledWindow {
                        property_width_request: 150,
                        #[name="remote_ips_streams_treeview"]
                        gtk::TreeView {
                            activate_on_single_click: true,
                            model: Some(&self.model.remote_ips_streams_tree_store),
                            selection.changed(selection) => Msg::SelectRemoteIpStream(selection.clone()),
                        },
                    },
                    gtk::Separator {
                        orientation: gtk::Orientation::Vertical,
                    },
                    #[name="comm_remote_servers_stack"]
                    gtk::Stack {}
                },
            },
            delete_event(_, _) => (Msg::Quit, Inhibit(false)),
        }
    }
}
