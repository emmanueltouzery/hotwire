use super::comm_target_card;
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::headerbar_search::HeaderbarSearch;
use super::headerbar_search::Msg as HeaderbarSearchMsg;
use super::headerbar_search::Msg::SearchActiveChanged as HbsMsgSearchActiveChanged;
use super::headerbar_search::Msg::SearchExprChanged as HbsMsgSearchExprChanged;
use super::ips_and_streams_treeview;
use super::messages_treeview;
use super::preferences::Preferences;
use super::recent_file_item::RecentFileItem;
use crate::config;
use crate::config::Config;
use crate::http::http_message_parser::{Http, HttpMessageData, HttpStreamGlobals};
use crate::http2::http2_message_parser::{Http2, Http2StreamGlobals};
use crate::icons::Icon;
use crate::message_parser::{AnyMessagesData, MessageParser};
use crate::message_parser::{AnyStreamGlobals, ClientServerInfo};
use crate::message_parser::{MessageInfo, StreamData};
use crate::packets_read;
use crate::packets_read::{InputStep, ParseInputStep, TSharkInputType};
use crate::pgsql::postgres_message_parser::{Postgres, PostgresMessageData, PostgresStreamGlobals};
use crate::search_expr;
use crate::tshark_communication;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpStreamId};
use crate::widgets::comm_target_card::CommTargetCardKey;
use crate::widgets::comm_target_card::SummaryDetails;
use crate::BgFunc;
use gdk::prelude::*;
use gtk::prelude::*;
use gtk::traits::SettingsExt;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::IpAddr;
#[cfg(target_family = "unix")]
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::mpsc;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");
const SHORTCUTS_UI: &str = include_str!("shortcuts.ui");

const WELCOME_STACK_NAME: &str = "welcome";
const LOADING_STACK_NAME: &str = "loading";
const NORMAL_STACK_NAME: &str = "normal";

const PCAP_MIME_TYPE: &str = "application/vnd.tcpdump.pcap";

// gonna put the concrete type here later
pub const MESSAGE_PARSERS: MessageParserList = (Http, Postgres, Http2);

trait MessageParserList {
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
        win_msg_sender: relm::StreamHandle<Msg>,
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

pub trait Streams {
    fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String>;
    fn handle_got_packet(
        &mut self,
        p: TSharkPacket,
    ) -> Result<
        (
            usize,
            SessionChangeType,
            Option<ClientServerInfo>,
            usize,
            Option<&str>,
        ),
        (TcpStreamId, String),
    >;
    fn messages_len(&self, stream_id: TcpStreamId) -> usize;
    fn client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo>;
    fn protocol_index(&self, stream_id: TcpStreamId) -> Option<usize>;

    fn by_remote_ip(
        &self,
        card_key: CommTargetCardKey,
        constrain_remote_ips: &[IpAddr],
        constrain_stream_ids: &[TcpStreamId],
    ) -> HashMap<IpAddr, Vec<TcpStreamId>>;
    // let mut by_remote_ip = HashMap::new();
    // let parsers = win::get_message_parsers();
    // for (stream_id, messages) in streams {
    //     if !matches!(messages.client_server, Some(cs) if card.to_key().matches_server(cs)) {
    //         continue;
    //     }
    //     let allowed_all = constrain_remote_ips.is_empty() && constrain_stream_ids.is_empty();

    //     let allowed_ip = messages
    //         .client_server
    //         .as_ref()
    //         .filter(|cs| constrain_remote_ips.contains(&cs.client_ip))
    //         .is_some();
    //     let allowed_stream = constrain_stream_ids.contains(stream_id);
    //     let allowed = allowed_all || allowed_ip || allowed_stream;

    //     if !allowed {
    //         continue;
    //     }
    //     let remote_server_streams = by_remote_ip
    //         .entry(
    //             messages
    //                 .client_server
    //                 .as_ref()
    //                 .map(|cs| cs.client_ip)
    //                 .unwrap_or_else(|| IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))),
    //         )
    //         .or_insert_with(Vec::new);
    //     remote_server_streams.push((stream_id, messages));
    // }
}

pub fn is_flatpak() -> bool {
    // The Flatpak environment can be detected at runtime by looking for a file named /.flatpak-info. https://github.com/flathub/flathub/wiki/App-Maintenance
    Path::new("/.flatpak-info").exists()
}

#[derive(Debug, PartialEq, Eq)]
pub enum InfobarOptions {
    Default,
    ShowCloseButton,
    ShowSpinner,
    TimeLimitedWithCloseButton,
}

#[derive(Msg, Debug)]
pub enum Msg {
    SearchClicked,
    OpenFile,
    OpenRecentFile(usize),
    DisplayPreferences,
    DisplayAbout,
    DisplayShortcuts,
    CaptureToggled,
    SaveCapture,
    ChildProcessDied,

    DragDataReceived(gdk::DragContext, gtk::SelectionData),

    KeyPress(gdk::EventKey),
    SearchActiveChanged(bool),
    SearchExprChanged(Option<Result<(String, search_expr::SearchExpr), String>>),

    LoadedData(ParseInputStep),
    OpenFileFirstPacketDisplayed,

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

pub struct Model {
    relm: relm::Relm<Win>,
    bg_sender: mpsc::Sender<BgFunc>,

    recent_searches: Vec<String>,

    search_toggle_signal: Option<glib::SignalHandlerId>,
    window_subtitle: Option<String>,
    current_file: Option<(PathBuf, TSharkInputType)>,
    recent_files: Vec<PathBuf>,

    set_sidebar_height: bool,

    capture_toggle_signal: Option<glib::SignalHandlerId>,

    infobar_spinner: gtk::Spinner,
    infobar_label: gtk::Label,

