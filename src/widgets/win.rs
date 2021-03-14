use super::comm_remote_server::{CommRemoteServer, CommRemoteServerData, MessageData};
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::http_comm_entry::HttpMessageData;
use super::postgres_comm_entry;
use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::http_comm_entry::Http;
use crate::widgets::postgres_comm_entry::Postgres;
use crate::TSharkCommunication;
use glib::translate::ToGlib;
use gtk::prelude::*;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashMap;
use std::collections::HashSet;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");

pub fn get_message_parsers() -> Vec<Box<dyn MessageParser>> {
    vec![Box::new(Http), Box::new(Postgres)]
}

#[derive(Msg)]
pub enum Msg {
    SelectCard(Option<usize>),
    SelectRemoteIpStream(gtk::TreePath),

    SelectCardAll(CommTargetCardData),
    SelectCardFromRemoteIp(CommTargetCardData, String),
    SelectCardFromRemoteIpAndStream(CommTargetCardData, String, u32),

    Quit,
}

pub struct Model {
    relm: relm::Relm<Win>,
    streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    remote_ips_streams_tree_store: gtk::TreeStore,
    comm_remote_servers_stores: Vec<gtk::ListStore>,

    _comm_targets_components: Vec<Component<CommTargetCard>>,
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
            let tv = gtk::TreeViewBuilder::new().build();
            self.model
                .comm_remote_servers_stores
                .push(message_parser.prepare_treeview(&tv));
            let scroll = gtk::ScrolledWindowBuilder::new()
                .expand(true)
                .child(&tv)
                .build();
            self.widgets
                .comm_remote_servers_stack
                .add_named(&scroll, &idx.to_string());
            scroll.show_all();
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

        let comm_target_cards = streams
            .iter()
            .filter_map(|(id, comms)| {
                comms
                    .iter()
                    .find_map(|c| message_parsers.iter().position(|p| p.is_my_message(c)))
                    .map(|pos| (pos, id, comms))
            })
            .fold(
                HashMap::<(String, u32), CommTargetCardData>::new(),
                |mut sofar, (protocol_index, _, items)| {
                    let layers = &items.first().unwrap().source.layers;
                    let card_key = (
                        layers.ip.as_ref().unwrap().ip_dst.clone(),
                        layers.tcp.as_ref().unwrap().port_dst,
                    );
                    if let Some(target_card) = sofar.get_mut(&card_key) {
                        target_card
                            .remote_hosts
                            .insert(layers.ip.as_ref().unwrap().ip_src.clone());
                        target_card.incoming_session_count += 1;
                    } else {
                        let mut remote_hosts = HashSet::new();
                        remote_hosts.insert(layers.ip.as_ref().unwrap().ip_src.clone());
                        sofar.insert(
                            card_key,
                            CommTargetCardData {
                                ip: layers.ip.as_ref().unwrap().ip_dst.clone(),
                                protocol_index,
                                protocol_icon: message_parsers
                                    .get(protocol_index)
                                    .unwrap()
                                    .protocol_icon(),
                                port: layers.tcp.as_ref().unwrap().port_dst,
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
            streams,
            comm_target_cards,
            _comm_targets_components: vec![],
            selected_card: None,
            remote_ips_streams_tree_store,
            comm_remote_servers_stores: vec![],
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
            Msg::SelectRemoteIpStream(mut path) => match path.get_indices_with_depth().as_slice() {
                &[0] => self.model.relm.stream().emit(Msg::SelectCardAll(
                    self.model.selected_card.as_ref().unwrap().clone(),
                )),
                x if x.len() == 1 => {
                    if let Some(iter) = self.model.remote_ips_streams_tree_store.get_iter(&path) {
                        let remote_ip =
                            self.model.remote_ips_streams_tree_store.get_value(&iter, 0);
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
            },
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
        by_remote_ip: &HashMap<String, Vec<(Option<u32>, Vec<MessageData>)>>,
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

        for (remote_ip, tcp_sessions) in by_remote_ip {
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
            for session in tcp_sessions {
                let session_iter = self
                    .model
                    .remote_ips_streams_tree_store
                    .append(Some(&remote_ip_iter));
                self.model.remote_ips_streams_tree_store.set_value(
                    &session_iter,
                    0,
                    &format!("Session {}", session.0.unwrap()).to_value(),
                );
                self.model.remote_ips_streams_tree_store.set_value(
                    &session_iter,
                    1,
                    &pango::Weight::Normal.to_glib().to_value(),
                );
                self.model.remote_ips_streams_tree_store.set_value(
                    &session_iter,
                    2,
                    &session.0.unwrap().to_value(),
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
        if let Some(card) = self.model.selected_card.as_ref().map(|c| c.clone()) {
            let target_ip = card.ip.clone();
            let target_port = card.port;
            let mut by_remote_ip = HashMap::new();
            let parsers = get_message_parsers();
            for stream in &self.model.streams {
                let layers = &stream.1.first().unwrap().source.layers;
                if layers.ip.as_ref().unwrap().ip_dst != target_ip
                    || layers.tcp.as_ref().unwrap().port_dst != target_port
                {
                    continue;
                }
                let remote_ip = layers.ip.as_ref().unwrap().ip_src.clone();
                if let Some(ref constrained_remote) = constrain_remote_ip {
                    if constrained_remote != &remote_ip {
                        continue;
                    }
                }
                let tcp_stream_id = layers.tcp.as_ref().map(|t| t.stream);
                if constrain_stream_id.is_some() && constrain_stream_id != tcp_stream_id {
                    continue;
                }
                let stream_parser = stream
                    .1
                    .iter()
                    .find_map(|m| parsers.iter().find(|p| p.is_my_message(m)));
                if let Some(parser) = stream_parser {
                    let messages = parser.parse_stream(&stream.1);
                    let remote_server_streams = by_remote_ip.entry(remote_ip).or_insert(vec![]);
                    remote_server_streams.push((tcp_stream_id, messages));
                }
            }
            let mp = parsers.get(card.protocol_index).unwrap();
            self.widgets
                .comm_remote_servers_stack
                .set_visible_child_name(&card.protocol_index.to_string());
            if refresh_remote_ips_and_streams == RefreshRemoteIpsAndStreams::Yes {
                self.refresh_remote_ips_streams_tree(&card, &by_remote_ip);
            }
            let store = &self
                .model
                .comm_remote_servers_stores
                .get(card.protocol_index)
                .unwrap();
            for (remote_ip, tcp_sessions) in by_remote_ip {
                for (_, session) in tcp_sessions {
                    mp.populate_treeview(&store, &session);
                }
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
                        row_activated(_, path, _) => Msg::SelectRemoteIpStream(path.clone()),
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
