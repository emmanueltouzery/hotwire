use super::comm_target_card;
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::headerbar_search::HeaderbarSearch;
use super::headerbar_search::Msg as HeaderbarSearchMsg;
use super::headerbar_search::Msg::SearchActiveChanged as HbsMsgSearchActiveChanged;
use super::headerbar_search::Msg::SearchTextChanged as HbsMsgSearchTextChanged;
use super::ips_and_streams_treeview;
use super::messages_treeview;
use super::recent_file_item::RecentFileItem;
use crate::config;
use crate::http::http_message_parser::Http;
use crate::http2::http2_message_parser::Http2;
use crate::icons::Icon;
use crate::message_parser::ClientServerInfo;
use crate::message_parser::MessageParser;
use crate::message_parser::StreamData;
use crate::packets_read;
use crate::packets_read::{InputStep, ParseInputStep, TSharkInputType};
use crate::pgsql::postgres_message_parser::Postgres;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpStreamId};
use crate::widgets::comm_target_card::CommTargetCardKey;
use crate::widgets::comm_target_card::SummaryDetails;
use crate::BgFunc;
use gdk::prelude::*;
use gtk::prelude::*;
use itertools::Itertools;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::net::IpAddr;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::mpsc;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");

const WELCOME_STACK_NAME: &str = "welcome";
const LOADING_STACK_NAME: &str = "loading";
const NORMAL_STACK_NAME: &str = "normal";

