use super::http_comm_entry::HttpMessageData;
use super::http_comm_remote_server::{HttpCommRemoteServer, HttpCommRemoteServerData};
use super::http_comm_target_card::{HttpCommTargetCard, HttpCommTargetCardData};
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::{Component, ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Msg)]
pub enum Msg {
    SelectCard(Option<usize>),
    Quit,
}

pub struct Model {
    relm: relm::Relm<Win>,
    streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    http_comm_target_cards: Vec<HttpCommTargetCardData>,
    selected_card: Option<HttpCommTargetCardData>,

    _comm_targets_components: Vec<Component<HttpCommTargetCard>>,
    _comm_remote_servers_components: Vec<Component<HttpCommRemoteServer>>,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        self.refresh_http_comm_targets();
        self.refresh_remote_servers();
    }

    fn model(
        relm: &relm::Relm<Self>,
        streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    ) -> Model {
        let http_comm_target_cards = streams
            .iter()
            .fold(
                HashMap::<(String, u32), HttpCommTargetCardData>::new(),
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
                            HttpCommTargetCardData {
                                ip: layers.ip.as_ref().unwrap().ip_dst.clone(),
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
            http_comm_target_cards,
            _comm_targets_components: vec![],
            _comm_remote_servers_components: vec![],
            selected_card: None,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SelectCard(maybe_idx) => {
                self.model.selected_card = maybe_idx
                    .and_then(|idx| self.model.http_comm_target_cards.get(idx as usize))
                    .cloned();
                self.refresh_remote_servers();
            }
            Msg::Quit => {}
        }
    }

    fn refresh_http_comm_targets(&mut self) {
        for child in self.widgets.http_comm_target_list.get_children() {
            self.widgets.http_comm_target_list.remove(&child);
        }
        self.model._comm_targets_components = self
            .model
            .http_comm_target_cards
            .iter()
            .map(|card| {
                self.widgets
                    .http_comm_target_list
                    .add_widget::<HttpCommTargetCard>(card.clone())
            })
            .collect();
    }

    fn refresh_remote_servers(&mut self) {
        for child in self.widgets.http_comm_remote_servers.get_children() {
            self.widgets.http_comm_remote_servers.remove(&child);
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
                        if let Some(message_data) = http.and_then(HttpMessageData::from_json) {
                            Some(message_data)
                        } else {
                            eprintln!("failed to parse http message: {:?}", http);
                            None
                        }
                    })
                    .collect();
                remote_server_streams.push((tcp_stream_id, decoded_messages));
            }
            for (remote_ip, tcp_sessions) in by_remote_ip {
                components.push(
                    self.widgets
                        .http_comm_remote_servers
                        .add_widget::<HttpCommRemoteServer>(HttpCommRemoteServerData {
                            remote_ip,
                            tcp_sessions,
                        }),
                );
            }
        }
        self.model._comm_remote_servers_components = components;
    }

    view! {
        gtk::Window {
            gtk::Box {
                hexpand: true,
                gtk::ScrolledWindow {
                    hexpand: true,
                    #[name="http_comm_target_list"]
                    gtk::ListBox {
                        // selection_mode: gtk::SelectionMode::None,
                        row_selected(_, row) =>
                            Msg::SelectCard(row.map(|r| r.get_index() as usize))
                    }
                },
                gtk::ScrolledWindow {
                    hexpand: true,
                    #[name="http_comm_remote_servers"]
                    gtk::ListBox {
                    },
                }
            }
        }
    }
}
