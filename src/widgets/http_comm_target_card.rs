use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::HashSet;

#[derive(Msg)]
pub enum Msg {}

#[derive(Clone)]
pub struct HttpCommTargetCardData {
    pub ip: String,
    pub port: u32,
    pub remote_hosts: HashSet<String>,
    pub incoming_session_count: usize,
}

pub struct Model {
    data: HttpCommTargetCardData,
}

#[widget]
impl Widget for HttpCommTargetCard {
    fn model(relm: &relm::Relm<Self>, data: HttpCommTargetCardData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Grid {
            gtk::Label {
                label: "Server IP:"
            },
            gtk::Label {
                label: &self.model.data.ip
            },
            gtk::Label {
                label: "Server port:"
            },
            gtk::Label {
                label: &self.model.data.port.to_string()
            },
            gtk::Label {
                label: "Remote hosts count:"
            },
            gtk::Label {
                label: &self.model.data.remote_hosts.len().to_string()
            },
            gtk::Label {
                label: "Incoming session count:"
            },
            gtk::Label {
                label: &self.model.data.incoming_session_count.to_string()
            },
        }
    }
}
