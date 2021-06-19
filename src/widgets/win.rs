use super::comm_target_card;
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::headerbar_search::HeaderbarSearch;
use super::headerbar_search::Msg as HeaderbarSearchMsg;
use super::headerbar_search::Msg::SearchActiveChanged as HbsMsgSearchActiveChanged;
use super::headerbar_search::Msg::SearchTextChanged as HbsMsgSearchTextChanged;
use super::recent_file_item::RecentFileItem;
use crate::colors;
use crate::config;
use crate::http::http_message_parser::Http;
use crate::http2::http2_message_parser::Http2;
use crate::icons::Icon;
use crate::message_parser::ClientServerInfo;
use crate::message_parser::StreamData;
use crate::message_parser::{MessageInfo, MessageParser};
use crate::pgsql::postgres_message_parser::Postgres;
use crate::tshark_communication;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpStreamId};
use crate::widgets::comm_target_card::CommTargetCardKey;
use crate::widgets::comm_target_card::SummaryDetails;
use crate::BgFunc;
use gdk::prelude::*;
use glib::translate::ToGlib;
use gtk::prelude::*;
use itertools::Itertools;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use quick_xml::events::Event;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use signal_hook::iterator::Signals;
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::BufRead;
use std::io::BufReader;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
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
    StartedTShark(Child),
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
    CaptureToggled,
    SaveCapture,
    ChildProcessDied,

    DragDataReceived(gdk::DragContext, gtk::SelectionData),

    KeyPress(gdk::EventKey),
    SearchActiveChanged(bool),
    SearchTextChanged(String),

    LoadedData(ParseInputStep),

    SelectCard(Option<usize>),
    SelectRemoteIpStream(gtk::TreeSelection),

    InfoBarShow(Option<String>, InfobarOptions),
    InfoBarEvent(gtk::ResponseType),

    SelectCardFromRemoteIpsAndStreams(CommTargetCardData, Vec<IpAddr>, Vec<TcpStreamId>),

    DisplayDetails(TcpStreamId, u32),

    Quit,
}

#[derive(Debug)]
pub struct StreamInfo {
    stream_id: TcpStreamId,
    target_ip: IpAddr,
    target_port: NetworkPort,
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
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum FileType {
    File,
    Fifo,
}

pub struct Model {
    relm: relm::Relm<Win>,
    bg_sender: mpsc::Sender<BgFunc>,

    window_subtitle: Option<String>,
    current_file: Option<(PathBuf, FileType)>,
    recent_files: Vec<PathBuf>,

    capture_toggle_signal: Option<glib::SignalHandlerId>,

    infobar_spinner: gtk::Spinner,
    infobar_label: gtk::Label,

    sidebar_selection_change_signal_id: Option<glib::SignalHandlerId>,

    streams: HashMap<TcpStreamId, StreamData>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    comm_remote_servers_treeviews: Vec<(gtk::TreeView, TreeViewSignals)>,

    _loaded_data_channel: relm::Channel<ParseInputStep>,
    loaded_data_sender: relm::Sender<ParseInputStep>,

    cur_liststore: Option<(CommTargetCardKey, gtk::ListStore, i32)>,
    remote_ips_streams_treestore: gtk::TreeStore,
    remote_ips_streams_iptopath: HashMap<IpAddr, gtk::TreePath>,

    comm_targets_components: HashMap<CommTargetCardKey, Component<CommTargetCard>>,
    _recent_file_item_components: Vec<Component<RecentFileItem>>,

    details_component_emitters: Vec<Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)>>,
    details_adjustments: Vec<gtk::Adjustment>,

    tcpdump_child: Option<Child>,
    tshark_child: Option<Child>,
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
    let mut tshark_params = vec![
        if input_type == TSharkInputType::File {
            "-r"
        } else {
            "-i"
        },
        fname.to_str().expect("invalid filename"),
        "-Tpdml",
        // "-o",
        // "ssl.keylog_file:/home/emmanuel/chrome_keylog.txt",
        // "tcp.stream eq 104",
    ];
    let pcap_output = config::get_tshark_pcap_output_path();
    if input_type == TSharkInputType::Fifo {
        // -l == flush after each packet
        tshark_params.extend(&["-w", pcap_output.to_str().unwrap(), "-l"]);
    } else {
        // if I filter in fifo mode then tshark doesn't write the output pcap file
        tshark_params.extend(&[filters]);
    }
    let tshark_child = Command::new("tshark")
        .args(&tshark_params)
        .stdout(Stdio::piped())
        .spawn();
    if tshark_child.is_err() {
        sender
            .send(Err(format!("Error launching tshark: {:?}", tshark_child)))
            .unwrap();
        return;
    }
    let mut tshark_child = tshark_child.unwrap();
    let buf_reader = BufReader::new(tshark_child.stdout.take().unwrap());
    sender
        .send(Ok(InputStep::StartedTShark(tshark_child)))
        .unwrap();
    parse_pdml_stream(buf_reader, sender);
}

