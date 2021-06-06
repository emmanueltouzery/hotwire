use super::comm_remote_server::MessageData;
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::headerbar_search::HeaderbarSearch;
use super::headerbar_search::Msg as HeaderbarSearchMsg;
use super::headerbar_search::Msg::SearchActiveChanged as HbsMsgSearchActiveChanged;
use super::headerbar_search::Msg::SearchTextChanged as HbsMsgSearchTextChanged;
use super::recent_file_item::RecentFileItem;
use crate::colors;
use crate::http::http_message_parser::Http;
use crate::http2::http2_message_parser::Http2;
use crate::icons::Icon;
use crate::message_parser::ClientServerInfo;
use crate::message_parser::StreamData;
use crate::message_parser::{MessageInfo, MessageParser};
use crate::pgsql::postgres_message_parser::Postgres;
use crate::tshark_communication;
use crate::tshark_communication::TSharkPacket;
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
use std::io::BufRead;
use std::io::BufReader;
use std::net::IpAddr;
use std::path::Path;
use std::path::PathBuf;
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

#[derive(Debug)]
pub enum InputStep {
    Packet(TSharkPacket),
    Eof,
}

pub type ParseInputStep = Result<InputStep, String>;

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
    OpenRecentFile(usize),
    DisplayAbout,

    KeyPress(gdk::EventKey),
    SearchActiveChanged(bool),
    SearchTextChanged(String),

    FinishedTShark,
    LoadedData(ParseInputStep),

    SelectCard(Option<usize>),
    SelectRemoteIpStream(gtk::TreeSelection),

    InfoBarShow(Option<String>, InfobarOptions),
    InfoBarEvent(gtk::ResponseType),

    SelectCardFromRemoteIpsAndStreams(CommTargetCardData, Vec<IpAddr>, Vec<u32>),

    DisplayDetails(u32, u32),

    Quit,
}

#[derive(Debug)]
pub struct StreamInfo {
    stream_id: u32,
    target_ip: IpAddr,
    target_port: u32,
    source_ip: IpAddr,
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
    recent_files: Vec<PathBuf>,

    infobar_spinner: gtk::Spinner,
    infobar_label: gtk::Label,

    message_parsers: Vec<Box<dyn MessageParser>>,

    sidebar_selection_change_signal_id: Option<glib::SignalHandlerId>,

    streams: HashMap<u32, StreamData>, // tcp_stream_id => streamdata
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    comm_remote_servers_treeviews: Vec<(gtk::TreeView, TreeViewSignals)>,

    _finished_tshark_channel: relm::Channel<()>,
    finished_tshark_sender: relm::Sender<()>,

    _loaded_data_channel: relm::Channel<ParseInputStep>,
    loaded_data_sender: relm::Sender<ParseInputStep>,

    _comm_targets_components: Vec<Component<CommTargetCard>>,
    _recent_file_item_components: Vec<Component<RecentFileItem>>,

    details_component_emitters: Vec<Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)>>,
    details_adjustments: Vec<gtk::Adjustment>,
}

#[derive(PartialEq, Eq)]
enum RefreshRemoteIpsAndStreams {
    Yes,
    No,
}

#[derive(PartialEq, Eq)]
pub enum TSharkInputType {
    File,
    Fifo,
}