pub fn get_message_parsers() -> Vec<Box<dyn MessageParser>> {
    vec![Box::new(Http), Box::new(Postgres), Box::new(Http2)]
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

    window_subtitle: Option<String>,
    current_file: Option<(PathBuf, TSharkInputType)>,
    recent_files: Vec<PathBuf>,

    capture_toggle_signal: Option<glib::SignalHandlerId>,

    infobar_spinner: gtk::Spinner,
    infobar_label: gtk::Label,

    sidebar_selection_change_signal_id: Option<glib::SignalHandlerId>,

    streams: HashMap<TcpStreamId, StreamData>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    search_text: String,

    messages_treeview_state: Option<messages_treeview::MessagesTreeviewState>,
    ips_and_streams_treeview_state: Option<ips_and_streams_treeview::IpsAndStreamsTreeviewState>,

    _loaded_data_channel: relm::Channel<ParseInputStep>,
    loaded_data_sender: relm::Sender<ParseInputStep>,

    comm_targets_components: HashMap<CommTargetCardKey, Component<CommTargetCard>>,
    _recent_file_item_components: Vec<Component<RecentFileItem>>,

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
        params: (mpsc::Sender<BgFunc>, Option<(PathBuf, TSharkInputType)>),
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
            infobar_spinner: gtk::SpinnerBuilder::new()
                .width_request(24)
                .height_request(24)
                .build(),
            infobar_label: gtk::LabelBuilder::new().build(),
            comm_targets_components: HashMap::new(),
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
            search_text: "".to_string(),
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
                if let Some(card) = self.model.selected_card.as_ref() {
                    messages_treeview::search_text_changed(
                        self.model.messages_treeview_state.as_ref().unwrap(),
                        card.protocol_index,
                        if is_active {
                            &self.model.search_text
                        } else {
                            ""
                        },
                    );
                }
            }
            Msg::SearchTextChanged(txt) => {
                self.model.search_text = txt.clone();
                if let Some(card) = self.model.selected_card.as_ref() {
                    messages_treeview::search_text_changed(
                        self.model.messages_treeview_state.as_ref().unwrap(),
                        card.protocol_index,
                        &txt,
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
                self.widgets.infobar.set_visible(false);
            }
            Msg::InfoBarEvent(_) => {}
            Msg::LoadedData(Err(msg)) => {
                self.handle_got_loading_error(&msg);
            }
            Msg::LoadedData(Ok(InputStep::StartedTShark(pid))) => {
                self.model.tshark_child = Some(pid);
            }
            Msg::LoadedData(Ok(InputStep::Packet(p))) => {
                self.handle_got_packet(p);
            }
            Msg::LoadedData(Ok(InputStep::Eof)) => {
                self.handle_got_input_eof();
            }
            Msg::SelectCard(maybe_idx) => {
                self.handle_select_card(maybe_idx);
            }
            Msg::SelectRemoteIpStream(selection) => {
                let (mut paths, model) = selection.get_selected_rows();
                println!("remote selection changed");
                ips_and_streams_treeview::refresh_remote_ip_stream(
                    self.model.relm.stream(),
                    self.model.selected_card.as_ref(),
                    &self.widgets.remote_ips_streams_treeview,
                    &mut paths,
                );
            }
            Msg::SelectCardFromRemoteIpsAndStreams(_, remote_ips, stream_ids) => {
                let mut ips_treeview_state =
                    self.model.ips_and_streams_treeview_state.take().unwrap();
                ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
                self.model.ips_and_streams_treeview_state = Some(ips_treeview_state);
                messages_treeview::refresh_remote_servers(
                    self.model.messages_treeview_state.as_ref().unwrap(),
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
                        msg_data,
                    );
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
        self.model.infobar_label.set_text(&msg);
        self.widgets.infobar.set_visible(true);
    }

    fn handle_select_card(&mut self, maybe_idx: Option<usize>) {
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
        let mut ips_treeview_state = self.model.ips_and_streams_treeview_state.take().unwrap();
        ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
        self.model.ips_and_streams_treeview_state = Some(ips_treeview_state);
        let refresh_streams_tree = messages_treeview::refresh_remote_servers(
            self.model.messages_treeview_state.as_ref().unwrap(),
            self.model.selected_card.as_ref(),
            &self.model.streams,
            &self.widgets.remote_ips_streams_treeview,
            self.model.sidebar_selection_change_signal_id.as_ref(),
            &[],
            &[],
        );
        if let RefreshRemoteIpsAndStreams::Yes(card, ips) = refresh_streams_tree {
            let mut treeview_state = self.model.ips_and_streams_treeview_state.take().unwrap();
            ips_and_streams_treeview::refresh_remote_ips_streams_tree(
                &mut treeview_state,
                &self.widgets.remote_ips_streams_treeview,
                &self.model.streams,
                &card,
                &ips,
            );
            self.model.ips_and_streams_treeview_state = Some(treeview_state);
        }
        messages_treeview::refresh_remote_servers_handle_selection(
            self.model.messages_treeview_state.as_ref().unwrap(),
            self.model.selected_card.as_ref(),
            &self.widgets.remote_ips_streams_treeview,
            self.model.sidebar_selection_change_signal_id.as_ref(),
        );
        if let Some(p) = self.widgets.root_stack.get_parent_window() {
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
        let mut ips_treeview_state = self.model.ips_and_streams_treeview_state.take().unwrap();
        ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
        self.model.ips_and_streams_treeview_state = Some(ips_treeview_state);
        let refresh_streams_tree = messages_treeview::refresh_remote_servers(
            self.model.messages_treeview_state.as_ref().unwrap(),
            self.model.selected_card.as_ref(),
            &self.model.streams,
            &self.widgets.remote_ips_streams_treeview,
            self.model.sidebar_selection_change_signal_id.as_ref(),
            &[],
            &[],
        );
        if let RefreshRemoteIpsAndStreams::Yes(card, ips) = refresh_streams_tree {
            let mut treeview_state = self.model.ips_and_streams_treeview_state.take().unwrap();
            ips_and_streams_treeview::refresh_remote_ips_streams_tree(
                &mut treeview_state,
                &self.widgets.remote_ips_streams_treeview,
                &self.model.streams,
                &card,
                &ips,
            );
            self.model.ips_and_streams_treeview_state = Some(treeview_state);
        }
        messages_treeview::refresh_remote_servers_handle_selection(
            self.model.messages_treeview_state.as_ref().unwrap(),
            self.model.selected_card.as_ref(),
            &self.widgets.remote_ips_streams_treeview,
            self.model.sidebar_selection_change_signal_id.as_ref(),
        );
        Self::display_error_block("Cannot load file", Some(&msg));
    }

    fn handle_got_packet(&mut self, p: TSharkPacket) {
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
                    self.add_update_comm_target_data(
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
                    self.add_update_comm_target_data(
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
                        let mut treeview_state =
                            self.model.ips_and_streams_treeview_state.take().unwrap();
                        ips_and_streams_treeview::got_packet_refresh_remote_ips_treeview(
                            &mut treeview_state,
                            &stream_data,
                            packet_stream_id,
                        );
                        self.model.ips_and_streams_treeview_state = Some(treeview_state);
                    }
                }
                stream_data
            };
            let mut tv_state = self.model.messages_treeview_state.take().unwrap();
            messages_treeview::refresh_grids_new_messages(
                &mut tv_state,
                self.model.relm.stream(),
                self.model.selected_card.clone(),
                packet_stream_id,
                parser_index,
                message_count_before,
                &stream_data,
                self.get_follow_packets(),
            );
            self.model.messages_treeview_state = Some(tv_state);
            self.model.streams.insert(packet_stream_id, stream_data);
        }
    }

    fn get_follow_packets(&self) -> messages_treeview::FollowPackets {
        if self.widgets.follow_packets_btn.is_visible()
            && self.widgets.follow_packets_btn.get_active()
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
        let keys: Vec<TcpStreamId> = self.model.streams.keys().map(|k| *k).collect();
        for stream_id in keys {
            let stream_data = self.model.streams.remove(&stream_id).unwrap();
            let message_count_before = stream_data.messages.len();
            let parsers = get_message_parsers();
            let parser_index = stream_data.parser_index;
            let parser = parsers.get(parser_index).unwrap();
            match parser.finish_stream(stream_data) {
                Ok(sd) => {
                    let mut tv_state = self.model.messages_treeview_state.take().unwrap();
                    messages_treeview::refresh_grids_new_messages(
                        &mut tv_state,
                        self.model.relm.stream(),
                        self.model.selected_card.clone(),
                        stream_id,
                        parser_index,
                        message_count_before,
                        &sd,
                        self.get_follow_packets(),
                    );

                    // finishing the stream may well have caused us to
                    // update the comm target data stats, update them
                    self.add_update_comm_target_data(
                        parser_index,
                        &parser,
                        sd.client_server.as_ref().unwrap(),
                        sd.summary_details.as_deref(),
                    );

                    self.model.messages_treeview_state = Some(tv_state);
                    self.model.streams.insert(stream_id, sd);
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
                "Hotwire doesn't know how to read any useful data from this file".to_string(),
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

    fn add_update_comm_target_data(
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
            self.reset_open_file(None, TSharkInputType::Fifo);
            let (tcpdump_child, fifo_path) = packets_read::invoke_tcpdump()?;

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

    fn reset_open_file(&mut self, fname: Option<PathBuf>, filetype: TSharkInputType) {
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
        let mut ips_treeview_state = self.model.ips_and_streams_treeview_state.take().unwrap();
        ips_and_streams_treeview::init_remote_ips_streams_tree(&mut ips_treeview_state);
        ips_and_streams_treeview::connect_remote_ips_streams_tree(
            &ips_treeview_state,
            &self.widgets.remote_ips_streams_treeview,
        );
        self.model.ips_and_streams_treeview_state = Some(ips_treeview_state);
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
        if let Some(ref mut tv_state) = self.model.messages_treeview_state {
            tv_state.file_closed();
        }
        if let Some(ref mut tv_state) = self.model.ips_and_streams_treeview_state {
            tv_state.file_closed();
        }
        self.model.selected_card = None;
        self.model.comm_target_cards.clear();
        for child in self.widgets.comm_target_list.get_children() {
            self.widgets.comm_target_list.remove(&child);
        }
        self.model.comm_targets_components.clear();
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
                TSharkInputType::Fifo
            } else {
                TSharkInputType::File
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
        packets_read::invoke_tshark(file_type, &fname, &filter, sender);
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
