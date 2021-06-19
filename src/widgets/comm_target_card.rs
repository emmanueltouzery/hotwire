use crate::icons::Icon;
use crate::tshark_communication::{NetworkPort, TcpStreamId};
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
    pub details: String, // non public on purpose, please use ::new
}

impl SummaryDetails {
    pub fn new(details: &str, ip: IpAddr, port: NetworkPort) -> Option<SummaryDetails> {
        // meant to avoid for http to have ip+port repeated for ip+port display,
        // and then again for the details, which is the hostname, in case the hostname
        // was just the IP
        if !CommTargetCard::server_ip_port_display_format(ip, port).starts_with(details) {
            Some(SummaryDetails {
                details: details.to_string(),
            })
        } else {
            None
        }
    }
}

#[derive(PartialEq, Eq, Hash)]
pub struct CommTargetCardKey {
    pub ip: IpAddr,
    pub port: NetworkPort,
    pub protocol_index: usize,
}

#[derive(Clone, Debug)]
pub struct CommTargetCardData {
    pub ip: IpAddr,
    pub port: NetworkPort,
    pub protocol_index: usize,
    pub remote_hosts: BTreeSet<String>, // TODO change String to IpAddr?
    pub protocol_icon: Icon,
    pub summary_details: Option<SummaryDetails>,
    pub remotes_summary: String,
    incoming_session_count: usize,
}

impl CommTargetCardData {
    pub fn new(
        ip: IpAddr,
        port: NetworkPort,
        protocol_index: usize,
        remote_hosts: BTreeSet<String>,
        protocol_icon: Icon,
        summary_details: Option<SummaryDetails>,
        incoming_session_count: usize,
    ) -> CommTargetCardData {
        CommTargetCardData {
            remotes_summary: Self::format_remotes_summary(&remote_hosts, incoming_session_count),
            ip,
            port,
            protocol_index,
            remote_hosts,
            protocol_icon,
            summary_details,
            incoming_session_count,
        }
    }

    pub fn increase_incoming_session_count(&mut self) {
        self.incoming_session_count += 1;
        self.remotes_summary =
            Self::format_remotes_summary(&self.remote_hosts, self.incoming_session_count);
    }

    fn format_remotes_summary(
        remote_hosts: &BTreeSet<String>,
        incoming_session_count: usize,
    ) -> String {
        format!(
            "{} remote hosts, {} sessions",
            remote_hosts.len(),
            incoming_session_count
        )
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
                self.model.remotes_summary = d.remotes_summary;
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
            gtk::Image {
                margin_end: 10,
                property_icon_name: Some(self.model.protocol_icon.name()),
                // https://github.com/gtk-rs/gtk/issues/837
                property_icon_size: 3, // gtk::IconSize::LargeToolbar,
            },
            gtk::Grid {
                #[style_class="target_server_ip_port"]
                gtk::Label {
                    label: &CommTargetCard::server_ip_port_display(self.model),
                    cell: {
                        left_attach: 0,
                        top_attach: 1,
                    },
                },
                gtk::Label {
                    label: &self.model.remotes_summary,
                    cell: {
                        left_attach: 0,
                        top_attach: 2,
                    },
                },
                gtk::Label {
                    cell: {
                        left_attach: 0,
                        top_attach: 0,
                    },
                    label: self.model.summary_details.as_ref().map(|d| d.details.as_str()).unwrap_or(""),
                    visible: self.model.summary_details.is_some(),
                }
            }
        }
    }
}
