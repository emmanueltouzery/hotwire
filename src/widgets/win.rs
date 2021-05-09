use super::comm_remote_server::MessageData;
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use crate::colors;
use crate::http::http_message_parser::Http;
use crate::http2::http2_message_parser::Http2;
use crate::message_parser::{MessageInfo, MessageParser};
use crate::pgsql::postgres_message_parser::Postgres;
use crate::tshark_communication;
use crate::widgets::comm_target_card::SummaryDetails;
use crate::BgFunc;
use gdk::prelude::*;
use glib::translate::ToGlib;
use gtk::prelude::*;
use itertools::Itertools;
use quick_xml::events::Event;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Instant;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");

const WELCOME_STACK_NAME: &str = "welcome";
const LOADING_STACK_NAME: &str = "loading";
const NORMAL_STACK_NAME: &str = "normal";

pub fn get_message_parsers() -> Vec<Box<dyn MessageParser>> {
    vec![Box::new(Http), Box::new(Postgres), Box::new(Http2)]
}

pub type LoadedDataParams = (
    PathBuf,
    Vec<CommTargetCardData>,
    Vec<(StreamInfo, Vec<MessageData>)>,
);

#[derive(Debug, PartialEq, Eq)]
pub enum InfobarOptions {
    Default,
    ShowCloseButton,
    ShowSpinner,
    TimeLimitedWithCloseButton,
}

#[derive(Msg, Debug)]
pub enum Msg {
    OpenFile,

    FinishedTShark,
    LoadedData(LoadedDataParams),

    SelectCard(Option<usize>),
    SelectRemoteIpStream(gtk::TreeSelection),

    InfoBarShow(Option<String>, InfobarOptions),
    InfoBarEvent(gtk::ResponseType),

    SelectCardFromRemoteIpsAndStreams(CommTargetCardData, Vec<String>, Vec<u32>),

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

    infobar_spinner: gtk::Spinner,
    infobar_label: gtk::Label,

    sidebar_selection_change_signal_id: Option<glib::SignalHandlerId>,

    streams: Vec<(StreamInfo, Vec<MessageData>)>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    comm_remote_servers_treeviews: Vec<(gtk::TreeView, TreeViewSignals)>,

    _finished_tshark_channel: relm::Channel<()>,
    finished_tshark_sender: relm::Sender<()>,

    _loaded_data_channel: relm::Channel<LoadedDataParams>,
    loaded_data_sender: relm::Sender<LoadedDataParams>,

    _comm_targets_components: Vec<Component<CommTargetCard>>,

    details_component_emitters: Vec<Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)>>,
    details_adjustments: Vec<gtk::Adjustment>,
}

#[derive(PartialEq, Eq)]
enum RefreshRemoteIpsAndStreams {
    Yes,
    No,
}

#[derive(PartialEq, Eq)]
pub enum TSharkMode {
    // TODO obsolete
    Json,
    JsonRaw,
}