    sidebar_selection_change_signal_id: Option<glib::SignalHandlerId>,

    // streams: HashMap<TcpStreamId, StreamData<AnyStreamGlobals, AnyMessagesData>>, // hashmap<tcpstreamid ,anystreamdata>
    streams: Streams, // gonna put the concrete type here later
    //     (
    //     HashMap<TcpStreamId, StreamData<HttpStreamGlobals, Vec<HttpMessageData>>>,
    //     HashMap<TcpStreamId, StreamData<PostgresStreamGlobals, Vec<PostgresMessageData>>>,
    //     HashMap<TcpStreamId, StreamData<Http2StreamGlobals, Vec<HttpMessageData>>>,
    // ),
    // // hashmap<tcpstreamid ,anystreamdata>
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    search_expr: Option<Result<(String, search_expr::SearchExpr), String>>,

    messages_treeview_state: Option<messages_treeview::MessagesTreeviewState>,
    ips_and_streams_treeview_state: Option<ips_and_streams_treeview::IpsAndStreamsTreeviewState>,

    _loaded_data_channel: relm::Channel<ParseInputStep>,
    loaded_data_sender: relm::Sender<ParseInputStep>,

    comm_targets_components: HashMap<CommTargetCardKey, Component<CommTargetCard>>,
    _recent_file_item_components: Vec<Component<RecentFileItem>>,

    prefs_win: Option<Component<Preferences>>,

    capture_malformed_packets: usize,
    tcpdump_child: Option<Child>,
    tshark_child: Option<Child>,
}

pub enum RefreshRemoteIpsAndStreams {
    Yes(CommTargetCardData, HashSet<IpAddr>),
    No,
}

#[derive(Debug)]
pub enum RefreshOngoing {
    Yes,
    No,
}

#[derive(PartialEq, Eq, Copy, Clone)]
enum SessionChangeType {
    NewSession,
    NewDataInSession,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        if let Err(err) = self.load_style() {
            eprintln!("Error loading the CSS: {}", err);
        }

        self.left_align_menu_entries();

        self.widgets.welcome_label.drag_dest_set(
            gtk::DestDefaults::ALL,
            &[],
            gdk::DragAction::COPY,
        );
        self.widgets.welcome_label.drag_dest_add_uri_targets();

        self.model.search_toggle_signal = {
            let r = self.model.relm.clone();
            Some(self.widgets.search_toggle.connect_toggled(move |_| {
                r.stream().emit(Msg::SearchClicked);
            }))
        };

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
                    .selection()
                    .connect_changed(move |selection| {
                        stream.emit(Msg::SelectRemoteIpStream(selection.clone()));
                    }),
            )
        };

        self.widgets
            .remote_ips_streams_treeview
            .selection()
            .set_mode(gtk::SelectionMode::Multiple);

        let infobar_box = gtk::BoxBuilder::new().spacing(15).build();
        infobar_box.add(&self.model.infobar_spinner);
        infobar_box.add(&self.model.infobar_label);
        infobar_box.show_all();
        self.widgets.infobar.content_area().add(&infobar_box);

        self.model.infobar_spinner.set_visible(false);

        // https://bugzilla.gnome.org/show_bug.cgi?id=305277
        gtk::Settings::default()
            .unwrap()
            .set_gtk_alternative_sort_arrows(true);

        self.model.messages_treeview_state = Some(messages_treeview::init_grids_and_panes(
            &self.model.relm,
            &self.model.bg_sender,
            self.widgets.comm_remote_servers_stack.clone(),
        ));

        self.model.ips_and_streams_treeview_state =
            Some(ips_and_streams_treeview::init_remote_ip_streams_tv(
                &self.widgets.remote_ips_streams_treeview,
            ));

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

    fn left_align_menu_entries(&self) {
        for menu_item in self.widgets.menu_box.children() {
            if let Some(label) = menu_item
                .dynamic_cast::<gtk::ModelButton>()
                .unwrap()
                .child()
                .and_then(|c| c.dynamic_cast::<gtk::Label>().ok())
            {
                label.set_xalign(0.0);
                label.set_hexpand(true);
            }
        }
    }

    fn is_display_capture_btn() -> bool {
        !cfg!(target_os = "windows")
    }

    fn refresh_recent_files(&mut self) {
        for child in self.widgets.recent_files_list.children() {
            self.widgets.recent_files_list.remove(&child);
        }
        self.model.recent_files.clear();
        self.model._recent_file_item_components.clear();
        let rm = gtk::RecentManager::default().unwrap();
        let mut items = rm.items();
        let normalize_uri = |uri: Option<glib::GString>| {
            uri.map(|u| u.to_string()).map(|u| {
                if u.starts_with('/') {
                    format!("file://{}", u)
                } else {
                    u
                }
            })
        };
        items.sort_by_key(|i| normalize_uri(i.uri()));
        items.dedup_by_key(|i| normalize_uri(i.uri()));
        items.sort_by_key(|i| Reverse(i.modified()));
        items
            .into_iter()
            .filter(|i| {
                i.last_application().map(|a| a.to_string()) == Some("hotwire".to_string())
                    // if we don't also filter by mimetype, we get also the files we saved (for instance
                    // when saving http bodies to files on disk)
                    && i.mime_type() == Some(PCAP_MIME_TYPE.into())
            })
            .take(5)
            .flat_map(|fi| fi.uri())
            .map(|gs| tshark_communication::string_to_path(gs.as_str()))
            .for_each(|pb| {
                self.model.recent_files.push(pb.clone());
                self.model._recent_file_item_components.push(
                    self.widgets
                        .recent_files_list
                        .add_widget::<RecentFileItem>(pb),
                );
            });
    }

