use super::http_comm_entry::{HttpCommEntry, HttpCommEntryData};
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
    _comm_entries_components: Vec<Component<HttpCommEntry>>,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        self.refresh_http_comm_targets();
        self.refresh_store();
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
            _comm_entries_components: vec![],
            selected_card: None,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SelectCard(maybe_idx) => {
                self.model.selected_card = maybe_idx
                    .and_then(|idx| self.model.http_comm_target_cards.get(idx as usize))
                    .cloned();
                self.refresh_store();
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

    fn refresh_store(&mut self) {
        for child in self.widgets.http_comm_entries.get_children() {
            self.widgets.http_comm_entries.remove(&child);
        }
        let mut comm_entries_group_start_indexes = HashMap::new();
        let mut components = vec![];
        if let Some(card) = &self.model.selected_card {
            let target_ip = card.ip.clone();
            let target_port = card.port;
            for stream in &self.model.streams {
                let layers = &stream.1.first().unwrap().source.layers;
                if layers.ip.as_ref().unwrap().ip_dst != target_ip
                    || layers.tcp.as_ref().unwrap().port_dst != target_port
                {
                    continue;
                }
                let tcp_stream_id = layers.tcp.as_ref().map(|t| t.stream);
                comm_entries_group_start_indexes
                    // TODO put client details rather
                    .insert(components.len(), format!("tcp stream {:?}", tcp_stream_id));
                for request in stream.1.iter() {
                    // search for the field which is an object and for which the object contains a field "http.request.method"
                    let http = request.source.layers.http.as_ref();
                    let req_verb = http.and_then(Self::get_http_request_verb);
                    let display_verb = req_verb
                        .map(|t| t.0.to_string())
                        .unwrap_or_else(|| "Parse error".to_string());
                    components.push(self.widgets.http_comm_entries.add_widget::<HttpCommEntry>(
                        HttpCommEntryData {
                            request_verb: display_verb,
                        },
                    ));
                }
            }
        }
        self.model._comm_entries_components = components;

        self.widgets
            .http_comm_entries
            .set_header_func(Some(Box::new(move |row, _h| {
                if let Some(group_name) =
                    comm_entries_group_start_indexes.get(&(row.get_index() as usize))
                {
                    let vbox = gtk::BoxBuilder::new()
                        .orientation(gtk::Orientation::Vertical)
                        .build();
                    vbox.add(&gtk::SeparatorBuilder::new().build());
                    let label = gtk::LabelBuilder::new()
                        .label(&format!("<b>{}</b>", group_name))
                        .use_markup(true)
                        .xalign(0.0)
                        .build();
                    label.get_style_context().add_class("project_item_header");
                    vbox.add(&label);
                    vbox.show_all();
                    row.set_header(Some(&vbox));
                } else {
                    row.set_header::<gtk::ListBoxRow>(None)
                }
            })));
    }

    fn get_http_request_verb(
        serde_json: &serde_json::Value,
    ) -> Option<(&String, &serde_json::Value)> {
        if let serde_json::Value::Object(http_map) = serde_json {
            http_map.iter().find(|(_,v)| matches!(v,
                        serde_json::Value::Object(fields) if fields.contains_key("http.request.method") || fields.contains_key("http.response.code")
                    ))
        } else {
            None
        }
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
                    #[name="http_comm_entries"]
                    gtk::ListBox {
                    },
                }
            }
        }
    }
}
