use super::comm_remote_server::{CommRemoteServer, CommRemoteServerData, MessageData};
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::http_comm_entry::HttpMessageData;
use super::postgres_comm_entry;
use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::comm_remote_server::MessageParserDetailsMsg;
use crate::widgets::http_comm_entry::Http;
use crate::widgets::postgres_comm_entry::Postgres;
use crate::TSharkCommunication;
use glib::translate::ToGlib;
use gtk::prelude::*;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");

pub fn get_message_parsers() -> Vec<Box<dyn MessageParser>> {
    vec![Box::new(Http), Box::new(Postgres)]
}

#[derive(Msg)]
pub enum Msg {
    SelectCard(Option<usize>),
    SelectRemoteIpStream(gtk::TreeSelection),

    SelectCardAll(CommTargetCardData),
    SelectCardFromRemoteIp(CommTargetCardData, String),
    SelectCardFromRemoteIpAndStream(CommTargetCardData, String, u32),

    DisplayDetails(u32, u32),

    Quit,
}

struct StreamInfo {
    stream_id: u32,
    target_ip: String,
    target_port: u32,
    source_ip: String,
}

pub struct Model {
    relm: relm::Relm<Win>,
    streams: Vec<(StreamInfo, Vec<MessageData>)>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    remote_ips_streams_tree_store: gtk::TreeStore,
    comm_remote_servers_stores: Vec<gtk::ListStore>,
    comm_remote_servers_treeviews: Vec<gtk::TreeView>,
    disable_tree_view_selection_events: bool,

    _comm_targets_components: Vec<Component<CommTargetCard>>,

    details_component_streams: Vec<relm::StreamHandle<MessageParserDetailsMsg>>,
}

#[derive(PartialEq, Eq)]
enum RefreshRemoteIpsAndStreams {
    Yes,
    No,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        if let Err(err) = self.load_style() {
            println!("Error loading the CSS: {}", err);
        }

