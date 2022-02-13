use super::http_body_widget;
use super::http_body_widget::HttpBodyWidget;
use super::http_streams_store::{HttpMessageData, HttpRequestResponseData};
use crate::icons::Icon;
use crate::tshark_communication::TcpStreamId;
use crate::widgets::comm_info_header;
use crate::widgets::comm_info_header::CommInfoHeader;
use crate::widgets::win;
use crate::BgFunc;
use gtk::prelude::*;
use itertools::Itertools;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::mpsc;

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, IpAddr, TcpStreamId, HttpMessageData),
    RemoveFormatToggled,
    CopyContentsClick,
    ToggleDisplayPassword,
}

pub struct Model {
    win_msg_sender: relm::StreamHandle<win::Msg>,
    bg_sender: mpsc::Sender<BgFunc>,
    stream_id: TcpStreamId,
    client_ip: IpAddr,
    data: HttpMessageData,
    basic_auth_username: Option<String>,
    basic_auth_password: Option<String>,

    options_popover: gtk::Popover,
    format_contents_btn: gtk::CheckButton,

    format_request_response: bool,
}

#[widget]
impl Widget for HttpCommEntry {
    fn init_options_overlay(
        relm: &relm::Relm<Self>,
        overlay: &gtk::Overlay,
        format_contents_btn: &gtk::CheckButton,
    ) -> gtk::Popover {
        let popover_box = gtk::builders::BoxBuilder::new()
            .orientation(gtk::Orientation::Vertical)
            .margin(10)
            .spacing(10)
            .build();

        relm::connect!(
            relm,
            format_contents_btn,
            connect_toggled(_),
            Msg::RemoveFormatToggled
        );
        popover_box.add(format_contents_btn);
        let copy_to_clipboard_lbl = gtk::builders::ButtonBuilder::new()
            .label("Copy to clipboard")
            .build();
        popover_box.add(&copy_to_clipboard_lbl);
        popover_box.show_all();

        relm::connect!(
            relm,
            copy_to_clipboard_lbl,
            connect_clicked(_),
            Msg::CopyContentsClick
        );

        let options_popover = gtk::builders::PopoverBuilder::new()
            .child(&popover_box)
            .build();

        let options_btn = gtk::builders::MenuButtonBuilder::new()
            .always_show_image(true)
            .image(&gtk::Image::from_icon_name(
                Some(Icon::COG.name()),
                gtk::IconSize::Menu,
            ))
            .valign(gtk::Align::Start)
            .halign(gtk::Align::End)
            .margin_top(10)
            .margin_end(10)
            .build();
        options_btn.set_popover(Some(&options_popover));
        overlay.add_overlay(&options_btn);

        options_popover
    }

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
        let format_contents_btn = gtk::builders::CheckButtonBuilder::new()
            .active(true)
            .label("Format contents")
            .build();
        let options_popover = Self::init_options_overlay(relm, &overlay, &format_contents_btn);

