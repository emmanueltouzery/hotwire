use crate::icons::Icon;
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
    pub protocol_icon: Icon,
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

    fn server_ip_port_display(data: &HttpCommTargetCardData) -> String {
        format!("{}:{}", data.ip, data.port)
    }

    fn details_display(data: &HttpCommTargetCardData) -> String {
        format!(
            "{} remote hosts, {} sessions",
            data.remote_hosts.len(),
            data.incoming_session_count
        )
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Horizontal,
            margin_top: 7,
            margin_start: 7,
            margin_end: 7,
            margin_bottom: 7,
            gtk::Image {
                margin_end: 10,
                property_icon_name: Some(self.model.data.protocol_icon.name()),
                // https://github.com/gtk-rs/gtk/issues/837
                property_icon_size: 3, // gtk::IconSize::LargeToolbar,
            },
            gtk::Grid {
                #[style_class="target_server_ip_port"]
                gtk::Label {
                    label: &HttpCommTargetCard::server_ip_port_display(&self.model.data),
                    cell: {
                        left_attach: 0,
                        top_attach: 1,
                    },
                },
                gtk::Label {
                    label: &HttpCommTargetCard::details_display(&self.model.data),
                    cell: {
                        left_attach: 0,
                        top_attach: 2,
                    },
                },
            }
        }
    }
}
