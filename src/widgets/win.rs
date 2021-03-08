use super::comm_remote_server::{CommRemoteServer, CommRemoteServerData, MessageData};
use super::comm_target_card::{CommTargetCard, CommTargetCardData};
use super::http_comm_entry::HttpMessageData;
use super::postgres_comm_entry::PostgresMessageData;
use crate::icons::Icon;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashMap;
use std::collections::HashSet;

const CSS_DATA: &[u8] = include_bytes!("../../resources/style.css");

#[derive(Msg)]
pub enum Msg {
    SelectCard(Option<usize>),
    Quit,
}

pub struct Model {
    relm: relm::Relm<Win>,
    streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    comm_target_cards: Vec<CommTargetCardData>,
    selected_card: Option<CommTargetCardData>,

    _comm_targets_components: Vec<Component<CommTargetCard>>,
    _comm_remote_servers_components: Vec<Component<CommRemoteServer>>,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        if let Err(err) = self.load_style() {
            println!("Error loading the CSS: {}", err);
        }
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
        Model {
            relm: relm.clone(),
            streams,
            comm_target_cards,
            _comm_targets_components: vec![],
            _comm_remote_servers_components: vec![],
            selected_card: None,
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
            for stream in &self.model.streams {
                let layers = &stream.1.first().unwrap().source.layers;
                if layers.ip.as_ref().unwrap().ip_dst != target_ip
                    || layers.tcp.as_ref().unwrap().port_dst != target_port
                {
                    continue;
                }
                let remote_server_streams = by_remote_ip
                    .entry(layers.ip.as_ref().unwrap().ip_src.clone())
                    .or_insert(vec![]);
                let tcp_stream_id = layers.tcp.as_ref().map(|t| t.stream);
                let decoded_messages = stream
                    .1
                    .iter()
                    .filter_map(|m| {
                        // search for the field which is an object and for which the object contains a field "http.request.method"
                        let http = m.source.layers.http.as_ref();
                        if http.is_some() {
                            return if let Some(message_data) =
                                http.and_then(HttpMessageData::from_json)
                            {
                                return Some(MessageData::Http(message_data));
                            } else {
                                eprintln!("failed to parse http message: {:?}", http);
                                return None;
                            };
                        }
                        let pgsql = m.source.layers.pgsql.as_ref();
                        if let Some(serde_json::Value::Array(pgsql_arr)) = pgsql {
                            return pgsql_arr
                                .iter()
                                .filter_map(|v| v.as_object().and_then(|o| o.get("pgsql.query")))
                                .next()
                                .and_then(|q| q.as_str())
                                .map(|query| {
                                    MessageData::Postgres(PostgresMessageData {
                                        query: query.to_string(),
                                    })
                                });
                        }
                        None
                    })
                    .collect();
                remote_server_streams.push((tcp_stream_id, decoded_messages));
            }
            for (remote_ip, tcp_sessions) in by_remote_ip {
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
                    hexpand: true,
                    property_width_request: 250,
                    hexpand: false,
                    #[name="comm_target_list"]
                    gtk::ListBox {
                        row_selected(_, row) =>
                            Msg::SelectCard(row.map(|r| r.get_index() as usize))
                    }
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