        Model {
            win_msg_sender,
            bg_sender,
            data,
            stream_id,
            client_ip,
            format_contents_btn,
            options_popover,
            format_request_response: true,
            basic_auth_username: None,
            basic_auth_password: None,
        }
    }

    fn update_basic_auth_data(&mut self, message_data: &HttpMessageData) {
        let empty = vec![];
        let mut req_headers = message_data
            .request
            .as_ref()
            .map(|r| &r.headers)
            .unwrap_or(&empty)
            .iter();
        let auth_prefix = "Basic ";
        let auth_header = req_headers
            .find(|(k, v)| k == "Authorization" && v.starts_with(auth_prefix))
            .map(|(_k, v)| &v[(auth_prefix.len())..])
            .and_then(|s| base64::decode(s).ok())
            .and_then(|s| String::from_utf8(s).ok())
            .and_then(|s| {
                s.split_once(':')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            });
        if let Some((k, v)) = auth_header {
            self.model.basic_auth_username = Some(k);
            self.model.basic_auth_password = Some(v);
        } else {
            self.model.basic_auth_username = None;
            self.model.basic_auth_password = None;
        }
        self.refresh_display_password();
        self.widgets
            .basic_auth_info
            .set_visible(self.model.basic_auth_username.is_some());
    }

    fn refresh_display_password(&mut self) {
        let display_password = self.widgets.display_password_toggle_btn.is_active();
        self.widgets.label_pass.set_label(if display_password {
            self.model.basic_auth_password.as_deref().unwrap_or("")
        } else {
            "●●●●●"
        });
    }

    fn update(&mut self, event: Msg) {
        // dbg!(&event);
        match event {
            Msg::DisplayDetails(.., client_ip, stream_id, message_data) => {
                self.update_basic_auth_data(&message_data);
                self.model.data = message_data;
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
                self.model.format_request_response = self.model.format_contents_btn.is_active();
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
            Msg::CopyContentsClick => {
                if let Some(clip) =
                    gtk::Clipboard::default(&self.widgets.comm_info_header.display())
                {
                    let format_reqresp = |r: &HttpRequestResponseData| {
                        format!(
                            "{}\n{}\n\n{}",
                            r.first_line,
                            r.headers
                                .iter()
                                .map(|(k, v)| format!("{}: {}", k, v))
                                .join("\n"),
                            r.body_as_str().unwrap_or(Cow::Borrowed(""))
                        )
                    };
                    let clip_contents = format!(
                        "{}\n-------\n{}",
                        self.model
                            .data
                            .request
                            .as_ref()
                            .map(format_reqresp)
                            .as_deref()
                            .unwrap_or(""),
                        self.model
                            .data
                            .response
                            .as_ref()
                            .map(format_reqresp)
                            .as_deref()
                            .unwrap_or("")
                    );
                    clip.set_text(&clip_contents);
                    self.model.win_msg_sender.emit(win::Msg::InfoBarShow(
                        Some("Copied to the clipboard".to_string()),
                        win::InfobarOptions::TimeLimitedWithCloseButton,
                    ))
                }
                self.model.options_popover.popdown();
            }
            Msg::ToggleDisplayPassword => {
                self.refresh_display_password();
            }
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
                label: self.model.data.request.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing request info"),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                label: self.model.data.request.as_ref()
                                            .map(|r| &r.headers[..])
                                            .map(Self::format_headers)
                                            .as_deref()
                                            .unwrap_or(""),
                xalign: 0.0,
                selectable: true,
            },
            #[name="basic_auth_info"]
            gtk::Box {
                spacing: 5,
                gtk::Image {
                    icon_name: Some(Icon::LOCK.name()),
                    icon_size: gtk::IconSize::SmallToolbar,
                },
                #[style_class="label"]
                gtk::Label {
                    label: "HTTP Basic Authentication",
                    halign: gtk::Align::End,
                },
                gtk::Label {
                    label: self.model.basic_auth_username.as_deref().unwrap_or(""),
                    selectable: true,
                },
                #[style_class="label"]
                gtk::Label {
                    label: "/",
                    halign: gtk::Align::End,
                },
                #[name="label_pass"]
                gtk::Label {
                    label: "●●●●●",
                    selectable: true,
                },
                #[name="display_password_toggle_btn"]
                gtk::ToggleButton {
                    always_show_image: true,
                    image: Some(&gtk::Image::from_icon_name(
                        Some(Icon::EYE.name()), gtk::IconSize::Menu)),
                    toggled => Msg::ToggleDisplayPassword,
                    hexpand: true,
                    halign: gtk::Align::End,
                },
            },
            #[name="request_body"]
            HttpBodyWidget((self.model.win_msg_sender.clone(), self.model.bg_sender.clone())),
            gtk::Separator {},
            #[style_class="http_first_line"]
            gtk::Label {
                label: self.model.data.response.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing response info"),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                label: self.model.data.response.as_ref()
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
