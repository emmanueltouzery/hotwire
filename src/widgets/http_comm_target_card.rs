use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::HashSet;

#[derive(Msg)]
pub enum Msg {}

#[derive(Clone)]
pub struct HttpCommTargetCardInfo {
    pub ip: String,
    pub port: u32,
    pub remote_hosts: HashSet<String>,
    pub incoming_session_count: usize,
}

pub struct Model {
    card_info: HttpCommTargetCardInfo,
}

#[widget]
impl Widget for HttpCommTargetCard {
    fn model(relm: &relm::Relm<Self>, card_info: HttpCommTargetCardInfo) -> Model {
        Model { card_info }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Grid {
            gtk::Label {
                label: "Server IP:"
            },
            gtk::Label {
                label: &self.model.card_info.ip
            },
            gtk::Label {
                label: "Server port:"
            },
            gtk::Label {
                label: &self.model.card_info.port.to_string()
            },
            gtk::Label {
                label: "Remote hosts count:"
            },
            gtk::Label {
                label: &self.model.card_info.remote_hosts.len().to_string()
            },
            gtk::Label {
                label: "Incoming session count:"
            },
            gtk::Label {
                label: &self.model.card_info.incoming_session_count.to_string()
            },
        }
    }
}