// it would be possible to ask tshark to "mix in" a keylog file
// when opening the pcap file
// (obtain the keylog file through `SSLKEYLOGFILE=browser_keylog.txt google-chrome` or firefox,
// pass it to tshark through -o ssh.keylog_file:/path/to/keylog)
// but we get in flatpak limitations (can only access the file that the user opened
// due to the sandbox) => better to just mix in the secrets manually and open a single
// file. this is done through => editcap --inject-secrets tls,/path/to/keylog.txt ~/testtls.pcap ~/outtls.pcapng
pub fn invoke_tshark(
    fname: &Path,
    tshark_mode: TSharkMode,
    filters: &str,
) -> Result<Vec<tshark_communication::TSharkPacket>, Box<dyn std::error::Error>> {
    dbg!(&filters);
    // piping from tshark, not to load the entire JSON in ram...
    let tshark_child = Command::new("tshark")
        .args(&[
            "-r",
            fname.to_str().expect("invalid filename"),
            "-Tpdml",
            // "-o",
            // "ssl.keylog_file:/home/emmanuel/chrome_keylog.txt",
            filters,
            // "tcp.stream eq 104",
        ])
        .stdout(Stdio::piped())
        .spawn()?;
    let buf_reader = BufReader::new(tshark_child.stdout.unwrap());
    let mut xml_reader = quick_xml::Reader::from_reader(buf_reader);
    let mut buf = vec![];
    let mut r = vec![];
    loop {
        match xml_reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name() == b"packet" {
                    if let Some(packet) =
                        tshark_communication::parse_packet(&mut xml_reader, &mut buf).ok()
                    {
                        r.push(packet);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Box::new(e)),
            _ => {}
        };
        // buf.clear();
    }
    Ok(r)
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
        self.widgets.infobar.set_visible(false);

        self.model.sidebar_selection_change_signal_id = {
            let stream = self.model.relm.stream().clone();
            Some(
                self.widgets
                    .remote_ips_streams_treeview
                    .get_selection()
                    .connect_changed(move |selection| {
                        stream.emit(Msg::SelectRemoteIpStream(selection.clone()));
                    }),
            )
        };

        self.widgets
            .remote_ips_streams_treeview
            .get_selection()
            .set_mode(gtk::SelectionMode::Multiple);

        let infobar_box = gtk::BoxBuilder::new().spacing(15).build();
        infobar_box.add(&self.model.infobar_spinner);
        infobar_box.add(&self.model.infobar_label);
        infobar_box.show_all();
        self.widgets.infobar.get_content_area().add(&infobar_box);

        self.model.infobar_spinner.set_visible(false);

        // https://bugzilla.gnome.org/show_bug.cgi?id=305277
        gtk::Settings::get_default()
            .unwrap()
            .set_property_gtk_alternative_sort_arrows(true);

        for (idx, message_parser) in get_message_parsers().iter().enumerate() {
            self.add_message_parser_grid_and_pane(&message_parser, idx);
        }

        self.init_remote_ip_streams_tv();

        self.refresh_comm_targets();
        self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
    }

    fn add_message_parser_grid_and_pane(
        &mut self,
        message_parser: &Box<dyn MessageParser>,
        idx: usize,
    ) {
        let tv = gtk::TreeViewBuilder::new()
            .activate_on_single_click(true)
            .build();
        message_parser.prepare_treeview(&tv);

        let selection_change_signal_id = {
            let rstream = self.model.relm.stream().clone();
            let tv = tv.clone();
            tv.get_selection().connect_changed(move |selection| {
                if let Some((model, iter)) = selection.get_selected() {
                    let modelsort = model.dynamic_cast::<gtk::TreeModelSort>().unwrap();
                    let model = modelsort
                        .get_model()
                        .dynamic_cast::<gtk::ListStore>()
                        .unwrap();
                    if let Some(path) = modelsort
                        .get_path(&iter)
                        .and_then(|p| modelsort.convert_path_to_child_path(&p))
                    {
                        Self::row_selected(&model, &path, &rstream);
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
        self.model
            .details_component_emitters
            .push(message_parser.add_details_to_scroll(
                &scroll2,
                overlay.as_ref(),
                self.model.bg_sender.clone(),
                self.model.relm.stream().clone(),
            ));
        self.model
            .details_adjustments
            .push(scroll2.get_vadjustment().unwrap());

        self.widgets
            .comm_remote_servers_stack
            .add_named(&paned, &idx.to_string());
        paned.show_all();
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
                    self.widgets
                        .remote_ips_streams_treeview
                        .get_selection()
                        .block_signal(
                            self.model
                                .sidebar_selection_change_signal_id
                                .as_ref()
                                .unwrap(),
                        );
                    tv.get_selection()
                        .block_signal(&signals.selection_change_signal_id);
                    // tv.unblock_signal(&signals.row_activation_signal_id);
                }
            }
            RefreshOngoing::No => {
                for (tv, signals) in &self.model.comm_remote_servers_treeviews {
                    self.widgets
                        .remote_ips_streams_treeview
                        .get_selection()
                        .unblock_signal(
                            self.model
                                .sidebar_selection_change_signal_id
                                .as_ref()
                                .unwrap(),
                        );
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

        let (_loaded_data_channel, loaded_data_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |ch_data: LoadedDataParams| {
                stream.emit(Msg::LoadedData(ch_data));
            })
        };

        let (_finished_tshark_channel, finished_tshark_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |_| {
                stream.emit(Msg::FinishedTShark);
            })
        };

        Model {
            relm: relm.clone(),
            bg_sender,
            infobar_spinner: gtk::SpinnerBuilder::new()
                .width_request(24)
                .height_request(24)
                .build(),
            infobar_label: gtk::LabelBuilder::new().build(),
            _comm_targets_components: vec![],
            selected_card: None,
            comm_remote_servers_treeviews: vec![],
            details_component_emitters: vec![],
            details_adjustments: vec![],
            loaded_data_sender,
            _loaded_data_channel,
            finished_tshark_sender,
            _finished_tshark_channel,
            sidebar_selection_change_signal_id: None,
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
            Msg::InfoBarShow(Some(msg), options) => {
                self.widgets.infobar.set_show_close_button(matches!(
                    options,
                    InfobarOptions::ShowCloseButton | InfobarOptions::TimeLimitedWithCloseButton
                ));
                let has_spinner = options == InfobarOptions::ShowSpinner;
                if self.model.infobar_spinner.get_visible() != has_spinner {
                    if has_spinner {
                        println!("start spinner");
                        self.model.infobar_spinner.start();
                    } else {
                        println!("stop spinner");
                        self.model.infobar_spinner.stop();
                    }
                    self.model.infobar_spinner.set_visible(has_spinner);
                }
                if options == InfobarOptions::TimeLimitedWithCloseButton {
                    relm::timeout(self.model.relm.stream(), 1500, || {
                        Msg::InfoBarShow(None, InfobarOptions::Default)
                    });
                }
                self.model.infobar_label.set_text(&msg);
                self.widgets.infobar.set_visible(true);
            }
            Msg::InfoBarShow(None, _) | Msg::InfoBarEvent(gtk::ResponseType::Close) => {
                self.widgets.infobar.set_visible(false);
            }
            Msg::InfoBarEvent(_) => {}
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
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
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
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
                if let Some(p) = self.widgets.root_stack.get_parent_window() {
                    p.set_cursor(None);
                }
                self.widgets.comm_target_list.set_sensitive(true);
                self.widgets.remote_ips_streams_treeview.set_sensitive(true);
                // if let Some(vadj) = self.widgets.remote_servers_scroll.get_vadjustment() {
                //     vadj.set_value(0.0);
                // }
            }
            Msg::SelectRemoteIpStream(selection) => {
                let (mut paths, model) = selection.get_selected_rows();
                println!("remote selection changed");
                self.refresh_remote_ip_stream(&mut paths);
            }
            Msg::SelectCardFromRemoteIpsAndStreams(_, remote_ips, stream_ids) => {
                self.refresh_remote_servers(
                    RefreshRemoteIpsAndStreams::No,
                    &remote_ips,
                    &stream_ids,
                );
            }
            Msg::DisplayDetails(stream_id, idx) => {
                if let Some((stream_info, msg_data)) = self
                    .model
                    .streams
                    .iter()
                    .find(|(stream_info, items)| stream_info.stream_id == stream_id)
                    .and_then(|s| s.1.get(idx as usize).map(|f| (&s.0, f)))
                {
                    for adj in &self.model.details_adjustments {
                        adj.set_value(0.0);
                    }
                    for component_emitter in &self.model.details_component_emitters {
                        component_emitter(
                            self.model.bg_sender.clone(),
                            self.model.current_file_path.as_ref().unwrap().clone(),
                            MessageInfo {
                                stream_id: stream_info.stream_id,
                                client_ip: stream_info.source_ip.clone(),
                                message_data: msg_data.clone(),
                            },
                        );
                    }
                }
            }
            Msg::Quit => gtk::main_quit(),
        }
    }

    fn refresh_remote_ip_stream(&mut self, paths: &mut [gtk::TreePath]) {
        let mut allowed_ips = vec![];
        let mut allowed_stream_ids = vec![];
        let remote_ips_streams_tree_store = self
            .widgets
            .remote_ips_streams_treeview
            .get_model()
            .unwrap();
        for path in paths {
            match path.get_indices_with_depth().as_slice() {
                &[0] => {
                    // everything is allowed
                    allowed_ips.clear();
                    allowed_stream_ids.clear();
                    break;
                }
                x if x.len() == 1 => {
                    // remote ip
                    if let Some(iter) = remote_ips_streams_tree_store.get_iter(&path) {
                        let remote_ip = remote_ips_streams_tree_store.get_value(&iter, 0);
                        allowed_ips.push(remote_ip.get().unwrap().unwrap());
                    }
                }
                x if x.len() == 2 => {
                    // stream
                    let stream_iter = remote_ips_streams_tree_store.get_iter(&path).unwrap();
                    let stream_id = remote_ips_streams_tree_store.get_value(&stream_iter, 2);
                    allowed_stream_ids.push(stream_id.get().unwrap().unwrap());
                }
                _ => panic!(path.get_depth()),
            }
        }
        self.model
            .relm
            .stream()
            .emit(Msg::SelectCardFromRemoteIpsAndStreams(
                self.model.selected_card.as_ref().unwrap().clone(),
                allowed_ips,
                allowed_stream_ids,
            ));
    }

    fn open_file(&mut self) {
        let dialog = gtk::FileChooserNativeBuilder::new()
            .action(gtk::FileChooserAction::Open)
            .title("Select file")
            .modal(true)
            .build();
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.pcap");
        filter.add_pattern("*.pcapng");
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
        let packets = invoke_tshark(&fname, TSharkMode::Json, "http || pgsql || http2")
            .expect("tshark error");
        finished_tshark.send(()).unwrap();
        Self::handle_packets(fname, packets, sender)
    }

    fn handle_packets(
        fname: PathBuf,
        packets: Vec<tshark_communication::TSharkPacket>,
        sender: relm::Sender<LoadedDataParams>,
    ) {
        let by_stream = {
            let mut by_stream: Vec<_> = packets
                .into_iter()
                .map(|p| (p.tcp_stream_id, p))
                .into_group_map()
                .into_iter()
                .collect();
            by_stream.sort_by_key(|p| Reverse(p.1.len()));
            by_stream
        };

        let message_parsers = get_message_parsers();

        let parsed_streams = {
            let mut parsed_streams: Vec<_> = by_stream
                .into_iter()
                .filter_map(|(id, comms)| {
                    let parser = comms
                        .iter()
                        .find_map(|c| message_parsers.iter().find(|p| p.is_my_message(c)));

                    parser
                        .map(|p| {
                            let stream_data = p.parse_stream(comms);
                            let card_key = (stream_data.server_ip.clone(), stream_data.server_port);
                            (
                                p,
                                id,
                                stream_data.server_ip.clone(),
                                stream_data.client_ip.clone(),
                                card_key,
                                stream_data,
                            )
                        })
                        .filter(|(_p, _id, _srv_ip, _client_ip, _card_key, stream_data)| {
                            !stream_data.messages.is_empty()
                        })
                })
                .collect();
            parsed_streams
                .sort_by_key(|(_parser, id, _srv_ip, _client_ip, _card_key, _pstream)| *id);
            parsed_streams
        };

        let comm_target_cards = parsed_streams
            .iter()
            .fold(
                HashMap::<(String, u32), CommTargetCardData>::new(),
                |mut sofar, (parser, _stream_id, srv_ip, client_ip, card_key, items)| {
                    if let Some(target_card) = sofar.get_mut(&card_key) {
                        target_card.remote_hosts.insert(client_ip.to_string());
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
                        remote_hosts.insert(client_ip.to_string());
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
            .map(|(_parser, id, srv_ip, client_ip, card_key, pstream)| {
                (
                    StreamInfo {
                        stream_id: id,
                        target_ip: card_key.0,
                        target_port: card_key.1,
                        source_ip: client_ip,
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
        let remote_ips_streams_tree_store = gtk::TreeStore::new(&[
            String::static_type(),
            pango::Weight::static_type(),
            u32::static_type(),
        ]);
        remote_ips_streams_tree_store.insert_with_values(
            None,
            None,
            &[0, 1],
            &[&"All".to_value(), &pango::Weight::Bold.to_glib().to_value()],
        );
        // self.widgets.remote_ips_streams_treeview.set_cursor(
        //     &gtk::TreePath::new_first(),
        //     None::<&gtk::TreeViewColumn>,
        //     false,
        // );
        let target_ip = card.ip.clone();
        let target_port = card.port;

        for remote_ip in remote_ips {
            let remote_ip_iter = remote_ips_streams_tree_store.insert_with_values(
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
                remote_ips_streams_tree_store.insert_with_values(
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

        self.widgets
            .remote_ips_streams_treeview
            .set_model(Some(&remote_ips_streams_tree_store));
        self.widgets.remote_ips_streams_treeview.expand_all();
    }

    fn refresh_remote_servers(
        &mut self,
        refresh_remote_ips_and_streams: RefreshRemoteIpsAndStreams,
        constrain_remote_ips: &[String],
        constrain_stream_ids: &[u32],
    ) {
        self.setup_selection_signals(RefreshOngoing::Yes);
        if let Some(card) = self.model.selected_card.as_ref().cloned() {
            let target_ip = card.ip.clone();
            let target_port = card.port;
            let mut by_remote_ip = HashMap::new();
            let parsers = get_message_parsers();
            for (stream_info, messages) in &self.model.streams {
                if stream_info.target_ip != target_ip || stream_info.target_port != target_port {
                    continue;
                }
                let allowed_all =
                    constrain_remote_ips.is_empty() && constrain_stream_ids.is_empty();

                let allowed_ip = constrain_remote_ips.contains(&stream_info.source_ip);
                let allowed_stream = constrain_stream_ids.contains(&stream_info.stream_id);
                let allowed = allowed_all || allowed_ip || allowed_stream;

                if !allowed {
                    continue;
                }
                let remote_server_streams = by_remote_ip
                    .entry(stream_info.source_ip.clone())
                    .or_insert_with(Vec::new);
                remote_server_streams.push((stream_info.stream_id, messages));
            }
            let mp = parsers.get(card.protocol_index).unwrap();
            self.widgets
                .comm_remote_servers_stack
                .set_visible_child_name(&card.protocol_index.to_string());
            let (ref tv, ref _signals) = &self
                .model
                .comm_remote_servers_treeviews
                .get(card.protocol_index)
                .unwrap();
            let ls = mp.get_empty_liststore();
            for (remote_ip, tcp_sessions) in &by_remote_ip {
                for (session_id, session) in tcp_sessions {
                    let mut idx = 0;
                    for chunk in session.chunks(100) {
                        mp.populate_treeview(&ls, *session_id, chunk, idx);
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
                                    },
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
                    orientation: gtk::Orientation::Vertical,
                    hexpand: true,
                    #[name="infobar"]
                    gtk::InfoBar {
                        response(_, r) => Msg::InfoBarEvent(r)
                    },
                    gtk::Box {
                        vexpand: true,
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
                                // connecting manually to collect the signal id for blocking
                                // selection.changed(selection) => Msg::SelectRemoteIpStream(selection.clone()),
                            },
                        },
                        gtk::Separator {
                            orientation: gtk::Orientation::Vertical,
                        },
                        #[name="comm_remote_servers_stack"]
                        gtk::Stack {}
                    }
                },
            },
            delete_event(_, _) => (Msg::Quit, Inhibit(false)),
        }
    }
}