        for (idx, message_parser) in get_message_parsers().iter().enumerate() {
            let tv = gtk::TreeViewBuilder::new()
                .activate_on_single_click(true)
                .build();
            let store = message_parser.prepare_treeview(&tv);
            self.model.comm_remote_servers_stores.push(store.clone());
            self.model.comm_remote_servers_treeviews.push(tv.clone());
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
                .push(message_parser.add_details_to_scroll(&scroll2));
            scroll2.set_property_height_request(200);
            paned.pack2(&scroll2, false, true);
            let rstream = self.model.relm.stream().clone();
            tv.get_selection().connect_changed(move |selection| {
                if let Some((model, iter)) = selection.get_selected() {
                    if let Some(path) = model.get_path(&iter) {
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
                }
            });
            self.widgets
                .comm_remote_servers_stack
                .add_named(&paned, &idx.to_string());
            paned.show_all();
        }

        let remote_ip_col = gtk::TreeViewColumnBuilder::new()
            .title("Incoming conns")
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new()
            .weight(1)
            .weight_set(true)
            .build();
        remote_ip_col.pack_start(&cell_r_txt, true);
        remote_ip_col.add_attribute(&cell_r_txt, "text", 0);
        remote_ip_col.add_attribute(&cell_r_txt, "weight", 1);
        self.widgets
            .remote_ips_streams_treeview
            .append_column(&remote_ip_col);

        self.refresh_comm_targets();
        self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, None, None);
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
        streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    ) -> Model {
        gtk::IconTheme::get_default()
            .unwrap()
            .add_resource_path("/icons");
        let message_parsers = get_message_parsers();

        let mut parsed_streams: Vec<_> = streams
            .iter()
            .filter_map(|(id, comms)| {
                comms
                    .iter()
                    .find_map(|c| message_parsers.iter().find(|p| p.is_my_message(c)))
                    .map(|parser| {
                        let layers = &comms.first().unwrap().source.layers;
                        let card_key = (
                            layers.ip.as_ref().unwrap().ip_dst.clone(),
                            layers.tcp.as_ref().unwrap().port_dst,
                        );
                        let ip_src = layers.ip.as_ref().unwrap().ip_src.clone();
                        (parser, id, ip_src, card_key, parser.parse_stream(&comms))
                    })
            })
            .collect();
        parsed_streams.sort_by_key(|(_parser, id, _ip_src, _card_key, _pstream)| *id);

        let comm_target_cards: Vec<_> = parsed_streams
            .iter()
            .fold(
                HashMap::<(String, u32), CommTargetCardData>::new(),
                |mut sofar, (parser, _stream_id, ip_src, card_key, items)| {
                    if let Some(target_card) = sofar.get_mut(&card_key) {
                        target_card.remote_hosts.insert(ip_src.to_string());
                        target_card.incoming_session_count += 1;
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
                            },
                        );
                    }
                    sofar
                },
            )
            .into_iter()
            .map(|(k, v)| v)
            .collect();

        let remote_ips_streams_tree_store = gtk::TreeStore::new(&[
            String::static_type(),
            pango::Weight::static_type(),
            u32::static_type(),
        ]);

        Model {
            relm: relm.clone(),
            streams: parsed_streams
                .into_iter()
                .map(|(_parser, id, ip_src, card_key, pstream)| {
                    (
                        StreamInfo {
                            stream_id: id.unwrap(),
                            target_ip: card_key.0,
                            target_port: card_key.1,
                            source_ip: ip_src,
                        },
                        pstream,
                    )
                })
                .collect(),
            comm_target_cards,
            _comm_targets_components: vec![],
            selected_card: None,
            remote_ips_streams_tree_store,
            comm_remote_servers_stores: vec![],
            comm_remote_servers_treeviews: vec![],
            disable_tree_view_selection_events: false,
            details_component_streams: vec![],
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SelectCard(maybe_idx) => {
                self.model.selected_card = maybe_idx
                    .and_then(|idx| self.model.comm_target_cards.get(idx as usize))
                    .cloned();
                self.refresh_remote_servers(RefreshRemoteIpsAndStreams::Yes, None, None);
                // if let Some(vadj) = self.widgets.remote_servers_scroll.get_vadjustment() {
                //     vadj.set_value(0.0);
                // }
            }
            Msg::SelectRemoteIpStream(selection) => {
                if let Some((model, iter)) = selection.get_selected() {
                    if let Some(mut path) = model.get_path(&iter) {
                        match path.get_indices_with_depth().as_slice() {
                            &[0] => self.model.relm.stream().emit(Msg::SelectCardAll(
                                self.model.selected_card.as_ref().unwrap().clone(),
                            )),
                            x if x.len() == 1 => {
                                if let Some(iter) =
                                    self.model.remote_ips_streams_tree_store.get_iter(&path)
                                {
                                    let remote_ip = self
                                        .model
                                        .remote_ips_streams_tree_store
                                        .get_value(&iter, 0);
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
                                self.model.relm.stream().emit(
                                    Msg::SelectCardFromRemoteIpAndStream(
                                        self.model.selected_card.as_ref().unwrap().clone(),
                                        remote_ip.get().unwrap().unwrap(),
                                        stream_id.get().unwrap().unwrap(),
                                    ),
                                );
                            }
                            _ => panic!(path.get_depth()),
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
                let msg_data = self
                    .model
                    .streams
                    .iter()
                    .find(|(stream_info, items)| stream_info.stream_id == stream_id)
                    .unwrap()
                    .1
                    .get(idx as usize)
                    .unwrap();
                for component_stream in &self.model.details_component_streams {
                    // println!("{:?}", msg_data);
                    component_stream
                        .emit(MessageParserDetailsMsg::DisplayDetails(msg_data.clone()));
                }
            }
            Msg::Quit => gtk::main_quit(),
        }
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
        let all_iter = self.model.remote_ips_streams_tree_store.append(None);
        self.model
            .remote_ips_streams_tree_store
            .set_value(&all_iter, 0, &"All".to_value());
        self.model.remote_ips_streams_tree_store.set_value(
            &all_iter,
            1,
            &pango::Weight::Bold.to_glib().to_value(),
        );
        self.widgets.remote_ips_streams_treeview.set_cursor(
            &gtk::TreePath::new_first(),
            None::<&gtk::TreeViewColumn>,
            false,
        );
        let target_ip = card.ip.clone();
        let target_port = card.port;

        for remote_ip in remote_ips {
            let remote_ip_iter = self.model.remote_ips_streams_tree_store.append(None);
            self.model.remote_ips_streams_tree_store.set_value(
                &remote_ip_iter,
                0,
                &remote_ip.to_value(),
            );
            self.model.remote_ips_streams_tree_store.set_value(
                &remote_ip_iter,
                1,
                &pango::Weight::Normal.to_glib().to_value(),
            );
            for (stream_info, _messages) in &self.model.streams {
                if stream_info.target_ip != target_ip
                    || stream_info.target_port != target_port
                    || stream_info.source_ip != *remote_ip
                {
                    continue;
                }
                let session_iter = self
                    .model
                    .remote_ips_streams_tree_store
                    .append(Some(&remote_ip_iter));
                self.model.remote_ips_streams_tree_store.set_value(
                    &session_iter,
                    0,
                    &format!("Session {}", stream_info.stream_id).to_value(),
                );
                self.model.remote_ips_streams_tree_store.set_value(
                    &session_iter,
                    1,
                    &pango::Weight::Normal.to_glib().to_value(),
                );
                self.model.remote_ips_streams_tree_store.set_value(
                    &session_iter,
                    2,
                    &stream_info.stream_id.to_value(),
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
            self.model.disable_tree_view_selection_events = true;
            for (remote_ip, tcp_sessions) in &by_remote_ip {
                for (session_id, session) in tcp_sessions {
                    mp.populate_treeview(&store, *session_id, &session);
                }
            }
            self.model.disable_tree_view_selection_events = false;
            self.model
                .comm_remote_servers_treeviews
                .get(card.protocol_index)
                .unwrap()
                .get_selection()
                .select_path(&gtk::TreePath::new_first());
            if refresh_remote_ips_and_streams == RefreshRemoteIpsAndStreams::Yes {
                let ip_hash = by_remote_ip
                    .keys()
                    .map(|c| c.to_string())
                    .collect::<HashSet<_>>();
                self.refresh_remote_ips_streams_tree(&card, &ip_hash);
            }
        }
    }

    view! {
        #[name="window"]
        gtk::Window {
            titlebar: view! {
                gtk::HeaderBar {
                    show_close_button: true,
                    title: Some("Hotwire"),
                }
            },
            gtk::Box {
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
            delete_event(_, _) => (Msg::Quit, Inhibit(false)),
        }
    }
}
