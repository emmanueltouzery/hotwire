use crate::icons::Icon;
use crate::message_parser::ClientServerInfo;
use crate::tshark_communication::NetworkPort;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::BTreeSet;
use std::net::IpAddr;

#[derive(Msg)]
pub enum Msg {
    Update(CommTargetCardData),
}

#[derive(Clone, Debug)]
pub struct SummaryDetails {
    details: String, // non public on purpose, please use ::new
}

impl SummaryDetails {
    pub fn new(details: String, card_key: CommTargetCardKey) -> Option<SummaryDetails> {
        // meant to avoid for http to have ip+port repeated for ip+port display,
        // and then again for the details, which is the hostname, in case the hostname
        // was just the IP
        if !CommTargetCard::server_ip_port_display_format(card_key.ip, card_key.port)
            .starts_with(&details)
        {
            Some(SummaryDetails { details })
        } else {
            None
        }
    }
}

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub struct CommTargetCardKey {
    pub ip: IpAddr,
    pub port: NetworkPort,
    pub protocol_index: usize,
}

impl CommTargetCardKey {
    pub fn matches_server(&self, cs_info: ClientServerInfo) -> bool {
        cs_info.server_ip == self.ip && cs_info.server_port == self.port
    }
}

#[derive(Clone, Debug)]
pub struct CommTargetCardData {
    pub ip: IpAddr,
    pub port: NetworkPort,
    pub protocol_index: usize,
    pub protocol_name: &'static str,
    pub remote_hosts: BTreeSet<IpAddr>,
    pub protocol_icon: Icon,
    pub summary_details: Option<SummaryDetails>,
    incoming_session_count: usize,
}

impl CommTargetCardData {
    pub fn new(
        ip: IpAddr,
        port: NetworkPort,
        protocol_index: usize,
        remote_hosts: BTreeSet<IpAddr>,
        protocol_icon: Icon,
        protocol_name: &'static str,
        summary_details: Option<SummaryDetails>,
        incoming_session_count: usize,
    ) -> CommTargetCardData {
        CommTargetCardData {
            ip,
            port,
            protocol_index,
            protocol_name,
            remote_hosts,
            protocol_icon,
            summary_details,
            incoming_session_count,
        }
    }

    pub fn increase_incoming_session_count(&mut self) {
        self.incoming_session_count += 1;
    }

    pub fn to_key(&self) -> CommTargetCardKey {
        CommTargetCardKey {
            ip: self.ip,
            port: self.port,
            protocol_index: self.protocol_index,
        }
    }
}

#[widget]
impl Widget for CommTargetCard {
    fn model(_relm: &relm::Relm<Self>, data: CommTargetCardData) -> CommTargetCardData {
        data
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::Update(d) => {
                self.model.remote_hosts = d.remote_hosts;
                self.model.summary_details = d.summary_details;
                self.model.incoming_session_count = d.incoming_session_count;
                dbg!(&self.model);
            }
        }
    }

    fn server_ip_port_display(data: &CommTargetCardData) -> String {
        Self::server_ip_port_display_format(data.ip, data.port)
    }

    fn server_ip_port_display_format(ip: IpAddr, port: NetworkPort) -> String {
        format!("{}:{}", ip, port)
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Horizontal,
            margin_top: 7,
            margin_start: 7,
            margin_end: 7,
            margin_bottom: 7,
            spacing: 5,
            gtk::Box {
                child: {
                    expand: true,
                },
                // property_expand: true,
                orientation: gtk::Orientation::Vertical,
                gtk::Box {
                    orientation: gtk::Orientation::Horizontal,
                    spacing: 5,
                    gtk::Image {
                        property_icon_name: Some(self.model.protocol_icon.name()),
                        // https://github.com/gtk-rs/gtk/issues/837
                        property_icon_size: 2, // gtk::IconSize::SmallToolbar,
                    },
                    #[style_class="target_server_ip_port"]
                    gtk::Label {
                        label: &CommTargetCard::server_ip_port_display(self.model),
                        ellipsize: pango::EllipsizeMode::End,
                    },
                },
                gtk::Box {
                    orientation: gtk::Orientation::Horizontal,
                    spacing: 3,
                    #[style_class="card_stats"]
                    gtk::Image {
                        property_icon_name: Some(Icon::REMOTE_HOST.name()),
                        property_icon_size: 2, // gtk::IconSize::SmallToolbar,
                    },
                    #[style_class="card_stats"]
                    gtk::Label {
                        label: &self.model.remote_hosts.len().to_string(),
                    },
                    #[style_class="card_stats"]
                    gtk::Image {
                        margin_start: 3,
                        property_icon_name: Some(Icon::SESSION.name()),
                        property_icon_size: 2, // gtk::IconSize::SmallToolbar,
                    },
                    #[style_class="card_stats"]
                    gtk::Label {
                        label: &self.model.incoming_session_count.to_string(),
                    },
                    gtk::Label {
                        margin_start: 2,
                        label: self.model.summary_details.as_ref().map(|d| d.details.as_str()).unwrap_or(""),
                        ellipsize: pango::EllipsizeMode::End,
                        visible: self.model.summary_details.is_some(),
                    }
                },
            }
        }
    }
}
