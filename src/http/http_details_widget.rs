use super::http_body_widget;
use super::http_body_widget::HttpBodyWidget;
use super::http_message_parser::HttpMessageData;
use crate::icons::Icon;
use crate::message_parser::{MessageData, MessageInfo};
use crate::tshark_communication::TcpStreamId;
use crate::widgets::comm_info_header;
use crate::widgets::comm_info_header::CommInfoHeader;
use crate::widgets::win;
use crate::BgFunc;
use gtk::prelude::*;
use itertools::Itertools;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::net::IpAddr;
use std::sync::mpsc;

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, MessageInfo),
    RemoveFormatToggled,
}

pub struct Model {
    win_msg_sender: relm::StreamHandle<win::Msg>,
    bg_sender: mpsc::Sender<BgFunc>,
    stream_id: TcpStreamId,
    client_ip: IpAddr,
    data: HttpMessageData,

    format_request_response: bool,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(
        relm: &relm::Relm<Self>,
        params: (
            relm::StreamHandle<win::Msg>,
            TcpStreamId,
            IpAddr,
            HttpMessageData,
            gtk::Overlay,
            mpsc::Sender<BgFunc>,
        ),
    ) -> Model {
        let (win_msg_sender, stream_id, client_ip, data, overlay, bg_sender) = params;

        let disable_formatting_btn = gtk::ToggleButtonBuilder::new()
            .label("Disable formatting")
            .always_show_image(true)
            .image(&gtk::Image::from_icon_name(
                Some(Icon::REMOVE_FORMAT.name()),
                gtk::IconSize::Menu,
            ))
            .valign(gtk::Align::Start)
            .halign(gtk::Align::End)
            .margin_top(10)
            .margin_end(10)
            .build();
        overlay.add_overlay(&disable_formatting_btn);
        relm::connect!(
            relm,
            disable_formatting_btn,
            connect_clicked(_),
            Msg::RemoveFormatToggled
        );
        Model {
            win_msg_sender,
            bg_sender,
            data,
            stream_id,
            client_ip,
            format_request_response: true,
        }
    }

    fn update(&mut self, event: Msg) {
        // dbg!(&event);
        match event {
            Msg::DisplayDetails(
                ..,
                MessageInfo {
                    client_ip,
                    stream_id,
                    message_data: MessageData::Http(msg),
                },
            ) => {
                self.model.data = msg;
                self.streams
                    .comm_info_header
                    .emit(comm_info_header::Msg::Update(client_ip, stream_id));
                self.model.stream_id = stream_id;
                self.model.client_ip = client_ip;
                self.streams
                    .request_body
                    .emit(http_body_widget::Msg::RequestResponseChanged {
                        http_data: self.model.data.request.clone(),
                        request_first_line_if_response: None,
                    });
                self.streams
                    .response_body
                    .emit(http_body_widget::Msg::RequestResponseChanged {
                        http_data: self.model.data.response.clone(),
                        request_first_line_if_response: self
                            .model
                            .data
                            .request
                            .as_ref()
                            .map(|r| r.first_line.clone()),
                    });
            }
            Msg::RemoveFormatToggled => {
                self.model.format_request_response = !self.model.format_request_response;
                self.streams
                    .request_body
                    .emit(http_body_widget::Msg::FormatCodeChanged(
                        self.model.format_request_response,
                    ));
                self.streams
                    .response_body
                    .emit(http_body_widget::Msg::FormatCodeChanged(
                        self.model.format_request_response,
                    ));
            }
            _ => {}
        }
    }

    fn format_headers(headers: &[(String, String)]) -> String {
        headers
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .join("\n")
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            margin_top: 10,
            margin_bottom: 10,
            margin_start: 10,
            margin_end: 10,
            spacing: 10,
            #[name="comm_info_header"]
            CommInfoHeader(self.model.client_ip.clone(), self.model.stream_id) {
            },
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.request.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing request info"),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                label: &self.model.data.request.as_ref()
                                            .map(|r| &r.headers[..])
                                            .map(Self::format_headers)
                                            .as_deref()
                                            .unwrap_or(""),
                xalign: 0.0,
                selectable: true,
            },
            #[name="request_body"]
            HttpBodyWidget((self.model.win_msg_sender.clone(), self.model.bg_sender.clone())),
            gtk::Separator {},
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.response.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing response info"),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                label: &self.model.data.response.as_ref()
                                            .map(|r| &r.headers[..])
                                            .map(Self::format_headers)
                                            .as_deref()
                                            .unwrap_or(""),
                xalign: 0.0,
                selectable: true,
            },
            #[name="response_body"]
            HttpBodyWidget((self.model.win_msg_sender.clone(), self.model.bg_sender.clone())),
        }
    }
}
