use super::http_comm_target_card::Msg as HttpCommTargetCardMsg;
use super::http_comm_target_card::{HttpCommTargetCard, HttpCommTargetCardInfo};
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
    tree_store: gtk::TreeStore,
    streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    http_comm_target_cards: Vec<HttpCommTargetCardInfo>,
    selected_card: Option<HttpCommTargetCardInfo>,

    _children_components: Vec<Component<HttpCommTargetCard>>,
}

#[widget]
impl Widget for Win {
    fn init_view(&mut self) {
        let col1 = gtk::TreeViewColumnBuilder::new().title("Messages").build();
        let cell_r_txt = gtk::CellRendererText::new();
        col1.pack_start(&cell_r_txt, true);
        col1.add_attribute(&cell_r_txt, "text", 0);
        self.widgets.tree.append_column(&col1);

        // let col2 = gtk::TreeViewColumnBuilder::new()
        //     .title("source port")
        //     .build();
        // col2.pack_start(&cell_r_txt, true);
        // col2.add_attribute(&cell_r_txt, "text", 1);
        // self.widgets.tree.append_column(&col2);

        // let col3 = gtk::TreeViewColumnBuilder::new().title("dest IP").build();
        // col3.pack_start(&cell_r_txt, true);
        // col3.add_attribute(&cell_r_txt, "text", 2);
        // self.widgets.tree.append_column(&col3);

        // let col4 = gtk::TreeViewColumnBuilder::new().title("dest port").build();
        // col4.pack_start(&cell_r_txt, true);
        // col4.add_attribute(&cell_r_txt, "text", 3);
        // self.widgets.tree.append_column(&col4);

        // let col5 = gtk::TreeViewColumnBuilder::new()
        //     .title("packet count")
        //     .build();
        // col5.pack_start(&cell_r_txt, true);
        // col5.add_attribute(&cell_r_txt, "text", 4);
        // self.widgets.tree.append_column(&col5);

        self.refresh_http_comm_targets();
        self.refresh_store();
    }

    fn model(
        relm: &relm::Relm<Self>,
        streams: Vec<(Option<u32>, Vec<TSharkCommunication>)>,
    ) -> Model {
        let tree_store = gtk::TreeStore::new(&[
            String::static_type(),
            // i32::static_type(),
            // String::static_type(),
            // i32::static_type(),
            // i32::static_type(),
            // String::static_type(),
        ]);
        let http_comm_target_cards = streams
            .iter()
            .fold(
                HashMap::<(String, u32), HttpCommTargetCardInfo>::new(),
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
                            HttpCommTargetCardInfo {
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
            tree_store,
            streams,
            http_comm_target_cards,
            _children_components: vec![],
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
        let mut components = vec![];
        for card in &self.model.http_comm_target_cards {
            let component = self
                .widgets
                .http_comm_target_list
                .add_widget::<HttpCommTargetCard>(card.clone());
            components.push(component);
        }
        self.model._children_components = components;
    }

    fn refresh_store(&mut self) {
        self.model.tree_store.clear();
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
                // self.model.tree_store.set_value(
                //     &iter,
                //     0,
                //     &format!(
                //         "{}:{} -> {}:{}",
                //         layers.ip.as_ref().unwrap().ip_src,
                //         layers.tcp.as_ref().unwrap().port_src,
                //         layers.ip.as_ref().unwrap().ip_dst,
                //         layers.tcp.as_ref().unwrap().port_dst
                //     )
                //     .to_value(),
                // );
                // self.model.tree_store.set_value(
                //     &iter,
                //     1,
                //     &layers.tcp.as_ref().unwrap().port_src.to_value(),
                // );
                // self.model.tree_store.set_value(
                //     &iter,
                //     2,
                //     &layers.ip.as_ref().unwrap().ip_dst.to_value(),
                // );
                // self.model.tree_store.set_value(
                //     &iter,
                //     3,
                //     &layers.tcp.as_ref().unwrap().port_dst.to_value(),
                // );
                // self.model
                //     .tree_store
                //     .set_value(&iter, 4, &(stream.1.len() as i64).to_value());
                println!("items: {}", &stream.1.len());
                for request in &stream.1 {
                    let iter = self.model.tree_store.append(None);
                    // search for the field which is an object and for which the object contains a field "http.request.method"
                    // let child = self.model.tree_store.append(Some(&iter));
                    if let Some(serde_json::Value::Object(http_map)) =
                        request.source.layers.http.as_ref()
                    {
                        let missing = format!("{:?}", http_map);
                        let req_info = http_map.iter().find(|(_,v)| matches!(v,
                        serde_json::Value::Object(fields) if fields.contains_key("http.request.method") || fields.contains_key("http.response.code")
                    )).unwrap_or((&missing, &serde_json::json!(null))).0;
                        self.model
                            .tree_store
                            .set_value(&iter, 0, &req_info.to_value());
                    }
                }
            }
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
                    #[name="tree"]
                    gtk::TreeView {
                        model: Some(&self.model.tree_store)
                    },
                }
            }
        }
    }
}
