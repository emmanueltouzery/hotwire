use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::net::IpAddr;

#[derive(Msg)]
pub enum Msg {
    Update(IpAddr, u32),
}

pub struct Model {
    client_ip: IpAddr,
    stream_id: u32,
}

#[widget]
impl Widget for CommInfoHeader {
    fn model(relm: &relm::Relm<Self>, data: (IpAddr, u32)) -> Model {
        let (client_ip, stream_id) = data;
        Model {
            client_ip,
            stream_id,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::Update(client_ip, stream_id) => {
                self.model.client_ip = client_ip;
                self.model.stream_id = stream_id;
            }
        }
    }

    fn format_stream_id(stream_id: u32) -> String {
        format!("{}", stream_id)
    }

    view! {
        gtk::Box {
            #[style_class="label"]
            gtk::Label {
                label: "Client IP: ",
            },
            gtk::Label {
                label: &self.model.client_ip.to_string(),
                xalign: 0.0,
            },
            #[style_class="label"]
            gtk::Label {
                label: "TCP stream: ",
                margin_start: 10,
            },
            gtk::Label {
                label: &CommInfoHeader::format_stream_id(self.model.stream_id),
                xalign: 0.0,
            },
        }
    }
}