    fn load_style(&self) -> Result<(), Box<dyn std::error::Error>> {
        let screen = self.widgets.window.screen().unwrap();
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
        params: (
            mpsc::Sender<BgFunc>,
            Option<(PathBuf, TSharkInputType)>,
            Vec<String>,
        ),
    ) -> Model {
        let (bg_sender, current_file, recent_searches) = params;
        gtk::IconTheme::default()
            .unwrap()
            .add_resource_path("/icons");

        let config = Config::read_config();
        gtk::Settings::default()
            .unwrap()
            .set_gtk_application_prefer_dark_theme(config.prefer_dark_theme);

        let (_loaded_data_channel, loaded_data_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |ch_data: ParseInputStep| {
                stream.emit(Msg::LoadedData(ch_data));
            })
        };

        // the problem i'm trying to fix is the user triggering
        // a capture... so we call pkexec to launch tcpdump.. but the user closes pkexec and
        // so tcpdump will never be launched.
        // then we have tshark blocking on the fifo, forever.
        // i catch the SIGCHLD signal, which tells me that one of my child processes died
        // (potentially pkexec). At that point if the capture is running, I stop it and clean up.
        let stream = relm.stream().clone();
        packets_read::register_child_process_death(
            relm::Channel::new(move |()| {
                // This closure is executed whenever a message is received from the sender.
                // We send a message to the current widget.
                stream.emit(Msg::ChildProcessDied);
            })
            .1,
        );