pub fn parse_pdml_stream<B: BufRead>(buf_reader: B, sender: relm::Sender<ParseInputStep>) {
    let mut xml_reader = quick_xml::Reader::from_reader(buf_reader);
    let mut buf = vec![];
    loop {
        match xml_reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name() == b"packet" {
                    match tshark_communication::parse_packet(&mut xml_reader) {
                        Ok(packet) => sender.send(Ok(InputStep::Packet(packet))).unwrap(),
                        Err(e) => {
                            sender
                                .send(Err(format!(
                                    "xml parsing error: {} at tshark output offset {}",
                                    e,
                                    xml_reader.buffer_position()
                                )))
                                .unwrap();
                            break;
                        }
                    }
                }
            }
            Ok(Event::Eof) => {
                sender.send(Ok(InputStep::Eof)).unwrap();
                break;
            }
            Err(e) => {
                sender
                    .send(Err(format!(
                        "xml parsing error: {} at tshark output offset {}",
                        e,
                        xml_reader.buffer_position()
                    )))
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

        self.widgets.welcome_label.drag_dest_set(
            gtk::DestDefaults::ALL,
            &[],
            gdk::DragAction::COPY,
        );
        self.widgets.welcome_label.drag_dest_add_uri_targets();

        // the capture depends on pkexec for privilege escalation
        // which is linux-specific, and then fifos which are unix-specific.
        self.widgets
            .capture_btn
            .set_visible(Self::is_display_capture_btn());

        let stream = self.model.relm.stream().clone();
        self.model.capture_toggle_signal =
            Some(self.widgets.capture_btn.connect_toggled(move |_| {
                stream.emit(Msg::CaptureToggled);
            }));

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
        let path = self.model.current_file.as_ref().map(|(p, _t)| p).cloned();
        if let Some(p) = path {
            self.gui_load_file(p);
        }
    }

    fn is_display_capture_btn() -> bool {
        cfg!(target_os = "linux")
    }

    fn refresh_recent_files(&mut self) {
        for child in self.widgets.recent_files_list.get_children() {
            self.widgets.recent_files_list.remove(&child);
        }
        self.model.recent_files.clear();
        self.model._recent_file_item_components.clear();
        let rm = gtk::RecentManager::get_default().unwrap();
        let mut items = rm.get_items();
        let normalize_uri = |uri: Option<glib::GString>| {
            uri.map(|u| u.to_string()).map(|u| {
                if u.starts_with("/") {
                    format!("file://{}", u)
                } else {
                    u
                }
            })
        };
        items.sort_by_key(|i| normalize_uri(i.get_uri()));
        items.dedup_by_key(|i| normalize_uri(i.get_uri()));
        items.sort_by_key(|i| Reverse(i.get_modified()));
        items
            .into_iter()
            .filter(|i| i.last_application().map(|a| a.to_string()) == Some("hotwire".to_string()))
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
        mp_idx: usize,
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
            .add_named(&paned, &mp_idx.to_string());
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
            TcpStreamId(stream_id.get::<u32>().unwrap().unwrap()),
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

    fn model(
        relm: &relm::Relm<Self>,
        params: (mpsc::Sender<BgFunc>, Option<(PathBuf, FileType)>),
    ) -> Model {
        let (bg_sender, current_file) = params;
        gtk::IconTheme::get_default()
            .unwrap()
            .add_resource_path("/icons");

        let (_loaded_data_channel, loaded_data_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |ch_data: ParseInputStep| {
                stream.emit(Msg::LoadedData(ch_data));
            })
        };

        {
            // the problem i'm trying to fix is the user triggering
            // a capture... so we call pkexec to launch tcpdump.. but the user closes pkexec and
            // so tcpdump will never be launched.
            // then we have tshark blocking on the fifo, forever.
            // i catch the SIGCHLD signal, which tells me that one of my child processes died
            // (potentially pkexec). At that point if the capture is running, I stop it and clean up.
            let stream = relm.stream().clone();

            let (_channel, sender) = relm::Channel::new(move |()| {
                // This closure is executed whenever a message is received from the sender.
                // We send a message to the current widget.
                stream.emit(Msg::ChildProcessDied);
            });
            thread::spawn(move || {
                const SIGNALS: &[libc::c_int] = &[signal_hook::consts::signal::SIGCHLD];
                let mut sigs = Signals::new(SIGNALS).unwrap();
                for signal in &mut sigs {
                    sender.send(()).expect("send child died msg");
                    if let Err(e) = signal_hook::low_level::emulate_default_handler(signal) {
                        eprintln!("Error calling the low-level signal hook handling: {:?}", e);
                    }
                }
            });
        }

        Model {
            relm: relm.clone(),
            bg_sender,
            infobar_spinner: gtk::SpinnerBuilder::new()
                .width_request(24)
                .height_request(24)
                .build(),
            infobar_label: gtk::LabelBuilder::new().build(),
            comm_targets_components: HashMap::new(),
            _recent_file_item_components: vec![],
            recent_files: vec![],
            selected_card: None,
            comm_remote_servers_treeviews: vec![],
            details_component_emitters: vec![],
            details_adjustments: vec![],
            loaded_data_sender,
            _loaded_data_channel,
            sidebar_selection_change_signal_id: None,
            comm_target_cards: vec![],
            streams: HashMap::new(),
            cur_liststore: None,
            current_file,
            capture_toggle_signal: None,
            window_subtitle: None,
            remote_ips_streams_iptopath: HashMap::new(),
            remote_ips_streams_treestore: gtk::TreeStore::new(&[
                String::static_type(),
                pango::Weight::static_type(),
                u32::static_type(),
            ]),
            tcpdump_child: None,
            tshark_child: None,
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
            Msg::DragDataReceived(context, sel_data) => {
                if let Some(uri) = sel_data
                    .get_uris()
                    .first()
                    .map(|u| u.as_str())
                    .and_then(|u| u.strip_prefix("file://"))
                {
                    self.gui_load_file(uri.into());
                }
            }
            Msg::OpenFile => {
                self.open_file();
            }
            Msg::CaptureToggled => {
                if let Err(e) = self.handle_capture_toggled() {
                    Self::display_error_block(
                        "Error capturing network traffic",
                        Some(&e.to_string()),
                    );
                    self.widgets.capture_btn.set_active(false);
                }
            }
            Msg::SaveCapture => {
                self.handle_save_capture();
            }
            Msg::ChildProcessDied => {
                // the problem i'm trying to fix is the user triggering
                // a capture... so we call pkexec to launch tcpdump.. but the user closes pkexec and
                // so tcpdump will never be launched.
                // then we have tshark blocking on the fifo, forever.
                // i catch the SIGCHLD signal, which tells me that one of my child processes died
                // (potentially pkexec). At that point if the capture is running, I stop it and clean up.
                self.widgets.capture_btn.set_active(false);
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
                // TODO clear the streams like we do when opening a new file?
                if let Err(e) = self.cleanup_child_processes() {
                    // not sure why loading failed.. maybe don't get too hung up
                    // about error handling in this case?
                    eprintln!("Error cleaning up child processes: {:?}", e);
                }
                self.widgets
                    .capture_btn
                    .block_signal(&self.model.capture_toggle_signal.as_ref().unwrap());
                self.widgets.capture_btn.set_active(false);
                self.widgets.capture_spinner.set_visible(false);
                self.widgets.capture_spinner.stop();
                self.widgets
                    .capture_btn
                    .unblock_signal(&self.model.capture_toggle_signal.as_ref().unwrap());
                self.widgets.loading_spinner.stop();
                self.widgets
                    .root_stack
                    .set_visible_child_name(WELCOME_STACK_NAME);
                self.model.window_subtitle = None;
                self.model.current_file = None;
                self.model.streams = HashMap::new();
                // self.refresh_comm_targets();
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
                Self::display_error_block("Cannot load file", Some(&msg));
            }
            Msg::LoadedData(Ok(InputStep::StartedTShark(pid))) => {
                self.model.tshark_child = Some(pid);
            }
            Msg::LoadedData(Ok(InputStep::Packet(p))) => {
                if let Some((parser_index, parser)) = get_message_parsers()
                    .iter()
                    .enumerate()
                    .find(|(_idx, ps)| ps.is_my_message(&p))
                {
                    let packet_stream_id = p.basic_info.tcp_stream_id;
                    let existing_stream = self.model.streams.remove(&packet_stream_id);
                    let message_count_before;
                    let stream_data = if let Some(stream_data) = existing_stream {
                        message_count_before = stream_data.messages.len();
                        let had_client_server = stream_data.client_server.is_some();
                        let stream_data = match parser.add_to_stream(stream_data, p) {
                            Ok(sd) => sd,
                            Err(msg) => {
                                self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                                    "Error parsing file, in stream {}: {}",
                                    packet_stream_id, msg
                                ))));
                                return;
                            }
                        };
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
                        stream_data
                    } else {
                        // new stream
                        message_count_before = 0;
                        let mut stream_data = StreamData {
                            parser_index,
                            stream_globals: parser.initial_globals(),
                            client_server: None,
                            messages: vec![],
                            summary_details: None,
                        };
                        match parser.add_to_stream(stream_data, p) {
                            Ok(sd) => {
                                stream_data = sd;
                            }
                            Err(msg) => {
                                self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                                    "Error parsing file, in stream {}: {}",
                                    packet_stream_id, msg
                                ))));
                                return;
                            }
                        }
                        if stream_data.client_server.is_some() {
                            // we got the client-server info for this stream, add the
                            // comm target data.
                            self.add_comm_target_data(
                                parser_index,
                                &parser,
                                stream_data.client_server.as_ref().unwrap(),
                                stream_data.summary_details.as_deref(),
                            );

                            let is_for_current_card = matches!(
                            (stream_data.client_server, self.model.selected_card.as_ref()),
                            (Some(clientserver), Some(card)) if clientserver.server_ip == card.ip
                                && clientserver.server_port == card.port
                                && parser_index == card.protocol_index);

                            if is_for_current_card {
                                let treestore = self.model.remote_ips_streams_treestore.clone();

                                let remote_ip_iter = self
                                    .model
                                    .remote_ips_streams_iptopath
                                    .get(&stream_data.client_server.as_ref().unwrap().client_ip)
                                    .and_then(|path| treestore.get_iter(&path))
                                    .unwrap_or_else(|| {
                                        let new_iter = treestore.insert_with_values(
                                            None,
                                            None,
                                            &[0, 1],
                                            &[
                                                &stream_data
                                                    .client_server
                                                    .as_ref()
                                                    .unwrap()
                                                    .client_ip
                                                    .to_string()
                                                    .to_value(),
                                                &pango::Weight::Normal.to_glib().to_value(),
                                            ],
                                        );
                                        self.model.remote_ips_streams_iptopath.insert(
                                            stream_data.client_server.as_ref().unwrap().client_ip,
                                            treestore.get_path(&new_iter).unwrap(),
                                        );
                                        new_iter
                                    });
                                // TODO some duplication with refresh_remote_ips_streams_tree()
                                self.model.remote_ips_streams_treestore.insert_with_values(
                                Some(&remote_ip_iter),
                                None,
                                &[0, 1, 2],
                                &[
                                    &format!(
                                        r#"<span foreground="{}" size="smaller">⬤</span> <span rise="-1700">Stream {}</span>"#,
                                        colors::STREAM_COLORS
                                            [packet_stream_id.as_u32() as usize % colors::STREAM_COLORS.len()],
                                        packet_stream_id.as_u32()
                                    )
                                        .to_value(),
                                    &pango::Weight::Normal.to_glib().to_value(),
                                    &packet_stream_id.as_u32().to_value(),
                                ],
                            );
                            }
                        }
                        stream_data
                    };
                    self.refresh_grids_new_messages(
                        packet_stream_id,
                        parser_index,
                        message_count_before,
                        stream_data,
                    );
                }
            }
            Msg::LoadedData(Ok(InputStep::Eof)) => {
                if let Err(e) = self.cleanup_child_processes() {
                    self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                        "Error cleaning up children processes: {}",
                        e
                    ))));
                }
                let keys: Vec<TcpStreamId> = self.model.streams.keys().map(|k| *k).collect();
                for stream_id in keys {
                    let stream_data = self.model.streams.remove(&stream_id).unwrap();
                    let message_count_before = stream_data.messages.len();
                    let parsers = get_message_parsers();
                    let parser_index = stream_data.parser_index;
                    let parser = parsers.get(parser_index).unwrap();
                    match parser.finish_stream(stream_data) {
                        Ok(sd) => {
                            self.refresh_grids_new_messages(
                                stream_id,
                                parser_index,
                                message_count_before,
                                sd,
                            );
                        }
                        Err(e) => {
                            self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                                "Error parsing file after collecting the final packets: {}",
                                e
                            ))));
                            return;
                        }
                    }
                }
                if self.model.streams.is_empty() {
                    self.model.relm.stream().emit(Msg::LoadedData(Err(
                        "Hotwire doesn't know how to read any useful data from this file"
                            .to_string(),
                    )));
                    return;
                }
                self.widgets.loading_spinner.stop();
                self.widgets.open_btn.set_sensitive(true);
                self.widgets.capture_btn.set_sensitive(true);
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
                if let Some((stream_client_server, msg_data)) = self
                    .model
                    .streams
                    .get(&stream_id)
                    .and_then(|s| s.messages.get(idx as usize).map(|f| (&s.client_server, f)))
                {
                    for adj in &self.model.details_adjustments {
                        adj.set_value(0.0);
                    }
                    for component_emitter in &self.model.details_component_emitters {
                        component_emitter(
                            self.model.bg_sender.clone(),
                            MessageInfo {
                                stream_id,
                                client_ip: stream_client_server.as_ref().unwrap().client_ip,
                                message_data: msg_data.clone(),
                            },
                        );
                    }
                } else {
                    println!(
                        "NO DATA for {}/{} -- stream length {:?}",
                        stream_id,
                        idx,
                        self.model.streams.get(&stream_id).unwrap().messages.len()
                    );
                }
            }
            Msg::Quit => {
                // needed for the pcap save temp files at least
                if let Err(e) =
                    config::remove_obsolete_tcpdump_files(config::RemoveMode::OldFilesAndMyFiles)
                {
                    eprintln!("Error removing the obsolete tcpdump files: {:?}", e);
                }
                gtk::main_quit()
            }
        }
    }

    fn display_error_block(msg: &str, secondary: Option<&str>) {
        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::all(),
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            msg,
        );
        dialog.set_property_secondary_text(secondary);
        let _r = dialog.run();
        dialog.close();
    }

    fn refresh_grids_new_messages(
        &mut self,
        stream_id: TcpStreamId,
        parser_index: usize,
        message_count_before: usize,
        stream_data: StreamData,
    ) {
        let parsers = get_message_parsers();
        let parser = parsers.get(parser_index).unwrap();
        let added_messages = stream_data.messages.len() - message_count_before;
        // self.refresh_comm_targets();

        // self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, &[], &[]);
        let selected_card = self.model.selected_card.clone();
        match (stream_data.client_server, selected_card) {
            (Some(client_server), Some(card)) => {
                if client_server.server_ip == card.ip
                    && client_server.server_port == card.port
                    && parser_index == card.protocol_index
                {
                    let ls = self
                        .model
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
                            self.model.cur_liststore = Some((key, ls.clone(), 0));
                            let (ref tv, ref _signals) = &self
                                .model
                                .comm_remote_servers_treeviews
                                .get(card.protocol_index)
                                .unwrap();
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
                    let mut store = self.model.cur_liststore.take().unwrap();
                    store.2 += added_messages as i32;
                    self.model.cur_liststore = Some(store);

                    if self.widgets.follow_packets_btn.is_visible()
                        && self.widgets.follow_packets_btn.get_active()
                    {
                        // we're capturing network traffic. scroll to
                        // reveal new packets
                        let scrolledwindow = self
                            .widgets
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
                        self.model
                            .comm_remote_servers_treeviews
                            .get(card.protocol_index)
                            .unwrap()
                            .0
                            .get_selection()
                            .select_path(&gtk::TreePath::new_first());

                        if self.model.current_file.is_none()
                            || self
                                .model
                                .current_file
                                .as_ref()
                                .filter(|f| f.1 == FileType::Fifo)
                                .is_some()
                        {
                            // we're capturing network traffic. start displaying data
                            // realtime as we're receiving packets.
                            self.widgets
                                .root_stack
                                .set_visible_child_name(NORMAL_STACK_NAME);
                        }
                    }
                }
            }
            _ => {}
        }
        self.model.streams.insert(stream_id, stream_data);
    }

    fn add_comm_target_data(
        &mut self,
        protocol_index: usize,
        parser: &Box<dyn MessageParser>,
        client_server_info: &ClientServerInfo,
        summary_details: Option<&str>,
    ) {
        let card_key = CommTargetCardKey {
            ip: client_server_info.server_ip,
            port: client_server_info.server_port,
            protocol_index,
        };
        if let Some(card_idx) = self.model.comm_target_cards.iter().position(|c| {
            c.protocol_index == protocol_index
                && c.port == client_server_info.server_port
                && c.ip == client_server_info.server_ip
        }) {
            let mut card = self.model.comm_target_cards.get_mut(card_idx).unwrap();
            // update existing card
            card.increase_incoming_session_count();
            card.remote_hosts
                .insert(client_server_info.client_ip.to_string());
            if card.summary_details.is_none() && summary_details.is_some() {
                card.summary_details = Some(SummaryDetails {
                    details: summary_details.unwrap().to_string(),
                });
            }
            dbg!(&card);
            dbg!(&self.model.comm_targets_components.len());
            self.model
                .comm_targets_components
                .get(&card_key)
                .unwrap()
                .emit(comm_target_card::Msg::Update(card.clone()));
        } else {
            // add new card
            let card = CommTargetCardData::new(
                client_server_info.server_ip,
                client_server_info.server_port,
                protocol_index,
                {
                    let mut bs = BTreeSet::new();
                    bs.insert(client_server_info.client_ip.to_string());
                    bs
                },
                parser.protocol_icon(),
                summary_details.map(|d| SummaryDetails {
                    details: d.to_string(),
                }),
                1,
            );
            self.model.comm_target_cards.push(card.clone());
            self.model.comm_targets_components.insert(
                card_key,
                self.widgets
                    .comm_target_list
                    .add_widget::<CommTargetCard>(card),
            );
            if self.model.comm_target_cards.len() == 1 {
                self.widgets
                    .comm_target_list
                    .select_row(self.widgets.comm_target_list.get_row_at_index(0).as_ref());
            }
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
                    allowed_stream_ids.push(TcpStreamId(stream_id.get().unwrap().unwrap()));
                }
                _ => panic!(path.get_depth()),
            }
        }
        if let Some(card) = self.model.selected_card.as_ref() {
            self.model
                .relm
                .stream()
                .emit(Msg::SelectCardFromRemoteIpsAndStreams(
                    card.clone(),
                    allowed_ips,
                    allowed_stream_ids,
                ));
        }
    }

    fn display_about(&mut self) {
        let tshark_version = Command::new("tshark")
            .args(&["--version"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "Failed running tshark".to_string());
        let pkexec_version = Command::new("pkexec")
            .args(&["--version"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "No pkexec installed".to_string());
        let tcpdump_version = Command::new("tcpdump")
            .args(&["--version"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "No tcpdump installed".to_string());
        let dlg = gtk::AboutDialogBuilder::new()
            .name("Hotwire")
            .version(env!("CARGO_PKG_VERSION"))
            .logo_icon_name(Icon::APP_ICON.name())
            .website("https://github.com/emmanueltouzery/hotwire/")
            .comments("Explore the contents of network capture files")
            .build();
        dlg.add_credit_section("tshark", &[&tshark_version]);
        dlg.add_credit_section("pkexec", &[&pkexec_version]);
        dlg.add_credit_section("tcpdump", &[&tcpdump_version]);
        dlg.set_license(Some(include_str!("../../LICENSE.md")));
        dlg.run();
        dlg.close();
    }

    fn handle_capture_toggled(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // TODO don't call from the GUI thread
        let is_active = self.widgets.capture_btn.get_active();
        self.widgets.capture_spinner.set_visible(is_active);
        self.widgets.follow_packets_btn.set_active(true);
        self.widgets.follow_packets_btn.set_visible(is_active);
        if is_active {
            self.widgets.capture_spinner.start();
            // i wanted to use the temp folder but I got permissions issues,
            // which I don't fully understand.
            let fifo_path = config::get_tcpdump_fifo_path();
            if !fifo_path.exists() {
                nix::unistd::mkfifo(
                    &fifo_path,
                    nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
                )?;
            }
            self.reset_open_file(None, FileType::Fifo);
            let mut tcpdump_child = Command::new("pkexec")
                .args(&[
                    "tcpdump",
                    "-ni",
                    "any",
                    "-s0",
                    "--immediate-mode",
                    "--packet-buffered",
                    "-w",
                    fifo_path.to_str().unwrap(),
                ])
                .spawn()
                .map_err(|e| format!("Error launching pkexec: {:?}", e))?;

            // yeah sleeping 50ms in the gui thread...
            // but it's the easiest. pkexec needs some tome to init, try to launch that
            // app and fail... on my computer 50ms is consistently enough.
            std::thread::sleep(Duration::from_millis(50));
            if let Ok(Some(status)) = tcpdump_child.try_wait() {
                return Err(
                    format!("Failed to execute tcpdump, pkexec exit code {}", status).into(),
                );
            }

            self.model.tcpdump_child = Some(tcpdump_child);
            let s = self.model.loaded_data_sender.clone();
            self.model
                .bg_sender
                .send(BgFunc::new(move || {
                    Self::load_file(TSharkInputType::Fifo, fifo_path.clone(), s.clone());
                }))
                .unwrap();
        } else {
            self.widgets.capture_spinner.stop();
            self.widgets.open_btn.set_sensitive(true);
            self.widgets.capture_btn.set_sensitive(true);
            self.widgets
                .root_stack
                .set_visible_child_name(if self.model.streams.is_empty() {
                    WELCOME_STACK_NAME
                } else {
                    NORMAL_STACK_NAME
                });
            self.cleanup_child_processes()?;
            let fifo_path = config::get_tcpdump_fifo_path();
            if fifo_path.exists() {
                std::fs::remove_file(fifo_path)?;
            }
            self.widgets.save_capture_btn.set_visible(true);
            self.widgets
                .capture_btn
                .set_visible(Self::is_display_capture_btn());
        }
        Ok(())
    }

    fn cleanup_child_processes(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(_tcpdump_child) = self.model.tcpdump_child.take() {
            let mut tcpdump_child = _tcpdump_child;
            // seems like we can't kill tcpdump, even though it's our child (owned by another user),
            // but it's not needed (presumably because we kill tshark, which reads from the fifo,
            // and the fifo itself)
            // if let Err(e) = tcpdump_pid.kill() {
            //     eprintln!("kill1 fails {:?}", e);
            // }

            // try_wait doesn't work, wait hangs, not doing anything leaves zombie processes
            // i found this way of regularly calling try_wait until it succeeds...
            glib::idle_add_local(move || {
                glib::Continue(
                    !matches!(tcpdump_child.try_wait(), Ok(Some(s)) if s.code().is_some() || s.signal().is_some()),
                )
            });
        }
        if let Some(_tshark_child) = self.model.tshark_child.take() {
            let mut tshark_child = _tshark_child;

            // soooooooo... if I use child.kill() then when I read from a local fifo file (mkfifo)
            // and I cancel the reading from the fifo, and nothing was written to the fifo at all,
            // we do kill the tshark process, but our read() on the pipe from tshark hangs.
            // I don't know why. However if I use nix to send a SIGINT, our read() is interrupted
            // and all is good...
            //
            // tshark_child.kill()?;
            nix::sys::signal::kill(
                Pid::from_raw(tshark_child.id() as libc::pid_t),
                Some(Signal::SIGINT),
            )?;

            // try_wait doesn't work, wait hangs, not doing anything leaves zombie processes
            // i found this way of regularly calling try_wait until it succeeds...
            glib::idle_add_local(move || {
                glib::Continue(
                    !matches!(tshark_child.try_wait(), Ok(Some(s)) if s.code().is_some() || s.signal().is_some()),
                )
            });
        }
        Ok(())
    }

    fn handle_save_capture(&mut self) {
        let dialog = gtk::FileChooserNativeBuilder::new()
            .action(gtk::FileChooserAction::Save)
            .title("Select file")
            .modal(true)
            .build();
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.pcap");
        filter.add_pattern("*.pcapng");
        dialog.set_filter(&filter);
        if dialog.run() == gtk::ResponseType::Accept {
            if let Some(fname) = dialog.get_filename() {
                if let Err(e) = std::fs::copy(config::get_tshark_pcap_output_path(), fname.clone())
                {
                    Self::display_error_block("Error saving capture file", Some(&e.to_string()));
                } else {
                    Self::add_to_recent_files(&fname);
                    self.refresh_recent_files();
                }
            }
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
        filter.add_pattern("*.pcapng");
        dialog.set_filter(&filter);
        if dialog.run() == gtk::ResponseType::Accept {
            if let Some(fname) = dialog.get_filename() {
                self.gui_load_file(fname);
            }
        }
    }

    fn reset_open_file(&mut self, fname: Option<PathBuf>, filetype: FileType) {
        self.widgets.open_btn.set_sensitive(false);
        if filetype != FileType::Fifo {
            // prevent capture when we're opening a file, but obviously
            // we want it when capturing (to stop the capture)
            self.widgets.capture_btn.set_sensitive(false);
        }
        let pcap_output_file = config::get_tshark_pcap_output_path();
        if pcap_output_file.exists() {
            if let Err(e) = std::fs::remove_file(pcap_output_file) {
                eprintln!("Error removing pcap capture file: {}", e);
            }
        }
        self.widgets.save_capture_btn.set_visible(false);
        self.init_remote_ips_streams_tree();
        self.connect_remote_ips_streams_tree();
        self.components
            .headerbar_search
            .emit(HeaderbarSearchMsg::SearchActiveChanged(false));
        self.model.window_subtitle = Some(
            fname
                .as_ref()
                .map(|p| p.to_string_lossy())
                .unwrap_or(std::borrow::Cow::Borrowed("Network Capture"))
                .to_string(),
        );
        self.model.current_file = fname.map(|p| (p, filetype));
        self.widgets.loading_spinner.start();
        self.widgets.loading_parsing_label.set_visible(false);
        self.widgets.loading_tshark_label.set_visible(true);
        self.widgets
            .root_stack
            .set_visible_child_name(LOADING_STACK_NAME);
        self.model.streams.clear();
        self.model.cur_liststore = None;
        self.model.selected_card = None;
        self.model.comm_target_cards.clear();
        for child in self.widgets.comm_target_list.get_children() {
            self.widgets.comm_target_list.remove(&child);
        }
        self.model.comm_targets_components.clear();
        self.model.remote_ips_streams_iptopath.clear();
    }

    fn add_to_recent_files(fname: &Path) {
        if let Some(rm) = gtk::RecentManager::get_default() {
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
        }
    }

    fn gui_load_file(&mut self, fname: PathBuf) {
        self.widgets.open_btn.set_active(false);
        Self::add_to_recent_files(&fname);
        let is_fifo = std::fs::metadata(&fname)
            .ok()
            .filter(|m| m.file_type().is_fifo())
            .is_some();

        self.reset_open_file(
            Some(fname.clone()),
            if is_fifo {
                FileType::Fifo
            } else {
                FileType::File
            },
        );

        if is_fifo {
            self.widgets
                .capture_btn
                .block_signal(&self.model.capture_toggle_signal.as_ref().unwrap());
            // the capture button is invisible by default except on linux
            // in case of file opening through fifo, make it visible always
            // (so the user can stop the import)
            self.widgets.capture_btn.set_visible(true);
            self.widgets.capture_btn.set_active(true);
            self.widgets.capture_spinner.set_visible(true);
            self.widgets.capture_spinner.start();
            self.widgets
                .capture_btn
                .unblock_signal(&self.model.capture_toggle_signal.as_ref().unwrap());
        }

        let s = self.model.loaded_data_sender.clone();
        // self.init_remote_ips_streams_tree();
        self.model
            .bg_sender
            .send(BgFunc::new(move || {
                Self::load_file(
                    if is_fifo {
                        TSharkInputType::Fifo
                    } else {
                        TSharkInputType::File
                    },
                    fname.clone(),
                    s.clone(),
                );
            }))
            .unwrap();
        self.refresh_recent_files();
    }

    fn load_file(file_type: TSharkInputType, fname: PathBuf, sender: relm::Sender<ParseInputStep>) {
        let filter = get_message_parsers()
            .into_iter()
            .map(|p| p.tshark_filter_string())
            .join(" || ");
        invoke_tshark(file_type, &fname, &filter, sender);
    }

    fn init_remote_ips_streams_tree(&mut self) {
        self.model.remote_ips_streams_iptopath.clear();
        self.model.remote_ips_streams_treestore = gtk::TreeStore::new(&[
            // TODO duplicated in model init
            String::static_type(),
            pango::Weight::static_type(),
            u32::static_type(),
        ]);
        self.model.remote_ips_streams_treestore.insert_with_values(
            None,
            None,
            &[0, 1],
            &[&"All".to_value(), &pango::Weight::Bold.to_glib().to_value()],
        );
    }

    fn connect_remote_ips_streams_tree(&mut self) {
        let model_sort = gtk::TreeModelSort::new(&self.model.remote_ips_streams_treestore);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(2), gtk::SortType::Ascending);
        self.widgets
            .remote_ips_streams_treeview
            .set_model(Some(&model_sort));
        self.widgets.remote_ips_streams_treeview.expand_all();
    }

    fn refresh_remote_ips_streams_tree(
        &mut self,
        card: &CommTargetCardData,
        remote_ips: &HashSet<IpAddr>,
    ) {
        // self.widgets.remote_ips_streams_treeview.set_cursor(
        //     &gtk::TreePath::new_first(),
        //     None::<&gtk::TreeViewColumn>,
        //     false,
        // );
        let target_ip = card.ip;
        let target_port = card.port;

        for remote_ip in remote_ips {
            let remote_ip_iter = self.model.remote_ips_streams_treestore.insert_with_values(
                None,
                None,
                &[0, 1],
                &[
                    &remote_ip.to_string().to_value(),
                    &pango::Weight::Normal.to_glib().to_value(),
                ],
            );
            self.model.remote_ips_streams_iptopath.insert(
                *remote_ip,
                self.model
                    .remote_ips_streams_treestore
                    .get_path(&remote_ip_iter)
                    .unwrap(),
            );
            for (stream_id, messages) in &self.model.streams {
                if messages.client_server.as_ref().map(|cs| cs.server_ip) != Some(target_ip)
                    || messages.client_server.as_ref().map(|cs| cs.server_port) != Some(target_port)
                    || messages.client_server.as_ref().map(|cs| cs.client_ip) != Some(*remote_ip)
                {
                    continue;
                }
                self.model.remote_ips_streams_treestore.insert_with_values(
                    Some(&remote_ip_iter),
                    None,
                    &[0, 1, 2],
                    &[
                        &format!(
                            r#"<span foreground="{}" size="smaller">⬤</span> <span rise="-1700">Stream {}</span>"#,
                            colors::STREAM_COLORS
                                [stream_id.as_u32() as usize % colors::STREAM_COLORS.len()],
                            stream_id.as_u32()
                        )
                        .to_value(),
                        &pango::Weight::Normal.to_glib().to_value(),
                        &stream_id.as_u32().to_value(),
                    ],
                );
            }
        }

        self.connect_remote_ips_streams_tree();
    }

    fn refresh_remote_servers(
        &mut self,
        refresh_remote_ips_and_streams: RefreshRemoteIpsAndStreams,
        constrain_remote_ips: &[IpAddr],
        constrain_stream_ids: &[TcpStreamId],
    ) {
        self.init_remote_ips_streams_tree();
        self.setup_selection_signals(RefreshOngoing::Yes);
        if let Some(card) = self.model.selected_card.as_ref().cloned() {
            let target_ip = card.ip;
            let target_port = card.port;
            let mut by_remote_ip = HashMap::new();
            let parsers = get_message_parsers();
            for (stream_id, messages) in &self.model.streams {
                if messages.client_server.as_ref().map(|cs| cs.server_ip) != Some(target_ip)
                    || messages.client_server.as_ref().map(|cs| cs.server_port) != Some(target_port)
                {
                    continue;
                }
                let allowed_all =
                    constrain_remote_ips.is_empty() && constrain_stream_ids.is_empty();

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
                    #[name="capture_btn"]
                    gtk::ToggleButton {
                        gtk::Box {
                            #[name="capture_spinner"]
                            gtk::Spinner {
                                visible: false,
                            },
                            gtk::Label {
                                text: "Capture"
                            }
                        }
                    },
                    #[name="follow_packets_btn"]
                    gtk::ToggleButton {
                        image: Some(&gtk::Image::from_icon_name(Some("angle-double-down"), gtk::IconSize::Menu)),
                        always_show_image: true,
                        label: "Scroll to follow packets",
                        visible: false,
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
                    #[name="save_capture_btn"]
                    gtk::Button {
                        child: {
                            pack_type: gtk::PackType::End
                        },
                        visible: false,
                        label: "Save capture...",
                        clicked => Msg::SaveCapture,
                    },
                }
            },
            #[name="root_stack"]
            gtk::Stack {
                #[name="welcome_label"]
                gtk::Label {
                    child: {
                        name: Some(WELCOME_STACK_NAME)
                    },
                    label: "Welcome to Hotwire!",
                    drag_data_received(_widget, ctx, _x, _y, sel_data, _info, _time) => Msg::DragDataReceived(ctx.clone(), sel_data.clone()),
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
