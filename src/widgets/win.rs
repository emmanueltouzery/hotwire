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
    Quit,
}

pub struct Model {
    relm: relm::Relm<Win>,
    streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    remote_ips_streams_tree_store: gtk::TreeStore,

    _comm_targets_components: Vec<Component<CommTargetCard>>,
    _comm_remote_servers_components: Vec<Component<CommRemoteServer>>,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        if let Err(err) = self.load_style() {
            println!("Error loading the CSS: {}", err);
        }

        let remote_ip_col = gtk::TreeViewColumnBuilder::new()
            .title("Incoming connections")
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
        self.refresh_remote_servers();
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

        let comm_target_cards = streams
            .iter()
            .filter_map(|(id, comms)| {
                let recognized_comms: Vec<_> = comms
                    .iter()
                    .filter(|c| {
                        let layers = &c.source.layers;
                        layers.http.is_some() || layers.pgsql.is_some()
                    })
                    .collect();
                if recognized_comms.len() > 0 {
                    Some((id, recognized_comms))
                } else {
                    None
                }
            })
            .fold(
                HashMap::<(String, u32), CommTargetCardData>::new(),
                |mut sofar, (_, items)| {
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
                                protocol_icon: if layers.http.is_some() {
                                    Icon::HTTP
                                } else {
                                    Icon::DATABASE
                                },
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

        let remote_ips_streams_tree_store =
            gtk::TreeStore::new(&[String::static_type(), pango::Weight::static_type()]);

        Model {
            relm: relm.clone(),
            streams,
            comm_target_cards,
            _comm_targets_components: vec![],
            _comm_remote_servers_components: vec![],
            selected_card: None,
            remote_ips_streams_tree_store,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SelectCard(maybe_idx) => {
                self.model.selected_card = maybe_idx
                    .and_then(|idx| self.model.comm_target_cards.get(idx as usize))
                    .cloned();
                self.refresh_remote_servers();
                if let Some(vadj) = self.widgets.remote_servers_scroll.get_vadjustment() {
                    vadj.set_value(0.0);
                }
            }
            Msg::SelectRemoteIpStream(path) => {
                println!("remote ip or stream selected");
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

    fn refresh_remote_servers(&mut self) {
        for child in self.widgets.comm_remote_servers.get_children() {
            self.widgets.comm_remote_servers.remove(&child);
        }
        let mut components = vec![];
        if let Some(card) = &self.model.selected_card {
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
                let tcp_stream_id = layers.tcp.as_ref().map(|t| t.stream);
                let stream_parser = stream
                    .1
                    .iter()
                    .find_map(|m| parsers.iter().find(|p| p.is_my_message(m)));
                if let Some(parser) = stream_parser {
                    let messages = parser.parse_stream(&stream.1);
                    let remote_server_streams = by_remote_ip
                        .entry(layers.ip.as_ref().unwrap().ip_src.clone())
                        .or_insert(vec![]);
                    remote_server_streams.push((tcp_stream_id, messages));
                }
            }
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
                for session in &tcp_sessions {
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
                }
                components.push(
                    self.widgets
                        .comm_remote_servers
                        .add_widget::<CommRemoteServer>(CommRemoteServerData {
                            remote_ip,
                            tcp_sessions,
                        }),
                );
            }
        }
        self.widgets.remote_ips_streams_treeview.expand_all();
        self.model._comm_remote_servers_components = components;
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
                #[name="remote_servers_scroll"]
                gtk::ScrolledWindow {
                    hexpand: true,
                    #[name="comm_remote_servers"]
                    gtk::Box {
                        orientation: gtk::Orientation::Vertical,
                    },
                }
            },
            delete_event(_, _) => (Msg::Quit, Inhibit(false)),
        }
    }
}