        Model {
            relm: relm.clone(),
            bg_sender,
            recent_searches,
            infobar_spinner: gtk::SpinnerBuilder::new()
                .width_request(24)
                .height_request(24)
                .build(),
            prefs_win: None,
            search_toggle_signal: None,
            infobar_label: gtk::LabelBuilder::new().build(),
            comm_targets_components: HashMap::new(),
            set_sidebar_height: false,
            _recent_file_item_components: vec![],
            recent_files: vec![],
            selected_card: None,
            loaded_data_sender,
            _loaded_data_channel,
            messages_treeview_state: None,
            ips_and_streams_treeview_state: None,
            sidebar_selection_change_signal_id: None,
            comm_target_cards: vec![],
            streams: HashMap::new(),
            current_file,
            capture_toggle_signal: None,
            window_subtitle: None,
            search_expr: None,
            capture_malformed_packets: 0,
            tcpdump_child: None,
            tshark_child: None,
        }
    }

    fn update(&mut self, event: Msg) {
        // match &event {
        //     Msg::LoadedData(_) => println!("event: loadeddata"),
        //     _ => {
        //         dbg!(&event);
        //     }
        // }
        match event {
            Msg::DisplayPreferences => {
                self.display_preferences();
            }
            Msg::DisplayAbout => {
                self.display_about();
            }
            Msg::DisplayShortcuts => self.display_shortcuts(),
            Msg::DragDataReceived(_context, sel_data) => {
                if let Some(uri) = sel_data
                    .uris()
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
            Msg::SearchClicked => {
                let is_active = self.widgets.search_toggle.is_active();
                self.widgets
                    .headerbar_search_revealer
                    .set_reveal_child(is_active);
                self.components
                    .headerbar_search
                    .emit(HeaderbarSearchMsg::SearchActiveChanged(is_active));
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

                if let Some(tshark_child) = self.model.tshark_child.as_mut() {
                    // normally when we stop tshark ourselves, we first remove the pid
                    // from self.model so if we find it there and tshark is dead, it
                    // died on us
                    if packets_read::try_wait_has_exited(tshark_child) {
                        self.handle_got_loading_error("tshark exited");
                    }
                }
            }
            Msg::KeyPress(e) => {
                self.handle_keypress(e);
            }
            Msg::SearchActiveChanged(is_active) => {
                self.widgets.search_toggle.set_active(is_active);
                if let Some(card) = self.model.selected_card.as_ref() {
                    messages_treeview::search_text_changed(
                        self.model.messages_treeview_state.as_ref().unwrap(),
                        &self.model.streams,
                        card.protocol_index,
                        if is_active {
                            self.model
                                .search_expr
                                .as_ref()
                                .and_then(|r| r.as_ref().ok())
                                .map(|(_rest, parsed)| parsed)
                        } else {
                            None
                        },
                    );
                }
            }
            Msg::SearchExprChanged(expr) => {
                self.model.search_expr = expr.clone();
                if let Some(card) = self.model.selected_card.as_ref() {
                    messages_treeview::search_text_changed(
                        self.model.messages_treeview_state.as_ref().unwrap(),
                        &self.model.streams,
                        card.protocol_index,
                        expr.and_then(|r| r.ok())
                            .map(|(_rest, parsed)| parsed)
                            .as_ref(),
                    );
                }
            }
            Msg::OpenRecentFile(idx) => {
                let path = self.model.recent_files[idx].clone();
                self.gui_load_file(path);
            }
            Msg::OpenFileFirstPacketDisplayed => {
                if self.model.current_file.is_none()
                    || self
                        .model
                        .current_file
                        .as_ref()
                        .filter(|f| f.1 == TSharkInputType::Fifo)
                        .is_some()
                {
                    // we're capturing network traffic. start displaying data
                    // realtime as we're receiving packets.
                    self.widgets
                        .root_stack
                        .set_visible_child_name(NORMAL_STACK_NAME);
                }
            }
            Msg::InfoBarShow(Some(msg), options) => {
                self.handle_infobar_show(&msg, options);
            }
            Msg::InfoBarShow(None, _) | Msg::InfoBarEvent(gtk::ResponseType::Close) => {
                self.widgets.infobar.set_revealed(false);
            }
            Msg::InfoBarEvent(_) => {}
            Msg::LoadedData(Err(msg)) => {
                self.handle_got_loading_error(&msg);
            }
            Msg::LoadedData(Ok(InputStep::StartedTShark(pid))) => {
                self.model.tshark_child = Some(pid);
            }
            Msg::LoadedData(Ok(InputStep::Packet(p))) => {
                self.handle_got_packet(*p);
            }
            Msg::LoadedData(Ok(InputStep::Eof)) => {
                self.handle_got_input_eof();
            }
            Msg::SelectCard(maybe_idx) => {
                self.handle_select_card(maybe_idx);
                if let Some(idx) = maybe_idx {
                    self.components
                        .headerbar_search
                        .emit(HeaderbarSearchMsg::MainWinSelectCard(idx));
                }
            }
            Msg::SelectRemoteIpStream(selection) => {
                let (mut paths, _model) = selection.selected_rows();
                ips_and_streams_treeview::refresh_remote_ip_stream(
                    self.model.relm.stream(),
                    self.model.selected_card.as_ref(),
                    &self.widgets.remote_ips_streams_treeview,
                    &mut paths,
                );
            }
            Msg::SelectCardFromRemoteIpsAndStreams(_, remote_ips, stream_ids) => {
                let mut ips_treeview_state =
                    self.model.ips_and_streams_treeview_state.as_mut().unwrap();
                ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
                messages_treeview::refresh_remote_servers(
                    self.model.messages_treeview_state.as_mut().unwrap(),
                    self.model.selected_card.as_ref(),
                    &self.model.streams,
                    &self.widgets.remote_ips_streams_treeview,
                    self.model.sidebar_selection_change_signal_id.as_ref(),
                    &remote_ips,
                    &stream_ids,
                );
                messages_treeview::refresh_remote_servers_handle_selection(
                    self.model.messages_treeview_state.as_ref().unwrap(),
                    self.model.selected_card.as_ref(),
                    &self.widgets.remote_ips_streams_treeview,
                    self.model.sidebar_selection_change_signal_id.as_ref(),
                );
            }
            Msg::DisplayDetails(stream_id, idx) => {
                //
                if let Some((stream_client_server, msg_data)) = self
                    .model
                    .streams
                    .get(&stream_id)
                    .and_then(|s| s.messages.get(idx as usize).map(|f| (&s.client_server, f)))
                {
                    messages_treeview::handle_display_details(
                        self.model.messages_treeview_state.as_ref().unwrap(),
                        &self.model.bg_sender,
                        stream_id,
                        stream_client_server,
                        &msg_data,
                    );
                } else {
                    println!(
                        "NO DATA for {}/{} -- stream length {:?}",
                        stream_id,
                        idx,
                        self.model.streams.get(&stream_id).map(|s| s.messages.len())
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

    fn handle_infobar_show(&mut self, msg: &str, options: InfobarOptions) {
        self.widgets.infobar.set_show_close_button(matches!(
            options,
            InfobarOptions::ShowCloseButton | InfobarOptions::TimeLimitedWithCloseButton
        ));
        let has_spinner = options == InfobarOptions::ShowSpinner;
        if self.model.infobar_spinner.get_visible() != has_spinner {
            if has_spinner {
                self.model.infobar_spinner.start();
            } else {
                self.model.infobar_spinner.stop();
            }
            self.model.infobar_spinner.set_visible(has_spinner);
        }
        if options == InfobarOptions::TimeLimitedWithCloseButton {
            relm::timeout(self.model.relm.stream(), 1500, || {
                Msg::InfoBarShow(None, InfobarOptions::Default)
            });
        }
        self.model.infobar_label.set_text(msg);
        self.widgets.infobar.set_revealed(true);
    }

    fn handle_select_card(&mut self, maybe_idx: Option<usize>) {
        let wait_cursor =
            gdk::Cursor::for_display(&self.widgets.window.display(), gdk::CursorType::Watch);
        if let Some(p) = self.widgets.root_stack.parent_window() {
            p.set_cursor(Some(&wait_cursor));
        }
        self.widgets.comm_target_list.set_sensitive(false);
        self.widgets
            .remote_ips_streams_treeview
            .set_sensitive(false);
        self.model.selected_card = maybe_idx
            .and_then(|idx| self.model.comm_target_cards.get(idx as usize))
            .cloned();
        if let Some(card) = self.model.selected_card.as_ref() {
            // let parsers = get_message_parsers();
            // let mp = parsers.get(card.protocol_index).unwrap();

            self.streams
                .headerbar_search
                .emit(HeaderbarSearchMsg::SearchFilterKeysChanged(
                    // mp.supported_filter_keys().iter().cloned().collect(),
                    MESSAGE_PARSERS.supported_filter_keys(card.protocol_index),
                ));
        }
        let mut ips_treeview_state = self.model.ips_and_streams_treeview_state.as_mut().unwrap();
        ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
        let refresh_streams_tree = messages_treeview::refresh_remote_servers(
            self.model.messages_treeview_state.as_mut().unwrap(),
            self.model.selected_card.as_ref(),
            &self.model.streams,
            &self.widgets.remote_ips_streams_treeview,
            self.model.sidebar_selection_change_signal_id.as_ref(),
            &[],
            &[],
        );
        if let RefreshRemoteIpsAndStreams::Yes(card, ips) = refresh_streams_tree {
            let mut treeview_state = self.model.ips_and_streams_treeview_state.as_mut().unwrap();
            ips_and_streams_treeview::refresh_remote_ips_streams_tree(
                &mut treeview_state,
                &self.widgets.remote_ips_streams_treeview,
                &self.model.streams,
                &card,
                &ips,
                // it is a hackish way to find out...
                if self.model.tshark_child.is_some() {
                    ips_and_streams_treeview::IsNewDataStillIncoming::Yes
                } else {
                    ips_and_streams_treeview::IsNewDataStillIncoming::No
                },
            );
        }
        messages_treeview::refresh_remote_servers_handle_selection(
            self.model.messages_treeview_state.as_ref().unwrap(),
            self.model.selected_card.as_ref(),
            &self.widgets.remote_ips_streams_treeview,
            self.model.sidebar_selection_change_signal_id.as_ref(),
        );
        if let Some(p) = self.widgets.root_stack.parent_window() {
            p.set_cursor(None);
        }
        self.widgets.comm_target_list.set_sensitive(true);
        self.widgets.remote_ips_streams_treeview.set_sensitive(true);
        // if let Some(vadj) = self.widgets.remote_servers_scroll.get_vadjustment() {
        //     vadj.set_value(0.0);
        // }
    }

    fn handle_got_loading_error(&mut self, msg: &str) {
        // TODO clear the streams like we do when opening a new file?
        if let Err(e) = packets_read::cleanup_child_processes(
            self.model.tcpdump_child.take(),
            self.model.tshark_child.take(),
        ) {
            // not sure why loading failed.. maybe don't get too hung up
            // about error handling in this case?
            eprintln!("Error cleaning up child processes: {:?}", e);
        }
        self.widgets
            .capture_btn
            .block_signal(self.model.capture_toggle_signal.as_ref().unwrap());
        self.widgets.capture_btn.set_active(false);
        self.widgets.capture_spinner.set_visible(false);
        self.widgets.capture_spinner.stop();
        self.widgets
            .capture_btn
            .unblock_signal(self.model.capture_toggle_signal.as_ref().unwrap());
        self.widgets.loading_spinner.stop();

        self.capture_finished();

        if self.model.streams.is_empty() {
            // didn't load any data from the file at the time of the error,
            // abort loading
            self.model.window_subtitle = None;
            self.model.current_file = None;
            self.model.streams = HashMap::new();
            // self.refresh_comm_targets();
            let mut ips_treeview_state =
                self.model.ips_and_streams_treeview_state.as_mut().unwrap();
            ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
            let refresh_streams_tree = messages_treeview::refresh_remote_servers(
                self.model.messages_treeview_state.as_mut().unwrap(),
                self.model.selected_card.as_ref(),
                &self.model.streams,
                &self.widgets.remote_ips_streams_treeview,
                self.model.sidebar_selection_change_signal_id.as_ref(),
                &[],
                &[],
            );
            if let RefreshRemoteIpsAndStreams::Yes(card, ips) = refresh_streams_tree {
                let mut treeview_state =
                    self.model.ips_and_streams_treeview_state.as_mut().unwrap();
                ips_and_streams_treeview::refresh_remote_ips_streams_tree(
                    &mut treeview_state,
                    &self.widgets.remote_ips_streams_treeview,
                    &self.model.streams,
                    &card,
                    &ips,
                    ips_and_streams_treeview::IsNewDataStillIncoming::No,
                );
            }
            messages_treeview::refresh_remote_servers_handle_selection(
                self.model.messages_treeview_state.as_ref().unwrap(),
                self.model.selected_card.as_ref(),
                &self.widgets.remote_ips_streams_treeview,
                self.model.sidebar_selection_change_signal_id.as_ref(),
            );
            Self::display_error_block("Cannot load file", Some(msg));
        } else {
            // had already loaded some data, display what we have
            self.handle_got_input_eof();
        }
    }

    fn handle_got_packet(&mut self, p: TSharkPacket) {
        if p.is_malformed {
            self.model.capture_malformed_packets += 1;
        }
        match self.model.streams.handle_got_packet(p) {
            Err((packet_stream_id, msg)) => {
                self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                    "Error parsing file, in stream {}: {}",
                    packet_stream_id, msg
                ))));
            }
            Ok((
                parser_index,
                session_change_type,
                client_server,
                message_count_before,
                summary_details,
            )) => {
                let follow_packets = self.get_follow_packets();
                let mut tv_state = self.model.messages_treeview_state.as_mut().unwrap();
                let packet_stream_id = p.basic_info.tcp_stream_id;
                messages_treeview::refresh_grids_new_messages(
                    &mut tv_state,
                    self.model.relm.stream(),
                    self.model.selected_card.clone(),
                    packet_stream_id,
                    message_count_before,
                    &self.model.streams,
                    follow_packets,
                );

                if let Some(cs) = client_server {
                    let icon = MESSAGE_PARSERS.protocol_icon(parser_index);
                    self.add_update_comm_target_data(
                        parser_index,
                        icon,
                        cs,
                        summary_details.as_deref(),
                        session_change_type,
                    );
                }
            }
        }
        // if let Some((parser_index, parser)) = get_message_parsers()
        //     .iter()
        //     .enumerate()
        //     .find(|(_idx, ps)| ps.is_my_message(&p))
        // {
        //     let packet_stream_id = p.basic_info.tcp_stream_id;
        //     let existing_stream = self.model.streams.remove(&packet_stream_id);
        //     let message_count_before;
        //     let session_change_type;
        //     let stream_data = if let Some(stream_data) = existing_stream {
        //         // existing stream
        //         session_change_type = SessionChangeType::NewDataInSession;
        //         message_count_before = stream_data.messages.len();
        //         let stream_data = match parser.add_to_stream(stream_data, p) {
        //             Ok(sd) => sd,
        //             Err(msg) => {
        //                 self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
        //                     "Error parsing file, in stream {}: {}",
        //                     packet_stream_id, msg
        //                 ))));
        //                 return;
        //             }
        //         };
        //         stream_data
        //     } else {
        //         // new stream
        //         session_change_type = SessionChangeType::NewSession;
        //         message_count_before = 0;
        //         let mut stream_data = StreamData {
        //             parser_index,
        //             stream_globals: parser.initial_globals(),
        //             client_server: None,
        //             messages: parser.empty_messages_data(),
        //             summary_details: None,
        //         };
        //         match parser.add_to_stream(stream_data, p) {
        //             Ok(sd) => {
        //                 stream_data = sd;
        //             }
        //             Err(msg) => {
        //                 self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
        //                     "Error parsing file, in stream {}: {}",
        //                     packet_stream_id, msg
        //                 ))));
        //                 return;
        //             }
        //         }
        //         if stream_data.client_server.is_some() {
        //             let is_for_current_card = matches!(
        //                     (stream_data.client_server, self.model.selected_card.as_ref()),
        //                     (Some(clientserver), Some(card)) if clientserver.server_ip == card.ip
        //                         && clientserver.server_port == card.port
        //                         && parser_index == card.protocol_index);

        //             if is_for_current_card {
        //                 let mut treeview_state =
        //                     self.model.ips_and_streams_treeview_state.as_mut().unwrap();
        //                 ips_and_streams_treeview::got_packet_refresh_remote_ips_treeview(
        //                     &mut treeview_state,
        //                     &stream_data,
        //                     packet_stream_id,
        //                 );
        //             }
        //         }
        //         stream_data
        //     };

        //     self.model.streams.insert(packet_stream_id, stream_data);
        // }
    }

    fn get_follow_packets(&self) -> messages_treeview::FollowPackets {
        if self.widgets.follow_packets_btn.is_visible()
            && self.widgets.follow_packets_btn.is_active()
        {
            messages_treeview::FollowPackets::Follow
        } else {
            messages_treeview::FollowPackets::DontFollow
        }
    }

    fn handle_got_input_eof(&mut self) {
        if let Err(e) = packets_read::cleanup_child_processes(
            self.model.tcpdump_child.take(),
            self.model.tshark_child.take(),
        ) {
            self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                "Error cleaning up children processes: {}",
                e
            ))));
        }
        let keys: Vec<TcpStreamId> = self.model.streams.keys().copied().collect();
        for stream_id in keys {
            if let Err(e) = self.model.streams.finish_stream(stream_id) {
                self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
                    "Error parsing file after collecting the final packets: {}",
                    e
                ))));
                return;
            }
            // let stream_data = self.model.streams.remove(&stream_id).unwrap();
            // let message_count_before = stream_data.messages.len();
            // let parsers = get_message_parsers();
            // let parser_index = stream_data.parser_index;
            // let parser = parsers.get(parser_index).unwrap();
            // match parser.finish_stream(stream_data) {
            //     Ok(sd) => {
            //         let follow_packets = self.get_follow_packets();
            //         let mut tv_state = self.model.messages_treeview_state.as_mut().unwrap();
            //         messages_treeview::refresh_grids_new_messages(
            //             &mut tv_state,
            //             self.model.relm.stream(),
            //             self.model.selected_card.clone(),
            //             stream_id,
            //             message_count_before,
            //             &sd,
            //             follow_packets,
            //         );

            //         // finishing the stream may well have caused us to
            //         // update the comm target data stats, update them
            //         if let Some(cs) = sd.client_server.as_ref() {
            //             self.add_update_comm_target_data(
            //                 parser_index,
            //                 parser.as_ref(),
            //                 *cs,
            //                 sd.summary_details.as_deref(),
            //                 SessionChangeType::NewDataInSession,
            //             );
            //         }

            //         self.model.streams.insert(stream_id, sd);
            //     }
            //     Err(e) => {
            //         self.model.relm.stream().emit(Msg::LoadedData(Err(format!(
            //             "Error parsing file after collecting the final packets: {}",
            //             e
            //         ))));
            //         return;
            //     }
            // }
        }
        if self.model.streams.is_empty() {
            self.model.relm.stream().emit(Msg::LoadedData(Err(
                "Hotwire doesn't know how to read any useful data from this file".to_string(),
            )));
            self.widgets.loading_spinner.stop();
            self.widgets.open_btn.set_sensitive(true);
            self.widgets.capture_btn.set_sensitive(true);
            self.widgets
                .root_stack
                .set_visible_child_name(WELCOME_STACK_NAME);
            return;
        }

        if let Some(card) = self.model.selected_card.as_ref() {
            // when we load data, we do NOT update the number of messages
            // per stream in the tree model. We'll now update it after
            // we finished the loading
            let mut treeview_state = self.model.ips_and_streams_treeview_state.as_mut().unwrap();
            let remote_ips = treeview_state.remote_ips();
            ips_and_streams_treeview::init_remote_ips_streams_tree(&mut treeview_state);
            ips_and_streams_treeview::refresh_remote_ips_streams_tree(
                &mut treeview_state,
                &self.widgets.remote_ips_streams_treeview,
                &self.model.streams,
                card,
                &remote_ips,
                ips_and_streams_treeview::IsNewDataStillIncoming::No,
            );
        }
        self.widgets.loading_spinner.stop();
        self.widgets.open_btn.set_sensitive(true);
        self.widgets.capture_btn.set_sensitive(true);
        self.widgets
            .root_stack
            .set_visible_child_name(NORMAL_STACK_NAME);
    }

    fn display_error_block(msg: &str, secondary: Option<&str>) {
        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::all(),
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            msg,
        );
        dialog.set_secondary_text(secondary);
        let _r = dialog.run();
        dialog.close();
    }

    fn add_update_comm_target_data(
        &mut self,
        protocol_index: usize,
        protocol_icon: Icon,
        client_server_info: ClientServerInfo,
        summary_details: Option<&str>,
        session_change_type: SessionChangeType,
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
            if session_change_type == SessionChangeType::NewSession {
                card.increase_incoming_session_count();
            }
            card.remote_hosts.insert(client_server_info.client_ip);
            if card.summary_details.is_none() {
                if let Some(details) = summary_details {
                    card.summary_details = SummaryDetails::new(details.to_string(), card_key);
                }
            }
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
                    bs.insert(client_server_info.client_ip);
                    bs
                },
                protocol_icon,
                summary_details.and_then(|d| SummaryDetails::new(d.to_string(), card_key)),
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
                    .select_row(self.widgets.comm_target_list.row_at_index(0).as_ref());
            }
        }
    }

    fn handle_keypress(&self, e: gdk::EventKey) {
        if e.keyval() == gdk::keys::constants::Escape {
            self.components
                .headerbar_search
                .emit(HeaderbarSearchMsg::SearchActiveChanged(false));
        }
        if !(e.state() & gdk::ModifierType::CONTROL_MASK).is_empty() {
            let is_search_active = self.widgets.headerbar_search_revealer.is_child_revealed();
            match e.keyval().to_unicode() {
                Some('s') => {
                    self.model
                        .relm
                        .stream()
                        .emit(Msg::SearchActiveChanged(!is_search_active));
                }
                Some('k') if is_search_active => {
                    self.components
                        .headerbar_search
                        .emit(HeaderbarSearchMsg::OpenSearchAddPopover);
                }
                _ => {}
            }
        }
    }

    fn display_preferences(&mut self) {
        self.model.prefs_win =
            Some(relm::init::<Preferences>(()).expect("Error initializing the preferences window"));
        let prefs_win = self.model.prefs_win.as_ref().unwrap();
        prefs_win
            .widget()
            .set_transient_for(Some(&self.widgets.window));
        prefs_win
            .widget()
            .set_position(gtk::WindowPosition::CenterOnParent);
        prefs_win.widget().set_modal(true);
        prefs_win.widget().show();
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

    fn display_shortcuts(&self) {
        let win = gtk::Builder::from_string(SHORTCUTS_UI)
            .object::<gtk::Window>("shortcuts")
            .unwrap();
        win.set_title("Keyboard Shortcuts");
        win.set_transient_for(Some(&self.widgets.window));
        win.show();
    }

    fn handle_capture_toggled(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // TODO don't call from the GUI thread
        let is_active = self.widgets.capture_btn.is_active();
        self.widgets.capture_spinner.set_visible(is_active);
        self.widgets.follow_packets_btn.set_active(true);
        self.widgets.follow_packets_btn.set_visible(is_active);
        let config = Config::read_config();
        if is_active {
            self.widgets.capture_spinner.start();
            self.reset_open_file(None, TSharkInputType::Fifo);

            let fifo_path = packets_read::setup_fifo_path()?;
            if is_flatpak() || !cfg!(target_os = "linux") || !config.tcpdump_use_pkexec_if_possible
            {
                self.handle_capture_non_pkexec(&fifo_path)?;
            } else {
                let tcpdump_child = packets_read::invoke_tcpdump(&fifo_path)?;
                self.model.tcpdump_child = Some(tcpdump_child);
            }
            let s = self.model.loaded_data_sender.clone();
            self.model
                .bg_sender
                .send(BgFunc::new(move || {
                    Self::load_file(TSharkInputType::Fifo, fifo_path.clone(), s.clone());
                }))
                .unwrap();
        } else {
            self.capture_finished();
            self.widgets.capture_spinner.stop();
            self.widgets.open_btn.set_sensitive(true);
            self.widgets.capture_btn.set_sensitive(true);
            packets_read::cleanup_child_processes(
                self.model.tcpdump_child.take(),
                self.model.tshark_child.take(),
            )?;
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

    fn handle_capture_non_pkexec(&mut self, fifo: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::all(),
            gtk::MessageType::Info,
            gtk::ButtonsType::Close,
            "Please run tcpdump manually",
        );
        let command = "sudo ".to_string() + &packets_read::get_tcpdump_params(fifo).join(" ");
        dialog.set_secondary_text(Some(&format!(
            "Due to privilege issues, hotwire cannot capture packets itself. \
             Please launch an external program to write the packets to a fifo \
             that hotwire will listen to:\n\n<tt>{}</tt>",
            command
        )));
        dialog.set_secondary_use_markup(true);
        dialog.add_button("Copy command", gtk::ResponseType::Accept);
        let r = dialog.run();
        if r == gtk::ResponseType::Accept {
            if let Some(clip) = gtk::Clipboard::default(&self.widgets.window.display()) {
                clip.set_text(&command);
            }
        }
        dialog.close();
        Ok(())
    }

    fn capture_finished(&mut self) {
        self.widgets
            .root_stack
            .set_visible_child_name(if self.model.streams.is_empty() {
                WELCOME_STACK_NAME
            } else {
                NORMAL_STACK_NAME
            });
        if self.model.capture_malformed_packets > 0 {
            Self::display_error_block(
                &format!(
                    "Encountered {} malformed packets during capture.",
                    self.model.capture_malformed_packets
                ),
                Some("Consider increasing the capture buffer size in the settings"),
            );
        }
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
            if let Some(fname) = dialog.filename() {
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
            if let Some(fname) = dialog.filename() {
                self.gui_load_file(fname);
            }
        }
    }

    fn reset_open_file(&mut self, fname: Option<PathBuf>, filetype: TSharkInputType) {
        // we can't set the height directly when loading the app, because
        // by then the window is not fully displayed and we get funny numbers.
        // so we do it later here, but we want to do it just once per app running,
        // not everytime a file is opened
        if !self.model.set_sidebar_height {
            self.widgets
                .sidebar_pane
                .set_position((self.widgets.window.allocated_height() as f32 * 0.6) as i32);
            self.model.set_sidebar_height = true;
        }

        self.model.capture_malformed_packets = 0;
        self.widgets.open_btn.set_sensitive(false);
        if filetype != TSharkInputType::Fifo {
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
        let mut ips_treeview_state = self.model.ips_and_streams_treeview_state.as_mut().unwrap();
        ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
        ips_and_streams_treeview::connect_remote_ips_streams_tree(
            ips_treeview_state,
            &self.widgets.remote_ips_streams_treeview,
        );
        self.components
            .headerbar_search
            .emit(HeaderbarSearchMsg::SearchActiveChanged(false));
        self.model.window_subtitle = Some(
            fname
                .as_ref()
                .and_then(|p| {
                    if is_flatpak() {
                        // can't get the folder name within a flatpak
                        // https://github.com/flatpak/xdg-desktop-portal/issues/475
                        p.file_name().map(|f| f.to_string_lossy())
                    } else {
                        Some(p.to_string_lossy())
                    }
                })
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
        if let Some(ref mut tv_state) = self.model.messages_treeview_state {
            tv_state.file_closed();
        }
        if let Some(ref mut tv_state) = self.model.ips_and_streams_treeview_state {
            tv_state.file_closed();
        }
        self.model.selected_card = None;
        self.model.comm_target_cards.clear();
        for child in self.widgets.comm_target_list.children() {
            self.widgets.comm_target_list.remove(&child);
        }
        self.model.comm_targets_components.clear();
    }

    fn add_to_recent_files(fname: &Path) {
        if let Some(rm) = gtk::RecentManager::default() {
            if let Some(fname_str) = fname.to_str() {
                let recent_data = gtk::RecentData {
                    display_name: None,
                    description: None,
                    mime_type: PCAP_MIME_TYPE.to_string(),
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
        let is_fifo = if cfg!(unix) {
            std::fs::metadata(&fname)
                .ok()
                .filter(|m| m.file_type().is_fifo())
                .is_some()
        } else {
            false
        };

        self.reset_open_file(
            Some(fname.clone()),
            if is_fifo {
                TSharkInputType::Fifo
            } else {
                TSharkInputType::File
            },
        );

        if is_fifo {
            self.widgets
                .capture_btn
                .block_signal(self.model.capture_toggle_signal.as_ref().unwrap());
            // the capture button is invisible by default except on linux
            // in case of file opening through fifo, make it visible always
            // (so the user can stop the import)
            self.widgets.capture_btn.set_visible(true);
            self.widgets.capture_btn.set_active(true);
            self.widgets.capture_spinner.set_visible(true);
            self.widgets.capture_spinner.start();
            self.widgets
                .capture_btn
                .unblock_signal(self.model.capture_toggle_signal.as_ref().unwrap());
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
        // let filter = get_message_parsers()
        //     .into_iter()
        //     .map(|p| p.tshark_filter_string())
        //     .join(" || ");
        let filter = MESSAGE_PARSERS.combine_tshark_filter_strings();
        packets_read::invoke_tshark(file_type, &fname, &filter, sender);
    }

    view! {
        #[name="window"]
        gtk::Window {
            default_width: 1000,
            default_height: 650,
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
                                width_request: 450,
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
                                                Msg::OpenRecentFile(row.index() as usize)
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
                        },
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
                                #[name="menu_box"]
                                gtk::Box {
                                    orientation: gtk::Orientation::Vertical,
                                    margin_top: 10,
                                    margin_start: 10,
                                    margin_end: 10,
                                    margin_bottom: 10,
                                    gtk::ModelButton {
                                        label: "Preferences",
                                        hexpand: true,
                                        clicked => Msg::DisplayPreferences,
                                    },
                                    gtk::ModelButton {
                                        label: "Keyboard Shortcuts",
                                        hexpand: true,
                                        clicked => Msg::DisplayShortcuts,
                                    },
                                    gtk::ModelButton {
                                        label: "About Hotwire",
                                        hexpand: true,
                                        clicked => Msg::DisplayAbout,
                                    },
                                }
                            }
                        },
                    },
                    #[name="search_toggle"]
                    gtk::ToggleButton {
                        child: {
                            pack_type: gtk::PackType::End
                        },
                        image: Some(&gtk::Image::from_icon_name(Some("edit-find-symbolic"), gtk::IconSize::Menu)),
                        margin_start: 10,
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
                    #[name="headerbar_search_revealer"]
                    gtk::Revealer {
                        #[name="headerbar_search"]
                        HeaderbarSearch((self.model.bg_sender.clone(), self.model.recent_searches.clone())) {
                            HbsMsgSearchActiveChanged(is_active) => Msg::SearchActiveChanged(is_active),
                            HbsMsgSearchExprChanged(ref m_expr) => Msg::SearchExprChanged(m_expr.clone()),
                        },
                    },
                    #[name="infobar"]
                    gtk::InfoBar {
                        response(_, r) => Msg::InfoBarEvent(r),
                        visible: true,
                        revealed: false,
                    },
                    gtk::Box {
                        vexpand: true,
                        #[name="sidebar_pane"]
                        gtk::Paned {
                            orientation: gtk::Orientation::Vertical,
                            #[style_class="sidebar"]
                            gtk::ScrolledWindow {
                                child: {
                                    resize: true,
                                },
                                width_request: 250,
                                #[name="comm_target_list"]
                                gtk::ListBox {
                                    row_selected(_, row) =>
                                        Msg::SelectCard(row.map(|r| r.index() as usize))
                                }
                            },
                            gtk::ScrolledWindow {
                                width_request: 150,
                                #[name="remote_ips_streams_treeview"]
                                gtk::TreeView {
                                    activate_on_single_click: true,
                                    // connecting manually to collect the signal id for blocking
                                    // selection.changed(selection) => Msg::SelectRemoteIpStream(selection.clone()),
                                },
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