// it would be possible to ask tshark to "mix in" a keylog file
// when opening the pcap file
// (obtain the keylog file through `SSLKEYLOGFILE=browser_keylog.txt google-chrome` or firefox,
// pass it to tshark through -o ssh.keylog_file:/path/to/keylog)
// but we get in flatpak limitations (can only access the file that the user opened
// due to the sandbox) => better to just mix in the secrets manually and open a single
// file. this is done through => editcap --inject-secrets tls,/path/to/keylog.txt ~/testtls.pcap ~/outtls.pcapng
pub fn invoke_tshark(
    input_type: TSharkInputType,
    fname: &Path,
    filters: &str,
    sender: relm::Sender<ParseInputStep>,
) {
    dbg!(&filters);
    // piping from tshark, not to load the entire JSON in ram...
    let tshark_child = Command::new("tshark")
        .args(&[
            if input_type == TSharkInputType::File {
                "-r"
            } else {
                "-i"
            },
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
    parse_pdml_stream(buf_reader, sender);
}

pub fn parse_pdml_stream<B: BufRead>(buf_reader: B, sender: relm::Sender<ParseInputStep>) {
    let mut xml_reader = quick_xml::Reader::from_reader(buf_reader);
    let mut buf = vec![];
    loop {
        match xml_reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name() == b"packet" {
                    if let Ok(packet) = tshark_communication::parse_packet(&mut xml_reader) {
                        sender.send(Ok(InputStep::Packet(packet))).unwrap();
                    }
                }
            }
            Ok(Event::Eof) => {
                sender.send(Ok(InputStep::Eof)).unwrap();
                break;
            }
            Err(e) => {
                sender
                    .send(Err(format!("xml parsing error: {}", e)))
                    .unwrap();
                break;
            }
            _ => {}
        };
        buf.clear();
    }
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

        self.widgets
            .recent_files_list
            .set_header_func(Some(Box::new(|row, _h| {
                row.set_header(Some(&gtk::SeparatorBuilder::new().build()));
            })));

        self.refresh_recent_files();

        // self.refresh_comm_targets();
        // self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
        let path = self.model.current_file_path.as_ref().cloned();
        if let Some(p) = path {
            self.gui_load_file(p);
        }
    }

    fn refresh_recent_files(&mut self) {
        for child in self.widgets.recent_files_list.get_children() {
            self.widgets.recent_files_list.remove(&child);
        }
        self.model.recent_files.clear();
        self.model._recent_file_item_components.clear();
        let rm = gtk::RecentManager::get_default().unwrap();
        rm.get_items()
            .into_iter()
            .filter(|fi| {
                fi.get_mime_type()
                    .filter(|m| m.as_str() == "application/vnd.tcpdump.pcap")
                    .is_some()
            })
            .take(5)
            .flat_map(|fi| fi.get_uri())
            .map(|gs| PathBuf::from(gs.as_str()))
            .for_each(|pb| {
                self.model.recent_files.push(pb.clone());
                self.model._recent_file_item_components.push(
                    self.widgets
                        .recent_files_list
                        .add_widget::<RecentFileItem>(pb),
                );
            });
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
                    if let Some(childpath) =
                        path.and_then(|p| modelsort.convert_path_to_child_path(&p))
                    {
                        Self::row_selected(&model, &childpath, &rstream);
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

    fn model(relm: &relm::Relm<Self>, params: (mpsc::Sender<BgFunc>, Option<PathBuf>)) -> Model {
        let (bg_sender, current_file_path) = params;
        gtk::IconTheme::get_default()
            .unwrap()
            .add_resource_path("/icons");

        let (_loaded_data_channel, loaded_data_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |ch_data: ParseInputStep| {
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
            message_parsers: get_message_parsers(),
            infobar_spinner: gtk::SpinnerBuilder::new()
                .width_request(24)
                .height_request(24)
                .build(),
            infobar_label: gtk::LabelBuilder::new().build(),
            _comm_targets_components: vec![],
            _recent_file_item_components: vec![],
            recent_files: vec![],
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
            current_file_path,
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
            Msg::DisplayAbout => {
                self.display_about();
            }
            Msg::OpenFile => {
                self.open_file();
            }
            Msg::KeyPress(e) => {
                self.handle_keypress(e);
            }
            Msg::SearchActiveChanged(is_active) => {
                if let Some((protocol_index, tv, model_sort)) = self.get_model_sort() {
                    tv.set_model(Some(&model_sort));
                }
            }
            Msg::SearchTextChanged(txt) => {
                self.search_text_changed(txt);
            }
            Msg::OpenRecentFile(idx) => {
                let path = self.model.recent_files[idx].clone();
                self.gui_load_file(path);
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
            Msg::LoadedData(Err(msg)) => {
                self.widgets.loading_spinner.stop();
                self.widgets
                    .root_stack
                    .set_visible_child_name(WELCOME_STACK_NAME);
                self.model.window_subtitle = None;
                self.model.current_file_path = None;
                self.model.streams = vec![];
                self.refresh_comm_targets();
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
                let dialog = gtk::MessageDialog::new(
                    None::<&gtk::Window>,
                    gtk::DialogFlags::all(),
                    gtk::MessageType::Error,
                    gtk::ButtonsType::Close,
                    "Cannot load file",
                );
                dialog.set_property_secondary_text(Some(&msg));
                let _r = dialog.run();
                dialog.close();
            }
            Msg::LoadedData(Ok(InputStep::Packet(p))) => {
                if let Some((parser_index, parser)) = self
                    .model
                    .message_parsers
                    .iter()
                    .enumerate()
                    .find(|(_idx, ps)| ps.is_my_message(&p))
                {
                    let packet_stream_id = p.basic_info.tcp_stream_id;
                    let existing_stream = self.model.streams.get(&packet_stream_id);
                    if let Some(ref mut stream_data) = existing_stream {
                        let had_client_server = stream_data.client_server.is_some();
                        parser.add_to_stream(stream_data, p);
                        if !had_client_server && stream_data.client_server.is_some() {
                            // we got the client-server info for this stream, add the
                            // comm target data.
                            self.add_comm_target_data(
                                parser_index,
                                &parser,
                                stream_data.client_server.as_ref().unwrap(),
                                stream_data.summary_details.as_deref(),
                            );
                        }
                    } else {
                        // new stream
                        let mut stream_data = StreamData {
                            stream_globals: parser.initial_globals(),
                            client_server: None,
                            messages: vec![],
                            summary_details: None,
                        };
                        if let Err(msg) = parser.add_to_stream(&mut stream_data, p) {
                            self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                                "Error parsing file, in stream {}: {}",
                                packet_stream_id, msg
                            ))));
                            return;
                        }
                        self.model.streams.insert(packet_stream_id, stream_data);
                        if stream_data.client_server.is_some() {
                            // we got the client-server info for this stream, add the
                            // comm target data.
                            self.add_comm_target_data(
                                parser_index,
                                &parser,
                                stream_data.client_server.as_ref().unwrap(),
                                stream_data.summary_details.as_deref(),
                            );
                        }
                    }
                    self.refresh_comm_targets();
                    self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
                }
            }
            Msg::LoadedData(Ok(InputStep::Eof)) => {
                if self.model.streams.is_empty() {
                    self.model.relm.stream().emit(Msg::LoadedData(Err(
                        "Hotwire doesn't know how to read any useful data from this file"
                            .to_string(),
                    )));
                    return;
                }
                self.widgets.loading_spinner.stop();
                self.model.window_subtitle = Some(
                    fname
                        // .file_name().unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
                self.model.current_file_path = Some(fname);
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
                                client_ip: stream_info.source_ip,
                                message_data: msg_data.clone(),
                            },
                        );
                    }
                }
            }
            Msg::Quit => gtk::main_quit(),
        }
    }

    fn add_comm_target_data(
        &mut self,
        protocol_index: usize,
        parser: &Box<dyn MessageParser>,
        client_server_info: &ClientServerInfo,
        summary_details: Option<&str>,
    ) {
        if let Some(card) = self.model.comm_target_cards.iter().find(|c| {
            c.protocol_index == protocol_index
                && c.port == client_server_info.server_port
                && c.ip == client_server_info.server_ip
        }) {
            // update existing card
            card.incoming_session_count += 1;
            card.remote_hosts.insert(client_server_info.client_ip);
            if card.summary_details.is_none() && summary_details.is_some() {
                card.summary_details = Some(SummaryDetails {
                    details: summary_details.unwrap().to_string(),
                });
            }
        } else {
            // add new card
            self.model.comm_target_cards.push(CommTargetCardData {
                ip: client_server_info.server_ip,
                port: client_server_info.server_port,
                protocol_index,
                remote_hosts: {
                    let bs = BTreeSet::new();
                    bs.insert(client_server_info.client_ip.to_string());
                    bs
                },
                protocol_icon: parser.protocol_icon(),
                summary_details: summary_details.map(|d| SummaryDetails {
                    details: d.to_string(),
                }),
                incoming_session_count: 1,
            });
        }
    }

    fn handle_keypress(&self, e: gdk::EventKey) {
        if e.get_keyval() == gdk::keys::constants::Escape {
            self.components
                .headerbar_search
                .emit(HeaderbarSearchMsg::SearchActiveChanged(false));
        }
        if let Some(k) = e.get_keyval().to_unicode() {
            if Self::is_plaintext_key(&e) {
                // we don't want to trigger the global search if the
                // note search text entry is focused.
                if self
                    .widgets
                    .window
                    .get_focus()
                    // is an entry focused?
                    .and_then(|w| w.downcast::<gtk::Entry>().ok())
                    // is it visible? (because when global search is off,
                    // the global search entry can be focused but invisible)
                    .filter(|w| w.get_visible())
                    .is_some()
                {
                    // the focused widget is a visible entry, and
                    // we're not in search mode => don't grab this
                    // key event, this is likely a note search
                    return;
                }

                // self.model
                //     .relm
                //     .stream()
                //     .emit(Msg::SearchActiveChanged(true));
                // self.components
                //     .headerbar_search
                //     .emit(SearchViewMsg::FilterChanged(Some(k.to_string())));
                self.components.headerbar_search.emit(
                    HeaderbarSearchMsg::SearchTextChangedFromElsewhere((k.to_string(), e)),
                );
            }
        }
    }

    fn get_model_sort(&self) -> Option<(usize, &gtk::TreeView, gtk::TreeModelSort)> {
        if let Some(card) = self.model.selected_card.as_ref() {
            let (ref tv, ref _signals) = &self
                .model
                .comm_remote_servers_treeviews
                .get(card.protocol_index)
                .unwrap();
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
            Some((card.protocol_index, tv, model_sort))
        } else {
            None
        }
    }

    fn search_text_changed(&mut self, txt: String) {
        if let Some((protocol_index, tv, model_sort)) = self.get_model_sort() {
            let parsers = get_message_parsers();
            let new_model_filter = gtk::TreeModelFilter::new(&model_sort, None);
            new_model_filter.set_visible_func(move |model, iter| {
                let mp = parsers.get(protocol_index).unwrap();
                mp.matches_filter(&txt, model, iter)
            });
            tv.set_model(Some(&new_model_filter));
        }
    }

    fn is_plaintext_key(e: &gdk::EventKey) -> bool {
        // return false if control and others were pressed
        // (then the state won't be empty)
        // could be ctrl-c on notes for instance
        // whitelist MOD2 (num lock) and LOCK (shift or caps lock)
        let mut state = e.get_state();
        state.remove(gdk::ModifierType::MOD2_MASK);
        state.remove(gdk::ModifierType::LOCK_MASK);
        state.is_empty()
            && e.get_keyval() != gdk::keys::constants::Return
            && e.get_keyval() != gdk::keys::constants::KP_Enter
            && e.get_keyval() != gdk::keys::constants::Escape
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
                        let remote_ip: Option<String> = remote_ips_streams_tree_store
                            .get_value(&iter, 0)
                            .get()
                            .unwrap();
                        allowed_ips.push(remote_ip.unwrap().parse::<IpAddr>().unwrap());
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

    fn display_about(&mut self) {
        let tshark_version = Command::new("tshark")
            .args(&["--version"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "Failed running tshark".to_string());
        let dlg = gtk::AboutDialogBuilder::new()
            .name("Hotwire")
            .version(env!("CARGO_PKG_VERSION"))
            .logo_icon_name(Icon::APP_ICON.name())
            .website("https://github.com/emmanueltouzery/hotwire/")
            .comments("Explore the contents of network capture files")
            .build();
        dlg.add_credit_section("tshark", &[&tshark_version]);
        dlg.run();
        dlg.close();
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
                self.gui_load_file(fname);
            }
        }
    }

    fn gui_load_file(&mut self, fname: PathBuf) {
        self.components
            .headerbar_search
            .emit(HeaderbarSearchMsg::SearchActiveChanged(false));
        self.widgets.open_btn.set_active(false);
        let rm = gtk::RecentManager::get_default().unwrap();
        if let Some(fname_str) = fname.to_str() {
            let recent_data = gtk::RecentData {
                display_name: None,
                description: None,
                mime_type: "application/vnd.tcpdump.pcap".to_string(),
                app_name: "hotwire".to_string(),
                app_exec: "hotwire".to_string(),
                groups: vec![],
                is_private: false,
            };
            rm.add_full(fname_str, &recent_data);
        }
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
        self.refresh_recent_files();
    }

    fn load_file(
        fname: PathBuf,
        sender: relm::Sender<ParseInputStep>,
        finished_tshark: relm::Sender<()>,
    ) {
        invoke_tshark(&fname, "http || pgsql || http2", sender);
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
        remote_ips: &HashSet<IpAddr>,
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
        let target_ip = card.ip;
        let target_port = card.port;

        for remote_ip in remote_ips {
            let remote_ip_iter = remote_ips_streams_tree_store.insert_with_values(
                None,
                None,
                &[0, 1],
                &[
                    &remote_ip.to_string().to_value(),
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
        constrain_remote_ips: &[IpAddr],
        constrain_stream_ids: &[u32],
    ) {
        self.setup_selection_signals(RefreshOngoing::Yes);
        if let Some(card) = self.model.selected_card.as_ref().cloned() {
            let target_ip = card.ip;
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
                    .entry(stream_info.source_ip)
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
                let ip_hash = by_remote_ip.keys().copied().collect::<HashSet<_>>();
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
                    #[name="open_btn"]
                    gtk::MenuButton {
                        image: Some(&gtk::Image::from_icon_name(Some("pan-down-symbolic"), gtk::IconSize::Menu)),
                        image_position: gtk::PositionType::Right,
                        label: "Open",
                        always_show_image: true,
                        active: false,
                        popover: view! {
                            gtk::Popover {
                                property_width_request: 450,
                                visible: false,
                                gtk::Box {
                                    orientation: gtk::Orientation::Vertical,
                                    margin_top: 10,
                                    margin_start: 10,
                                    margin_end: 10,
                                    margin_bottom: 10,
                                    spacing: 10,
                                    gtk::Frame {
                                        #[name="recent_files_list"]
                                        gtk::ListBox {
                                            // no selection: i dont want the blue background on the first recent file by default,
                                            // it doesn't look good. I tried preventing it by focusing the "Other documents" button, but failed.
                                            selection_mode: gtk::SelectionMode::None,
                                            activate_on_single_click: true,
                                            row_activated(_, row) =>
                                                Msg::OpenRecentFile(row.get_index() as usize)
                                        },
                                    },
                                    gtk::Button {
                                        label: "Other Documents...",
                                        hexpand: true,
                                        clicked => Msg::OpenFile,
                                    }
                                }
                            }
                        },
                    },
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
                                    orientation: gtk::Orientation::Vertical,
                                    margin_top: 10,
                                    margin_start: 10,
                                    margin_end: 10,
                                    margin_bottom: 10,
                                    gtk::ModelButton {
                                        label: "About Hotwire",
                                        hexpand: true,
                                        clicked => Msg::DisplayAbout,
                                    },
                                }
                            }
                        },
                    },
                    #[name="headerbar_search"]
                    HeaderbarSearch {
                        child: {
                            pack_type: gtk::PackType::End,
                        },
                        HbsMsgSearchActiveChanged(is_active) => Msg::SearchActiveChanged(is_active),
                        HbsMsgSearchTextChanged(ref txt) => Msg::SearchTextChanged(txt.clone()),
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
            key_press_event(_, event) => (Msg::KeyPress(event.clone()), Inhibit(false)),
            delete_event(_, _) => (Msg::Quit, Inhibit(false)),
        }
    }
}
